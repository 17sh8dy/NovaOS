//! Tauri commands for importing host files. The UI never supplies a host path: the native dialog
//! runs in Rust, the chosen paths stay in `ImportState`, and the UI only holds an opaque id.

use crate::db::Db;
use crate::import::{self, Ctl, JobState, Outcome, Progress, ScanReport, Sink};
use crate::import_store::{self as store, ImportRecord};
use crate::vfs::Vfs;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

const MAX_SELECTION: usize = 500;
const MAX_PENDING_SELECTIONS: usize = 8;
pub const EVENT: &str = "import-progress";
const FILE_BATCH: usize = 500;

struct Selection {
    paths: Vec<PathBuf>,
    scan: Option<ScanReport>,
    cancel: Arc<AtomicBool>,
}

struct Job {
    ctl: Arc<Ctl>,
    snapshot: Mutex<Progress>,
    finished: AtomicBool,
}

pub struct ImportState {
    pub imports_dir: PathBuf,
    /// Folders owned by NovaOS itself that can never be linked.
    pub avoid: Vec<PathBuf>,
    selections: Mutex<HashMap<String, Selection>>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    counter: AtomicU64,
}

impl ImportState {
    pub fn new(imports_dir: PathBuf, avoid: Vec<PathBuf>) -> Self {
        Self {
            imports_dir,
            avoid,
            selections: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
        }
    }

