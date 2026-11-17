//! One check run: due items, API budget, fetch + diff, and persisting the results.

use crate::api::{ApiClient, Envelope, RizeniDef};
use crate::db::{Db, ItemRow, RunRecord};
use crate::diff::{diff_object, diff_plomby, FieldChange};
use crate::error::CoreError;
use crate::lock::RunLock;
use crate::model::{item_key, ItemIdent};
use crate::resolve::{ensure_ciselniky, resolve, Endpoint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// Calls kept in reserve below the daily limit.
pub const SAFETY_MARGIN: i64 = 5;
/// Extra reserve: `client.calls()` already counts the StavUctu call, which the API's
/// `provedenoVolani` does not, so this is deliberately one call conservative.
const STAV_UCTU_RESERVE: i64 = 1;
/// Worst case for one item without retries: resolve + GET, then on 404 re-resolve + GET.
const MAX_CALLS_PER_ITEM: i64 = 4;
/// Attempts one logical API call can make (1 + `MAX_RETRIES` in the client). Every request
/// in a run needs this much budget left, so retries can never push usage past the budget.
const MAX_ATTEMPTS_PER_CALL: i64 = 3;
/// Worst case for `ensure_ciselniky`: three lists, each with retries.
const CISELNIKY_CALLS: i64 = 3 * MAX_ATTEMPTS_PER_CALL;
pub const DEFAULT_INTERVAL_DAYS: u32 = 14;
/// Settings key holding the per-item check interval in days.
pub const INTERVAL_SETTING: &str = "interval_days";
const MAX_ERROR_LEN: usize = 300;
/// Error stored on a `running` row left behind by a process that died mid-run.
const MSG_INTERRUPTED: &str = "Kontrola byla přerušena.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    Manual,
    Scheduled,
}

impl Trigger {
    /// Value stored in `runs.trigger`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemStatus {
    First,
    Unchanged,
    Changed,
    Error,
    Deferred,
}

impl ItemStatus {
    /// Value stored in `item_results.status`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Unchanged => "unchanged",
            Self::Changed => "changed",
            Self::Error => "error",
            Self::Deferred => "deferred",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    Ok,
    Partial,
    Failed,
}

impl RunStatus {
    /// Value stored in `runs.status`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

/// A new plomba with best-effort details from `GET /Rizeni/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlombaDetail {
    pub rizeni: RizeniDef,
    pub datum_prijeti: Option<String>,
    pub stav: Option<String>,
    pub operace: Vec<String>,
}

/// Stored as `item_results.changes_json` for `changed` results, and for `first` results
/// whose object already carries plomby (then only `plomby_nove` is filled).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Changes {
    pub fields: Vec<FieldChange>,
    pub plomby_nove: Vec<PlombaDetail>,
    pub plomby_zanikle: Vec<RizeniDef>,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.plomby_nove.is_empty() && self.plomby_zanikle.is_empty()
    }
}

/// Outcome of a run. `checked` counts items fetched successfully (first, unchanged or
/// changed); `errors` and `deferred` are counted separately. `api_used_today` and
/// `api_limit` are 0 when StavUctu was not called (nothing was due).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: i64,
    pub status: RunStatus,
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

/// API calls made by this run (the client may be shared between runs).
struct Meter<'a> {
    client: &'a ApiClient,
    base: u32,
    /// `limit − provedeno − 1 − 5`, set once StavUctu is known.
    budget: i64,
}

impl Meter<'_> {
    fn used(&self) -> u32 {
        self.client.calls().saturating_sub(self.base)
    }

    fn remaining(&self) -> i64 {
        self.budget - i64::from(self.used())
    }

    /// `Err(Budget)` unless one more logical call (with all its retries) still fits.
    fn guard(&self) -> Result<(), CoreError> {
        if self.remaining() >= MAX_ATTEMPTS_PER_CALL {
            Ok(())
        } else {
            Err(CoreError::Budget)
        }
    }
}

