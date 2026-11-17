use rusqlite::Connection;

/// Ordered migrations; migration `i` brings `PRAGMA user_version` to `i + 1`.
const MIGRATIONS: &[&str] = &[V1];

const V1: &str = "
CREATE TABLE items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    popis TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    ident_json TEXT NOT NULL,
    item_key TEXT NOT NULL,
    last_data TEXT NULL,
    last_success_at TEXT NULL,
    resolve_error TEXT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE resolved (
    item_id INTEGER PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
    item_key TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    iskn_id INTEGER NOT NULL
);
CREATE TABLE ku (
    kod INTEGER PRIMARY KEY,
    nazev TEXT NOT NULL,
    nazev_norm TEXT NOT NULL,
    kod_obce INTEGER NOT NULL
);
CREATE TABLE obce (kod INTEGER PRIMARY KEY, nazev TEXT NOT NULL);
CREATE TABLE casti_obci (
    kod INTEGER PRIMARY KEY,
    nazev TEXT NOT NULL,
    kod_obce INTEGER NOT NULL
);
CREATE TABLE ciselniky_meta (name TEXT PRIMARY KEY, fetched_at TEXT NOT NULL);
CREATE TABLE runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at TEXT NOT NULL,
    finished_at TEXT NULL,
    status TEXT NOT NULL,
    trigger TEXT NOT NULL,
    checked INTEGER NOT NULL DEFAULT 0,
    changed INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0,
    deferred INTEGER NOT NULL DEFAULT 0,
    api_calls INTEGER NOT NULL DEFAULT 0,
    api_used_today INTEGER NULL,
    api_limit INTEGER NULL,
    aktualnost_dat TEXT NULL,
    error TEXT NULL
);
CREATE TABLE item_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    changes_json TEXT NULL,
    error TEXT NULL,
    seen INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_item_results_item ON item_results(item_id);
CREATE INDEX idx_item_results_run ON item_results(run_id);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// Applies all pending migrations, each in its own transaction. Idempotent.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let current = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        conn.execute_batch(&format!(
            "BEGIN; {sql} PRAGMA user_version = {}; COMMIT;",
            i + 1
        ))?;
    }
    Ok(())
}
