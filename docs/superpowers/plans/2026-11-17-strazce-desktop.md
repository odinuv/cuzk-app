# Strážce katastru – desktop (Tauri) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Windows desktop app (Tauri 2) where a user enters their ČÚZK API key and their properties, and the app checks them against REST API KN, shows changes (plomby first), runs headless from the OS scheduler, and keeps everything in local SQLite.

**Architecture:** A Rust library crate `strazce-core` holds everything that matters: the API client, ISKN id resolution, diff, SQLite storage, and the check run. It has no Tauri dependency and is tested with a mock HTTP transport and in-memory SQLite. One binary `strazce` either opens the Tauri UI or, with `--check`, runs the check headless and exits. The Svelte UI only calls Tauri commands, which are thin wrappers over core.

**Tech Stack:** Rust stable (MSVC), Tauri 2, `reqwest` (blocking, rustls), `rusqlite` (bundled), `serde`/`serde_json`, `thiserror`, `keyring`, `fs4` (file lock), `notify-rust`, `dirs`, `chrono`. Frontend: Vite + Svelte 5 + TypeScript. Installer: Tauri bundler NSIS.

**Spec:** `docs/superpowers/specs/2026-11-17-strazce-desktop-design.md`. The behaviour reference is the PoC in `function/src/` (`resolve.ts`, `diff.ts`, `run.ts`, `cuzk/client.ts`). Read it for API shapes and edge cases; do not port it line by line.

## Global Constraints

- Code lives in `desktop/`: Cargo workspace `desktop/Cargo.toml` with members `core` (crate `strazce-core`) and `src-tauri` (crate and binary `strazce`). The frontend is in `desktop/src/` with `desktop/package.json`.
- **Build outside OneDrive.** Windows Defender Controlled Folder Access blocks `cargo`/`node` writes under `D:\OneDrive`. Execute in a git worktree outside OneDrive (e.g. the session scratchpad).
- Prerequisites: Rust stable via rustup (`x86_64-pc-windows-msvc`); MSVC is installed (VS 18 Community). Node 22 for the frontend. Tauri CLI comes from npm `@tauri-apps/cli@^2`.
- Tauri identifier: `cz.strazce-katastru.app`. Product name: `Strážce katastru`.
- API base URL: `https://api-kn.cuzk.gov.cz/api/v1`. Auth header: `ApiKey: <key>`. Daily limit: 500 calls per key.
- Response envelope: `{data, zpravy, aktualnostDatK, provedenoVolani}`. Exception: `GET /AplikacniSluzby/StavUctu` returns the flat body `{idSluzby, aktualniObdobi, provedenoVolani, limitVolani, expiraceApiKey}`. Its `provedenoVolani` does not include the StavUctu call itself.
- Keyring entry: service `strazce-katastru`, user `api-key`. The API key never goes into SQLite, logs or error messages.
- DB file: `<dirs::data_dir()>/cz.strazce-katastru.app/strazce.db`. Lock file: `strazce.lock` in the same directory. Only `strazce_core::paths::data_dir()` computes this path; the UI and `--check` both use it.
- Safety margin: 5 calls kept in reserve below the limit. Default interval: 14 days. Allowed intervals: 7, 14, 30 days. Default schedule time: `08:00`.
- Windows scheduled task name: `StrazceKatastru`. It runs daily at the chosen time and executes `"<exe path>" --check`.
- All user-facing text is in Czech. Commit messages use conventional commits and carry **no** Claude/Anthropic co-author trailer.

## Review Focus

1. **API outage mid-run** (5xx or network error after retries for some items): those items keep their previous `last_data`, get status `error`, stay due, and produce no field changes. Test in Task 6.
2. **Budget runs out mid-run**: the remaining due items are `deferred`, not `error`. The run status is `partial`. The next run picks the deferred items first (oldest `last_success_at` first, never-checked before everything). Test in Task 6.
3. **Second run while one is in progress** (scheduler fires during a manual check): it fails fast with `CoreError::AlreadyRunning` and touches nothing. Test in Task 6.
4. **Headless run with a missing or rejected key** (no keyring entry, or HTTP 401/403 from StavUctu): no item is touched, the notification says the key is the problem, and the exit code is 2. Test in Task 7.
5. **Ambiguous or missing část obce for a jednotka at save time**: the save still persists the item, and the item view carries a Czech error that tells the user to pick the část obce manually (`kod_casti_obce` override). Test in Task 4.

