mod db;

use db::Db;
use serde::Serialize;
use tauri::{Manager, State, WebviewWindow};

#[derive(Serialize)]
struct PlatformInfo {
    os: &'static str,
    arch: &'static str,
    version: &'static str,
    /// NovaOS is an application, not a system shell. Flip this only when Shell Mode truly exists.
    shell_mode_available: bool,
}

#[tauri::command]
fn platform_info() -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        version: env!("CARGO_PKG_VERSION"),
        shell_mode_available: false,
    }
}

#[tauri::command]
fn kv_get(db: State<'_, Db>, key: String) -> Result<Option<String>, String> {
    db.get(&key)
}

#[tauri::command]
fn kv_set(db: State<'_, Db>, key: String, value: String) -> Result<(), String> {
    db.set(&key, &value)
}

/// Immersive Mode is plain borderless fullscreen: no always-on-top, no input grab, no kiosk lock,
/// so Alt+Tab, the Windows key and Ctrl+Alt+Del keep working and the user can always leave.
#[tauri::command]
fn set_fullscreen(window: WebviewWindow, fullscreen: bool) -> Result<(), String> {
    window.set_fullscreen(fullscreen).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&dir)?;
            app.manage(Db::open(&dir.join("novaos.db"))?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![platform_info, kv_get, kv_set, set_fullscreen])
        .run(tauri::generate_context!())
        .expect("error while running NovaOS");
}
