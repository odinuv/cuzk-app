#![windows_subsystem = "windows"]

mod commands;
mod ipc;

use chrono::Utc;
use commands::AppState;
use std::sync::{Arc, Mutex};
use strazce_core::api::ReqwestTransport;
use strazce_core::db::Db;
use strazce_core::headless::{run_headless, ToastNotifier};
use strazce_core::paths;
use strazce_core::schedule::WindowsTaskScheduler;
use strazce_core::secrets::KeyringStore;
use tauri::Manager;

/// Best-effort line in `<data_dir>/strazce.log`. `msg` is app text or a core error, never the key.
fn log_line(msg: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(paths::data_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::data_dir().join("strazce.log"))
    {
        let _ = writeln!(f, "{} {msg}", Utc::now().to_rfc3339());
    }
}

/// With `windows_subsystem = "windows"` there is no console, so tell the user in a dialog.
#[cfg(windows)]
fn message_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let wide = |s: &str| {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(text).as_ptr(),
            wide("Strážce katastru").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn message_box(_text: &str) {}

fn fatal(cause: &str) -> ! {
    log_line(cause);
    message_box(&format!(
        "Aplikaci Strážce katastru se nepodařilo spustit: {cause}"
    ));
    std::process::exit(1)
}

/// Scheduled run (`strazce --check`): no window, no Tauri; the exit code is the result.
fn headless() -> i32 {
    match ReqwestTransport::new() {
        Ok(t) => run_headless(
            &KeyringStore,
            Arc::new(t),
            &ToastNotifier,
            &paths::db_path(),
            &paths::lock_path(),
            Utc::now(),
        ),
        Err(e) => {
            log_line(&format!("--check: {e}"));
            1
        }
    }
}

fn build_state() -> Result<AppState, String> {
    std::fs::create_dir_all(paths::data_dir())
        .map_err(|_| "Datovou složku aplikace nelze vytvořit.".to_string())?;
    let db = Db::open(&paths::db_path()).map_err(|e| e.to_string())?;
    let transport = ReqwestTransport::new().map_err(|e| e.to_string())?;
    Ok(AppState {
        db: Mutex::new(db),
        db_path: paths::db_path(),
        secrets: Arc::new(KeyringStore),
        transport: Arc::new(transport),
        scheduler: Arc::new(WindowsTaskScheduler),
        lock_path: paths::lock_path(),
        exe_path: std::env::current_exe().map_err(|e| e.to_string())?,
    })
}

fn main() {
    if std::env::args().any(|a| a == "--check") {
        std::process::exit(headless());
    }
    let state = build_state().unwrap_or_else(|e| fatal(&e));
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .manage(Arc::new(state))
        .invoke_handler(tauri::generate_handler![
            ipc::api_status,
            ipc::set_api_key,
            ipc::delete_api_key,
            ipc::list_items,
            ipc::save_item,
            ipc::delete_item,
            ipc::search_ku,
            ipc::casti_obci_for_ku,
            ipc::run_check,
            ipc::results,
            ipc::mark_seen,
            ipc::item_detail,
            ipc::due_overview,
            ipc::get_settings,
            ipc::save_settings,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| fatal(&e.to_string()));
}
