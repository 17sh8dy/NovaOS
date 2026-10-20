//! Virtual filesystem for Nova Files and every later app that touches disk.
//!
//! The UI never sees raw host paths. It names a *root* (an id such as `nova` or `host-documents`)
//! plus a `/`-separated path *relative to that root*. Every operation resolves that pair through
//! [`resolve_in`], which fails closed if the result could land outside the root.

use serde::Serialize;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::UNIX_EPOCH;

pub const MAX_TEXT_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_COPY_DEPTH: usize = 64;

/// Opening these through the host would execute code. NovaOS refuses; run them from Windows.
const BLOCKED_OPEN: &[&str] = &[
    "exe",
    "com",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "msi",
    "msp",
    "scr",
    "lnk",
    "url",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "reg",
    "dll",
    "hta",
    "jar",
    "cpl",
    "msc",
    "appref-ms",
    "application",
];

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum RootKind {
    /// NovaOS's own organized user environment inside the app data directory.
    Nova,
    /// A standard host user folder (Documents, Pictures, ...).
    Host,
    /// A folder the user explicitly added.
    Custom,
}

#[derive(Serialize, Clone, Debug)]
pub struct Root {
    pub id: String,
    pub label: String,
    pub kind: RootKind,
    #[serde(serialize_with = "ser_path")]
    pub path: PathBuf,
}

fn ser_path<S: serde::Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&p.to_string_lossy())
}

#[derive(Serialize, Debug)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub modified: Option<u64>,
}

#[derive(Serialize, Debug)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

// ---------------------------------------------------------------- names and path resolution

/// A single path component: what a user may type as a file or folder name.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 255 {
        return Err("name must be 1-255 characters".into());
    }
    if name == "." || name == ".." {
        return Err("invalid name".into());
    }
    if name
        .chars()
        .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err("name contains a character that is not allowed".into());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("name cannot end with a dot or space".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0');
    if reserved {
        return Err("that name is reserved by Windows".into());
    }
    Ok(())
}

fn segments(rel: &str) -> Result<Vec<&str>, String> {
    let mut out = Vec::new();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        validate_name(seg)?; // also rejects "..", drive letters and alternate data streams
        out.push(seg);
    }
    Ok(out)
}

/// Resolve `rel` under `root` (which must already be canonical). Symlinks and junctions that lead
/// outside the root are caught by canonicalizing the deepest existing ancestor.
pub fn resolve_in(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let segs = segments(rel)?;
    let mut path = root.to_path_buf();
    for s in &segs {
        path.push(s);
    }
    let mut existing = path.clone();
    let mut tail: Vec<OsString> = Vec::new();
    while fs::symlink_metadata(&existing).is_err() {
        match existing.file_name() {
            Some(n) => tail.push(n.to_os_string()),
            None => return Err("path does not exist".into()),
        }
        if !existing.pop() {
            return Err("path does not exist".into());
        }
    }
    let canon = existing.canonicalize().map_err(|e| e.to_string())?;
    if !canon.starts_with(root) {
        return Err("path leaves this location".into());
    }
    let mut out = canon;
    for t in tail.iter().rev() {
        out.push(t);
    }
    Ok(out)
}

fn is_root_path(rel: &str) -> Result<bool, String> {
    Ok(segments(rel)?.is_empty())
}

fn join_rel(dir: &str, name: &str) -> Result<String, String> {
    let mut segs = segments(dir)?;
    segs.push(name);
    Ok(segs.join("/"))
}

fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| fs::symlink_metadata(p).is_err())
        .expect("unbounded range")
}