/// Everything recorded for one item; written in the final transaction, and only if the
/// item still exists with the identification it had when it was checked.
struct Outcome {
    item_id: i64,
    /// `item_key` of the item as checked (set by `execute`).
    item_key: String,
    status: ItemStatus,
    changes_json: Option<String>,
    error: Option<String>,
    /// `Some` -> `set_item_success` with this snapshot.
    new_data: Option<String>,
    /// `Some(x)` -> `set_resolve_error(x)`.
    resolve_error: Option<Option<String>>,
    plomby_nove: u32,
}

impl Outcome {
    fn new(item_id: i64, status: ItemStatus) -> Self {
        Self {
            item_id,
            item_key: String::new(),
            status,
            changes_json: None,
            error: None,
            new_data: None,
            resolve_error: None,
            plomby_nove: 0,
        }
    }

    fn error(item_id: i64, msg: String) -> Self {
        Self {
            error: Some(msg),
            ..Self::new(item_id, ItemStatus::Error)
        }
    }
}

fn truncate(s: String) -> String {
    if s.chars().count() <= MAX_ERROR_LEN {
        s
    } else {
        let mut t: String = s.chars().take(MAX_ERROR_LEN).collect();
        t.push('…');
        t
    }
}

fn invalid_response(what: &str) -> CoreError {
    CoreError::Http {
        status: 200,
        body: format!("Neplatná odpověď API: {what}"),
    }
}

fn plomby_of(v: &Value) -> Result<Vec<RizeniDef>, CoreError> {
    match v.get("rizeniPlomby") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(p) => serde_json::from_value(p.clone())
            .map_err(|e| invalid_response(&format!("rizeniPlomby ({e})"))),
    }
}

/// A string field, or the `nazev` of an object-valued field.
fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("nazev").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

/// Best effort: any failure (or no budget left) yields the plomba without details.
/// Only an invalid API key is propagated, because it aborts the whole run.
fn plomba_detail(
    client: &ApiClient,
    meter: &Meter,
    rizeni: RizeniDef,
) -> Result<PlombaDetail, CoreError> {
    let mut d = PlombaDetail {
        rizeni,
        datum_prijeti: None,
        stav: None,
        operace: Vec::new(),
    };
    if meter.guard().is_err() {
        return Ok(d);
    }
    let data = match client.get::<Value>(&format!("/Rizeni/{}", d.rizeni.id), &[]) {
        Ok(env) => env.data,
        Err(CoreError::InvalidApiKey) => return Err(CoreError::InvalidApiKey),
        Err(_) => return Ok(d),
    };
    d.datum_prijeti = text(data.get("datumPrijeti"));
    d.stav = text(data.get("stav"));
    d.operace = data
        .get("provedeneOperace")
        .and_then(Value::as_array)
        .map(|ops| ops.iter().filter_map(|o| text(o.get("nazev"))).collect())
        .unwrap_or_default();
    Ok(d)
}

fn object_path(endpoint: Endpoint, id: i64) -> String {
    format!("/{}/{id}", endpoint.as_str())
}

fn is_cached(db: &Db, row: &ItemRow) -> Result<bool, CoreError> {
    let key = item_key(&row.item.ident);
    Ok(db
        .get_resolved(row.item.id)?
        .is_some_and(|r| r.item_key == key))
}

/// `resolve` behind the budget guard; a lookup is one logical API call (a cache hit
/// costs nothing and needs no budget).
fn guarded_resolve(
    client: &ApiClient,
    db: &Db,
    meter: &Meter,
    row: &ItemRow,
    cached: bool,
) -> Result<(Endpoint, i64), CoreError> {
    if !cached {
        meter.guard()?;
    }
    resolve(client, db, row)
}

