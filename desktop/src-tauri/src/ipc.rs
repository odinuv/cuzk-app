//! Thin `#[tauri::command]` wrappers over `commands`. Each body runs on the blocking pool
//! (SQLite, network and the OS scheduler are all synchronous).

use crate::commands::{self, *};
use std::sync::Arc;
use tauri::State;

type Shared<'a> = State<'a, Arc<AppState>>;

async fn blocking<T, F>(state: Shared<'_>, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&AppState) -> Result<T, String> + Send + 'static,
{
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || f(&st))
        .await
        .map_err(|e| format!("Interní chyba: {e}"))?
}

#[tauri::command]
pub async fn api_status(state: Shared<'_>) -> Result<ApiStatus, String> {
    blocking(state, commands::api_status).await
}

#[tauri::command]
pub async fn set_api_key(state: Shared<'_>, key: String) -> Result<ApiStatus, String> {
    blocking(state, move |s| commands::set_api_key(s, key)).await
}

#[tauri::command]
pub async fn delete_api_key(state: Shared<'_>) -> Result<(), String> {
    blocking(state, commands::delete_api_key).await
}

#[tauri::command]
pub async fn list_items(state: Shared<'_>) -> Result<Vec<ItemView>, String> {
    blocking(state, commands::list_items).await
}

#[tauri::command]
pub async fn save_item(state: Shared<'_>, input: ItemInput) -> Result<ItemView, String> {
    blocking(state, move |s| commands::save_item(s, input)).await
}

#[tauri::command]
pub async fn delete_item(state: Shared<'_>, id: i64) -> Result<(), String> {
    blocking(state, move |s| commands::delete_item(s, id)).await
}

#[tauri::command]
pub async fn search_ku(
    state: Shared<'_>,
    q: String,
) -> Result<Vec<strazce_core::db::KuOption>, String> {
    blocking(state, move |s| commands::search_ku(s, q)).await
}

#[tauri::command]
pub async fn casti_obci_for_ku(state: Shared<'_>, ku: u32) -> Result<Vec<CastObceOption>, String> {
    blocking(state, move |s| commands::casti_obci_for_ku(s, ku)).await
}

#[tauri::command]
pub async fn run_check(state: Shared<'_>) -> Result<RunSummaryView, String> {
    blocking(state, commands::run_check).await
}

#[tauri::command]
pub async fn results(state: Shared<'_>) -> Result<ResultsView, String> {
    blocking(state, commands::results).await
}

#[tauri::command]
pub async fn mark_seen(state: Shared<'_>) -> Result<(), String> {
    blocking(state, commands::mark_seen).await
}

#[tauri::command]
pub async fn item_detail(state: Shared<'_>, id: i64) -> Result<serde_json::Value, String> {
    blocking(state, move |s| commands::item_detail(s, id)).await
}

#[tauri::command]
pub async fn due_overview(state: Shared<'_>) -> Result<DueOverview, String> {
    blocking(state, commands::due_overview).await
}

#[tauri::command]
pub async fn get_settings(state: Shared<'_>) -> Result<SettingsView, String> {
    blocking(state, commands::get_settings).await
}

#[tauri::command]
pub async fn save_settings(state: Shared<'_>, settings: Settings) -> Result<(), String> {
    blocking(state, move |s| commands::save_settings(s, settings)).await
}
