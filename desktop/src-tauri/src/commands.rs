//! Command bodies: plain functions over [`AppState`], so tests can run them with mocks.
//! The `#[tauri::command]` wrappers live in `ipc.rs`.
//!
//! Serde naming: every struct defined here crosses the IPC boundary as `camelCase`.
//! Core types that already serialize on their own (`ItemIdent`, `FieldChange`, `RizeniDef`,
//! `KuOption`) are passed as-is; `ItemIdent` therefore keeps its snake_case fields and
//! PascalCase enum tags. Run status strings are lowercase (`ok`, `partial`, `failed`,
//! `running`); `ItemStatus` uses its default PascalCase representation (`Changed`, ...).

use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use strazce_core::api::{ApiClient, RizeniDef, StavUctu, Transport};
use strazce_core::db::{Db, FindingRow, ItemRow, KuOption, NewItem, RunRow};
use strazce_core::diff::FieldChange;
use strazce_core::error::CoreError;
use strazce_core::model::{describe, item_key, validate, Item, ItemIdent};
use strazce_core::resolve::{ensure_ciselniky, resolve};
use strazce_core::run::{
    self, Changes, ItemStatus, Trigger, DEFAULT_INTERVAL_DAYS, INTERVAL_SETTING, SAFETY_MARGIN,
};
use strazce_core::schedule::{validate_time, Scheduler};
use strazce_core::secrets::SecretStore;

const ALLOWED_INTERVALS: [u32; 3] = [7, 14, 30];
const DEFAULT_TIME: &str = "08:00";
const SCHEDULE_ENABLED_KEY: &str = "schedule_enabled";
const SCHEDULE_TIME_KEY: &str = "schedule_time";
/// When the schedule was last switched on (RFC 3339), for the heartbeat grace period.
const SCHEDULE_ENABLED_AT_KEY: &str = "schedule_enabled_at";
/// A daily task that has not run for this long is reported as stale.
const SCHEDULE_STALE_DAYS: i64 = 2;
const KU_SEARCH_LIMIT: u32 = 20;
const RUNS_LIMIT: u32 = 30;
const MSG_NO_KEY: &str = "Chybí API klíč.";
const MSG_RUNNING: &str = "Kontrola už běží.";
const MSG_UI_BUDGET: &str =
    "Dnešní limit API je vyčerpán – položka se dohledá při příští kontrole.";
const MSG_UI_BUDGET_KU: &str =
    "Dnešní limit API je vyčerpán – číselník katastrálních území se stáhne při příští kontrole nebo zítra.";
/// Settings key prefix of the per-day counter of API calls made from the UI.
const UI_CALLS_PREFIX: &str = "ui_api_calls_";
/// Calls the UI leaves free for the checks when it decides whether to call the API.
const UI_RESERVE: i64 = 10;
/// Assumed daily limit while no run of today has reported the real one.
const DEFAULT_DAILY_LIMIT: i64 = 500;

pub struct AppState {
    pub db: Mutex<Db>,
    pub secrets: Arc<dyn SecretStore>,
    pub transport: Arc<dyn Transport>,
    pub scheduler: Arc<dyn Scheduler>,
    pub lock_path: PathBuf,
    /// Database file; `run_check` opens its own connection to it.
    pub db_path: PathBuf,
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
    /// When the next item becomes due (`None` without checked, enabled items); lets the UI
    /// say when the next check will do something after a run that had nothing due.
    pub next_due: Option<DateTime<Utc>>,
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
    /// Czech error text of a failed item (core errors are fixed text, never the key).
    pub error: Option<String>,
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
    /// Per-item results of `last_run` (all statuses).
    pub last_run_items: Vec<FindingView>,
    /// The newest run with a result (items checked, failed or deferred, or the run failed);
    /// daily scheduled runs with nothing due do not hide it.
    pub last_run: Option<RunView>,
    /// History without the runs that had nothing due.
    pub runs: Vec<RunView>,
    /// Runs with nothing due since `last_run` (all of them when there is none).
    pub empty_runs: u32,
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

/// `get_settings` result: the editable settings plus the schedule heartbeat.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    #[serde(flatten)]
    pub settings: Settings,
    /// When the newest scheduled (`--check`) run started.
    pub last_scheduled_run: Option<DateTime<Utc>>,
    /// The schedule is on but no scheduled run happened in the last two days (or never,
    /// once it has been on for two days).
    pub schedule_stale: bool,
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
    Ok(st
        .secrets
        .get()
        .map_err(msg)?
        .filter(|k| !k.trim().is_empty()))
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
    let left = i64::from(s.limit_volani) - i64::from(s.provedeno_volani) - SAFETY_MARGIN;
    left.max(0) as u32
}