---

### Task 1: Workspace scaffold and domain model

**Files:**
- Create: `desktop/Cargo.toml` (workspace), `desktop/core/Cargo.toml`, `desktop/core/src/lib.rs`, `desktop/core/src/model.rs`, `desktop/core/src/paths.rs`, `desktop/core/src/error.rs`, `desktop/.gitignore` (`target/`, `node_modules/`, `dist/`)
- Test: `desktop/core/src/model.rs` (unit tests module)

**Interfaces:**
- Produces:
  - `enum DruhCislovani { Stavebni, Pozemkova }` with API code `1` and `2` (`fn api_code(&self) -> u8`).
  - `enum TypStavby { Cp, Ce }` with API code `1` and `2`.
  - `struct ParcelaRef { druh: DruhCislovani, kmenove: u32, poddeleni: Option<u32> }`
  - `enum ItemIdent { Parcela { ku: u32, parcela: ParcelaRef }, Stavba { ku: u32, typ: TypStavby, cislo_domovni: u32, na_parcele: ParcelaRef }, Jednotka { ku: u32, typ: TypStavby, cislo_domovni: u32, cislo_jednotky: u32, kod_casti_obce: Option<u32> } }`
  - `struct Item { id: i64, popis: String, enabled: bool, ident: ItemIdent }`
  - `fn item_key(ident: &ItemIdent) -> String`: a stable fingerprint of the identification only.
  - `fn describe(ident: &ItemIdent) -> String`, for example `"parcela st. 182/2, k.ú. 618853"`, `"stavba č.e. 3 na p.č. 110/10, k.ú. 738395"`, `"jednotka 893/37, k.ú. 610704"`.
  - `fn validate(ident: &ItemIdent) -> Result<(), String>`: Czech message on failure.
  - `paths::data_dir() -> PathBuf`, `paths::db_path()`, `paths::lock_path()`.
  - `enum CoreError` (thiserror) with variants `Http { status: u16, body: String }`, `Network(String)`, `NotFound`, `Resolve(String)`, `Db(String)`, `AlreadyRunning`, `NoApiKey`, `InvalidApiKey`, `Budget`.
  - All of the above derive `Serialize`/`Deserialize` where they cross into the UI.

- [ ] **Step 1: Write failing tests.** Cover:
  - `item_key` equals for idents that differ only in popis/enabled, and differs when `poddeleni` changes from `None` to `Some(0)`;
  - the three `describe` examples above;
  - `validate` rejects `kmenove == 0` and `cislo_domovni == 0`.
- [ ] **Step 2: Run** `cargo test -p strazce-core`. Expected: compile failure / FAIL.
- [ ] **Step 3: Implement** the model, paths and error types. `item_key` is `serde_json::to_string` of the `ItemIdent`.
- [ ] **Step 4: Run** `cargo test -p strazce-core`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): workspace scaffold and domain model`.

### Task 2: API client

**Files:**
- Create: `desktop/core/src/api/mod.rs`, `desktop/core/src/api/transport.rs`, `desktop/core/src/api/types.rs`
- Test: `desktop/core/tests/api_client.rs`

**Interfaces:**
- Consumes: `CoreError`.
- Produces:
  - `trait Transport: Send + Sync { fn get(&self, url: &str, api_key: &str) -> Result<(u16, String), CoreError>; }`. `Err` is used only for network errors and timeouts.
  - `struct ReqwestTransport`, with a 30 s timeout.
  - `struct MockTransport`, under `#[cfg(any(test, feature = "test-util"))]`. It routes by path prefix to queued `(status, body)` responses and records the requested URLs.
  - `struct ApiClient { /* transport, key, base_url, calls: AtomicU32 */ }` with these methods:
    - `fn new(transport: Arc<dyn Transport>, api_key: String) -> Self`
    - `fn calls(&self) -> u32`: counts every HTTP attempt, retries included.
    - `fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<Envelope<T>, CoreError>`
    - `fn stav_uctu(&self) -> Result<StavUctu, CoreError>`
  - `struct Envelope<T> { data: T, aktualnost_dat_k: Option<String>, provedeno_volani: Option<u32> }` (serde camelCase).
  - `struct StavUctu { provedeno_volani: u32, limit_volani: u32, expirace_api_key: Option<String> }`.
  - `struct RizeniDef { id: i64, typ_rizeni: String, poradove_cislo: u32, rok: u32, kod_pracoviste: u32 }`, with `fn label(&self) -> String` returning `"V-1234/2026-701"`.
  - Errors: 404 maps to `CoreError::NotFound`, 401/403 to `CoreError::InvalidApiKey`, any other non-2xx to `CoreError::Http`.
  - Retries: on 429, 5xx and network errors, at most 2 retries with delays of `retry_delay * 2^attempt`. `retry_delay` is configurable; tests use 0.