/// Resolves and fetches the object; on 404 for a cached ISKN id, clears the cache and
/// re-resolves once. `resolved_ok` tells whether the last resolve attempt succeeded.
/// `Err(Budget)` when a request no longer fits the budget (the item is deferred).
fn fetch(
    client: &ApiClient,
    db: &Db,
    meter: &Meter,
    row: &ItemRow,
    resolved_ok: &mut bool,
) -> Result<Envelope<Value>, CoreError> {
    let was_cached = is_cached(db, row)?;
    let (endpoint, id) = guarded_resolve(client, db, meter, row, was_cached)?;
    *resolved_ok = true;
    meter.guard()?;
    match client.get::<Value>(&object_path(endpoint, id), &[]) {
        Err(CoreError::NotFound) if was_cached => {
            db.clear_resolved(row.item.id)?;
            *resolved_ok = false;
            let (endpoint, id) = guarded_resolve(client, db, meter, row, false)?;
            *resolved_ok = true;
            meter.guard()?;
            client.get::<Value>(&object_path(endpoint, id), &[])
        }
        other => other,
    }
}

/// Checks one item. `Err` only for an invalid API key (the run aborts).
fn check_item(
    client: &ApiClient,
    db: &Db,
    meter: &Meter,
    row: &ItemRow,
    aktualnost: &mut Option<String>,
) -> Result<Outcome, CoreError> {
    let id = row.item.id;
    let mut resolved_ok = false;
    let fetched = fetch(client, db, meter, row, &mut resolved_ok).and_then(|env| {
        if env.data.is_object() {
            Ok(env)
        } else {
            Err(invalid_response("data nejsou objekt"))
        }
    });
    let env = match fetched {
        Ok(env) => env,
        Err(CoreError::InvalidApiKey) => return Err(CoreError::InvalidApiKey),
        // Data untouched, the item stays due.
        Err(CoreError::Budget) => return Ok(Outcome::new(id, ItemStatus::Deferred)),
        Err(CoreError::Resolve(msg)) => {
            let msg = truncate(msg);
            let mut o = Outcome::error(id, msg.clone());
            o.resolve_error = Some(Some(msg));
            return Ok(o);
        }
        Err(e) => {
            let mut o = Outcome::error(id, truncate(e.to_string()));
            if resolved_ok {
                o.resolve_error = Some(None);
            }
            return Ok(o);
        }
    };
    if aktualnost.is_none() {
        aktualnost.clone_from(&env.aktualnost_dat_k);
    }
    let data = env.data;
    let new_plomby = match plomby_of(&data) {
        Ok(p) => p,
        Err(e) => {
            let mut o = Outcome::error(id, truncate(e.to_string()));
            o.resolve_error = Some(None);
            return Ok(o);
        }
    };
    let old = row
        .last_data
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());

    let mut o = match old {
        // Never checked, or the identification changed (update_item cleared last_data).
        // Plomby already present are reported: an ongoing proceeding matters right away.
        None => {
            let mut o = Outcome::new(id, ItemStatus::First);
            if !new_plomby.is_empty() {
                let mut changes = Changes {
                    fields: Vec::new(),
                    plomby_nove: Vec::with_capacity(new_plomby.len()),
                    plomby_zanikle: Vec::new(),
                };
                for r in new_plomby {
                    changes.plomby_nove.push(plomba_detail(client, meter, r)?);
                }
                o.plomby_nove = changes.plomby_nove.len() as u32;
                o.changes_json = Some(
                    serde_json::to_string(&changes).map_err(|e| CoreError::Db(e.to_string()))?,
                );
            }
            o
        }
        Some(old) => {
            let (nove, zanikle) = diff_plomby(&plomby_of(&old).unwrap_or_default(), &new_plomby);
            let mut changes = Changes {
                fields: diff_object(&old, &data),
                plomby_nove: Vec::with_capacity(nove.len()),
                plomby_zanikle: zanikle,
            };
            for r in nove {
                changes.plomby_nove.push(plomba_detail(client, meter, r)?);
            }
            if changes.is_empty() {
                Outcome::new(id, ItemStatus::Unchanged)
            } else {
                let mut o = Outcome::new(id, ItemStatus::Changed);
                o.plomby_nove = changes.plomby_nove.len() as u32;
                o.changes_json = Some(
                    serde_json::to_string(&changes).map_err(|e| CoreError::Db(e.to_string()))?,
                );
                o
            }
        }
    };
    o.new_data = Some(serde_json::to_string(&data).map_err(|e| CoreError::Db(e.to_string()))?);
    o.resolve_error = Some(None);
    Ok(o)
}

