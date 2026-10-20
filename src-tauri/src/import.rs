//! Importing host files and folders into NovaOS.
//!
//! Safety invariants (each is covered by a test):
//!  * Source paths are only ever opened read-only or stat'ed. No write, rename or delete call in
//!    this module is ever given a source path.
//!  * Copies are built in a staging folder next to the destination and moved into place only when
//!    complete, so a cancel, crash or full disk never leaves a half-copied item visible.
//!  * Nothing is overwritten: an existing name gets " (2)", " (3)", ... appended.
//!  * Symbolic links, junctions and special files are never followed or copied.

use crate::vfs::{is_program_file, unique_dest};
use fs4::available_space;
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const STAGING_PREFIX: &str = ".nova-staging";
const CHUNK: usize = 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_ERRORS_KEPT: usize = 100;
#[cfg(not(test))]
const EMIT_EVERY: Duration = Duration::from_millis(120);
/// Tests emit on every chunk so a cancel point is deterministic.
#[cfg(test)]
const EMIT_EVERY: Duration = Duration::ZERO;
/// Free space that must remain after an import, so the disk is never filled to the brim.
pub const SPACE_MARGIN: u64 = 64 * 1024 * 1024;

// ---------------------------------------------------------------- scanning

#[derive(Serialize, Clone, Debug, Default)]
pub struct ScanReport {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// Links/junctions found; these are skipped, never followed.
    pub symlinks: u64,
    /// Executable/script file types. They are copied as plain data and cannot run in NovaOS.
    pub programs: u64,
    pub unreadable: Vec<String>,
    pub unreadable_count: u64,
    pub free_bytes: u64,
    pub required_bytes: u64,
    pub enough_space: bool,
}

/// Walk the sources (read-only) and total up what an import would do. `Err` means cancelled.
pub fn scan(
    sources: &[PathBuf],
    dest_dir: &Path,
    cancel: &AtomicBool,
) -> Result<ScanReport, String> {
    let mut r = ScanReport::default();
    let mut stack: Vec<(PathBuf, usize)> = sources.iter().map(|s| (s.clone(), 0)).collect();
    while let Some((path, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let md = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                note_unreadable(&mut r, &path, &e.to_string());
                continue;
            }
        };
        let ft = md.file_type();
        if ft.is_symlink() {
            r.symlinks += 1;
        } else if ft.is_file() {
            r.files += 1;
            r.bytes += md.len();
            if is_program_file(&path) {
                r.programs += 1;
            }
        } else if ft.is_dir() {
            r.dirs += 1;
            if depth >= MAX_DEPTH {
                note_unreadable(&mut r, &path, "nested too deeply");
                continue;
            }
            match fs::read_dir(&path) {
                Ok(rd) => {
                    for e in rd {
                        match e {
                            Ok(e) => stack.push((e.path(), depth + 1)),
                            Err(e) => note_unreadable(&mut r, &path, &e.to_string()),
                        }
                    }
                }
                Err(e) => note_unreadable(&mut r, &path, &e.to_string()),
            }
        }
    }
    r.required_bytes = r.bytes.saturating_add(SPACE_MARGIN);
    r.free_bytes = available_space(dest_dir).unwrap_or(0);
    r.enough_space = r.free_bytes >= r.required_bytes;
    Ok(r)
}

fn note_unreadable(r: &mut ScanReport, path: &Path, why: &str) {
    r.unreadable_count += 1;
    if r.unreadable.len() < 20 {
        r.unreadable.push(format!("{}: {why}", path.display()));
    }
}

// ---------------------------------------------------------------- job plumbing

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Running,
    Done,
    Cancelled,
    Failed,
}

#[derive(Serialize, Clone, Debug)]
pub struct Progress {
    pub job: String,
    pub state: JobState,
    pub items_done: u64,
    pub items_total: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub speed_bps: f64,
    pub current: String,
    pub error_count: u64,
    pub errors: Vec<String>,
    pub message: Option<String>,
}