- [ ] **Step 1: Write failing tests:**
  - `envelope_parsed`: data and `provedeno_volani` are read.
  - `stav_uctu_flat`: the flat body parses to limit 500.
  - `retries_then_succeeds`: 503, 503, 200 gives Ok with `calls() == 3`.
  - `gives_up_after_two_retries`: three 500s give `Http{status:500}`.
  - `not_found_no_retry`: 404 gives `NotFound` with `calls() == 1`.
  - `unauthorized_maps_invalid_key`: 401 gives `InvalidApiKey`.
  - `sends_api_key_header_and_query`: the mock sees the key and `KodKatastralnihoUzemi=610771` in the URL.
  - `rizeni_label`: `"V-1234/2026-701"`.
- [ ] **Step 2: Run** `cargo test -p strazce-core --test api_client`. Expected: FAIL.
- [ ] **Step 3: Implement.** `ReqwestTransport` uses `reqwest::blocking`.
- [ ] **Step 4: Run** the tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(core): ČÚZK API client with retry and call counting`.

### Task 3: SQLite storage

**Files:**
- Create: `desktop/core/src/db/mod.rs`, `desktop/core/src/db/migrations.rs`
- Test: `desktop/core/tests/db.rs`

**Interfaces:**
- Consumes: `Item`, `ItemIdent`, `item_key`.
- Produces: `struct Db(rusqlite::Connection)` with `fn open(path: &Path) -> Result<Db>` and `fn open_in_memory() -> Result<Db>`. Migrations go through `PRAGMA user_version` (v1 is the schema below). WAL mode and foreign keys are on.
- Tables (v1):
  - `items(id INTEGER PK, popis TEXT, enabled INT, ident_json TEXT, item_key TEXT, last_data TEXT NULL, last_success_at TEXT NULL, resolve_error TEXT NULL, created_at TEXT)`
  - `resolved(item_id PK→items ON DELETE CASCADE, item_key TEXT, endpoint TEXT, iskn_id INTEGER)`
  - `ku(kod PK, nazev TEXT, nazev_norm TEXT, kod_obce INT)`, `obce(kod PK, nazev TEXT)`, `casti_obci(kod PK, nazev TEXT, kod_obce INT)`
  - `ciselniky_meta(name PK, fetched_at TEXT)`
  - `runs(id PK, started_at, finished_at, status TEXT, trigger TEXT, checked INT, changed INT, errors INT, deferred INT, api_calls INT, api_used_today INT NULL, api_limit INT NULL, aktualnost_dat TEXT NULL, error TEXT NULL)`
  - `item_results(id PK, run_id→runs ON DELETE CASCADE, item_id→items ON DELETE CASCADE, status TEXT, changes_json TEXT NULL, error TEXT NULL, seen INT DEFAULT 0)`
  - `settings(key PK, value TEXT)`
- Methods:
  - `insert_item(&NewItem) -> i64`, `update_item(&Item)`, `delete_item(id)`, `list_items() -> Vec<ItemRow>`
  - `due_items(now, interval_days) -> Vec<ItemRow>`: enabled items where `last_success_at` is NULL or `<= now - interval`. Ordered with NULLs first, then by `last_success_at` ascending, then by id.
  - `get_resolved / set_resolved / clear_resolved`
  - `begin_run(trigger) -> i64`, `finish_run(&RunRecord)`, `insert_item_result(...)`, `set_item_success(id, data_json, at)`, `set_resolve_error(id, Option<String>)`
  - `unseen_findings() -> Vec<FindingRow>`, `mark_all_seen()`, `recent_runs(limit)`
  - ku cache: `replace_ku`, `replace_obce`, `replace_casti_obci`, `search_ku(q, limit) -> Vec<KuOption{kod,nazev,obec}>` (diacritics- and case-insensitive substring match on `nazev_norm`, prefix matches first), `ciselnik_age(name)`
  - `get_setting(key) -> Option<String>`, `set_setting(key, value)`
- Rule: `update_item` with a changed `item_key` clears `resolved`, `last_data`, `last_success_at` and `resolve_error`. That makes the item due and its next result `first`; history rows stay.

- [ ] **Step 1: Write failing tests:**
  - `migrate_twice_is_noop`
  - `due_order_never_checked_first_then_oldest`
  - `due_excludes_disabled_and_fresh`: with a 14-day interval, an item checked 13 days ago is excluded and one checked 14 days ago is included.
  - `identity_change_resets_state_keeps_history`
  - `delete_item_cascades_results`
  - `search_ku_ignores_diacritics`: "zabrd" finds "Zábrdovice".
  - `unseen_findings_then_mark_seen`
- [ ] **Step 2–4:** run `cargo test -p strazce-core --test db` and watch it FAIL, implement, then run it again to PASS.
- [ ] **Step 5: Commit** `feat(core): SQLite storage with migrations`.

### Task 4: Resolution of ISKN ids and číselník cache

**Files:**
- Create: `desktop/core/src/resolve.rs`
- Test: `desktop/core/tests/resolve.rs`

**Interfaces:**
- Consumes: `ApiClient`, `Db`, `ItemIdent`.
- Produces:
  - `enum Endpoint { Parcely, Stavby, Jednotky }`.
  - `fn ensure_ciselniky(client, db, now) -> Result<u32 /*calls*/>`: re-fetches `GET /CiselnikyUzemnichJednotek/KatastralniUzemi`, `/Obce` and `/CastiObci` when the stored copy is missing or older than 30 days. That costs 3 calls.
  - `fn resolve(client, db, item: &ItemRow) -> Result<(Endpoint, i64), CoreError>`: returns the cached result when `resolved.item_key` matches, otherwise looks the id up and caches it.
- Lookup rules (from the PoC):
  - **parcela:** `GET /Parcely/Vyhledani` with `KodKatastralnihoUzemi`, `TypParcely=PKN`, `DruhCislovaniParcely` (1/2), `KmenoveCisloParcely` and `PoddeleniCislaParcely` (sent only when `Some`). Exactly one result is required.
  - **stavba:** look up the parcel the same way and take `data[0].stavba.id`. Its `cislaDomovni` must contain `cislo_domovni`.
  - **jednotka:** if no override is set, take the část obce from the cached `casti_obci` whose `kod_obce` equals the k.ú.'s `kod_obce` and whose `nazev` equals the k.ú. `nazev`. Then call `GET /Jednotky/Vyhledani` with `KodCastiObce`, `TypStavby`, `CisloDomovni` and `CisloJednotky`.
- Errors are `CoreError::Resolve` with Czech text:
  - zero matches: `"Objekt nenalezen v katastru – zkontrolujte zadání."`
  - ambiguous část obce: `"Nepodařilo se jednoznačně určit část obce – vyberte ji ručně."`

- [ ] **Step 1: Write failing tests** with `MockTransport` and an in-memory DB:
  - `parcela_with_and_without_poddeleni`: the query omits `PoddeleniCislaParcely` when it is `None`.
  - `stavba_via_parcel_checks_cislo_domovni`
  - `jednotka_picks_cast_obce_by_name`: Brno k.ú. Ponava yields část obce `411973`.
  - `jednotka_ambiguous_cast_obce_errors` (Review Focus 5)
  - `jednotka_override_skips_ciselnik`
  - `cache_hit_makes_no_calls`
  - `ciselniky_refetched_only_when_stale`
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5: Commit** `feat(core): resolve ISKN ids with číselník cache`.

### Task 5: Normalization and diff

**Files:**
- Create: `desktop/core/src/diff.rs`
- Test: unit tests in the same file

**Interfaces:**
- Produces:
  - `struct FieldChange { path: String, before: Option<Value>, after: Option<Value> }`.
  - `fn normalize(v: &Value) -> Value`: sorts object keys, and sorts arrays of objects deterministically by their serialized form.
  - `fn diff_object(before: &Value, after: &Value) -> Vec<FieldChange>`: dot paths, arrays compared as whole values, top-level `rizeniPlomby` excluded, sorted by path.
  - `fn diff_plomby(before: &[RizeniDef], after: &[RizeniDef]) -> (Vec<RizeniDef> /*nove*/, Vec<RizeniDef> /*zanikle*/)`, matched by `id`.

- [ ] **Step 1: Write failing tests:**
  - `reordered_array_is_no_change`
  - `nested_field_change_has_dot_path`: for example `"zpusobVyuziti.nazev"`.
  - `added_and_removed_fields`
  - `plomby_not_in_field_diff`
  - `plomby_new_and_gone_by_id`
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5: Commit** `feat(core): normalize and diff cadastre objects`.

### Task 6: Check run orchestration

**Files:**
- Create: `desktop/core/src/run.rs`, `desktop/core/src/lock.rs`
- Test: `desktop/core/tests/run.rs`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `enum Trigger { Manual, Scheduled }`.
  - `enum ItemStatus { First, Unchanged, Changed, Error, Deferred }`.
  - `struct Changes { fields: Vec<FieldChange>, plomby_nove: Vec<PlombaDetail>, plomby_zanikle: Vec<RizeniDef> }`, where `PlombaDetail { rizeni: RizeniDef, datum_prijeti: Option<String>, stav: Option<String>, operace: Vec<String> }`.
  - `struct RunSummary { run_id: i64, status: RunStatus /*Ok|Partial|Failed*/, checked: u32, changed: u32, plomby_nove: u32, errors: u32, deferred: u32, api_calls: u32, api_used_today: u32, api_limit: u32, aktualnost_dat: Option<String>, error: Option<String> }`.
  - `fn run_check(client: &ApiClient, db: &mut Db, lock_path: &Path, trigger: Trigger, now: DateTime<Utc>) -> Result<RunSummary, CoreError>`.
  - `lock::RunLock::acquire(path) -> Result<RunLock, CoreError::AlreadyRunning>`. It uses an exclusive `fs4` try-lock and releases on drop.
- Algorithm:
  1. Acquire the lock.
  2. Read `interval_days` from settings (default 14) and take the due items. If there are none, return `Ok`, record a run with 0 checked, and make 0 API calls.
  3. Call `stav_uctu()`. Set `budget = limit − provedeno − 1 − 5`.
  4. Call `ensure_ciselniky` only if a due jednotka lacks a cached resolution.
  5. For each due item, oldest first: if `budget − client.calls() < 4`, mark this item and all remaining ones `Deferred` and stop. Otherwise resolve and `GET /{endpoint}/{id}`.
     - On `NotFound` with a cached id: clear `resolved` and re-resolve once.
     - Compare against `last_data`. If it is NULL the status is `First`; otherwise `Changed` or `Unchanged`.
     - For new plomby, call `GET /Rizeni/{id}` while the budget allows. The detail is best-effort: if it fails or the budget is gone, the plomba is reported without it.
     - On success, store `last_data` and `last_success_at`.
     - On error, store an `Error` result and leave `last_data` untouched. If the error is `InvalidApiKey`, abort the whole run.
  6. Write the item results and the run row in one transaction. The status is `Partial` when anything was deferred, and `Failed` when the run aborted.

- [ ] **Step 1: Write failing tests** (mock transport, in-memory DB, temp lock dir):
  - `first_run_marks_first_and_stores_data`
  - `second_run_unchanged_no_findings`
  - `field_change_and_new_plomba_reported_with_detail`
  - `api_error_keeps_old_state_and_item_stays_due` (Review Focus 1)
  - `budget_exhaustion_defers_rest_and_next_run_continues` (Review Focus 2): limit 500 with `provedeno` 480 leaves budget for only some items; the next run starts with the deferred ones.
  - `nothing_due_makes_zero_api_calls`
  - `second_concurrent_run_fails_already_running` (Review Focus 3)
  - `stale_iskn_id_404_re_resolves`
  - `invalid_key_aborts_run_without_touching_items`
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5: Commit** `feat(core): check run with per-item due dates and API budget`.

### Task 7: Secrets, scheduler and headless `--check`

**Files:**
- Create: `desktop/core/src/secrets.rs`, `desktop/core/src/schedule.rs`, `desktop/core/src/headless.rs`
- Test: unit tests in `schedule.rs`, plus `desktop/core/tests/headless.rs`

**Interfaces:**
- Produces:
  - `trait SecretStore { fn get(&self) -> Result<Option<String>>; fn set(&self, key: &str) -> Result<()>; fn delete(&self) -> Result<()>; }`, implemented by `KeyringStore` (the Global Constraints entry) and by `MemoryStore` for tests.
  - `trait Scheduler { fn enable(&self, exe: &Path, time: &str /*HH:MM*/) -> Result<()>; fn disable(&self) -> Result<()>; fn status(&self) -> Result<Option<String>>; }`, implemented by `WindowsTaskScheduler`.
    - `enable` runs `schtasks /Create /F /SC DAILY /TN StrazceKatastru /ST <HH:MM> /TR "\"<exe>\" --check"`.
    - `disable` runs `/Delete /F /TN StrazceKatastru`, and succeeds if the task is already gone.
    - `status` runs `/Query /TN StrazceKatastru`.
    - Argument building is a pure function, `fn schtasks_create_args(exe, time) -> Vec<String>`, and it is unit-tested.
  - `trait Notifier { fn notify(&self, title: &str, body: &str); }`, implemented by `ToastNotifier` (`notify-rust` with app id `cz.strazce-katastru.app`) and by a recording mock.
  - `fn summary_text(s: &RunSummary) -> (String, String)`: the title and body. Examples:
    - `"Katastr: 1 plomba, 2 změny"`
    - `"Katastr: beze změn"`
    - `"Katastr: chyba kontroly"`
    - body suffix `"zbývá 12 položek na další dny"` when items were deferred.
  - `fn run_headless(secrets: &dyn SecretStore, transport: Arc<dyn Transport>, notifier: &dyn Notifier, db_path, lock_path, now) -> i32`: the exit code.
    - Returns 0 for ok or partial.
    - Returns 2 for no key or an invalid key, with the notification `"Chybí nebo je neplatný API klíč – otevřete aplikaci."`.
    - Returns 3 when another run is already in progress, with no notification.
    - Returns 1 for any other failure.
    - Sends no notification when nothing was due and nothing changed.

- [ ] **Step 1: Write failing tests:**
  - `schtasks_args_quote_exe_with_spaces`
  - `summary_text_variants`
  - `headless_no_key_exit_2_and_notifies` (Review Focus 4)
  - `headless_invalid_key_exit_2`
  - `headless_nothing_due_is_silent`
  - `headless_changes_notify_once`
- [ ] **Step 2–4:** FAIL, implement, PASS.
- [ ] **Step 5: Commit** `feat(core): keyring, Windows task scheduler and headless check`.

### Task 8: Tauri app shell and commands

**Files:**
- Create: `desktop/src-tauri/` (`Cargo.toml`, `build.rs`, `tauri.conf.json`, `capabilities/default.json`, `src/main.rs`, `src/commands.rs`), `desktop/package.json`, `desktop/vite.config.ts`, `desktop/index.html`, `desktop/src/main.ts`, `desktop/src/lib/api.ts` (typed `invoke` wrappers)
- Test: `desktop/src-tauri/src/commands.rs` (unit tests that call command bodies with a test state)

**Interfaces:**
- Consumes: the core crate.
- `main.rs`:
  - `#![windows_subsystem = "windows"]` so scheduled runs don't flash a console.
  - If `std::env::args()` contains `--check`, it calls `strazce_core::headless::run_headless(...)` and `std::process::exit(code)` without building Tauri.
  - Otherwise it builds Tauri with `tauri-plugin-single-instance` (a second launch focuses the window) and `AppState { db: Mutex<Db>, secrets, transport, scheduler }`.
