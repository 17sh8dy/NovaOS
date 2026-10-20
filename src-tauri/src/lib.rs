mod db;
mod import;
mod import_cmds;
mod import_store;
mod vfs;

use db::Db;
use import_cmds::ImportState;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use vfs::{Listing, Root, RootKind, Vfs};

const CUSTOM_ROOTS_KEY: &str = "files.roots";

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

// ------------------------------------------------------------------ filesystem commands
// Every command takes a root id plus a path relative to that root; see vfs.rs.

#[tauri::command]
fn fs_roots(vfs: State<'_, Vfs>) -> Vec<Root> {
    vfs.roots()
}

#[tauri::command]
fn fs_list(vfs: State<'_, Vfs>, root: String, path: String) -> Result<Listing, String> {
    vfs.list(&root, &path)
}

#[tauri::command]
fn fs_mkdir(vfs: State<'_, Vfs>, root: String, path: String) -> Result<(), String> {
    vfs.mkdir(&root, &path)
}

#[tauri::command]
fn fs_create_file(vfs: State<'_, Vfs>, root: String, path: String) -> Result<(), String> {
    vfs.create_file(&root, &path)
}

#[tauri::command]
fn fs_rename(
    vfs: State<'_, Vfs>,
    root: String,
    path: String,
    new_name: String,
) -> Result<String, String> {
    vfs.rename(&root, &path, &new_name)
}

#[tauri::command]
fn fs_copy(
    vfs: State<'_, Vfs>,
    root: String,
    path: String,
    dest_root: String,
    dest_dir: String,
) -> Result<String, String> {
    vfs.copy(&root, &path, &dest_root, &dest_dir)
}

#[tauri::command]
fn fs_move(
    vfs: State<'_, Vfs>,
    root: String,
    path: String,
    dest_root: String,
    dest_dir: String,
) -> Result<String, String> {
    vfs.mv(&root, &path, &dest_root, &dest_dir)
}

#[tauri::command]
fn fs_trash(vfs: State<'_, Vfs>, root: String, path: String) -> Result<(), String> {
    vfs.trash(&root, &path)
}

#[tauri::command]
fn fs_read_text(vfs: State<'_, Vfs>, root: String, path: String) -> Result<String, String> {
    vfs.read_text(&root, &path)
}

#[tauri::command]
fn fs_write_text(
    vfs: State<'_, Vfs>,
    root: String,
    path: String,
    text: String,
) -> Result<(), String> {
    vfs.write_text(&root, &path, &text)
}

/// Raw bytes for the image viewer, sent as a binary response rather than a JSON number array.
#[tauri::command]
fn fs_read_binary(
    vfs: State<'_, Vfs>,
    root: String,
    path: String,
) -> Result<tauri::ipc::Response, String> {
    vfs.read_binary(&root, &path).map(tauri::ipc::Response::new)
}

#[tauri::command]
fn fs_open_external(vfs: State<'_, Vfs>, root: String, path: String) -> Result<(), String> {
    vfs.open_externally(&root, &path)
}

/// Shows the native folder picker. Async so the blocking dialog never stalls the main thread.
#[tauri::command]
async fn fs_add_root(
    app: AppHandle,
    vfs: State<'_, Vfs>,
    db: State<'_, Db>,
) -> Result<Option<Root>, String> {
    let Some(picked) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let root = vfs.add_custom(&path)?;
    persist_custom_roots(&vfs, &db)?;
    Ok(Some(root))
}

#[tauri::command]
fn fs_remove_root(vfs: State<'_, Vfs>, db: State<'_, Db>, id: String) -> Result<(), String> {
    vfs.remove_custom(&id)?;
    persist_custom_roots(&vfs, &db)
}

fn persist_custom_roots(vfs: &Vfs, db: &Db) -> Result<(), String> {
    let json = serde_json::to_string(&vfs.custom_paths()).map_err(|e| e.to_string())?;
    db.set(CUSTOM_ROOTS_KEY, &json)
}

/// NovaOS's own organized user environment, plus the standard host user folders that exist.
fn build_vfs(app: &tauri::App, db: &Db) -> Result<Vfs, Box<dyn std::error::Error>> {
    let paths = app.path();
    let nova_home = paths.app_data_dir()?.join("NovaFiles");
    for sub in [
        "Documents",
        "Pictures",
        "Music",
        "Downloads",
        "Desktop",
        "Imports",
    ] {
        std::fs::create_dir_all(nova_home.join(sub))?;
    }
    let mut roots = vec![Root {
        id: "nova".into(),
        label: "Nova Files".into(),
        kind: RootKind::Nova,
        path: nova_home,
    }];
    let host: [(&str, &str, Result<PathBuf, tauri::Error>); 4] = [
        ("host-documents", "Documents", paths.document_dir()),
        ("host-pictures", "Pictures", paths.picture_dir()),
        ("host-downloads", "Downloads", paths.download_dir()),
        ("host-desktop", "Desktop", paths.desktop_dir()),
    ];
    for (id, label, dir) in host {
        if let Ok(path) = dir {
            if path.is_dir() {
                roots.push(Root {
                    id: id.into(),
                    label: label.into(),
                    kind: RootKind::Host,
                    path,
                });
            }
        }
    }
    let vfs = Vfs::new(roots);
    // Re-add folders the user chose earlier; skip any that vanished or are no longer allowed.
    if let Ok(Some(json)) = db.get(CUSTOM_ROOTS_KEY) {
        if let Ok(list) = serde_json::from_str::<Vec<String>>(&json) {
            for p in list {
                let _ = vfs.add_custom(&PathBuf::from(p));
            }
        }
    }
    Ok(vfs)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&dir)?;
            let db = Db::open(&dir.join("novaos.db"))?;
            let vfs = build_vfs(app, &db)?;
            let data_dir = dir.canonicalize()?;
            let imports =
                ImportState::new(data_dir.join("NovaFiles").join("Imports"), vec![data_dir]);
            import_cmds::restore(&db, &vfs, &imports);
            app.manage(db);
            app.manage(vfs);
            app.manage(imports);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            platform_info,
            kv_get,
            kv_set,
            set_fullscreen,
            fs_roots,
            fs_list,
            fs_mkdir,
            fs_create_file,
            fs_rename,
            fs_copy,
            fs_move,
            fs_trash,
            fs_read_text,
            fs_read_binary,
            fs_write_text,
            fs_open_external,
            fs_add_root,
            fs_remove_root,
            import_cmds::import_pick,
            import_cmds::import_scan,
            import_cmds::import_cancel,
            import_cmds::import_discard,
            import_cmds::import_status,
            import_cmds::import_start,
            import_cmds::import_list,
            import_cmds::import_unlink,
        ])
        .run(tauri::generate_context!())
        .expect("error while running NovaOS");
}