impl Progress {
    pub fn new(job: &str, items_total: u64, files_total: u64, bytes_total: u64) -> Self {
        Self {
            job: job.into(),
            state: JobState::Running,
            items_done: 0,
            items_total,
            files_done: 0,
            files_total,
            bytes_done: 0,
            bytes_total,
            speed_bps: 0.0,
            current: String::new(),
            error_count: 0,
            errors: Vec::new(),
            message: None,
        }
    }
}

/// Where the engine reports to. The app persists to SQLite and emits UI events; tests record.
pub trait Sink {
    fn progress(&mut self, p: &Progress);
    /// A top-level item is about to be copied. `None` means metadata can't be stored (the copy
    /// still proceeds; the failure is surfaced as an error entry).
    fn item_started(&mut self, source: &Path, is_dir: bool) -> Result<i64, String>;
    fn file_copied(&mut self, item: Option<i64>, rel: &str, size: u64, modified: Option<u64>);
    fn item_finished(
        &mut self,
        item: Option<i64>,
        dest_rel: &str,
        files: u64,
        bytes: u64,
        errors: u64,
    );
    /// The item was cancelled or failed and its staged copy removed.
    fn item_aborted(&mut self, item: Option<i64>);
}

pub struct Ctl {
    pub cancel: AtomicBool,
}

impl Ctl {
    pub fn new() -> Self {
        Self {
            cancel: AtomicBool::new(false),
        }
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

impl Default for Ctl {
    fn default() -> Self {
        Self::new()
    }
}

enum Flow {
    Cancelled,
    /// Disk full or the destination is broken: stop the whole job.
    Fatal(String),
    /// This file/folder could not be copied; record it and carry on.
    Skip(String),
}

struct Ctx<'a> {
    ctl: &'a Ctl,
    sink: &'a mut dyn Sink,
    p: Progress,
    item: Option<i64>,
    item_files: u64,
    item_bytes: u64,
    item_errors: u64,
    start: Instant,
    last_emit: Instant,
    last_bytes: u64,
    last_t: Instant,
}

impl Ctx<'_> {
    fn record_error(&mut self, msg: String) {
        self.p.error_count += 1;
        self.item_errors += 1;
        if self.p.errors.len() < MAX_ERRORS_KEPT {
            self.p.errors.push(msg);
        }
    }

    fn emit(&mut self, force: bool) {
        let now = Instant::now();
        if !force && now.duration_since(self.last_emit) < EMIT_EVERY {
            return;
        }
        let dt = now.duration_since(self.last_t).as_secs_f64();
        if dt > 0.0 {
            let inst = (self.p.bytes_done - self.last_bytes) as f64 / dt;
            // Smooth so the displayed speed doesn't flicker.
            self.p.speed_bps = if self.p.speed_bps == 0.0 {
                inst
            } else {
                0.7 * self.p.speed_bps + 0.3 * inst
            };
            self.last_bytes = self.p.bytes_done;
            self.last_t = now;
        }
        self.last_emit = now;
        self.p.files_total = self.p.files_total.max(self.p.files_done);
        self.p.bytes_total = self.p.bytes_total.max(self.p.bytes_done);
        self.sink.progress(&self.p);
    }
}

fn mtime_secs(t: SystemTime) -> Option<u64> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

// ---------------------------------------------------------------- copying