- Commands (all return `Result<T, String>` with Czech messages; the long ones run in `tauri::async_runtime::spawn_blocking`):
  - API key and account:
    - `api_status() -> ApiStatus { has_key, limit, used, expires }`
    - `set_api_key(key) -> ApiStatus`: validates via `stav_uctu`, stores the key, then runs `ensure_ciselniky`.
    - `delete_api_key()`
  - Items and k.ú.:
    - `list_items() -> Vec<ItemView>`
    - `save_item(input: ItemInput) -> ItemView`: inserts or updates, then resolves right away. A resolve failure is stored in `resolve_error` and is not a command error.
    - `delete_item(id)`
    - `search_ku(q) -> Vec<KuOption>`
    - `casti_obci_for_ku(ku) -> Vec<CastObceOption>`: feeds the override picker.
  - Checks and results:
    - `run_check() -> RunSummary`: uses `Trigger::Manual`; `AlreadyRunning` maps to `"Kontrola už běží."`.
    - `results() -> ResultsView { findings, last_run, runs }`
    - `mark_seen()`
    - `item_detail(id) -> serde_json::Value`
    - `due_overview() -> DueOverview { due_now, waiting, remaining_calls }`
  - Settings:
    - `get_settings() -> Settings { interval_days, schedule_enabled, schedule_time }`
    - `save_settings(Settings)`: persists the values, then calls `Scheduler::enable(current_exe, time)` or `disable`.