fn copy_tree(src: &Path, dst: &Path, depth: usize) -> Result<(), String> {
    if depth > MAX_COPY_DEPTH {
        return Err("folder is nested too deeply to copy".into());
    }
    let md = fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    if md.file_type().is_symlink() {
        return Ok(()); // links are never followed or duplicated
    }
    if md.is_dir() {
        fs::create_dir(dst).map_err(|e| e.to_string())?;
        for e in fs::read_dir(src).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            copy_tree(&e.path(), &dst.join(e.file_name()), depth + 1)?;
        }
    } else {
        fs::copy(src, dst).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------- the filesystem object

pub struct Vfs {
    roots: RwLock<Vec<Root>>,
}

impl Vfs {
    pub fn new(roots: Vec<Root>) -> Self {
        let roots = roots
            .into_iter()
            .filter_map(|mut r| {
                r.path = r.path.canonicalize().ok()?;
                Some(r)
            })
            .collect();
        Self {
            roots: RwLock::new(roots),
        }
    }

    pub fn roots(&self) -> Vec<Root> {
        self.roots.read().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn custom_paths(&self) -> Vec<String> {
        self.roots()
            .into_iter()
            .filter(|r| r.kind == RootKind::Custom)
            .map(|r| r.path.to_string_lossy().into_owned())
            .collect()
    }

    /// Register a user-chosen folder. Whole drives and system locations are refused.
    pub fn add_custom(&self, path: &Path) -> Result<Root, String> {
        self.add_custom_inner(path, true)
    }

    fn add_custom_inner(&self, path: &Path, check_protected: bool) -> Result<Root, String> {
        let canon = path.canonicalize().map_err(|e| e.to_string())?;
        if !canon.is_dir() {
            return Err("that is not a folder".into());
        }
        if canon.parent().is_none() {
            return Err("whole drives cannot be added; pick a folder".into());
        }
        if check_protected && is_protected(&canon) {
            return Err("system and program folders cannot be added".into());
        }
        let mut roots = self.roots.write().map_err(|_| "lock poisoned")?;
        if roots.iter().any(|r| r.path == canon) {
            return Err("that folder is already available".into());
        }
        let label = canon
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Folder".into());
        let id = (1..)
            .map(|n| format!("custom-{n}"))
            .find(|id| !roots.iter().any(|r| &r.id == id))
            .expect("unbounded range");
        let root = Root {
            id,
            label,
            kind: RootKind::Custom,
            path: canon,
        };
        roots.push(root.clone());
        Ok(root)
    }

    pub fn remove_custom(&self, id: &str) -> Result<(), String> {
        let mut roots = self.roots.write().map_err(|_| "lock poisoned")?;
        let before = roots.len();
        roots.retain(|r| !(r.id == id && r.kind == RootKind::Custom));
        if roots.len() == before {
            return Err("only folders you added can be removed".into());
        }
        Ok(())
    }

    fn path(&self, root: &str, rel: &str) -> Result<PathBuf, String> {
        let roots = self.roots.read().map_err(|_| "lock poisoned")?;
        let r = roots
            .iter()
            .find(|r| r.id == root)
            .ok_or("unknown location")?;
        resolve_in(&r.path, rel)
    }

    /// Like `path` but refuses the root itself, so a root can never be renamed, moved or deleted.
    fn path_inside(&self, root: &str, rel: &str) -> Result<PathBuf, String> {
        if is_root_path(rel)? {
            return Err("this location itself cannot be changed".into());
        }
        self.path(root, rel)
    }

    pub fn list(&self, root: &str, rel: &str) -> Result<Listing, String> {
        let dir = self.path(root, rel)?;
        let mut entries = Vec::new();
        let mut truncated = false;
        for e in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let Ok(e) = e else { continue };
            if entries.len() >= MAX_ENTRIES {
                truncated = true;
                break;
            }
            let Ok(md) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            let is_symlink = md.file_type().is_symlink();
            // Follow links only for display; navigation is re-checked by `resolve_in`.
            let target = if is_symlink {
                fs::metadata(e.path()).ok()
            } else {
                Some(md.clone())
            };
            let (is_dir, size) = target
                .as_ref()
                .map(|t| (t.is_dir(), t.len()))
                .unwrap_or((false, 0));
            entries.push(Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir,
                is_symlink,
                size: if is_dir { 0 } else { size },
                modified: md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()),
            });
        }
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(Listing { entries, truncated })
    }

    pub fn mkdir(&self, root: &str, rel: &str) -> Result<(), String> {
        let p = self.path_inside(root, rel)?;
        fs::create_dir(p).map_err(|e| e.to_string())
    }

    pub fn create_file(&self, root: &str, rel: &str) -> Result<(), String> {
        let p = self.path_inside(root, rel)?;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Returns the new relative path.
    pub fn rename(&self, root: &str, rel: &str, new_name: &str) -> Result<String, String> {
        validate_name(new_name)?;
        let from = self.path_inside(root, rel)?;
        let to = from.parent().ok_or("no parent folder")?.join(new_name);
        if fs::symlink_metadata(&to).is_ok() {
            return Err("something with that name already exists".into());
        }
        fs::rename(&from, &to).map_err(|e| e.to_string())?;
        let mut segs = segments(rel)?;
        segs.pop();
        segs.push(new_name);
        Ok(segs.join("/"))
    }

    /// Copy `rel` into the folder `dest_dir` of `dest_root`. Never overwrites: a clash gets
    /// " (2)" appended. Returns the new relative path.
    pub fn copy(
        &self,
        root: &str,
        rel: &str,
        dest_root: &str,
        dest_dir: &str,
    ) -> Result<String, String> {
        let (src, dest_dir_abs, name) = self.transfer_paths(root, rel, dest_root, dest_dir)?;
        let dst = unique_dest(&dest_dir_abs, &name);
        copy_tree(&src, &dst, 0)?;
        join_rel(
            dest_dir,
            &dst.file_name().unwrap_or_default().to_string_lossy(),
        )
    }

    pub fn mv(
        &self,
        root: &str,
        rel: &str,
        dest_root: &str,
        dest_dir: &str,
    ) -> Result<String, String> {
        let (src, dest_dir_abs, name) = self.transfer_paths(root, rel, dest_root, dest_dir)?;
        if src.parent() == Some(dest_dir_abs.as_path()) {
            return Ok(segments(rel)?.join("/")); // already there
        }
        let dst = unique_dest(&dest_dir_abs, &name);
        if fs::rename(&src, &dst).is_err() {
            // Different drives: copy then remove the original, but only if the copy fully worked.
            copy_tree(&src, &dst, 0)?;
            if src.is_dir() {
                fs::remove_dir_all(&src).map_err(|e| e.to_string())?;
            } else {
                fs::remove_file(&src).map_err(|e| e.to_string())?;
            }
        }
        join_rel(
            dest_dir,
            &dst.file_name().unwrap_or_default().to_string_lossy(),
        )
    }

    fn transfer_paths(
        &self,
        root: &str,
        rel: &str,
        dest_root: &str,
        dest_dir: &str,
    ) -> Result<(PathBuf, PathBuf, String), String> {
        let src = self.path_inside(root, rel)?;
        let dest_dir_abs = self.path(dest_root, dest_dir)?;
        if !dest_dir_abs.is_dir() {
            return Err("destination is not a folder".into());
        }
        if src.is_dir() && dest_dir_abs.starts_with(&src) {
            return Err("a folder cannot be placed inside itself".into());
        }
        let name = src
            .file_name()
            .ok_or("invalid source")?
            .to_string_lossy()
            .into_owned();
        Ok((src, dest_dir_abs, name))
    }

    /// Moves to the Recycle Bin rather than deleting permanently.
    pub fn trash(&self, root: &str, rel: &str) -> Result<(), String> {
        let p = self.path_inside(root, rel)?;
        trash::delete(p).map_err(|e| e.to_string())
    }

    pub fn read_text(&self, root: &str, rel: &str) -> Result<String, String> {
        let p = self.path_inside(root, rel)?;
        let md = fs::metadata(&p).map_err(|e| e.to_string())?;
        if !md.is_file() {
            return Err("not a file".into());
        }
        if md.len() > MAX_TEXT_BYTES {
            return Err("file is too large to edit (limit 1 MB)".into());
        }
        let bytes = fs::read(&p).map_err(|e| e.to_string())?;
        String::from_utf8(bytes).map_err(|_| "not a UTF-8 text file".to_string())
    }

    pub fn write_text(&self, root: &str, rel: &str, text: &str) -> Result<(), String> {
        if text.len() as u64 > MAX_TEXT_BYTES {
            return Err("text is too large (limit 1 MB)".into());
        }
        let p = self.path_inside(root, rel)?;
        if p.is_dir() {
            return Err("that is a folder".into());
        }
        fs::write(p, text).map_err(|e| e.to_string())
    }

    /// Hand a file to the host's default application, unless it could execute code.
    pub fn open_externally(&self, root: &str, rel: &str) -> Result<(), String> {
        let p = self.path_inside(root, rel)?;
        if p.is_dir() {
            return Err("that is a folder".into());
        }
        if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            if BLOCKED_OPEN.contains(&ext.to_ascii_lowercase().as_str()) {
                return Err(format!(
                    ".{ext} files can run code, so NovaOS will not launch them"
                ));
            }
        }
        opener::open(p).map_err(|e| e.to_string())
    }
}

