//! Command bodies: plain functions over [`AppState`], so tests can run them with mocks.
//! The `#[tauri::command]` wrappers live in `ipc.rs`.
//!
//! Serde naming: every struct defined here crosses the IPC boundary as `camelCase`.
//! Core types that already serialize on their own (`ItemIdent`, `FieldChange`, `RizeniDef`,
//! `KuOption`) are passed as-is; `ItemIdent` therefore keeps its snake_case fields and
//! PascalCase enum tags. Run status strings are lowercase (`ok`, `partial`, `failed`,
//! `running`); `ItemStatus` uses its default PascalCase representation (`Changed`, ...).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use strazce_core::api::{ApiClient, RizeniDef, StavUctu, Transport};
use strazce_core::db::{Db, ItemRow, KuOption, NewItem, RunRow};
use strazce_core::diff::FieldChange;
use strazce_core::error::CoreError;
use strazce_core::model::{describe, item_key, validate, Item, ItemIdent};
use strazce_core::resolve::{ensure_ciselniky, resolve};
use strazce_core::run::{self, Changes, ItemStatus, Trigger, INTERVAL_SETTING};
use strazce_core::schedule::{validate_time, Scheduler};
use strazce_core::secrets::SecretStore;

/// Calls kept in reserve below the daily limit (same as the core's run budget).
const SAFETY_MARGIN: u32 = 5;
const DEFAULT_INTERVAL_DAYS: u32 = 14;
const ALLOWED_INTERVALS: [u32; 3] = [7, 14, 30];
const DEFAULT_TIME: &str = "08:00";
const SCHEDULE_ENABLED_KEY: &str = "schedule_enabled";
const SCHEDULE_TIME_KEY: &str = "schedule_time";
const KU_SEARCH_LIMIT: u32 = 20;
const RUNS_LIMIT: u32 = 30;
const MSG_NO_KEY: &str = "Chybí API klíč.";
const MSG_RUNNING: &str = "Kontrola už běží.";

pub struct AppState {
    pub db: Mutex<Db>,
    pub secrets: Arc<dyn SecretStore>,
    pub transport: Arc<dyn Transport>,
    pub scheduler: Arc<dyn Scheduler>,
    pub lock_path: PathBuf,
    /// Executable registered with the scheduler (`std::env::current_exe()` in production).
    pub exe_path: PathBuf,
}