- The UI refreshes after a headless run: the window's `focus` event re-calls `results()` (Task 9).

- [ ] **Step 1: Write failing tests:**
  - `save_item_with_resolve_error_still_persists`
  - `save_settings_enables_scheduler_with_current_exe` (mock scheduler)
  - `run_check_already_running_message`
- [ ] **Step 2–4:** FAIL, implement, PASS (`cargo test -p strazce`). Then `npm run tauri dev` opens an empty window without errors.
- [ ] **Step 5: Commit** `feat(app): Tauri shell, headless entry and commands`.

### Task 9: UI screens

**Files:**
- Create: `desktop/src/App.svelte`, `desktop/src/screens/Onboarding.svelte`, `Items.svelte`, `ItemForm.svelte`, `Results.svelte`, `Settings.svelte`, `desktop/src/lib/format.ts`
- Test: `desktop/src/lib/format.test.ts` (vitest)

**Interfaces:**
- Consumes: `desktop/src/lib/api.ts` (Task 8).
- `format.ts`:
  - `formatChange(c: FieldChange): string` gives `"pole: staré → nové"`, with `"—"` for a missing value.
  - `formatRizeni(r): string`.
  - `statusLabel(s: ItemStatus): string`, mapping First, Unchanged, Changed, Error and Deferred to `"první kontrola"`, `"beze změn"`, `"změna"`, `"chyba"` and `"čeká na další den"`.
