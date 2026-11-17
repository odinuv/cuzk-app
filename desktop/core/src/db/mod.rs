//! SQLite storage. `Result` is `Result<T, CoreError>`; rusqlite errors map to `CoreError::Db`.
//!
//! All methods take `&self`. To run several writes atomically use [`Db::transaction`]; the
//! closure receives the same `Db`, and everything is rolled back if it returns `Err`
//! (transactions do not nest).

mod migrations;

use std::path::Path;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::model::{describe, item_key, Item, ItemIdent};

pub type Result<T> = std::result::Result<T, CoreError>;

fn db_err(e: rusqlite::Error) -> CoreError {
    CoreError::Db(e.to_string())
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| CoreError::Db(format!("neplatné datum '{s}': {e}")))
}

fn parse_opt_ts(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    s.as_deref().map(parse_ts).transpose()
}

/// Lowercase with Czech diacritics stripped (used for `ku.nazev_norm` and search queries).
pub fn normalize_cz(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'ä' => 'a',
            'č' => 'c',
            'ď' => 'd',
            'é' | 'ě' | 'ë' => 'e',
            'í' => 'i',
            'ň' => 'n',
            'ó' | 'ö' => 'o',
            'ř' => 'r',
            'š' => 's',
            'ť' => 't',
            'ú' | 'ů' | 'ü' => 'u',
            'ý' => 'y',
            'ž' => 'z',
            other => other,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewItem {
    pub popis: String,
    pub enabled: bool,
    pub ident: ItemIdent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemRow {
    pub item: Item,
    /// JSON snapshot of the last successful check.
    pub last_data: Option<String>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub resolve_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolved {
    pub item_key: String,
    pub endpoint: String,
    pub iskn_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: i64,
    pub finished_at: DateTime<Utc>,
    pub status: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRow {
    pub id: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingRow {
    pub result_id: i64,
    pub run_id: i64,
    pub item_id: i64,
    pub popis: String,
    pub describe: String,
    pub status: String,
    pub changes_json: Option<String>,
    pub error: Option<String>,
    pub run_finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemResultRow {
    pub id: i64,
    pub run_id: i64,
    pub item_id: i64,
    pub status: String,
    pub changes_json: Option<String>,
    pub error: Option<String>,
    pub seen: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KuOption {
    pub kod: u32,
    pub nazev: String,
    pub obec: String,
}

pub struct Db(Connection);

const ITEM_COLS: &str = "id, popis, enabled, ident_json, last_data, last_success_at, resolve_error";

fn item_row(r: &Row) -> rusqlite::Result<ItemRowRaw> {
    let ident_json: String = r.get(3)?;
    let ident: ItemIdent = serde_json::from_str(&ident_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(ItemRowRaw {
        item: Item {
            id: r.get(0)?,
            popis: r.get(1)?,
            enabled: r.get::<_, i64>(2)? != 0,
            ident,
        },
        last_data: r.get(4)?,
        last_success_at: r.get(5)?,
        resolve_error: r.get(6)?,
    })
}

struct ItemRowRaw {
    item: Item,
    last_data: Option<String>,
    last_success_at: Option<String>,
    resolve_error: Option<String>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        Self::init(Connection::open(path).map_err(db_err)?)
    }

    pub fn open_in_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory().map_err(db_err)?)
    }

    fn init(conn: Connection) -> Result<Db> {
        // journal_mode returns a row; in-memory databases just report "memory".
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
            .map_err(db_err)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(db_err)?;
        migrations::migrate(&conn).map_err(db_err)?;
        Ok(Db(conn))
    }

    /// Runs `f` inside one transaction: commit on `Ok`, roll back on `Err`, on a failed
    /// `COMMIT` and when `f` panics, so the connection never stays inside a transaction.
    pub fn transaction<T>(&self, f: impl FnOnce(&Db) -> Result<T>) -> Result<T> {
        struct RollbackGuard<'a>(Option<&'a Connection>);
        impl Drop for RollbackGuard<'_> {
            fn drop(&mut self) {
                if let Some(conn) = self.0 {
                    if !conn.is_autocommit() {
                        let _ = conn.execute_batch("ROLLBACK");
                    }
                }
            }
        }

        self.0.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;
        let mut guard = RollbackGuard(Some(&self.0));
        let v = f(self)?;
        self.0.execute_batch("COMMIT").map_err(db_err)?;
        guard.0 = None;
        Ok(v)
    }

    // ---- items ----

    pub fn insert_item(&self, n: &NewItem) -> Result<i64> {
        let ident_json =
            serde_json::to_string(&n.ident).map_err(|e| CoreError::Db(e.to_string()))?;
        self.0
            .execute(
                "INSERT INTO items (popis, enabled, ident_json, item_key, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    n.popis,
                    n.enabled,
                    ident_json,
                    item_key(&n.ident),
                    ts(Utc::now())
                ],
            )
            .map_err(db_err)?;
        Ok(self.0.last_insert_rowid())
    }

    /// Updates popis/enabled/ident. If the identity (`item_key`) changed, also clears
    /// `resolved`, `last_data`, `last_success_at` and `resolve_error` (history rows stay).
    pub fn update_item(&self, item: &Item) -> Result<()> {
        let new_key = item_key(&item.ident);
        let ident_json =
            serde_json::to_string(&item.ident).map_err(|e| CoreError::Db(e.to_string()))?;
        self.transaction(|db| {
            let old_key: Option<String> = db
                .0
                .query_row("SELECT item_key FROM items WHERE id = ?1", [item.id], |r| r.get(0))
                .optional()
                .map_err(db_err)?;
            let Some(old_key) = old_key else { return Err(CoreError::NotFound) };
            db.0.execute(
                "UPDATE items SET popis = ?2, enabled = ?3, ident_json = ?4, item_key = ?5 WHERE id = ?1",
                params![item.id, item.popis, item.enabled, ident_json, new_key],
            )
            .map_err(db_err)?;
            if old_key != new_key {
                db.0.execute(
                    "UPDATE items SET last_data = NULL, last_success_at = NULL, resolve_error = NULL WHERE id = ?1",
                    [item.id],
                )
                .map_err(db_err)?;
                db.0.execute("DELETE FROM resolved WHERE item_id = ?1", [item.id])
                    .map_err(db_err)?;
            }
            Ok(())
        })
    }

    pub fn delete_item(&self, id: i64) -> Result<()> {
        self.0
            .execute("DELETE FROM items WHERE id = ?1", [id])
            .map_err(db_err)?;
        Ok(())
    }

    pub fn list_items(&self) -> Result<Vec<ItemRow>> {
        self.query_items(&format!("SELECT {ITEM_COLS} FROM items ORDER BY id"), [])
    }

    /// Enabled items never checked or last checked at or before `now - interval_days`;
    /// never-checked first, then oldest first, then by id.
    pub fn due_items(&self, now: DateTime<Utc>, interval_days: u32) -> Result<Vec<ItemRow>> {
        let cutoff = ts(now - Duration::days(interval_days as i64));
        self.query_items(
            &format!(
                "SELECT {ITEM_COLS} FROM items
                 WHERE enabled = 1 AND (last_success_at IS NULL OR last_success_at <= ?1)
                 ORDER BY last_success_at IS NOT NULL, last_success_at ASC, id ASC"
            ),
            [cutoff],
        )
    }

    fn query_items<P: rusqlite::Params>(&self, sql: &str, p: P) -> Result<Vec<ItemRow>> {
        let mut st = self.0.prepare(sql).map_err(db_err)?;
        let rows = st.query_map(p, item_row).map_err(db_err)?;
        rows.map(|r| {
            let r = r.map_err(db_err)?;
            Ok(ItemRow {
                item: r.item,
                last_data: r.last_data,
                last_success_at: parse_opt_ts(r.last_success_at)?,
                resolve_error: r.resolve_error,
            })
        })
        .collect()
    }

    pub fn set_item_success(&self, id: i64, data_json: &str, at: DateTime<Utc>) -> Result<()> {
        self.0
            .execute(
                "UPDATE items SET last_data = ?2, last_success_at = ?3 WHERE id = ?1",
                params![id, data_json, ts(at)],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn set_resolve_error(&self, id: i64, err: Option<String>) -> Result<()> {
        self.0
            .execute(
                "UPDATE items SET resolve_error = ?2 WHERE id = ?1",
                params![id, err],
            )
            .map_err(db_err)?;
        Ok(())
    }

    // ---- resolved ----

    pub fn get_resolved(&self, item_id: i64) -> Result<Option<Resolved>> {
        self.0
            .query_row(
                "SELECT item_key, endpoint, iskn_id FROM resolved WHERE item_id = ?1",
                [item_id],
                |r| {
                    Ok(Resolved {
                        item_key: r.get(0)?,
                        endpoint: r.get(1)?,
                        iskn_id: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(db_err)
    }

    pub fn set_resolved(&self, item_id: i64, r: &Resolved) -> Result<()> {
        self.0
            .execute(
                "INSERT OR REPLACE INTO resolved (item_id, item_key, endpoint, iskn_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![item_id, r.item_key, r.endpoint, r.iskn_id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn clear_resolved(&self, item_id: i64) -> Result<()> {
        self.0
            .execute("DELETE FROM resolved WHERE item_id = ?1", [item_id])
            .map_err(db_err)?;
        Ok(())
    }

    // ---- runs ----

    pub fn begin_run(&self, trigger: &str, now: DateTime<Utc>) -> Result<i64> {
        self.0
            .execute(
                "INSERT INTO runs (started_at, status, trigger) VALUES (?1, 'running', ?2)",
                params![ts(now), trigger],
            )
            .map_err(db_err)?;
        Ok(self.0.last_insert_rowid())
    }

    pub fn finish_run(&self, r: &RunRecord) -> Result<()> {
        self.0
            .execute(
                "UPDATE runs SET finished_at = ?2, status = ?3, checked = ?4, changed = ?5,
                 errors = ?6, deferred = ?7, api_calls = ?8, api_used_today = ?9, api_limit = ?10,
                 aktualnost_dat = ?11, error = ?12 WHERE id = ?1",
                params![
                    r.id,
                    ts(r.finished_at),
                    r.status,
                    r.checked,
                    r.changed,
                    r.errors,
                    r.deferred,
                    r.api_calls,
                    r.api_used_today,
                    r.api_limit,
                    r.aktualnost_dat,
                    r.error
                ],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// `status`: `first`, `unchanged`, `changed`, `error` or `deferred`.
    pub fn insert_item_result(
        &self,
        run_id: i64,
        item_id: i64,
        status: &str,
        changes_json: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        self.0
            .execute(
                "INSERT INTO item_results (run_id, item_id, status, changes_json, error)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![run_id, item_id, status, changes_json, error],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Results of one run in insertion order.
    pub fn item_results_for_run(&self, run_id: i64) -> Result<Vec<ItemResultRow>> {
        let mut st = self
            .0
            .prepare(
                "SELECT id, run_id, item_id, status, changes_json, error, seen
                 FROM item_results WHERE run_id = ?1 ORDER BY id",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([run_id], |r| {
                Ok(ItemResultRow {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    item_id: r.get(2)?,
                    status: r.get(3)?,
                    changes_json: r.get(4)?,
                    error: r.get(5)?,
                    seen: r.get::<_, i64>(6)? != 0,
                })
            })
            .map_err(db_err)?;
        rows.map(|r| r.map_err(db_err)).collect()
    }

    pub fn count_item_results(&self) -> Result<i64> {
        self.0
            .query_row("SELECT COUNT(*) FROM item_results", [], |r| r.get(0))
            .map_err(db_err)
    }

    /// Newest run first.
    pub fn recent_runs(&self, limit: u32) -> Result<Vec<RunRow>> {
        self.query_runs("1 = 1", limit, [])
    }

    /// The newest run started by `trigger` (`manual` | `scheduled`).
    pub fn last_run_by_trigger(&self, trigger: &str) -> Result<Option<RunRow>> {
        self.query_runs("trigger = ?2", 1, [trigger])
            .map(|mut v| v.pop())
    }

    /// Runs matching `cond` (SQL over `runs`; `?1` is the limit, extra params from `?2`),
    /// newest first.
    fn query_runs<const N: usize>(
        &self,
        cond: &str,
        limit: u32,
        extra: [&str; N],
    ) -> Result<Vec<RunRow>> {
        let sql = format!(
            "SELECT id, started_at, finished_at, status, trigger, checked, changed, errors,
                    deferred, api_calls, api_used_today, api_limit, aktualnost_dat, error
             FROM runs WHERE {cond} ORDER BY id DESC LIMIT ?1"
        );
        let mut st = self.0.prepare(&sql).map_err(db_err)?;
        let mut params: Vec<Value> = vec![Value::Integer(i64::from(limit))];
        params.extend(extra.iter().map(|s| Value::Text((*s).to_string())));
        let rows = st
            .query_map(rusqlite::params_from_iter(params), |r| {
                Ok((
                    RunRow {
                        id: r.get(0)?,
                        started_at: DateTime::<Utc>::MIN_UTC,
                        finished_at: None,
                        status: r.get(3)?,
                        trigger: r.get(4)?,
                        checked: r.get(5)?,
                        changed: r.get(6)?,
                        errors: r.get(7)?,
                        deferred: r.get(8)?,
                        api_calls: r.get(9)?,
                        api_used_today: r.get(10)?,
                        api_limit: r.get(11)?,
                        aktualnost_dat: r.get(12)?,
                        error: r.get(13)?,
                    },
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(db_err)?;
        rows.map(|r| {
            let (mut run, started, finished) = r.map_err(db_err)?;
            run.started_at = parse_ts(&started)?;
            run.finished_at = parse_opt_ts(finished)?;
            Ok(run)
        })
        .collect()
    }

    // ---- findings ----

    /// Unseen `changed` results, newest run first.
    pub fn unseen_findings(&self) -> Result<Vec<FindingRow>> {
        self.findings_where("r.seen = 0 AND r.status = 'changed'", None)
    }

    /// Every per-item result of one run (all statuses), in item order.
    pub fn run_findings(&self, run_id: i64) -> Result<Vec<FindingRow>> {
        self.findings_where("r.run_id = ?1", Some(run_id))
    }

    fn findings_where(&self, cond: &str, param: Option<i64>) -> Result<Vec<FindingRow>> {
        let sql = format!(
            "SELECT r.id, r.run_id, r.item_id, i.popis, i.ident_json, r.status,
                    r.changes_json, r.error, u.finished_at
             FROM item_results r
             JOIN items i ON i.id = r.item_id
             JOIN runs u ON u.id = r.run_id
             WHERE {cond}
             ORDER BY r.run_id DESC, r.id ASC"
        );
        let mut st = self.0.prepare(&sql).map_err(db_err)?;
        let map = |r: &rusqlite::Row| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        };
        let rows = match param {
            Some(p) => st.query_map([p], map),
            None => st.query_map([], map),
        }
        .map_err(db_err)?;
        rows.map(|r| {
            let (result_id, run_id, item_id, popis, ident_json, status, changes_json, error, fin) =
                r.map_err(db_err)?;
            let ident: ItemIdent =
                serde_json::from_str(&ident_json).map_err(|e| CoreError::Db(e.to_string()))?;
            Ok(FindingRow {
                result_id,
                run_id,
                item_id,
                popis,
                describe: describe(&ident),
                status,
                changes_json,
                error,
                run_finished_at: parse_opt_ts(fin)?,
            })
        })
        .collect()
    }

    pub fn mark_all_seen(&self) -> Result<()> {
        self.0
            .execute("UPDATE item_results SET seen = 1 WHERE seen = 0", [])
            .map_err(db_err)?;
        Ok(())
    }

    // ---- číselníky cache ----

    /// Replaces a whole cache table and stamps `ciselniky_meta`, in one transaction.
    fn replace_table<R>(
        &self,
        name: &str,
        now: DateTime<Utc>,
        rows: &[R],
        delete_sql: &str,
        insert_sql: &str,
        bind: impl Fn(&R) -> Vec<Value>,
    ) -> Result<()> {
        self.transaction(|db| {
            db.0.execute(delete_sql, []).map_err(db_err)?;
            let mut st = db.0.prepare(insert_sql).map_err(db_err)?;
            for r in rows {
                st.execute(rusqlite::params_from_iter(bind(r)))
                    .map_err(db_err)?;
            }
            db.0.execute(
                "INSERT OR REPLACE INTO ciselniky_meta (name, fetched_at) VALUES (?1, ?2)",
                params![name, ts(now)],
            )
            .map_err(db_err)?;
            Ok(())
        })
    }

    /// Rows are `(kod, nazev, kod_obce)`.
    pub fn replace_ku(&self, rows: &[(u32, String, u32)], now: DateTime<Utc>) -> Result<()> {
        self.replace_table(
            "ku",
            now,
            rows,
            "DELETE FROM ku",
            "INSERT INTO ku (kod, nazev, nazev_norm, kod_obce) VALUES (?1, ?2, ?3, ?4)",
            |(k, n, o)| {
                vec![
                    Value::Integer(*k as i64),
                    Value::Text(n.clone()),
                    Value::Text(normalize_cz(n)),
                    Value::Integer(*o as i64),
                ]
            },
        )
    }

    /// Rows are `(kod, nazev)`.
    pub fn replace_obce(&self, rows: &[(u32, String)], now: DateTime<Utc>) -> Result<()> {
        self.replace_table(
            "obce",
            now,
            rows,
            "DELETE FROM obce",
            "INSERT INTO obce (kod, nazev) VALUES (?1, ?2)",
            |(k, n)| vec![Value::Integer(*k as i64), Value::Text(n.clone())],
        )
    }

    /// Rows are `(kod, nazev, kod_obce)`.
    pub fn replace_casti_obci(
        &self,
        rows: &[(u32, String, u32)],
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.replace_table(
            "casti_obci",
            now,
            rows,
            "DELETE FROM casti_obci",
            "INSERT INTO casti_obci (kod, nazev, kod_obce) VALUES (?1, ?2, ?3)",
            |(k, n, o)| {
                vec![
                    Value::Integer(*k as i64),
                    Value::Text(n.clone()),
                    Value::Integer(*o as i64),
                ]
            },
        )
    }

    /// When the číselník (`ku`, `obce`, `casti_obci`) was last fetched.
    pub fn ciselnik_age(&self, name: &str) -> Result<Option<DateTime<Utc>>> {
        let s: Option<String> = self
            .0
            .query_row(
                "SELECT fetched_at FROM ciselniky_meta WHERE name = ?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?;
        parse_opt_ts(s)
    }

    /// Case- and diacritics-insensitive substring search; prefix matches first, then
    /// other substring matches, then alphabetical.
    pub fn search_ku(&self, q: &str, limit: u32) -> Result<Vec<KuOption>> {
        let norm = normalize_cz(q.trim());
        let esc = norm
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let mut st = self
            .0
            .prepare(
                "SELECT k.kod, k.nazev, COALESCE(o.nazev, '')
                 FROM ku k LEFT JOIN obce o ON o.kod = k.kod_obce
                 WHERE k.nazev_norm LIKE ?1 ESCAPE '\\'
                 ORDER BY (k.nazev_norm LIKE ?2 ESCAPE '\\') DESC, k.nazev_norm ASC, k.kod ASC
                 LIMIT ?3",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![format!("%{esc}%"), format!("{esc}%"), limit], |r| {
                Ok(KuOption {
                    kod: r.get(0)?,
                    nazev: r.get(1)?,
                    obec: r.get(2)?,
                })
            })
            .map_err(db_err)?;
        rows.map(|r| r.map_err(db_err)).collect()
    }

    /// `(nazev, kod_obce)` of a katastrální území.
    pub fn get_ku(&self, kod: u32) -> Result<Option<(String, u32)>> {
        self.0
            .query_row(
                "SELECT nazev, kod_obce FROM ku WHERE kod = ?1",
                [kod],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_err)
    }

    /// `(kod, nazev)` of části obce belonging to an obec.
    pub fn casti_obci_for_obec(&self, kod_obce: u32) -> Result<Vec<(u32, String)>> {
        let mut st = self
            .0
            .prepare("SELECT kod, nazev FROM casti_obci WHERE kod_obce = ?1 ORDER BY kod")
            .map_err(db_err)?;
        let rows = st
            .query_map([kod_obce], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(db_err)?;
        rows.map(|r| r.map_err(db_err)).collect()
    }

    // ---- settings ----

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        self.0
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_err)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.0
            .execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .map_err(db_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setting_count(db: &Db) -> i64 {
        db.0.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn transaction_rolls_back_on_err() {
        let db = Db::open_in_memory().unwrap();
        let r: Result<()> = db.transaction(|db| {
            db.set_setting("a", "1")?;
            Err(CoreError::NotFound)
        });
        assert_eq!(r, Err(CoreError::NotFound));
        assert_eq!(setting_count(&db), 0);
        assert!(db.0.is_autocommit());
    }

    #[test]
    fn transaction_rolls_back_on_panic() {
        let db = Db::open_in_memory().unwrap();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<()> = db.transaction(|db| {
                db.set_setting("a", "1")?;
                panic!("boom");
            });
        }));
        assert!(r.is_err());
        assert!(db.0.is_autocommit(), "connection left inside a transaction");
        assert_eq!(setting_count(&db), 0);
        db.transaction(|db| db.set_setting("b", "2")).unwrap();
        assert_eq!(setting_count(&db), 1);
    }

    #[test]
    fn transaction_rolls_back_when_commit_fails() {
        let db = Db::open_in_memory().unwrap();
        let r = db.transaction(|db| {
            // Deferred FK check makes COMMIT itself fail.
            db.0.execute_batch("PRAGMA defer_foreign_keys = ON")
                .map_err(db_err)?;
            db.set_setting("a", "1")?;
            db.insert_item_result(999, 999, "first", None, None)
        });
        assert!(matches!(r, Err(CoreError::Db(_))), "{r:?}");
        assert!(db.0.is_autocommit(), "connection left inside a transaction");
        assert_eq!(setting_count(&db), 0);
        db.transaction(|db| db.set_setting("b", "2")).unwrap();
    }
}
