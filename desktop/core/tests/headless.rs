use chrono::{DateTime, Duration, TimeZone, Utc};
use std::path::PathBuf;
use std::sync::Arc;
use strazce_core::api::MockTransport;
use strazce_core::db::{Db, NewItem};
use strazce_core::headless::{run_headless, summary_text, RecordingNotifier};
use strazce_core::lock::RunLock;
use strazce_core::model::*;
use strazce_core::run::{RunStatus, RunSummary};
use strazce_core::secrets::{MemoryStore, SecretStore};
use tempfile::TempDir;

const KEY_MSG: &str = "Chybí nebo je neplatný API klíč – otevřete aplikaci.";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 17, 8, 0, 0).unwrap()
}

struct Env {
    dir: TempDir,
    mock: Arc<MockTransport>,
    notifier: RecordingNotifier,
}

impl Env {
    fn new() -> Env {
        Env {
            dir: tempfile::tempdir().unwrap(),
            mock: Arc::new(MockTransport::new()),
            notifier: RecordingNotifier::default(),
        }
    }
    fn db_path(&self) -> PathBuf {
        self.dir.path().join("data").join("strazce.db")
    }
    fn lock_path(&self) -> PathBuf {
        self.dir.path().join("data").join("strazce.lock")
    }
    fn run(&self, secrets: &MemoryStore) -> i32 {
        run_headless(
            secrets,
            self.mock.clone(),
            &self.notifier,
            &self.db_path(),
            &self.lock_path(),
            now(),
        )
    }
    fn db(&self) -> Db {
        std::fs::create_dir_all(self.db_path().parent().unwrap()).unwrap();
        Db::open(&self.db_path()).unwrap()
    }
    fn stav(&self) {
        self.mock.push(
            "/AplikacniSluzby/StavUctu",
            200,
            r#"{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":10,"limitVolani":500,"expiraceApiKey":null}"#,
        );
    }
}

fn keyed() -> MemoryStore {
    let s = MemoryStore::default();
    s.set("fake-key").unwrap();
    s
}

fn summary() -> RunSummary {
    RunSummary {
        run_id: 1,
        status: RunStatus::Ok,
        checked: 0,
        changed: 0,
        plomby_nove: 0,
        errors: 0,
        deferred: 0,
        api_calls: 0,
        api_used_today: 0,
        api_limit: 0,
        aktualnost_dat: None,
        error: None,
    }
}

fn add_parcela(db: &Db) -> i64 {
    add_parcela_n(db, 182)
}

fn add_parcela_n(db: &Db, k: u32) -> i64 {
    db.insert_item(&NewItem {
        popis: "t".into(),
        enabled: true,
        ident: ItemIdent::Parcela {
            ku: 618853,
            parcela: ParcelaRef {
                druh: DruhCislovani::Stavebni,
                kmenove: k,
                poddeleni: None,
            },
        },
    })
    .unwrap()
}

#[test]
fn summary_text_variants() {
    let mut s = summary();
    s.checked = 3;
    assert_eq!(summary_text(&s).0, "Katastr: beze změn");

    s.plomby_nove = 1;
    s.changed = 2;
    assert_eq!(summary_text(&s).0, "Katastr: 1 plomba, 2 změny");
    s.plomby_nove = 5;
    s.changed = 5;
    assert_eq!(summary_text(&s).0, "Katastr: 5 plomb, 5 změn");
    s.plomby_nove = 2;
    s.changed = 0;
    assert_eq!(summary_text(&s).0, "Katastr: 2 plomby");
    s.plomby_nove = 0;
    s.changed = 1;
    assert_eq!(summary_text(&s).0, "Katastr: 1 změna");

    s.deferred = 12;
    let (_, body) = summary_text(&s);
    assert!(body.contains("zbývá 12 položek na další dny"), "{body}");
    s.deferred = 1;
    assert!(summary_text(&s).1.contains("zbývá 1 položka na další dny"));
    s.deferred = 3;
    assert!(summary_text(&s).1.contains("zbývá 3 položky na další dny"));

    let mut f = summary();
    f.status = RunStatus::Failed;
    assert_eq!(summary_text(&f).0, "Katastr: chyba kontroly");
}

#[test]
fn headless_no_key_exit_2_and_notifies() {
    let e = Env::new();
    let code = e.run(&MemoryStore::default());
    assert_eq!(code, 2);
    assert_eq!(e.notifier.calls(), vec![("Katastr".to_string(), KEY_MSG.to_string())]);
    assert!(!e.db_path().exists(), "DB must not be touched without a key");
}