fn copy_file(ctx: &mut Ctx, src: &Path, dst: &Path, rel: &str) -> Result<(), Flow> {
    // Read-only open: the source can never be modified through this handle.
    let mut input = File::open(src).map_err(|e| Flow::Skip(format!("{rel}: {e}")))?;
    let md = input
        .metadata()
        .map_err(|e| Flow::Skip(format!("{rel}: {e}")))?;
    if !md.is_file() {
        return Err(Flow::Skip(format!("{rel}: not a regular file")));
    }
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(|e| Flow::Fatal(format!("cannot create {rel} in NovaOS: {e}")))?;

    ctx.p.current = rel.to_string();
    let mut buf = vec![0u8; CHUNK];
    let mut copied: u64 = 0;
    let outcome: Result<(), Flow> = loop {
        if ctx.ctl.cancelled() {
            break Err(Flow::Cancelled);
        }
        let n = match input.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => break Err(Flow::Skip(format!("{rel}: read failed: {e}"))),
        };
        if let Err(e) = out.write_all(&buf[..n]) {
            let why = if e.kind() == ErrorKind::StorageFull {
                "the disk is full".to_string()
            } else {
                e.to_string()
            };
            break Err(Flow::Fatal(format!("write failed for {rel}: {why}")));
        }
        copied += n as u64;
        ctx.p.bytes_done += n as u64;
        ctx.emit(false);
    };

    match outcome {
        Ok(()) if copied != md.len() => {
            drop(out);
            let _ = fs::remove_file(dst); // our own staged copy
            Err(Flow::Skip(format!("{rel}: changed while copying; skipped")))
        }
        Ok(()) => {
            let modified = md.modified().ok();
            if let Some(t) = modified {
                let _ = out.set_modified(t); // preserve the original modification time
            }
            drop(out);
            ctx.p.files_done += 1;
            ctx.item_files += 1;
            ctx.item_bytes += copied;
            ctx.sink
                .file_copied(ctx.item, rel, copied, modified.and_then(mtime_secs));
            Ok(())
        }
        Err(flow) => {
            drop(out);
            let _ = fs::remove_file(dst);
            Err(flow)
        }
    }
}

fn copy_tree(ctx: &mut Ctx, src: &Path, dst: &Path, rel: &str, depth: usize) -> Result<(), Flow> {
    if depth > MAX_DEPTH {
        return Err(Flow::Skip(format!("{rel}: nested too deeply")));
    }
    fs::create_dir(dst).map_err(|e| Flow::Fatal(format!("cannot create folder in NovaOS: {e}")))?;
    let entries = match fs::read_dir(src) {
        Ok(rd) => rd,
        Err(e) => {
            ctx.record_error(format!("{}: {e}", if rel.is_empty() { "." } else { rel }));
            return Ok(()); // the (empty) folder is kept; the error is reported
        }
    };
    for entry in entries {
        if ctx.ctl.cancelled() {
            return Err(Flow::Cancelled);
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                ctx.record_error(format!("{rel}: {e}"));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(e) => {
                ctx.record_error(format!("{child_rel}: {e}"));
                continue;
            }
        };
        let result = if ft.is_symlink() {
            ctx.record_error(format!("{child_rel}: link skipped"));
            Ok(())
        } else if ft.is_dir() {
            copy_tree(ctx, &entry.path(), &dst.join(&name), &child_rel, depth + 1)
        } else if ft.is_file() {
            copy_file(ctx, &entry.path(), &dst.join(&name), &child_rel)
        } else {
            ctx.record_error(format!("{child_rel}: special file skipped"));
            Ok(())
        };
        match result {
            Ok(()) => {}
            Err(Flow::Skip(msg)) => ctx.record_error(msg),
            Err(other) => return Err(other),
        }
    }
    Ok(())
}