- Screens (layout as in the spec sketch):
  - **Onboarding** is shown while `api_status().has_key == false`. It has the key input, a link to the ČÚZK page about the API key, an "Ověřit a uložit" button, and shows limit and usage on success.
  - **Nemovitosti:**
    - The list shows popis, `describe`, an enabled toggle, `resolve_error` in red, and the last check date.
    - The form has a type switch and fields per type from the spec.
    - k.ú. is an autocomplete via `search_ku`. It shows `"Ponava (Brno)"` and stores the `kod`.
    - For jednotka there is an optional část obce picker, shown when `resolve_error` is set.
  - **Výsledky:**
    - Unseen findings come first: plomby (highlighted) and then changes.
    - Then come the last run summary, the history list, and a "Označit jako přečtené" button.
  - **Nastavení:** the API key (change or delete) with limit and usage; and the scheduled check with an enable toggle, interval 7/14/30 and a time.
  - **Header:** a "Zkontrolovat" button calling `run_check`, with a busy state, plus the `due_overview` line: `"Na řadě: 5 · čeká: 0 · zbývá volání dnes: 412"`.

- [ ] **Step 1: Write failing vitest** for `formatChange`, `formatRizeni` and `statusLabel`.
- [ ] **Step 2–4:** FAIL, implement, PASS (`npm test`). Then manually run `npm run tauri dev` with a real key and go through: onboarding, add one parcel, check, see "první kontrola", check again, see "beze změn".
- [ ] **Step 5: Commit** `feat(ui): onboarding, items, results and settings screens`.

### Task 10: Installer and README

**Files:**
- Modify: `desktop/src-tauri/tauri.conf.json` (bundle: `targets: ["nsis"]`, icons, `publisher`)
- Create: `desktop/README.md` (Czech: what the app does, how to get the API key, build prerequisites and commands, where the data lives, how the schedule works)

- [ ] **Step 1:** Run `npm run tauri build`. Expected: an NSIS installer in `desktop/src-tauri/target/release/bundle/nsis/`.
- [ ] **Step 2:** Manual check on Windows:
  - install;
  - enable the schedule in Nastavení, then `schtasks /Query /TN StrazceKatastru` shows the task;
  - `schtasks /Run /TN StrazceKatastru` runs the check without a console window and a toast appears;
  - disable the schedule and the task is gone.
- [ ] **Step 3: Commit** `build: NSIS installer and desktop README`.