    fn next_id(&self) -> String {
        format!("imp{}", self.counter.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Serialize)]
pub struct SelectionItem {
    name: String,
    path: String,
    is_dir: bool,
}

#[derive(Serialize)]
pub struct SelectionInfo {
    id: String,
    items: Vec<SelectionItem>,
}

#[derive(Serialize)]
pub struct ImportRow {
    #[serde(flatten)]
    record: ImportRecord,
    /// The copy still exists in NovaOS / the linked original is still reachable.
    available: bool,
}

/// Native picker. `kind` is "files" or "folder". Returns None if the user cancelled.
#[tauri::command]
pub async fn import_pick(
    app: AppHandle,
    state: State<'_, ImportState>,
    kind: String,
) -> Result<Option<SelectionInfo>, String> {
    let picker = app.dialog().file();
    let picked = match kind.as_str() {
        "files" => picker.blocking_pick_files(),
        "folder" => picker.blocking_pick_folders(),
        _ => return Err("unknown import kind".into()),
    };
    let Some(picked) = picked else {
        return Ok(None);
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    for p in picked {
        let path = p.into_path().map_err(|e| e.to_string())?;
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return Ok(None);
    }
    if paths.len() > MAX_SELECTION {
        return Err(format!("select at most {MAX_SELECTION} items at a time"));
    }
    let items = paths
        .iter()
        .map(|p| SelectionItem {
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned()),
            path: p.to_string_lossy().into_owned(),
            is_dir: p.is_dir(),
        })
        .collect();
    let id = state.next_id();
    let mut sel = state.selections.lock().map_err(|_| "lock poisoned")?;
    if sel.len() >= MAX_PENDING_SELECTIONS {
        sel.clear(); // abandoned dialogs; bound the memory
    }
    sel.insert(
        id.clone(),
        Selection {
            paths,
            scan: None,
            cancel: Arc::new(AtomicBool::new(false)),
        },
    );
    Ok(Some(SelectionInfo { id, items }))
}

/// Read-only preflight: counts files/bytes, finds links, unreadable items and free disk space.
#[tauri::command]
pub async fn import_scan(state: State<'_, ImportState>, id: String) -> Result<ScanReport, String> {
    let (paths, cancel) = {
        let sel = state.selections.lock().map_err(|_| "lock poisoned")?;
        let s = sel
            .get(&id)
            .ok_or("that selection has expired; choose again")?;
        s.cancel.store(false, Ordering::Relaxed);
        (s.paths.clone(), s.cancel.clone())
    };
    let dest = state.imports_dir.clone();
    let report = tauri::async_runtime::spawn_blocking(move || import::scan(&paths, &dest, &cancel))
        .await
        .map_err(|e| e.to_string())??;
    if let Some(s) = state
        .selections
        .lock()
        .map_err(|_| "lock poisoned")?
        .get_mut(&id)
    {
        s.scan = Some(report.clone());
    }
    Ok(report)
}

/// Stops a scan or a running copy.
#[tauri::command]
pub fn import_cancel(state: State<'_, ImportState>, id: String) {
    if let Ok(sel) = state.selections.lock() {
        if let Some(s) = sel.get(&id) {
            s.cancel.store(true, Ordering::Relaxed);
        }
    }
    if let Ok(jobs) = state.jobs.lock() {
        if let Some(j) = jobs.get(&id) {
            j.ctl.cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// The user closed the dialog without importing.
#[tauri::command]
pub fn import_discard(state: State<'_, ImportState>, id: String) {
    if let Ok(mut sel) = state.selections.lock() {
        if let Some(s) = sel.remove(&id) {
            s.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[tauri::command]
pub fn import_status(state: State<'_, ImportState>, id: String) -> Option<Progress> {
    let jobs = state.jobs.lock().ok()?;
    let snap = jobs.get(&id)?.snapshot.lock().ok()?.clone();
    Some(snap)
}

struct DbSink {
    app: AppHandle,
    job: Arc<Job>,
    pending: Vec<(String, u64, Option<u64>)>,
}

impl DbSink {
    fn db(&self) -> State<'_, Db> {
        self.app.state::<Db>()
    }
    fn flush(&mut self, item: Option<i64>) {
        if let Some(id) = item {
            if !self.pending.is_empty() {
                if let Err(e) = store::add_files(&self.db(), id, &self.pending) {
                    eprintln!("novaos: could not store file metadata: {e}");
                }
            }
        }
        self.pending.clear();
    }
}

impl Sink for DbSink {
    fn progress(&mut self, p: &Progress) {
        if let Ok(mut snap) = self.job.snapshot.lock() {
            *snap = p.clone();
        }
        let _ = self.app.emit(EVENT, p);
    }
    fn item_started(&mut self, source: &Path, is_dir: bool) -> Result<i64, String> {
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        store::insert(
            &self.db(),
            "copy",
            &name,
            &source.to_string_lossy(),
            is_dir,
            "running",
        )
    }
    fn file_copied(&mut self, item: Option<i64>, rel: &str, size: u64, modified: Option<u64>) {
        self.pending.push((rel.to_string(), size, modified));
        if self.pending.len() >= FILE_BATCH {
            self.flush(item);
        }
    }
    fn item_finished(
        &mut self,
        item: Option<i64>,
        dest_rel: &str,
        files: u64,
        bytes: u64,
        errors: u64,
    ) {
        self.flush(item);
        if let Some(id) = item {
            if let Err(e) = store::finish(&self.db(), id, dest_rel, files, bytes, errors) {
                eprintln!("novaos: could not finish import record: {e}");
            }
        }
    }
    fn item_aborted(&mut self, item: Option<i64>) {
        self.pending.clear();
        if let Some(id) = item {
            let _ = store::delete(&self.db(), id);
        }
    }
}

/// Begin importing a selection. `copy` runs in the background (watch the `import-progress`
/// event); `link` completes immediately and requires the user's acknowledgement.
#[tauri::command]
pub fn import_start(
    app: AppHandle,
    state: State<'_, ImportState>,
    db: State<'_, Db>,
    vfs: State<'_, Vfs>,
    id: String,
    mode: String,
    acknowledged: bool,
) -> Result<Progress, String> {
    let selection = {
        let mut sel = state.selections.lock().map_err(|_| "lock poisoned")?;
        sel.remove(&id)
            .ok_or("that selection has expired; choose again")?
    };
    match mode.as_str() {
        "link" => {
            if !acknowledged {
                return Err("confirm that you understand edits will change the originals".into());
            }
            Ok(link_items(&db, &vfs, &state, &id, &selection.paths))
        }
        "copy" => {
            let mut jobs = state.jobs.lock().map_err(|_| "lock poisoned")?;
            if jobs.values().any(|j| !j.finished.load(Ordering::Relaxed)) {
                return Err("another import is already running".into());
            }
            jobs.retain(|_, j| !j.finished.load(Ordering::Relaxed)); // forget finished jobs
            let totals = selection
                .scan
                .as_ref()
                .map(|s| (s.files, s.bytes))
                .unwrap_or((0, 0));
            let initial = Progress::new(&id, selection.paths.len() as u64, totals.0, totals.1);
            let job = Arc::new(Job {
                ctl: Arc::new(Ctl::new()),
                snapshot: Mutex::new(initial.clone()),
                finished: AtomicBool::new(false),
            });
            jobs.insert(id.clone(), job.clone());
            drop(jobs);

            let dest = state.imports_dir.clone();
            std::thread::spawn(move || {
                let mut sink = DbSink {
                    app: app.clone(),
                    job: job.clone(),
                    pending: Vec::new(),
                };
                let outcome =
                    import::run_copy(&id, &selection.paths, &dest, totals, &job.ctl, &mut sink);
                if let Outcome::Failed(m) = &outcome {
                    eprintln!("novaos: import {id} failed: {m}");
                }
                job.finished.store(true, Ordering::Relaxed);
            });
            Ok(initial)
        }
        _ => Err("unknown import mode".into()),
    }
}

fn link_items(db: &Db, vfs: &Vfs, state: &ImportState, job: &str, paths: &[PathBuf]) -> Progress {
    let mut p = Progress::new(job, paths.len() as u64, 0, 0);
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        p.current = name.clone();
        let is_dir = path.is_dir();
        let row = store::insert(db, "link", &name, &path.to_string_lossy(), is_dir, "linked");
        let result = row.and_then(|id| match vfs.add_link(id, path, &state.avoid) {
            Ok(_) => Ok(()),
            Err(e) => {
                let _ = store::delete(db, id); // no record of a link that was refused
                Err(e)
            }
        });
        match result {
            Ok(()) => p.items_done += 1,
            Err(e) => {
                p.error_count += 1;
                p.errors.push(format!("{name}: {e}"));
            }
        }
    }
    p.state = if p.items_done == 0 && p.error_count > 0 {
        JobState::Failed
    } else {
        JobState::Done
    };
    if p.state == JobState::Failed {
        p.message = p.errors.first().cloned();
    }
    p
}

#[tauri::command]
pub fn import_list(
    state: State<'_, ImportState>,
    db: State<'_, Db>,
) -> Result<Vec<ImportRow>, String> {
    Ok(store::list(&db)?
        .into_iter()
        .map(|record| {
            let available = match (record.mode.as_str(), &record.dest_rel) {
                ("copy", Some(rel)) => state.imports_dir.join(rel).exists(),
                ("copy", None) => false,
                _ => Path::new(&record.source_path).exists(),
            };
            ImportRow { record, available }
        })
        .collect())
}

/// Stop linking an item. Only the reference is removed; the original file is never touched.
#[tauri::command]
pub fn import_unlink(vfs: State<'_, Vfs>, db: State<'_, Db>, id: i64) -> Result<(), String> {
    let row = store::list(&db)?
        .into_iter()
        .find(|r| r.id == id && r.mode == "link")
        .ok_or("not a linked item")?;
    let _ = vfs.remove_link(&format!("link-{}", row.id)); // may already be absent if the source vanished
    store::delete(&db, id)
}

/// Startup: clear crash leftovers and bring saved links back.
pub fn restore(db: &Db, vfs: &Vfs, state: &ImportState) {
    import::clean_orphan_staging(&state.imports_dir);
    let _ = store::drop_interrupted(db);
    for (id, path) in store::links(db).unwrap_or_default() {
        let _ = vfs.add_link(id, Path::new(&path), &state.avoid); // missing sources just stay unavailable
    }
}