fn needs_ciselniky(db: &Db, row: &ItemRow) -> bool {
    if !matches!(
        row.item.ident,
        ItemIdent::Jednotka {
            kod_casti_obce: None,
            ..
        }
    ) {
        return false;
    }
    let key = item_key(&row.item.ident);
    !matches!(db.get_resolved(row.item.id), Ok(Some(r)) if r.item_key == key)
}

/// API usage known so far, for the failed-run record.
#[derive(Default)]
struct Usage {
    provedeno: Option<u32>,
    limit: Option<u32>,
}

/// Runs one check. Takes the run lock (`AlreadyRunning` if held; nothing is touched
/// then), checks the due items oldest first within today's API budget and writes the
/// item results together with the run row in one transaction.
///
/// On an invalid API key (from StavUctu or mid-run), or any other failure after the run
/// was started, the run is recorded as `failed` with no item results, items are left
/// untouched, and the error is returned.
pub fn run_check(
    client: &ApiClient,
    db: &mut Db,
    lock_path: &Path,
    trigger: Trigger,
    now: DateTime<Utc>,
) -> Result<RunSummary, CoreError> {
    let _lock = RunLock::acquire(lock_path)?;
    let db: &Db = db;
    // With the lock held no other run is in progress: `running` rows are leftovers.
    db.fail_running_runs(MSG_INTERRUPTED)?;
    let run_id = db.begin_run(trigger.as_str(), now)?;
    let mut meter = Meter {
        client,
        base: client.calls(),
        budget: 0,
    };
    let mut usage = Usage::default();
    match execute(client, db, run_id, now, &mut meter, &mut usage) {
        Ok(summary) => Ok(summary),
        Err(e) => {
            let used = meter.used();
            let _ = db.finish_run(&RunRecord {
                id: run_id,
                finished_at: now,
                status: RunStatus::Failed.as_str().into(),
                checked: 0,
                changed: 0,
                errors: 0,
                deferred: 0,
                api_calls: used,
                api_used_today: usage.provedeno.map(|p| p.saturating_add(used)),
                api_limit: usage.limit,
                aktualnost_dat: None,
                error: Some(truncate(e.to_string())),
            });
            Err(e)
        }
    }
}

