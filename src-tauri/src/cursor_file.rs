//! Adding a custom mouse cursor: validate a user-chosen cursor file and copy it into NovaOS's folder.
//!
//! Same rules as wallpapers: the original is only read, the copy is checked by its real contents
//! (magic bytes), size-limited, given a safe name, and never overwrites an existing file. NovaOS
//! never runs anything from a cursor file; the UI decodes it as an image.

use crate::import::{place, STAGING_PREFIX};
use crate::wallpaper::safe_name_or;
use serde::Serialize;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Cursor files are tiny; animated ones (.ani) carry many frames, so they get more room.
pub const MAX_CURSOR_BYTES: u64 = 2 * 1024 * 1024;
pub const EXTENSIONS: &[&str] = &["cur", "ani", "png", "gif"];

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct CursorInfo {
    /// File name inside the Cursors folder; this is what settings store.
    pub file: String,
}

/// True if `head` starts like a real file of the type the extension claims.
fn matches_magic(ext: &str, head: &[u8]) -> bool {
    match ext {
        // ICONDIR: reserved 0, type 2 = cursor.
        "cur" => head.starts_with(&[0, 0, 2, 0]),
        // RIFF container whose form type is ACON.
        "ani" => head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"ACON",
        "png" => head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "gif" => head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a"),
        _ => false,
    }
}

/// Copy `src` into `dir` (the Cursors folder). Returns the stored file name.
pub fn add_cursor(src: &Path, dir: &Path) -> Result<CursorInfo, String> {
    let md = fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    if md.file_type().is_symlink() || !md.is_file() {
        return Err("choose a cursor file".into());
    }
    if md.len() == 0 {
        return Err("that file is empty".into());
    }
    if md.len() > MAX_CURSOR_BYTES {
        return Err("that cursor is too large (limit 2 MB)".into());
    }
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| EXTENSIONS.contains(&e.as_str()))
        .ok_or("supported types: .cur, .ani, PNG, GIF (unzip downloaded packs first)")?;
    let mut input = File::open(src).map_err(|e| e.to_string())?;
    let mut head = [0u8; 12];
    let n = input.read(&mut head).map_err(|e| e.to_string())?;
    if !matches_magic(&ext, &head[..n]) {
        return Err(format!("that file is not a real .{ext} cursor"));
    }
    drop(input);

    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let staging = dir.join(format!("{STAGING_PREFIX}-cur-{nonce}"));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let staged = staging.join("f");
        fs::copy(src, &staged).map_err(|e| e.to_string())?; // reads src; writes only inside staging
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        place(&staged, dir, &safe_name_or(&stem, &ext, "cursor"), false)
            .map(|file| CursorInfo { file })
    })();
    let _ = fs::remove_dir_all(&staging); // our own staging folder
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const CUR: &[u8] = &[0, 0, 2, 0, 1, 0, 32, 32, 0, 0, 4, 0, 5, 0, 9, 9];
    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0, 1, 2, 3,
    ];

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn copies_a_real_cursor_and_leaves_the_original_alone() {
        let t = TempDir::new().unwrap();
        let src = write(t.path(), "Arrow.CUR", CUR);
        let dir = t.path().join("Cursors");
        let info = add_cursor(&src, &dir).unwrap();
        assert_eq!(info.file, "Arrow.cur", "extension is normalized");
        assert_eq!(fs::read(dir.join("Arrow.cur")).unwrap(), CUR);
        assert_eq!(fs::read(&src).unwrap(), CUR);
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "no staging leftovers"
        );
    }

    #[test]
    fn same_name_twice_never_overwrites() {
        let t = TempDir::new().unwrap();
        let a = write(t.path(), "c.cur", CUR);
        let dir = t.path().join("C");
        assert_eq!(add_cursor(&a, &dir).unwrap().file, "c.cur");
        assert_eq!(add_cursor(&a, &dir).unwrap().file, "c (2).cur");
    }

    #[test]
    fn accepts_each_supported_format_by_its_real_header() {
        let t = TempDir::new().unwrap();
        let dir = t.path().join("C");
        let cases: &[(&str, &[u8])] = &[
            ("a.cur", CUR),
            ("b.ani", b"RIFF\x20\x00\x00\x00ACONLIST"),
            ("c.png", PNG),
            ("d.gif", b"GIF89a\x01\x00"),
        ];
        for (name, bytes) in cases {
            assert!(
                add_cursor(&write(t.path(), name, bytes), &dir).is_ok(),
                "{name}"
            );
        }
    }

    #[test]
    fn rejects_files_that_only_pretend_to_be_cursors() {
        let t = TempDir::new().unwrap();
        let dir = t.path().join("C");
        let fake = write(t.path(), "cool.cur", b"MZ\x90\x00 a program");
        assert!(add_cursor(&fake, &dir)
            .unwrap_err()
            .contains("not a real .cur"));
        // An .ico is not a cursor, and RIFF/WAVE is not ACON.
        assert!(add_cursor(&write(t.path(), "i.cur", &[0, 0, 1, 0, 1, 0]), &dir).is_err());
        assert!(add_cursor(
            &write(t.path(), "w.ani", b"RIFF\x20\x00\x00\x00WAVEfmt "),
            &dir
        )
        .is_err());
        assert!(!dir.exists() || fs::read_dir(&dir).unwrap().count() == 0);
    }

    #[test]
    fn rejects_bad_types_sizes_empty_and_non_files() {
        let t = TempDir::new().unwrap();
        let dir = t.path().join("C");
        assert!(add_cursor(&write(t.path(), "a.exe", CUR), &dir).is_err());
        assert!(add_cursor(&write(t.path(), "a.zip", b"PK\x03\x04"), &dir)
            .unwrap_err()
            .contains("unzip"));
        assert!(add_cursor(&write(t.path(), "a.svg", b"<svg/>"), &dir).is_err());
        assert!(add_cursor(&write(t.path(), "e.cur", b""), &dir)
            .unwrap_err()
            .contains("empty"));
        let big = t.path().join("big.cur");
        let mut data = CUR.to_vec();
        data.resize(MAX_CURSOR_BYTES as usize + 1, 0);
        fs::write(&big, data).unwrap();
        assert!(add_cursor(&big, &dir).unwrap_err().contains("too large"));
        assert!(
            add_cursor(t.path(), &dir).is_err(),
            "a folder is not a cursor"
        );
        assert!(add_cursor(&t.path().join("missing.cur"), &dir).is_err());
    }

    #[test]
    fn an_unnameable_file_gets_a_safe_fallback_name() {
        let t = TempDir::new().unwrap();
        let dir = t.path().join("C");
        let src = write(t.path(), "...cur", CUR); // stem "..", cleans to nothing
        let name = add_cursor(&src, &dir).unwrap().file;
        assert!(name.ends_with(".cur"));
        assert!(name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ._()-".contains(c)));
    }
}