// ---- types crossing the IPC boundary ----

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiStatus {
    pub has_key: bool,
    pub limit: Option<u32>,
    pub used: Option<u32>,
    pub expires: Option<String>,
    /// Why the account state could not be read (key present but the call failed).
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemView {
    pub id: i64,
    pub popis: String,
    pub enabled: bool,
    /// `parcela` | `stavba` | `jednotka`
    pub kind: String,
    pub describe: String,
    pub ident: ItemIdent,
    pub resolve_error: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemInput {
    pub id: Option<i64>,
    pub popis: String,
    pub enabled: bool,
    pub ident: ItemIdent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CastObceOption {
    pub kod: u32,
    pub nazev: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryView {
    pub run_id: i64,
    /// `ok` | `partial` | `failed`
    pub status: String,
    pub checked: u32,
    pub changed: u32,
    pub plomby_nove: u32,
    pub errors: u32,
    pub deferred: u32,
    pub api_calls: u32,
    pub api_used_today: u32,
    pub api_limit: u32,
    pub aktualnost_dat: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlombaView {
    pub rizeni: RizeniDef,
    pub datum_prijeti: Option<String>,
    pub stav: Option<String>,
    pub operace: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesView {
    pub fields: Vec<FieldChange>,
    pub plomby_nove: Vec<PlombaView>,
    pub plomby_zanikle: Vec<RizeniDef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingView {
    pub result_id: i64,
    pub run_id: i64,
    pub item_id: i64,
    pub popis: String,
    pub describe: String,
    pub status: ItemStatus,
    pub changes: Option<ChangesView>,
    pub run_finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub id: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// `running` | `ok` | `partial` | `failed`
    pub status: String,
    /// `manual` | `scheduled`
    pub trigger: String,
    pub checked: u32,
    pub changed: u32,
    pub errors: u32,
    pub deferred: u32,
    pub api_calls: u32,
    pub api_used_today: Option<u32>,
    pub api_limit: Option<u32>,
    pub aktualnost_dat: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultsView {
    pub findings: Vec<FindingView>,
    pub last_run: Option<RunView>,
    pub runs: Vec<RunView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DueOverview {
    pub due_now: u32,
    /// Estimated number of due items that will not fit into today's remaining calls.
    pub waiting: u32,
    /// `limit - used - 5`, or `None` without a key / when the account call fails.
    pub remaining_calls: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub interval_days: u32,
    pub schedule_enabled: bool,
    pub schedule_time: String,
}

// ---- helpers ----

/// User-facing text for a core error. Never includes the API key; API response bodies are
/// left out.
fn msg(e: CoreError) -> String {
    match e {
        CoreError::AlreadyRunning => MSG_RUNNING.to_string(),
        CoreError::NoApiKey => MSG_NO_KEY.to_string(),
        CoreError::Http { status, .. } => format!("Chyba API (HTTP {status})."),
        other => other.to_string(),
    }
}

fn lock_db(st: &AppState) -> MutexGuard<'_, Db> {
    st.db.lock().unwrap_or_else(|e| e.into_inner())
}

fn stored_key(st: &AppState) -> Result<Option<String>, String> {
    Ok(st.secrets.get().map_err(msg)?.filter(|k| !k.trim().is_empty()))
}

/// A client for the stored key; `Err("Chybí API klíč.")` without one.
fn client(st: &AppState) -> Result<ApiClient, String> {
    let key = stored_key(st)?.ok_or_else(|| MSG_NO_KEY.to_string())?;
    Ok(ApiClient::new(st.transport.clone(), key))
}

fn kind_of(ident: &ItemIdent) -> &'static str {
    match ident {
        ItemIdent::Parcela { .. } => "parcela",
        ItemIdent::Stavba { .. } => "stavba",
        ItemIdent::Jednotka { .. } => "jednotka",
    }
}

fn item_view(r: &ItemRow) -> ItemView {
    ItemView {
        id: r.item.id,
        popis: r.item.popis.clone(),
        enabled: r.item.enabled,
        kind: kind_of(&r.item.ident).to_string(),
        describe: describe(&r.item.ident),
        ident: r.item.ident.clone(),
        resolve_error: r.resolve_error.clone(),
        last_success_at: r.last_success_at,
    }
}

fn find_item(db: &Db, id: i64) -> Result<ItemRow, String> {
    db.list_items()
        .map_err(msg)?
        .into_iter()
        .find(|r| r.item.id == id)
        .ok_or_else(|| "Položka neexistuje.".to_string())
}

fn status_from_db(s: &str) -> ItemStatus {
    match s {
        "first" => ItemStatus::First,
        "unchanged" => ItemStatus::Unchanged,
        "changed" => ItemStatus::Changed,
        "deferred" => ItemStatus::Deferred,
        _ => ItemStatus::Error,
    }
}

fn changes_view(c: Changes) -> ChangesView {
    ChangesView {
        fields: c.fields,
        plomby_nove: c
            .plomby_nove
            .into_iter()
            .map(|p| PlombaView {
                rizeni: p.rizeni,
                datum_prijeti: p.datum_prijeti,
                stav: p.stav,
                operace: p.operace,
            })
            .collect(),
        plomby_zanikle: c.plomby_zanikle,
    }
}

fn run_view(r: RunRow) -> RunView {
    RunView {
        id: r.id,
        started_at: r.started_at,
        finished_at: r.finished_at,
        status: r.status,
        trigger: r.trigger,
        checked: r.checked,
        changed: r.changed,
        errors: r.errors,
        deferred: r.deferred,
        api_calls: r.api_calls,
        api_used_today: r.api_used_today,
        api_limit: r.api_limit,
        aktualnost_dat: r.aktualnost_dat,
        error: r.error,
    }
}

fn read_interval(db: &Db) -> Result<u32, String> {
    Ok(db
        .get_setting(INTERVAL_SETTING)
        .map_err(msg)?
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| ALLOWED_INTERVALS.contains(v))
        .unwrap_or(DEFAULT_INTERVAL_DAYS))
}

fn remaining_from(s: &StavUctu) -> u32 {
    s.limit_volani.saturating_sub(s.provedeno_volani).saturating_sub(SAFETY_MARGIN)
}

// ---- API key and account ----

/// Costs one API call (`StavUctu`) when a key is stored. An account-call failure is
/// reported in `error`, not as a command error, so the UI still knows a key exists.
pub fn api_status(st: &AppState) -> Result<ApiStatus, String> {
    let mut out = ApiStatus { has_key: false, limit: None, used: None, expires: None, error: None };
    let Ok(c) = client(st) else {
        return Ok(out);
    };
    out.has_key = true;
    match c.stav_uctu() {
        Ok(s) => {
            out.limit = Some(s.limit_volani);
            out.used = Some(s.provedeno_volani);
            out.expires = s.expirace_api_key;
        }
        Err(e) => out.error = Some(msg(e)),
    }
    Ok(out)
}

/// Validates the key with `StavUctu`, stores it, then refreshes the číselníky.
/// A číselník failure does not fail the command (`search_ku` retries lazily).
pub fn set_api_key(st: &AppState, key: String) -> Result<ApiStatus, String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("Zadejte API klíč.".into());
    }
    let c = ApiClient::new(st.transport.clone(), key.clone());
    let s = c.stav_uctu().map_err(|e| match e {
        CoreError::InvalidApiKey => "API klíč je neplatný.".to_string(),
        other => msg(other),
    })?;
    st.secrets.set(&key).map_err(msg)?;
    let _ = ensure_ciselniky(&c, &lock_db(st), Utc::now());
    Ok(ApiStatus {
        has_key: true,
        limit: Some(s.limit_volani),
        used: Some(s.provedeno_volani),
        expires: s.expirace_api_key,
        error: None,
    })
}

pub fn delete_api_key(st: &AppState) -> Result<(), String> {
    st.secrets.delete().map_err(msg)
}

// ---- items and k.ú. ----

pub fn list_items(st: &AppState) -> Result<Vec<ItemView>, String> {
    Ok(lock_db(st).list_items().map_err(msg)?.iter().map(item_view).collect())
}

/// Inserts or updates, then resolves right away. A resolve failure is stored in
/// `resolve_error` and returned inside the view, not as a command error.
pub fn save_item(st: &AppState, input: ItemInput) -> Result<ItemView, String> {
    validate(&input.ident)?;
    let popis = input.popis.trim().to_string();
    let db = lock_db(st);
    let id = match input.id {
        Some(id) => {
            db.update_item(&Item { id, popis, enabled: input.enabled, ident: input.ident })
                .map_err(msg)?;
            id
        }
        None => db
            .insert_item(&NewItem { popis, enabled: input.enabled, ident: input.ident })
            .map_err(msg)?,
    };
    // Without a key there is nothing to resolve against; the item is simply saved.
    if let Some(key) = stored_key(st)? {
        let c = ApiClient::new(st.transport.clone(), key);
        let row = find_item(&db, id)?;
        let needs_ciselniky =
            matches!(&row.item.ident, ItemIdent::Jednotka { kod_casti_obce: None, .. });
        let resolved = if needs_ciselniky {
            ensure_ciselniky(&c, &db, Utc::now()).and_then(|_| resolve(&c, &db, &row))
        } else {
            resolve(&c, &db, &row)
        };
        db.set_resolve_error(id, resolved.err().map(msg)).map_err(msg)?;
    }
    Ok(item_view(&find_item(&db, id)?))
}

pub fn delete_item(st: &AppState, id: i64) -> Result<(), String> {
    lock_db(st).delete_item(id).map_err(msg)
}

/// Autocomplete for k.ú. by name. Lazily downloads the číselníky when the cache is empty.
pub fn search_ku(st: &AppState, q: String) -> Result<Vec<KuOption>, String> {
    let q = q.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let db = lock_db(st);
    if db.ciselnik_age("ku").map_err(msg)?.is_none() {
        if let Ok(c) = client(st) {
            ensure_ciselniky(&c, &db, Utc::now()).map_err(msg)?;
        }
    }
    db.search_ku(q, KU_SEARCH_LIMIT).map_err(msg)
}

/// All části obce of the municipality the k.ú. belongs to (override picker).
pub fn casti_obci_for_ku(st: &AppState, ku: u32) -> Result<Vec<CastObceOption>, String> {
    let db = lock_db(st);
    let (_, kod_obce) = db
        .get_ku(ku)
        .map_err(msg)?
        .ok_or_else(|| "Katastrální území není v číselníku.".to_string())?;
    Ok(db
        .casti_obci_for_obec(kod_obce)
        .map_err(msg)?
        .into_iter()
        .map(|(kod, nazev)| CastObceOption { kod, nazev })
        .collect())
}

// ---- checks and results ----

/// Manual check. Holds the Db mutex for the whole run (the core takes `&mut Db`), so other
/// commands that touch the database wait until it finishes.
pub fn run_check(st: &AppState) -> Result<RunSummaryView, String> {
    let c = client(st)?;
    let mut db = lock_db(st);
    let s = run::run_check(&c, &mut db, &st.lock_path, Trigger::Manual, Utc::now()).map_err(msg)?;
    Ok(RunSummaryView {
        run_id: s.run_id,
        status: s.status.as_str().to_string(),
        checked: s.checked,
        changed: s.changed,
        plomby_nove: s.plomby_nove,
        errors: s.errors,
        deferred: s.deferred,
        api_calls: s.api_calls,
        api_used_today: s.api_used_today,
        api_limit: s.api_limit,
        aktualnost_dat: s.aktualnost_dat,
        error: s.error,
    })
}

pub fn results(st: &AppState) -> Result<ResultsView, String> {
    let db = lock_db(st);
    let findings = db
        .unseen_findings()
        .map_err(msg)?
        .into_iter()
        .map(|f| FindingView {
            result_id: f.result_id,
            run_id: f.run_id,
            item_id: f.item_id,
            popis: f.popis,
            describe: f.describe,
            status: status_from_db(&f.status),
            changes: f
                .changes_json
                .and_then(|j| serde_json::from_str::<Changes>(&j).ok())
                .map(changes_view),
            run_finished_at: f.run_finished_at,
        })
        .collect();
    let runs: Vec<RunView> =
        db.recent_runs(RUNS_LIMIT).map_err(msg)?.into_iter().map(run_view).collect();
    Ok(ResultsView { findings, last_run: runs.first().cloned(), runs })
}

pub fn mark_seen(st: &AppState) -> Result<(), String> {
    lock_db(st).mark_all_seen().map_err(msg)
}

/// The item plus its last stored snapshot from the API (`lastData`, null before the first
/// successful check).
pub fn item_detail(st: &AppState, id: i64) -> Result<Value, String> {
    let row = find_item(&lock_db(st), id)?;
    let last_data = row.last_data.as_deref().and_then(|d| serde_json::from_str::<Value>(d).ok());
    Ok(json!({ "item": item_view(&row), "lastData": last_data }))
}

/// Calls `StavUctu` (1 API call) only when a key is stored. `waiting` is an estimate: due
/// items (oldest first) are charged 1 call when their ISKN id is cached and 2 otherwise
/// (resolve + fetch); those that do not fit into `remaining_calls` are counted as waiting.
/// Plomba details and retries are not modelled.
pub fn due_overview(st: &AppState) -> Result<DueOverview, String> {
    let (due_now, costs) = {
        let db = lock_db(st);
        let interval = read_interval(&db)?;
        let due = db.due_items(Utc::now(), interval).map_err(msg)?;
        let costs: Vec<u32> = due
            .iter()
            .map(|r| match db.get_resolved(r.item.id) {
                Ok(Some(res)) if res.item_key == item_key(&r.item.ident) => 1,
                _ => 2,
            })
            .collect();
        (due.len() as u32, costs)
    };
    let remaining = client(st).ok().and_then(|c| c.stav_uctu().ok()).map(|s| remaining_from(&s));
    let waiting = match remaining {
        None => 0,
        Some(mut left) => costs
            .iter()
            .filter(|&&cost| {
                if cost <= left {
                    left -= cost;
                    false
                } else {
                    left = 0;
                    true
                }
            })
            .count() as u32,
    };
    Ok(DueOverview { due_now, waiting, remaining_calls: remaining })
}

// ---- settings ----

pub fn get_settings(st: &AppState) -> Result<Settings, String> {
    let db = lock_db(st);
    Ok(Settings {
        interval_days: read_interval(&db)?,
        schedule_enabled: db.get_setting(SCHEDULE_ENABLED_KEY).map_err(msg)?.as_deref() == Some("1"),
        schedule_time: db
            .get_setting(SCHEDULE_TIME_KEY)
            .map_err(msg)?
            .unwrap_or_else(|| DEFAULT_TIME.to_string()),
    })
}

/// Validates, updates the OS scheduled task (current exe, daily at `schedule_time`) and
/// only then persists, so the stored settings never claim a task that was not created.
pub fn save_settings(st: &AppState, s: Settings) -> Result<(), String> {
    if !ALLOWED_INTERVALS.contains(&s.interval_days) {
        return Err("Interval musí být 7, 14 nebo 30 dní.".into());
    }
    if validate_time(&s.schedule_time).is_err() {
        return Err("Čas musí být ve formátu HH:MM.".into());
    }
    if s.schedule_enabled {
        st.scheduler.enable(&st.exe_path, &s.schedule_time).map_err(msg)?;
    } else {
        st.scheduler.disable().map_err(msg)?;
    }
    let db = lock_db(st);
    db.set_setting(INTERVAL_SETTING, &s.interval_days.to_string()).map_err(msg)?;
    db.set_setting(SCHEDULE_ENABLED_KEY, if s.schedule_enabled { "1" } else { "0" })
        .map_err(msg)?;
    db.set_setting(SCHEDULE_TIME_KEY, &s.schedule_time).map_err(msg)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use strazce_core::api::MockTransport;
    use strazce_core::lock::RunLock;
    use strazce_core::model::{DruhCislovani, ParcelaRef};
    use strazce_core::schedule::{MockScheduler, SchedulerCall};
    use strazce_core::secrets::MemoryStore;

    struct T {
        st: AppState,
        mock: Arc<MockTransport>,
        sched: Arc<MockScheduler>,
        _dir: tempfile::TempDir,
    }

    fn setup(with_key: bool) -> T {
        let dir = tempfile::tempdir().unwrap();
        let mock = Arc::new(MockTransport::new());
        let sched = Arc::new(MockScheduler::default());
        let secrets = Arc::new(MemoryStore::default());
        if with_key {
            secrets.set("fake-key").unwrap();
        }
        let st = AppState {
            db: Mutex::new(Db::open_in_memory().unwrap()),
            secrets,
            transport: mock.clone(),
            scheduler: sched.clone(),
            lock_path: dir.path().join("strazce.lock"),
            exe_path: PathBuf::from(r"C:\Test\strazce.exe"),
        };
        T { st, mock, sched, _dir: dir }
    }

    fn parcela_input() -> ItemInput {
        ItemInput {
            id: None,
            popis: "Pole".into(),
            enabled: true,
            ident: ItemIdent::Parcela {
                ku: 618853,
                parcela: ParcelaRef { druh: DruhCislovani::Stavebni, kmenove: 182, poddeleni: None },
            },
        }
    }

    #[test]
    fn save_item_with_resolve_error_still_persists() {
        let t = setup(true);
        t.mock.push("/Parcely/Vyhledani", 200, r#"{"data":[]}"#);
        let v = save_item(&t.st, parcela_input()).expect("a resolve failure is not a command error");
        assert!(v.resolve_error.is_some());
        assert_eq!(v.kind, "parcela");
        let items = list_items(&t.st).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].popis, "Pole");
        assert!(items[0].resolve_error.is_some());
    }

    #[test]
    fn save_settings_enables_scheduler_with_current_exe() {
        let t = setup(false);
        let s = Settings { interval_days: 7, schedule_enabled: true, schedule_time: "09:30".into() };
        save_settings(&t.st, s).unwrap();
        assert_eq!(
            t.sched.calls(),
            vec![SchedulerCall::Enable { exe: PathBuf::from(r"C:\Test\strazce.exe"), time: "09:30".into() }]
        );
    }

    #[test]
    fn run_check_already_running_message() {
        let t = setup(true);
        let _held = RunLock::acquire(&t.st.lock_path).unwrap();
        assert_eq!(run_check(&t.st).unwrap_err(), "Kontrola už běží.");
    }
    #[test]
    fn save_settings_rejects_bad_values_without_touching_scheduler() {
        let t = setup(false);
        let bad_interval = Settings { interval_days: 10, schedule_enabled: true, schedule_time: "08:00".into() };
        assert!(save_settings(&t.st, bad_interval).is_err());
        let bad_time = Settings { interval_days: 14, schedule_enabled: true, schedule_time: "8h".into() };
        assert!(save_settings(&t.st, bad_time).is_err());
        assert!(t.sched.calls().is_empty());
        let ok = Settings { interval_days: 30, schedule_enabled: false, schedule_time: "07:15".into() };
        save_settings(&t.st, ok.clone()).unwrap();
        assert_eq!(get_settings(&t.st).unwrap(), ok);
        assert_eq!(t.sched.calls(), vec![SchedulerCall::Disable]);
    }

    #[test]
    fn set_api_key_stores_valid_key_and_rejects_invalid_one() {
        let t = setup(false);
        assert_eq!(run_check(&t.st).unwrap_err(), "Chybí API klíč.");
        t.mock.push("/AplikacniSluzby/StavUctu", 401, "");
        assert_eq!(set_api_key(&t.st, "bad".into()).unwrap_err(), "API klíč je neplatný.");
        assert!(!api_status(&t.st).unwrap().has_key);

        t.mock.push(
            "/AplikacniSluzby/StavUctu",
            200,
            r#"{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":10,"limitVolani":500,"expiraceApiKey":null}"#,
        );
        for list in ["KatastralniUzemi", "Obce", "CastiObci"] {
            t.mock.push(&format!("/CiselnikyUzemnichJednotek/{list}"), 200, r#"{"data":[]}"#);
        }
        let s = set_api_key(&t.st, " good-key ".into()).unwrap();
        assert!(s.has_key);
        assert_eq!((s.limit, s.used), (Some(500), Some(10)));
        assert_eq!(t.st.secrets.get().unwrap().as_deref(), Some("good-key"));
    }
}