fn execute(
    client: &ApiClient,
    db: &Db,
    run_id: i64,
    now: DateTime<Utc>,
    meter: &mut Meter,
    usage: &mut Usage,
) -> Result<RunSummary, CoreError> {
    let interval = db
        .get_setting(INTERVAL_SETTING)?
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|d| *d > 0)
        .unwrap_or(DEFAULT_INTERVAL_DAYS);
    let due = db.due_items(now, interval)?;

    let mut summary = RunSummary {
        run_id,
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
    };
    if due.is_empty() {
        db.finish_run(&record(&summary, now, None, None))?;
        return Ok(summary);
    }

    let stav = client.stav_uctu()?;
    usage.provedeno = Some(stav.provedeno_volani);
    usage.limit = Some(stav.limit_volani);
    meter.budget = i64::from(stav.limit_volani)
        - i64::from(stav.provedeno_volani)
        - STAV_UCTU_RESERVE
        - SAFETY_MARGIN;

    let need_cis: HashSet<i64> = due
        .iter()
        .filter(|r| needs_ciselniky(db, r))
        .map(|r| r.item.id)
        .collect();
    let mut cis_error: Option<String> = None;
    // Not enough budget for all three lists with retries -> those jednotky wait.
    let cis_deferred = !need_cis.is_empty() && meter.remaining() < CISELNIKY_CALLS;
    if !need_cis.is_empty() && !cis_deferred {
        match ensure_ciselniky(client, db, now) {
            Ok(_) => {}
            Err(CoreError::InvalidApiKey) => return Err(CoreError::InvalidApiKey),
            Err(e) => {
                cis_error = Some(truncate(format!(
                    "Číselníky územních jednotek se nepodařilo stáhnout: {e}"
                )))
            }
        }
    }

    let mut outcomes = Vec::with_capacity(due.len());
    let mut aktualnost = None;
    let mut out_of_budget = false;
    for row in &due {
        let id = row.item.id;
        if !out_of_budget && meter.remaining() < MAX_CALLS_PER_ITEM {
            out_of_budget = true;
        }
        let mut o = if out_of_budget || (cis_deferred && need_cis.contains(&id)) {
            Outcome::new(id, ItemStatus::Deferred)
        } else if let (true, Some(msg)) = (need_cis.contains(&id), &cis_error) {
            Outcome::error(id, msg.clone())
        } else {
            let o = check_item(client, db, meter, row, &mut aktualnost)?;
            // A request did not fit: everything after this item waits too.
            out_of_budget |= o.status == ItemStatus::Deferred;
            o
        };
        o.item_key = item_key(&row.item.ident);
        outcomes.push(o);
    }

    summary.api_calls = meter.used();
    summary.api_used_today = stav.provedeno_volani.saturating_add(summary.api_calls);
    summary.api_limit = stav.limit_volani;
    summary.aktualnost_dat = aktualnost;

    db.transaction(|db| {
        // The UI may have deleted an item or changed its identification while it was being
        // checked: such outcomes are dropped (the new identity gets its own first check).
        let mut kept = Vec::with_capacity(outcomes.len());
        for o in outcomes {
            if db.item_key_is(o.item_id, &o.item_key)? {
                kept.push(o);
            }
        }
        tally(&mut summary, &kept);
        for o in &kept {
            db.insert_item_result(
                run_id,
                o.item_id,
                o.status.as_str(),
                o.changes_json.as_deref(),
                o.error.as_deref(),
            )?;
            if let Some(data) = &o.new_data {
                db.set_item_success(o.item_id, data, now)?;
            }
            if let Some(err) = &o.resolve_error {
                db.set_resolve_error(o.item_id, err.clone())?;
            }
        }
        db.finish_run(&record(
            &summary,
            now,
            Some(summary.api_used_today),
            Some(summary.api_limit),
        ))
    })?;
    Ok(summary)
}

/// Item counts, new plomby and run status from the outcomes that are recorded.
fn tally(summary: &mut RunSummary, outcomes: &[Outcome]) {
    for o in outcomes {
        match o.status {
            ItemStatus::First | ItemStatus::Unchanged => summary.checked += 1,
            ItemStatus::Changed => {
                summary.checked += 1;
                summary.changed += 1;
            }
            ItemStatus::Error => summary.errors += 1,
            ItemStatus::Deferred => summary.deferred += 1,
        }
        summary.plomby_nove += o.plomby_nove;
    }
    summary.status = if summary.deferred > 0 {
        RunStatus::Partial
    } else {
        RunStatus::Ok
    };
}

fn record(
    s: &RunSummary,
    now: DateTime<Utc>,
    api_used_today: Option<u32>,
    api_limit: Option<u32>,
) -> RunRecord {
    RunRecord {
        id: s.run_id,
        finished_at: now,
        status: s.status.as_str().into(),
        checked: s.checked,
        changed: s.changed,
        errors: s.errors,
        deferred: s.deferred,
        api_calls: s.api_calls,
        api_used_today,
        api_limit,
        aktualnost_dat: s.aktualnost_dat.clone(),
        error: s.error.clone(),
    }
}
