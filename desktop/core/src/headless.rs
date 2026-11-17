//! `strazce --check`: the run the OS scheduler executes, with toast notifications.

use crate::api::{ApiClient, Transport};
use crate::db::Db;
use crate::error::CoreError;
use crate::run::{run_check, RunStatus, RunSummary, Trigger};
use crate::secrets::SecretStore;
use chrono::{DateTime, Utc};
use std::path::Path;
use std::sync::Arc;

#[cfg(windows)]
const APP_ID: &str = "cz.strazce-katastru.app";
const KEY_MSG: &str = "Chybí nebo je neplatný API klíč – otevřete aplikaci.";
const ERROR_TITLE: &str = "Katastr: chyba kontroly";

pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str);
}

/// Windows toast via `notify-rust`. Failures are ignored (there is no one to tell).
#[derive(Debug, Default, Clone, Copy)]
pub struct ToastNotifier;

impl Notifier for ToastNotifier {
    fn notify(&self, title: &str, body: &str) {
        let mut n = notify_rust::Notification::new();
        n.summary(title).body(body);
        #[cfg(windows)]
        n.app_id(APP_ID);
        let _ = n.show();
    }
}

#[cfg(any(test, feature = "test-util"))]
pub use rec::RecordingNotifier;

#[cfg(any(test, feature = "test-util"))]
mod rec {
    use super::*;
    use std::sync::Mutex;

    /// Records `(title, body)` pairs.
    #[derive(Debug, Default)]
    pub struct RecordingNotifier(Mutex<Vec<(String, String)>>);

    impl RecordingNotifier {
        pub fn calls(&self) -> Vec<(String, String)> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Notifier for RecordingNotifier {
        fn notify(&self, title: &str, body: &str) {
            self.0.lock().unwrap().push((title.into(), body.into()));
        }
    }
}

/// `"1 plomba"`, `"2 plomby"`, `"5 plomb"`.
fn plural(n: u32, one: &str, few: &str, many: &str) -> String {
    let w = match n {
        1 => one,
        2..=4 => few,
        _ => many,
    };
    format!("{n} {w}")
}

/// Notification `(title, body)` for a finished run.
pub fn summary_text(s: &RunSummary) -> (String, String) {
    if s.status == RunStatus::Failed {
        let body = s
            .error
            .clone()
            .unwrap_or_else(|| "Kontrola selhala.".to_string());
        return (ERROR_TITLE.to_string(), body);
    }
    let mut parts = Vec::new();
    if s.plomby_nove > 0 {
        parts.push(plural(s.plomby_nove, "plomba", "plomby", "plomb"));
    }
    if s.changed > 0 {
        parts.push(plural(s.changed, "změna", "změny", "změn"));
    }
    let title = if !parts.is_empty() {
        format!("Katastr: {}", parts.join(", "))
    } else if s.errors > 0 {
        ERROR_TITLE.to_string()
    } else {
        "Katastr: beze změn".to_string()
    };

    let mut body = format!(
        "Zkontrolováno: {}",
        plural(s.checked, "položka", "položky", "položek")
    );
    if s.errors > 0 {
        body.push_str(&format!(", chyb: {}", s.errors));
    }
    if s.deferred > 0 {
        body.push_str(&format!(
            "; zbývá {} na další dny",
            plural(s.deferred, "položka", "položky", "položek")
        ));
    }
    body.push('.');
    (title, body)
}

/// Runs a scheduled check and returns the process exit code: 0 ok/partial, 1 failure,
/// 2 missing or invalid API key, 3 another run is in progress.
pub fn run_headless(
    secrets: &dyn SecretStore,
    transport: Arc<dyn Transport>,
    notifier: &dyn Notifier,
    db_path: &Path,
    lock_path: &Path,
    now: DateTime<Utc>,
) -> i32 {
    let no_key = || {
        notifier.notify("Katastr", KEY_MSG);
        2
    };
    let fail = |body: &str| {
        notifier.notify(ERROR_TITLE, body);
        1
    };
    let key = match secrets.get() {
        Ok(Some(k)) if !k.trim().is_empty() => k,
        Ok(_) => return no_key(),
        Err(_) => return fail("Klíč se nepodařilo načíst z úložiště přihlašovacích údajů."),
    };
    if let Some(dir) = db_path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return fail("Datovou složku aplikace nelze vytvořit.");
        }
    }
    let mut db = match Db::open(db_path) {
        Ok(db) => db,
        Err(_) => return fail("Databázi nelze otevřít."),
    };
    let client = ApiClient::new(transport, key);
    match run_check(&client, &mut db, lock_path, Trigger::Scheduled, now) {
        Err(CoreError::InvalidApiKey) => no_key(),
        Err(CoreError::AlreadyRunning) => 3,
        Err(_) => fail("Kontrolu se nepodařilo dokončit."),
        Ok(s) => {
            let silent = s.checked == 0 && s.errors == 0 && s.deferred == 0;
            if !silent || s.status == RunStatus::Failed {
                let (title, body) = summary_text(&s);
                notifier.notify(&title, &body);
            }
            i32::from(s.status == RunStatus::Failed)
        }
    }
}