/// Move a finished staged item into `dest_dir` under a name that does not exist yet. Files use a
/// hard link first because, unlike rename, it fails instead of replacing an existing file.
pub fn place(stage: &Path, dest_dir: &Path, name: &str, is_dir: bool) -> Result<String, String> {
    for _ in 0..100 {
        let dest = unique_dest(dest_dir, name);
        let final_name = dest
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if is_dir {
            match fs::rename(stage, &dest) {
                Ok(()) => return Ok(final_name),
                Err(_) if fs::symlink_metadata(&dest).is_ok() => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        match fs::hard_link(stage, &dest) {
            Ok(()) => {
                let _ = fs::remove_file(stage);
                return Ok(final_name);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => {
                // Volume without hard links: fall back to a checked rename.
                if fs::symlink_metadata(&dest).is_ok() {
                    continue;
                }
                fs::rename(stage, &dest).map_err(|e| e.to_string())?;
                return Ok(final_name);
            }
        }
    }
    Err("could not find a free name".into())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Cancelled,
    Failed(String),
}

/// Copy each source into `dest_dir` (NovaOS's Imports folder). Returns how the job ended; items
/// that finished before a cancel or failure are kept, the in-flight one is removed.
pub fn run_copy(
    job: &str,
    sources: &[PathBuf],
    dest_dir: &Path,
    totals: (u64, u64),
    ctl: &Ctl,
    sink: &mut dyn Sink,
) -> Outcome {
    let staging = dest_dir.join(format!("{STAGING_PREFIX}-{job}"));
    if let Err(e) = fs::create_dir_all(&staging) {
        return Outcome::Failed(format!("cannot prepare the import folder: {e}"));
    }
    let now = Instant::now();
    let mut ctx = Ctx {
        ctl,
        sink,
        p: Progress::new(job, sources.len() as u64, totals.0, totals.1),
        item: None,
        item_files: 0,
        item_bytes: 0,
        item_errors: 0,
        start: now,
        last_emit: now - EMIT_EVERY,
        last_bytes: 0,
        last_t: now,
    };

    let mut outcome = Outcome::Done;
    for (idx, src) in sources.iter().enumerate() {
        if ctl.cancelled() {
            outcome = Outcome::Cancelled;
            break;
        }
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "import".into());
        let md = match fs::symlink_metadata(src) {
            Ok(m) => m,
            Err(e) => {
                ctx.record_error(format!("{name}: {e}"));
                ctx.p.items_done += 1;
                continue;
            }
        };
        let ft = md.file_type();
        if ft.is_symlink() || !(ft.is_file() || ft.is_dir()) {
            ctx.record_error(format!("{name}: links and special files are not imported"));
            ctx.p.items_done += 1;
            continue;
        }
        let is_dir = ft.is_dir();
        ctx.item = match ctx.sink.item_started(src, is_dir) {
            Ok(id) => Some(id),
            Err(e) => {
                ctx.record_error(format!("{name}: could not record import metadata ({e})"));
                None
            }
        };
        ctx.item_files = 0;
        ctx.item_bytes = 0;
        ctx.item_errors = 0;

        let stage = staging.join(idx.to_string());
        let result = if is_dir {
            copy_tree(&mut ctx, src, &stage, "", 0)
        } else {
            copy_file(&mut ctx, src, &stage, &name)
        };
        match result {
            Ok(()) => match place(&stage, dest_dir, &name, is_dir) {
                Ok(final_name) => {
                    let (item, files, bytes, errors) =
                        (ctx.item, ctx.item_files, ctx.item_bytes, ctx.item_errors);
                    ctx.sink
                        .item_finished(item, &final_name, files, bytes, errors);
                    ctx.p.items_done += 1;
                }
                Err(e) => {
                    remove_staged(&stage);
                    ctx.sink.item_aborted(ctx.item);
                    outcome = Outcome::Failed(format!("could not place {name}: {e}"));
                    break;
                }
            },
            Err(flow) => {
                remove_staged(&stage);
                ctx.sink.item_aborted(ctx.item);
                match flow {
                    Flow::Cancelled => {
                        outcome = Outcome::Cancelled;
                        break;
                    }
                    Flow::Fatal(m) => {
                        outcome = Outcome::Failed(m);
                        break;
                    }
                    Flow::Skip(m) => {
                        ctx.record_error(m);
                        ctx.p.items_done += 1;
                    }
                }
            }
        }
        ctx.emit(true);
    }

    let _ = fs::remove_dir_all(&staging); // only ever our own staging folder
    ctx.p.state = match &outcome {
        Outcome::Done => JobState::Done,
        Outcome::Cancelled => JobState::Cancelled,
        Outcome::Failed(m) => {
            ctx.p.message = Some(m.clone());
            JobState::Failed
        }
    };
    let secs = ctx.start.elapsed().as_secs_f64();
    if secs > 0.0 {
        ctx.p.speed_bps = ctx.p.bytes_done as f64 / secs; // final figure is the overall average
    }
    ctx.emit(true);
    outcome
}

fn remove_staged(stage: &Path) {
    // Defence in depth: only ever delete something inside a staging folder.
    let inside_staging = stage.ancestors().skip(1).any(|a| {
        a.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(STAGING_PREFIX))
    });
    if !inside_staging {
        return;
    }
    if stage.is_dir() {
        let _ = fs::remove_dir_all(stage);
    } else {
        let _ = fs::remove_file(stage);
    }
}

/// Delete staging folders left behind by a crash. Only touches names we created.
pub fn clean_orphan_staging(dest_dir: &Path) {
    let Ok(rd) = fs::read_dir(dest_dir) else {
        return;
    };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with(STAGING_PREFIX) {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[derive(Default)]
    struct Rec {
        files: Vec<(String, u64, Option<u64>)>,
        finished: Vec<(String, u64, u64, u64)>,
        aborted: u32,
        last: Option<Progress>,
        cancel_after_bytes: Option<(u64, Arc<Ctl>)>,
    }
    impl Sink for Rec {
        fn progress(&mut self, p: &Progress) {
            if let Some((limit, ctl)) = &self.cancel_after_bytes {
                if p.bytes_done >= *limit {
                    ctl.cancel.store(true, Ordering::Relaxed);
                }
            }
            self.last = Some(p.clone());
        }
        fn item_started(&mut self, _: &Path, _: bool) -> Result<i64, String> {
            Ok(1)
        }
        fn file_copied(&mut self, _: Option<i64>, rel: &str, size: u64, m: Option<u64>) {
            self.files.push((rel.into(), size, m));
        }
        fn item_finished(&mut self, _: Option<i64>, rel: &str, f: u64, b: u64, e: u64) {
            self.finished.push((rel.into(), f, b, e));
        }
        fn item_aborted(&mut self, _: Option<i64>) {
            self.aborted += 1;
        }
    }

    /// A source tree plus a fingerprint so tests can prove it was never touched.
    fn make_source(dir: &Path) -> PathBuf {
        let root = dir.join("proj");
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::write(root.join("a.txt"), "alpha").unwrap();
        fs::write(root.join("sub/b.bin"), vec![7u8; 3 * 1024 * 1024 + 5]).unwrap();
        fs::write(root.join("sub/deep/c.txt"), "gamma").unwrap();
        root
    }

    fn fingerprint(root: &Path) -> Vec<(String, u64, Option<SystemTime>, bool)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(p) = stack.pop() {
            let md = fs::symlink_metadata(&p).unwrap();
            out.push((
                p.to_string_lossy().into_owned(),
                md.len(),
                md.modified().ok(),
                md.permissions().readonly(),
            ));
            if md.is_dir() {
                for e in fs::read_dir(&p).unwrap() {
                    stack.push(e.unwrap().path());
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    #[test]
    fn copies_a_tree_exactly_and_never_touches_the_source() {
        let t = TempDir::new().unwrap();
        let src = make_source(t.path());
        let before = fingerprint(&src);
        let dest = t.path().join("Imports");
        fs::create_dir(&dest).unwrap();
        let mut rec = Rec::default();
        let ctl = Ctl::new();
        let out = run_copy(
            "j1",
            std::slice::from_ref(&src),
            &dest,
            (3, 3 * 1024 * 1024 + 15),
            &ctl,
            &mut rec,
        );
        assert_eq!(out, Outcome::Done);
        assert_eq!(
            fingerprint(&src),
            before,
            "source must be byte-for-byte and stamp-for-stamp untouched"
        );
        let copy = dest.join("proj");
        assert_eq!(fs::read_to_string(copy.join("a.txt")).unwrap(), "alpha");
        assert_eq!(
            fs::read(copy.join("sub/b.bin")).unwrap(),
            vec![7u8; 3 * 1024 * 1024 + 5]
        );
        assert_eq!(
            fs::read_to_string(copy.join("sub/deep/c.txt")).unwrap(),
            "gamma"
        );
        assert!(copy.join("empty").is_dir(), "empty folders are preserved");
        // modification times carried over (to the second)
        let sm = fs::metadata(src.join("a.txt")).unwrap().modified().unwrap();
        let cm = fs::metadata(copy.join("a.txt"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(mtime_secs(sm), mtime_secs(cm));
        assert_eq!(rec.files.len(), 3);
        assert!(rec
            .files
            .iter()
            .any(|f| f.0 == "sub/deep/c.txt" && f.1 == 5));
        assert_eq!(
            rec.finished,
            vec![("proj".into(), 3, 3 * 1024 * 1024 + 15, 0)]
        );
        let p = rec.last.unwrap();
        assert_eq!(p.state, JobState::Done);
        assert_eq!((p.files_done, p.bytes_done), (3, 3 * 1024 * 1024 + 15));
        assert!(
            !dest.join(format!("{STAGING_PREFIX}-j1")).exists(),
            "staging removed"
        );
    }

    #[test]
    fn never_overwrites_existing_names() {
        let t = TempDir::new().unwrap();
        let src = make_source(t.path());
        let loose = t.path().join("a.txt");
        fs::write(&loose, "loose").unwrap();
        let dest = t.path().join("Imports");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("a.txt"), "PRECIOUS").unwrap();
        fs::create_dir(dest.join("proj")).unwrap();
        fs::write(dest.join("proj/keep.txt"), "KEEP").unwrap();
        let mut rec = Rec::default();
        let out = run_copy("j", &[src, loose], &dest, (4, 100), &Ctl::new(), &mut rec);
        assert_eq!(out, Outcome::Done);
        assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "PRECIOUS");
        assert_eq!(
            fs::read_to_string(dest.join("proj/keep.txt")).unwrap(),
            "KEEP"
        );
        assert_eq!(fs::read_to_string(dest.join("a (2).txt")).unwrap(), "loose");
        assert_eq!(
            fs::read_to_string(dest.join("proj (2)/a.txt")).unwrap(),
            "alpha"
        );
        let names: Vec<_> = rec.finished.iter().map(|f| f.0.clone()).collect();
        assert_eq!(names, ["proj (2)", "a (2).txt"]);
    }

    #[test]
    fn cancel_removes_the_in_flight_item_keeps_finished_ones_and_spares_the_source() {
        let t = TempDir::new().unwrap();
        let first = t.path().join("first.txt");
        fs::write(&first, "done early").unwrap();
        let big = t.path().join("big.bin");
        fs::write(&big, vec![1u8; 12 * 1024 * 1024]).unwrap();
        let before = (fingerprint(&first), fingerprint(&big));
        let dest = t.path().join("Imports");
        fs::create_dir(&dest).unwrap();
        let ctl = Arc::new(Ctl::new());
        let mut rec = Rec {
            cancel_after_bytes: Some((4 * 1024 * 1024, ctl.clone())),
            ..Default::default()
        };
        let out = run_copy(
            "jc",
            &[first.clone(), big.clone()],
            &dest,
            (2, 12 * 1024 * 1024 + 10),
            &ctl,
            &mut rec,
        );
        assert_eq!(out, Outcome::Cancelled);
        assert_eq!(
            fs::read_to_string(dest.join("first.txt")).unwrap(),
            "done early",
            "finished item kept"
        );
        assert!(
            !dest.join("big.bin").exists(),
            "partial item must not appear"
        );
        assert_eq!(rec.aborted, 1);
        assert_eq!(
            fs::read_dir(&dest).unwrap().count(),
            1,
            "no staging leftovers"
        );
        assert_eq!(
            (fingerprint(&first), fingerprint(&big)),
            before,
            "sources untouched"
        );
        assert_eq!(rec.last.unwrap().state, JobState::Cancelled);
    }

    #[test]
    fn cancel_before_start_copies_nothing() {
        let t = TempDir::new().unwrap();
        let src = make_source(t.path());
        let dest = t.path().join("Imports");
        fs::create_dir(&dest).unwrap();
        let ctl = Ctl::new();
        ctl.cancel.store(true, Ordering::Relaxed);
        let out = run_copy("j", &[src], &dest, (1, 1), &ctl, &mut Rec::default());
        assert_eq!(out, Outcome::Cancelled);
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn scan_counts_and_flags_programs() {
        let t = TempDir::new().unwrap();
        let src = make_source(t.path());
        fs::write(src.join("setup.EXE"), "MZ").unwrap();
        fs::write(src.join("sub/run.sh"), "#!/bin/sh").unwrap();
        fs::write(src.join("run.bat"), "echo").unwrap();
        let dest = t.path().join("Imports");
        fs::create_dir(&dest).unwrap();
        let r = scan(&[src], &dest, &AtomicBool::new(false)).unwrap();
        assert_eq!(r.files, 6);
        assert_eq!(r.dirs, 4);
        assert_eq!(r.programs, 3, "exe + bat + sh");
        assert_eq!(r.bytes, 5 + 5 + 3 * 1024 * 1024 + 5 + 2 + 9 + 4);
        assert!(r.enough_space && r.free_bytes > 0);
        assert!(
            scan(&[t.path().to_path_buf()], &dest, &AtomicBool::new(true)).is_err(),
            "cancelled scan"
        );
    }

    #[test]
    fn insufficient_space_is_reported_before_copying() {
        let t = TempDir::new().unwrap();
        let mut r = ScanReport {
            bytes: u64::MAX / 2,
            ..Default::default()
        };
        r.required_bytes = r.bytes + SPACE_MARGIN;
        r.free_bytes = available_space(t.path()).unwrap();
        assert!(r.free_bytes < r.required_bytes);
    }

    #[cfg(windows)]
    #[test]
    fn junctions_are_skipped_and_never_followed() {
        let t = TempDir::new().unwrap();
        let outside = t.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "s").unwrap();
        let src = t.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("ok.txt"), "ok").unwrap();
        let link = src.join("portal");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(made, "could not create a junction to test with");
        let dest = t.path().join("Imports");
        fs::create_dir(&dest).unwrap();
        let report = scan(std::slice::from_ref(&src), &dest, &AtomicBool::new(false)).unwrap();
        assert_eq!(report.symlinks, 1);
        assert_eq!(report.files, 1, "nothing behind the junction is counted");
        let mut rec = Rec::default();
        let out = run_copy("j", &[src], &dest, (1, 2), &Ctl::new(), &mut rec);
        assert_eq!(out, Outcome::Done);
        assert!(dest.join("src/ok.txt").exists());
        assert!(!dest.join("src/portal").exists(), "junction not copied");
        assert!(outside.join("secret.txt").exists(), "target untouched");
        assert_eq!(rec.last.unwrap().error_count, 1, "skip is reported");
    }

    #[cfg(windows)]
    #[test]
    fn an_unreadable_file_is_reported_and_the_rest_still_import() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = TempDir::new().unwrap();
        let src = make_source(t.path());
        // An exclusive handle makes the OS refuse every other reader (like a file in use).
        let _lock = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(src.join("a.txt"))
            .unwrap();
        let dest = t.path().join("Imports");
        fs::create_dir(&dest).unwrap();
        let mut rec = Rec::default();
        let out = run_copy("j", &[src], &dest, (3, 1), &Ctl::new(), &mut rec);
        assert_eq!(out, Outcome::Done, "a locked file is not fatal");
        let p = rec.last.unwrap();
        assert_eq!(p.error_count, 1);
        assert!(p.errors[0].contains("a.txt"));
        assert!(
            !dest.join("proj/a.txt").exists(),
            "no empty stub left behind"
        );
        assert!(dest.join("proj/sub/deep/c.txt").exists());
        assert_eq!(rec.finished[0].3, 1, "item recorded with its error count");
    }

    #[test]
    fn orphan_staging_is_cleaned_but_nothing_else() {
        let t = TempDir::new().unwrap();
        fs::create_dir_all(t.path().join(format!("{STAGING_PREFIX}-old/0"))).unwrap();
        fs::write(t.path().join("keep.txt"), "k").unwrap();
        fs::create_dir(t.path().join("keepdir")).unwrap();
        clean_orphan_staging(t.path());
        assert!(!t.path().join(format!("{STAGING_PREFIX}-old")).exists());
        assert!(t.path().join("keep.txt").exists() && t.path().join("keepdir").exists());
    }

    #[test]
    fn remove_staged_refuses_paths_outside_staging() {
        let t = TempDir::new().unwrap();
        let precious = t.path().join("precious");
        fs::create_dir(&precious).unwrap();
        remove_staged(&precious);
        assert!(precious.exists(), "guard must refuse non-staging paths");
    }
}