// ---- UI-side API budget ----

fn local_today() -> NaiveDate {
    Local::now().date_naive()
}

/// Settings key of the UI call counter for `day` (local date).
fn ui_calls_key(day: NaiveDate) -> String {
    format!("{UI_CALLS_PREFIX}{}", day.format("%Y-%m-%d"))
}

fn ui_calls(db: &Db, day: NaiveDate) -> Result<u32, String> {
    Ok(db
        .get_setting(&ui_calls_key(day))
        .map_err(msg)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

/// Adds the calls a UI command just made (`client.calls()`) to today's counter and drops
/// the counters of earlier days.
fn add_ui_calls(db: &Db, calls: u32) -> Result<(), String> {
    if calls == 0 {
        return Ok(());
    }
    let day = local_today();
    let key = ui_calls_key(day);
    let total = ui_calls(db, day)?.saturating_add(calls);
    db.set_setting(&key, &total.to_string()).map_err(msg)?;
    db.delete_settings_with_prefix_except(UI_CALLS_PREFIX, &key)
        .map_err(msg)
}

/// Whether the UI may still make an optional API call (resolve, číselníky) today: the usage
/// last reported by today's run plus the UI calls counted today, plus a reserve of 10,
/// must stay below the limit (500 when no run of today reported it).
fn ui_budget_ok(db: &Db) -> Result<bool, String> {
    let today = local_today();
    let run = db
        .recent_runs(RUNS_LIMIT)
        .map_err(msg)?
        .into_iter()
        .find(|r| {
            r.api_used_today.is_some() && r.started_at.with_timezone(&Local).date_naive() == today
        });
    let (used, limit) = match run {
        Some(r) => (
            i64::from(r.api_used_today.unwrap_or(0)),
            r.api_limit.map_or(DEFAULT_DAILY_LIMIT, i64::from),
        ),
        None => (0, DEFAULT_DAILY_LIMIT),
    };
    Ok(used + i64::from(ui_calls(db, today)?) + UI_RESERVE < limit)
}

// ---- API key and account ----

/// Costs one API call (`StavUctu`) when a key is stored. An account-call failure is
/// reported in `error`, not as a command error, so the UI still knows a key exists.
pub fn api_status(st: &AppState) -> Result<ApiStatus, String> {
    let mut out = ApiStatus {
        has_key: false,
        limit: None,
        used: None,
        expires: None,
        error: None,
    };
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
    add_ui_calls(&lock_db(st), c.calls())?;
    Ok(out)
}

/// Validates the key with `StavUctu`, stores it, then refreshes the číselníky when today's
/// API budget allows. A číselník failure does not fail the command (`search_ku` and the
/// check retry lazily).
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
    {
        let db = lock_db(st);
        add_ui_calls(&db, c.calls())?;
        if ui_budget_ok(&db)? {
            let before = c.calls();
            let _ = ensure_ciselniky(&c, &db, Utc::now());
            add_ui_calls(&db, c.calls() - before)?;
        }
    }
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
    Ok(lock_db(st)
        .list_items()
        .map_err(msg)?
        .iter()
        .map(item_view)
        .collect())
}

/// Inserts or updates, then resolves right away when today's API budget allows (otherwise
/// the next check resolves it). A resolve failure is stored in `resolve_error` and
/// returned inside the view, not as a command error.
pub fn save_item(st: &AppState, input: ItemInput) -> Result<ItemView, String> {
    validate(&input.ident)?;
    let popis = input.popis.trim().to_string();
    let db = lock_db(st);
    let id = match input.id {
        Some(id) => {
            db.update_item(&Item {
                id,
                popis,
                enabled: input.enabled,
                ident: input.ident,
            })
            .map_err(msg)?;
            id
        }
        None => db
            .insert_item(&NewItem {
                popis,
                enabled: input.enabled,
                ident: input.ident,
            })
            .map_err(msg)?,
    };
    // Without a key there is nothing to resolve against; the item is simply saved.
    if let Some(key) = stored_key(st)? {
        if !ui_budget_ok(&db)? {
            db.set_resolve_error(id, Some(MSG_UI_BUDGET.to_string()))
                .map_err(msg)?;
            return Ok(item_view(&find_item(&db, id)?));
        }
        let c = ApiClient::new(st.transport.clone(), key);
        let row = find_item(&db, id)?;
        let needs_ciselniky = matches!(
            &row.item.ident,
            ItemIdent::Jednotka {
                kod_casti_obce: None,
                ..
            }
        );
        let resolved = if needs_ciselniky {
            ensure_ciselniky(&c, &db, Utc::now()).and_then(|_| resolve(&c, &db, &row))
        } else {
            resolve(&c, &db, &row)
        };
        add_ui_calls(&db, c.calls())?;
        db.set_resolve_error(id, resolved.err().map(msg))
            .map_err(msg)?;
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
            if !ui_budget_ok(&db)? {
                return Err(MSG_UI_BUDGET_KU.to_string());
            }
            let r = ensure_ciselniky(&c, &db, Utc::now());
            add_ui_calls(&db, c.calls())?;
            r.map_err(msg)?;
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

/// Manual check. Uses its own connection to the database file (WAL), so the shared
/// `Mutex<Db>` is not held during the run and other commands keep working. The core's
/// `RunLock` makes a concurrent second run fail fast with "Kontrola už běží.".
pub fn run_check(st: &AppState) -> Result<RunSummaryView, String> {
    let c = client(st)?;
    let mut db = Db::open(&st.db_path).map_err(msg)?;
    let s = run::run_check(&c, &mut db, &st.lock_path, Trigger::Manual, Utc::now()).map_err(msg)?;
    let next_due = db.next_due(read_interval(&db)?).map_err(msg)?;
    Ok(RunSummaryView {
        next_due,
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

fn finding_view(f: FindingRow) -> FindingView {
    FindingView {
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
        error: f.error,
        run_finished_at: f.run_finished_at,
    }
}

pub fn results(st: &AppState) -> Result<ResultsView, String> {
    let db = lock_db(st);
    let findings: Vec<FindingView> = db
        .unseen_findings()
        .map_err(msg)?
        .into_iter()
        .map(finding_view)
        .collect();
    let runs: Vec<RunView> = db
        .recent_nonempty_runs(RUNS_LIMIT)
        .map_err(msg)?
        .into_iter()
        .map(run_view)
        .collect();
    let last_run = db.last_result_run().map_err(msg)?.map(run_view);
    let empty_runs = db
        .count_empty_runs_after(last_run.as_ref().map(|r| r.id))
        .map_err(msg)?;
    let last_run_items = match &last_run {
        Some(r) => db
            .run_findings(r.id)
            .map_err(msg)?
            .into_iter()
            .map(finding_view)
            .collect(),
        None => Vec::new(),
    };
    Ok(ResultsView {
        findings,
        last_run,
        last_run_items,
        runs,
        empty_runs,
    })
}

/// Marks exactly the findings the UI displayed as seen; anything that arrived since stays.
pub fn mark_seen(st: &AppState, result_ids: Vec<i64>) -> Result<(), String> {
    lock_db(st).mark_seen(&result_ids).map_err(msg)
}

/// The item plus its last stored snapshot from the API (`lastData`, null before the first
/// successful check).
pub fn item_detail(st: &AppState, id: i64) -> Result<Value, String> {
    let row = find_item(&lock_db(st), id)?;
    let last_data = row
        .last_data
        .as_deref()
        .and_then(|d| serde_json::from_str::<Value>(d).ok());
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
    let remaining = match client(st) {
        Ok(c) => {
            let s = c.stav_uctu().ok();
            add_ui_calls(&lock_db(st), c.calls())?;
            s.map(|s| remaining_from(&s))
        }
        Err(_) => None,
    };
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
    Ok(DueOverview {
        due_now,
        waiting,
        remaining_calls: remaining,
    })
}

// ---- settings ----

pub fn get_settings(st: &AppState) -> Result<SettingsView, String> {
    settings_view(st, Utc::now())
}

fn read_settings(db: &Db) -> Result<Settings, String> {
    Ok(Settings {
        interval_days: read_interval(db)?,
        schedule_enabled: db
            .get_setting(SCHEDULE_ENABLED_KEY)
            .map_err(msg)?
            .as_deref()
            == Some("1"),
        schedule_time: db
            .get_setting(SCHEDULE_TIME_KEY)
            .map_err(msg)?
            .unwrap_or_else(|| DEFAULT_TIME.to_string()),
    })
}

fn settings_view(st: &AppState, now: DateTime<Utc>) -> Result<SettingsView, String> {
    let db = lock_db(st);
    let settings = read_settings(&db)?;
    let last_scheduled_run = db
        .last_run_by_trigger(Trigger::Scheduled.as_str())
        .map_err(msg)?
        .map(|r| r.started_at);
    let stale_before = now - chrono::Duration::days(SCHEDULE_STALE_DAYS);
    let schedule_stale = settings.schedule_enabled
        && match last_scheduled_run {
            Some(t) => t < stale_before,
            // Never ran: only stale once the schedule has been on long enough.
            None => db
                .get_setting(SCHEDULE_ENABLED_AT_KEY)
                .map_err(msg)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .is_some_and(|t| t.with_timezone(&Utc) < stale_before),
        };
    Ok(SettingsView {
        settings,
        last_scheduled_run,
        schedule_stale,
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
        st.scheduler
            .enable(&st.exe_path, &s.schedule_time)
            .map_err(msg)?;
    } else {
        st.scheduler.disable().map_err(msg)?;
    }
    let db = lock_db(st);
    let was_enabled = db
        .get_setting(SCHEDULE_ENABLED_KEY)
        .map_err(msg)?
        .as_deref()
        == Some("1");
    if s.schedule_enabled && !was_enabled {
        db.set_setting(SCHEDULE_ENABLED_AT_KEY, &Utc::now().to_rfc3339())
            .map_err(msg)?;
    }
    db.set_setting(INTERVAL_SETTING, &s.interval_days.to_string())
        .map_err(msg)?;
    db.set_setting(
        SCHEDULE_ENABLED_KEY,
        if s.schedule_enabled { "1" } else { "0" },
    )
    .map_err(msg)?;
    db.set_setting(SCHEDULE_TIME_KEY, &s.schedule_time)
        .map_err(msg)?;
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
        let db_path = dir.path().join("strazce.db");
        let mock = Arc::new(MockTransport::new());
        let sched = Arc::new(MockScheduler::default());
        let secrets = Arc::new(MemoryStore::default());
        if with_key {
            secrets.set("fake-key").unwrap();
        }
        let st = AppState {
            db: Mutex::new(Db::open(&db_path).unwrap()),
            secrets,
            transport: mock.clone(),
            scheduler: sched.clone(),
            lock_path: dir.path().join("strazce.lock"),
            db_path,
            exe_path: PathBuf::from(r"C:\Test\strazce.exe"),
        };
        T {
            st,
            mock,
            sched,
            _dir: dir,
        }
    }

    fn parcela_input() -> ItemInput {
        ItemInput {
            id: None,
            popis: "Pole".into(),
            enabled: true,
            ident: ItemIdent::Parcela {
                ku: 618853,
                parcela: ParcelaRef {
                    druh: DruhCislovani::Stavebni,
                    kmenove: 182,
                    poddeleni: None,
                },
            },
        }
    }

    #[test]
    fn save_item_with_resolve_error_still_persists() {
        let t = setup(true);
        t.mock.push("/Parcely/Vyhledani", 200, r#"{"data":[]}"#);
        let v =
            save_item(&t.st, parcela_input()).expect("a resolve failure is not a command error");
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
        let s = Settings {
            interval_days: 7,
            schedule_enabled: true,
            schedule_time: "09:30".into(),
        };
        save_settings(&t.st, s).unwrap();
        assert_eq!(
            t.sched.calls(),
            vec![SchedulerCall::Enable {
                exe: PathBuf::from(r"C:\Test\strazce.exe"),
                time: "09:30".into()
            }]
        );
    }

    #[test]
    fn run_check_already_running_message() {
        let t = setup(true);
        let _held = RunLock::acquire(&t.st.lock_path).unwrap();
        assert_eq!(run_check(&t.st).unwrap_err(), "Kontrola už běží.");
        // No run row and no API call were made.
        assert!(lock_db(&t.st).recent_runs(10).unwrap().is_empty());
        assert!(t.mock.requested_urls().is_empty());
    }

    #[test]
    fn run_check_does_not_hold_the_shared_db_mutex() {
        let t = setup(true);
        t.mock.push(
            "/AplikacniSluzby/StavUctu",
            200,
            r#"{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":10,"limitVolani":500,"expiraceApiKey":null}"#,
        );
        // Nothing is due, so the run is quick; the point is that it works while the
        // shared connection is locked by someone else.
        let guard = lock_db(&t.st);
        let s = run_check(&t.st).unwrap();
        assert_eq!(s.checked, 0);
        drop(guard);
        // The run had nothing due: collapsed out of the history, but counted.
        let r = results(&t.st).unwrap();
        assert!(r.runs.is_empty() && r.last_run.is_none());
        assert_eq!(r.empty_runs, 1);
    }

    #[test]
    fn results_carry_error_text_of_last_run_items() {
        let t = setup(false);
        let v = save_item(&t.st, parcela_input()).unwrap();
        {
            let db = lock_db(&t.st);
            let run = db.begin_run("manual", Utc::now()).unwrap();
            db.insert_item_result(run, v.id, "error", None, Some("Parcela nenalezena."))
                .unwrap();
            db.insert_item_result(run, v.id, "unchanged", None, None)
                .unwrap();
            finish(&db, run, "ok", 1, 1, Utc::now());
        }
        let r = results(&t.st).unwrap();
        assert_eq!(r.last_run_items.len(), 2);
        assert_eq!(r.last_run_items[0].status, ItemStatus::Error);
        assert_eq!(
            r.last_run_items[0].error.as_deref(),
            Some("Parcela nenalezena.")
        );
        assert_eq!(r.last_run_items[1].error, None);
        assert!(r.findings.is_empty());
    }

    #[test]
    fn save_item_ambiguous_cast_obce_persists_with_manual_hint() {
        let t = setup(true);
        {
            let db = lock_db(&t.st);
            let now = Utc::now();
            db.replace_ku(&[(610704, "Ponava".into(), 582786)], now)
                .unwrap();
            db.replace_obce(&[(582786, "Brno".into())], now).unwrap();
            db.replace_casti_obci(&[(411981, "Zábrdovice".into(), 582786)], now)
                .unwrap();
        }
        let input = ItemInput {
            id: None,
            popis: "Byt".into(),
            enabled: true,
            ident: ItemIdent::Jednotka {
                ku: 610704,
                typ: strazce_core::model::TypStavby::Cp,
                cislo_domovni: 893,
                cislo_jednotky: 37,
                kod_casti_obce: None,
            },
        };
        let v = save_item(&t.st, input).unwrap();
        assert_eq!(v.kind, "jednotka");
        assert!(v.resolve_error.unwrap().contains("vyberte ji ručně"));
        assert_eq!(list_items(&t.st).unwrap().len(), 1);
    }
    #[test]
    fn save_settings_rejects_bad_values_without_touching_scheduler() {
        let t = setup(false);
        let bad_interval = Settings {
            interval_days: 10,
            schedule_enabled: true,
            schedule_time: "08:00".into(),
        };
        assert!(save_settings(&t.st, bad_interval).is_err());
        let bad_time = Settings {
            interval_days: 14,
            schedule_enabled: true,
            schedule_time: "8h".into(),
        };
        assert!(save_settings(&t.st, bad_time).is_err());
        assert!(t.sched.calls().is_empty());
        let ok = Settings {
            interval_days: 30,
            schedule_enabled: false,
            schedule_time: "07:15".into(),
        };
        save_settings(&t.st, ok.clone()).unwrap();
        assert_eq!(get_settings(&t.st).unwrap().settings, ok);
        assert_eq!(t.sched.calls(), vec![SchedulerCall::Disable]);
    }

    #[test]
    fn set_api_key_stores_valid_key_and_rejects_invalid_one() {
        let t = setup(false);
        assert_eq!(run_check(&t.st).unwrap_err(), "Chybí API klíč.");
        t.mock.push("/AplikacniSluzby/StavUctu", 401, "");
        assert_eq!(
            set_api_key(&t.st, "bad".into()).unwrap_err(),
            "API klíč je neplatný."
        );
        assert!(!api_status(&t.st).unwrap().has_key);

        t.mock.push(
            "/AplikacniSluzby/StavUctu",
            200,
            r#"{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":10,"limitVolani":500,"expiraceApiKey":null}"#,
        );
        for list in ["KatastralniUzemi", "Obce", "CastiObci"] {
            t.mock.push(
                &format!("/CiselnikyUzemnichJednotek/{list}"),
                200,
                r#"{"data":[]}"#,
            );
        }
        let s = set_api_key(&t.st, " good-key ".into()).unwrap();
        assert!(s.has_key);
        assert_eq!((s.limit, s.used), (Some(500), Some(10)));
        assert_eq!(t.st.secrets.get().unwrap().as_deref(), Some("good-key"));
    }

    fn scheduled_run(db: &Db, at: DateTime<Utc>) {
        let id = db.begin_run("scheduled", at).unwrap();
        finish(db, id, "ok", 0, 0, at);
    }

    fn finish(db: &Db, id: i64, status: &str, checked: u32, errors: u32, at: DateTime<Utc>) {
        db.finish_run(&strazce_core::db::RunRecord {
            id,
            finished_at: at,
            status: status.into(),
            checked,
            changed: 0,
            errors,
            deferred: 0,
            api_calls: 0,
            api_used_today: None,
            api_limit: None,
            aktualnost_dat: None,
            error: None,
        })
        .unwrap();
    }

    #[test]
    fn schedule_heartbeat_reports_last_scheduled_run_and_staleness() {
        let t = setup(false);
        // Stored timestamps have whole seconds.
        let now = DateTime::from_timestamp(Utc::now().timestamp(), 0).unwrap();
        // Schedule off: never stale.
        let v = settings_view(&t.st, now).unwrap();
        assert_eq!(v.last_scheduled_run, None);
        assert!(!v.schedule_stale);

        // Enabled just now, no run yet: not stale (grace period).
        save_settings(
            &t.st,
            Settings {
                interval_days: 14,
                schedule_enabled: true,
                schedule_time: "08:00".into(),
            },
        )
        .unwrap();
        let v = settings_view(&t.st, now).unwrap();
        assert!(v.settings.schedule_enabled);
        assert!(!v.schedule_stale);
        // Enabled more than two days ago and the task never ran: stale.
        assert!(
            settings_view(&t.st, now + chrono::Duration::days(3))
                .unwrap()
                .schedule_stale
        );

        // A manual run does not count; a scheduled one does.
        {
            let db = lock_db(&t.st);
            db.begin_run("manual", now).unwrap();
            scheduled_run(&db, now + chrono::Duration::days(2));
        }
        let later = now + chrono::Duration::days(3);
        let v = settings_view(&t.st, later).unwrap();
        assert_eq!(v.last_scheduled_run, Some(now + chrono::Duration::days(2)));
        assert!(!v.schedule_stale);
        let v = settings_view(&t.st, now + chrono::Duration::days(5)).unwrap();
        assert!(v.schedule_stale);
    }

    #[test]
    fn results_last_run_skips_empty_runs_and_history_collapses_them() {
        let t = setup(false);
        let v = save_item(&t.st, parcela_input()).unwrap();
        let now = Utc::now();
        {
            let db = lock_db(&t.st);
            let failed = db.begin_run("scheduled", now).unwrap();
            finish(&db, failed, "failed", 0, 0, now);
            let real = db.begin_run("manual", now).unwrap();
            db.insert_item_result(real, v.id, "unchanged", None, None)
                .unwrap();
            finish(&db, real, "ok", 1, 0, now);
            for _ in 0..3 {
                scheduled_run(&db, now);
            }
        }
        let r = results(&t.st).unwrap();
        let last = r.last_run.expect("a run with checked items");
        assert_eq!(last.checked, 1);
        assert_eq!(r.last_run_items.len(), 1);
        assert_eq!(
            r.runs.iter().map(|x| x.status.as_str()).collect::<Vec<_>>(),
            vec!["ok", "failed"]
        );
        assert_eq!(r.empty_runs, 3);

        // A failed run is a real result even though it checked nothing.
        {
            let db = lock_db(&t.st);
            let failed = db.begin_run("scheduled", now).unwrap();
            finish(&db, failed, "failed", 0, 0, now);
        }
        let r = results(&t.st).unwrap();
        assert_eq!(r.last_run.unwrap().status, "failed");
        assert_eq!(r.empty_runs, 0);
    }

    #[test]
    fn run_check_with_nothing_due_reports_next_due() {
        let t = setup(true);
        t.mock
            .push("/Parcely/Vyhledani", 200, r#"{"data":[{"id":101}]}"#);
        let v = save_item(&t.st, parcela_input()).unwrap();
        let checked_at = DateTime::from_timestamp(Utc::now().timestamp(), 0).unwrap()
            - chrono::Duration::days(3);
        lock_db(&t.st)
            .set_item_success(v.id, "{}", checked_at)
            .unwrap();
        // Nothing is due: no API call at all (the mock has nothing queued).
        let s = run_check(&t.st).unwrap();
        assert_eq!((s.checked, s.errors, s.deferred), (0, 0, 0));
        assert_eq!(s.next_due, Some(checked_at + chrono::Duration::days(14)));
    }

    fn ui_calls(t: &T) -> Option<String> {
        lock_db(&t.st)
            .get_setting(&ui_calls_key(local_today()))
            .unwrap()
    }

    #[test]
    fn ui_api_calls_are_counted_per_day() {
        let t = setup(true);
        t.mock
            .push("/Parcely/Vyhledani", 200, r#"{"data":[{"id":101}]}"#);
        save_item(&t.st, parcela_input()).unwrap();
        assert_eq!(ui_calls(&t).as_deref(), Some("1"));
        t.mock.push(
            "/AplikacniSluzby/StavUctu",
            200,
            r#"{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":10,"limitVolani":500,"expiraceApiKey":null}"#,
        );
        api_status(&t.st).unwrap();
        assert_eq!(ui_calls(&t).as_deref(), Some("2"));
    }

    #[test]
    fn save_item_skips_resolve_when_todays_budget_is_used_up() {
        let t = setup(true);
        {
            // Today's run reported 486/500 used; the UI has made 4 more calls since.
            let db = lock_db(&t.st);
            let run = db.begin_run("scheduled", Utc::now()).unwrap();
            db.finish_run(&strazce_core::db::RunRecord {
                id: run,
                finished_at: Utc::now(),
                status: "ok".into(),
                checked: 1,
                changed: 0,
                errors: 0,
                deferred: 0,
                api_calls: 3,
                api_used_today: Some(486),
                api_limit: Some(500),
                aktualnost_dat: None,
                error: None,
            })
            .unwrap();
            db.set_setting(&ui_calls_key(local_today()), "4").unwrap();
        }
        // No response queued: any API call would panic.
        let v = save_item(&t.st, parcela_input()).expect("the item is still saved");
        assert_eq!(v.resolve_error.as_deref(), Some(MSG_UI_BUDGET));
        assert_eq!(list_items(&t.st).unwrap().len(), 1);
        assert!(t.mock.requested_urls().is_empty());
    }

    #[test]
    fn set_api_key_skips_ciselniky_and_search_ku_skips_download_without_budget() {
        let t = setup(false);
        lock_db(&t.st)
            .set_setting(&ui_calls_key(local_today()), "495")
            .unwrap();
        t.mock.push(
            "/AplikacniSluzby/StavUctu",
            200,
            r#"{"idSluzby":1,"aktualniObdobi":"2026-11-17","provedenoVolani":10,"limitVolani":500,"expiraceApiKey":null}"#,
        );
        set_api_key(&t.st, "good-key".into()).unwrap();
        assert_eq!(t.mock.requested_urls().len(), 1, "only StavUctu");
        assert_eq!(ui_calls(&t).as_deref(), Some("496"));
        let err = search_ku(&t.st, "Pona".into()).unwrap_err();
        assert!(err.contains("limit"), "{err}");
        assert_eq!(t.mock.requested_urls().len(), 1);
    }

    #[test]
    fn mark_seen_leaves_findings_that_arrived_after_display() {
        let t = setup(false);
        let v = save_item(&t.st, parcela_input()).unwrap();
        let changed = |run| {
            lock_db(&t.st)
                .insert_item_result(run, v.id, "changed", Some("{}"), None)
                .unwrap()
        };
        let run = lock_db(&t.st).begin_run("manual", Utc::now()).unwrap();
        changed(run);
        let displayed: Vec<i64> = results(&t.st)
            .unwrap()
            .findings
            .iter()
            .map(|f| f.result_id)
            .collect();
        assert_eq!(displayed.len(), 1);
        // A scheduled run adds a finding while the user reads the list.
        let later = lock_db(&t.st).begin_run("scheduled", Utc::now()).unwrap();
        changed(later);
        mark_seen(&t.st, displayed).unwrap();
        let left = results(&t.st).unwrap().findings;
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].run_id, later);
    }
}