#[test]
fn headless_invalid_key_exit_2() {
    for status in [401u16, 403] {
        let e = Env::new();
        let db = e.db();
        let id = add_parcela(&db);
        db.set_item_success(id, r#"{"id":101}"#, now() - Duration::days(30))
            .unwrap();
        let snap = |db: &Db| {
            db.list_items()
                .unwrap()
                .into_iter()
                .map(|r| (r.item.id, r.last_data, r.last_success_at, r.resolve_error))
                .collect::<Vec<_>>()
        };
        let before = snap(&db);
        e.mock.push("/AplikacniSluzby/StavUctu", status, "{}");
        assert_eq!(e.run(&keyed()), 2, "{status}");
        let calls = e.notifier.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, KEY_MSG);
        // Only StavUctu was requested and the item is untouched.
        let urls = e.mock.requested_urls();
        assert_eq!(urls.len(), 1, "{urls:?}");
        assert!(urls[0].contains("/AplikacniSluzby/StavUctu"));
        assert_eq!(snap(&db), before);
    }
}

#[test]
fn headless_already_running_exit_3_silent() {
    let e = Env::new();
    e.db();
    let _held = RunLock::acquire(&e.lock_path()).unwrap();
    assert_eq!(e.run(&keyed()), 3);
    assert!(e.notifier.calls().is_empty());
}

#[test]
fn headless_other_error_exit_1_with_notification() {
    let e = Env::new();
    add_parcela(&e.db());
    e.mock.push("/AplikacniSluzby/StavUctu", 400, "bad");
    assert_eq!(e.run(&keyed()), 1);
    let calls = e.notifier.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "Katastr: chyba kontroly");
    assert!(calls[0].1.contains("HTTP chyba 400"), "{}", calls[0].1);
    assert!(!calls[0].1.contains("fake-key"));
}

#[test]
fn headless_deferred_only_notifies_exit_0() {
    let e = Env::new();
    add_parcela(&e.db());
    // budget = 500 - 495 - 1 - 5 < 0: nothing fits, the item is deferred.
    e.mock.push(
        "/AplikacniSluzby/StavUctu",
        200,
        r#"{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":495,"limitVolani":500,"expiraceApiKey":null}"#,
    );
    assert_eq!(e.run(&keyed()), 0);
    let calls = e.notifier.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.contains("zbývá 1 položka na další dny"), "{}", calls[0].1);
}

#[test]
fn headless_partial_with_errors_notifies_exit_0() {
    let e = Env::new();
    {
        let db = e.db();
        add_parcela_n(&db, 1);
        add_parcela_n(&db, 2);
    }
    // budget 5 minus the StavUctu call = 4: the first item errors after one call, the second no longer fits.
    e.mock.push(
        "/AplikacniSluzby/StavUctu",
        200,
        r#"{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":489,"limitVolani":500,"expiraceApiKey":null}"#,
    );
    e.mock.push("/Parcely/Vyhledani", 200, r#"{"data":[]}"#);
    assert_eq!(e.run(&keyed()), 0);
    let calls = e.notifier.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].1.contains("chyb: 1"), "{}", calls[0].1);
    assert!(calls[0].1.contains("zbývá 1 položka"), "{}", calls[0].1);
}

#[test]
fn headless_nothing_due_is_silent() {
    let e = Env::new();
    e.db(); // empty DB, nothing due, no API call expected
    assert_eq!(e.run(&keyed()), 0);
    assert!(e.notifier.calls().is_empty());
}

#[test]
fn headless_changes_notify_once() {
    let e = Env::new();
    {
        let db = e.db();
        let id = add_parcela(&db);
        db.set_item_success(
            id,
            r#"{"id":101,"vymera":400,"rizeniPlomby":[]}"#,
            now() - Duration::days(30),
        )
        .unwrap();
    }
    e.stav();
    e.mock.push("/Parcely/Vyhledani", 200, r#"{"data":[{"id":101}]}"#);
    e.mock.push(
        "/Parcely/",
        200,
        r#"{"data":{"id":101,"vymera":500,"rizeniPlomby":[]},"aktualnostDatK":"2026-11-16T23:00:00"}"#,
    );
    assert_eq!(e.run(&keyed()), 0);
    let calls = e.notifier.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].0, "Katastr: 1 změna");
}