/// Folders that must never become a NovaOS location, even by explicit user choice.
fn is_protected(p: &Path) -> bool {
    let mut blocked: Vec<PathBuf> = [
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .filter_map(|v| PathBuf::from(v).canonicalize().ok())
    .collect();
    if let Some(home) =
        std::env::var_os("USERPROFILE").and_then(|v| PathBuf::from(v).canonicalize().ok())
    {
        // The whole profile holds AppData, credentials and browser data; its subfolders are fine.
        if p == home {
            return true;
        }
        blocked.push(home.join("AppData"));
    }
    blocked.iter().any(|b| p.starts_with(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vfs_with(dir: &TempDir) -> Vfs {
        Vfs::new(vec![Root {
            id: "t".into(),
            label: "Test".into(),
            kind: RootKind::Nova,
            path: dir.path().to_path_buf(),
        }])
    }

    #[test]
    fn rejects_traversal_and_odd_paths() {
        let d = TempDir::new().unwrap();
        let v = vfs_with(&d);
        for bad in [
            "..",
            "a/../..",
            "../x",
            "C:/Windows",
            "a\\b",
            "a/b:stream",
            "CON",
            "x/NUL.txt",
            "a/b ",
            "a/b.",
        ] {
            assert!(v.list("t", bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(v.list("t", "").is_ok());
        assert!(v.list("t", "/./").is_ok());
        assert!(v.list("nope", "").is_err(), "unknown root");
    }

    #[test]
    fn root_itself_is_immutable() {
        let d = TempDir::new().unwrap();
        let v = vfs_with(&d);
        assert!(v.rename("t", "", "x").is_err());
        assert!(v.trash("t", "").is_err());
        assert!(v.mv("t", "", "t", "").is_err());
        assert!(v.mkdir("t", "").is_err());
    }

    #[test]
    fn basic_lifecycle_and_sorting() {
        let d = TempDir::new().unwrap();
        let v = vfs_with(&d);
        v.mkdir("t", "zeta").unwrap();
        v.mkdir("t", "Alpha").unwrap();
        v.create_file("t", "b.txt").unwrap();
        v.create_file("t", "a.txt").unwrap();
        assert!(v.create_file("t", "a.txt").is_err(), "no silent overwrite");
        let names: Vec<_> = v
            .list("t", "")
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(
            names,
            ["Alpha", "zeta", "a.txt", "b.txt"],
            "dirs first, case-insensitive"
        );
        assert_eq!(v.rename("t", "a.txt", "c.txt").unwrap(), "c.txt");
        assert!(
            v.rename("t", "c.txt", "b.txt").is_err(),
            "rename onto existing"
        );
        assert!(v.rename("t", "c.txt", "../evil").is_err());
        assert!(v.rename("t", "c.txt", "a/b").is_err());
    }

    #[test]
    fn copy_never_overwrites_and_copies_trees() {
        let d = TempDir::new().unwrap();
        let v = vfs_with(&d);
        v.mkdir("t", "src").unwrap();
        v.write_text("t", "src/f.txt", "hi").unwrap();
        v.mkdir("t", "dst").unwrap();
        assert_eq!(v.copy("t", "src", "t", "dst").unwrap(), "dst/src");
        assert_eq!(v.copy("t", "src", "t", "dst").unwrap(), "dst/src (2)");
        assert_eq!(v.read_text("t", "dst/src (2)/f.txt").unwrap(), "hi");
        assert_eq!(
            v.copy("t", "src/f.txt", "t", "src").unwrap(),
            "src/f (2).txt"
        );
    }

    #[test]
    fn move_works_and_refuses_into_self() {
        let d = TempDir::new().unwrap();
        let v = vfs_with(&d);
        v.mkdir("t", "a").unwrap();
        v.mkdir("t", "a/b").unwrap();
        v.create_file("t", "f.txt").unwrap();
        assert_eq!(v.mv("t", "f.txt", "t", "a/b").unwrap(), "a/b/f.txt");
        assert!(v
            .list("t", "a/b")
            .unwrap()
            .entries
            .iter()
            .any(|e| e.name == "f.txt"));
        assert!(
            v.mv("t", "a", "t", "a/b").is_err(),
            "into its own subfolder"
        );
        assert!(v.mv("t", "a", "t", "a").is_err(), "into itself");
        assert_eq!(
            v.mv("t", "a/b", "t", "a").unwrap(),
            "a/b",
            "already there is a no-op"
        );
    }

    #[test]
    fn text_limits_and_utf8() {
        let d = TempDir::new().unwrap();
        let v = vfs_with(&d);
        v.write_text("t", "n.txt", "héllo").unwrap();
        assert_eq!(v.read_text("t", "n.txt").unwrap(), "héllo");
        assert!(v
            .write_text("t", "big.txt", &"x".repeat(MAX_TEXT_BYTES as usize + 1))
            .is_err());
        fs::write(d.path().join("bin.dat"), [0xff, 0xfe, 0x00]).unwrap();
        assert!(v.read_text("t", "bin.dat").unwrap_err().contains("UTF-8"));
        fs::write(
            d.path().join("huge.txt"),
            vec![b'a'; MAX_TEXT_BYTES as usize + 1],
        )
        .unwrap();
        assert!(v
            .read_text("t", "huge.txt")
            .unwrap_err()
            .contains("too large"));
    }

    #[test]
    fn refuses_to_launch_executables() {
        let d = TempDir::new().unwrap();
        let v = vfs_with(&d);
        for n in ["a.exe", "b.BAT", "c.ps1", "d.lnk", "e.Js"] {
            v.create_file("t", n).unwrap();
            assert!(
                v.open_externally("t", n).unwrap_err().contains("run code"),
                "{n}"
            );
        }
        v.mkdir("t", "dir").unwrap();
        assert!(v.open_externally("t", "dir").is_err());
    }

    #[test]
    fn symlink_or_junction_escape_is_blocked() {
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.txt"), "s").unwrap();
        let d = TempDir::new().unwrap();
        let v = vfs_with(&d);
        let link = d.path().join("portal");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok()
            || std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(outside.path())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(outside.path(), &link).is_ok();
        assert!(made, "could not create a link to test with");
        assert!(v.list("t", "portal").is_err(), "list through link");
        assert!(
            v.read_text("t", "portal/secret.txt").is_err(),
            "read through link"
        );
        assert!(
            v.write_text("t", "portal/new.txt", "x").is_err(),
            "write through link"
        );
        assert!(!outside.path().join("new.txt").exists());
        // The link itself is visible in the parent listing, flagged as a link.
        assert!(v
            .list("t", "")
            .unwrap()
            .entries
            .iter()
            .any(|e| e.name == "portal" && e.is_symlink));
    }

    #[test]
    fn custom_roots_refuse_drives_and_system_folders() {
        let d = TempDir::new().unwrap();
        let v = vfs_with(&d);
        assert!(v.add_custom(Path::new("C:\\")).is_err());
        if let Some(win) = std::env::var_os("SystemRoot") {
            assert!(v.add_custom(Path::new(&win)).is_err());
            assert!(v.add_custom(&PathBuf::from(&win).join("System32")).is_err());
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            assert!(v.add_custom(Path::new(&home)).is_err(), "whole profile");
        }
        let extra = TempDir::new().unwrap();
        // Temp lives under AppData (protected), so exercise the bookkeeping unchecked.
        let r = v.add_custom_inner(extra.path(), false).unwrap();
        assert_eq!(r.kind, RootKind::Custom);
        assert!(
            v.add_custom_inner(extra.path(), false).is_err(),
            "duplicate"
        );
        assert!(v.add_custom(extra.path()).is_err(), "AppData is protected");
        assert!(
            v.remove_custom("t").is_err(),
            "built-in roots are not removable"
        );
        v.remove_custom(&r.id).unwrap();
    }
}
