//! Adding a custom wallpaper: validate a user-chosen image and copy it into NovaOS's own folder.
//!
//! The original is only ever read. The copy is checked by its real contents (magic bytes), not
//! just its extension, size-limited, given a safe name, and never overwrites an existing file.

use crate::import::{place, STAGING_PREFIX};
use serde::Serialize;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_WALLPAPER_BYTES: u64 = 30 * 1024 * 1024;
pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];
const MAX_NAME_CHARS: usize = 80;

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct WallpaperInfo {
    /// File name inside the Wallpapers folder; this is what settings store.
    pub file: String,
}

/// True if `bytes` start like a real image of the type the extension claims.
fn matches_magic(ext: &str, head: &[u8]) -> bool {
    match ext {
        "png" => head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "jpg" | "jpeg" => head.starts_with(&[0xFF, 0xD8, 0xFF]),
        "gif" => head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a"),
        "bmp" => head.starts_with(b"BM"),
        "webp" => head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP",
        _ => false,
    }
}

/// Letters, digits, space, dot, dash, underscore and parentheses only; everything else becomes `_`.
/// Keeps stored names trivially safe to put in settings, URLs and paths.
pub fn safe_name(stem: &str, ext: &str) -> String {
    safe_name_or(stem, ext, "wallpaper")
}

/// `safe_name` with a caller-chosen fallback for stems that clean down to nothing.
pub fn safe_name_or(stem: &str, ext: &str, fallback: &str) -> String {
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " ._()-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == ' ' || c == '.');
    let base: String = trimmed.chars().take(MAX_NAME_CHARS).collect();
    let base = if base.is_empty() {
        fallback.to_string()
    } else {
        base
    };
    format!("{base}.{ext}")
}

/// Copy `src` into `dir` (the Wallpapers folder). Returns the stored file name.
pub fn add_wallpaper(src: &Path, dir: &Path) -> Result<WallpaperInfo, String> {
    let md = fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    if md.file_type().is_symlink() || !md.is_file() {
        return Err("choose an image file".into());
    }
    if md.len() == 0 {
        return Err("that file is empty".into());
    }
    if md.len() > MAX_WALLPAPER_BYTES {
        return Err("that image is too large (limit 30 MB)".into());
    }
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| EXTENSIONS.contains(&e.as_str()))
        .ok_or("supported types: PNG, JPG, WebP, GIF, BMP")?;
    // Read-only open; verify the contents really are that image type before copying anything.
    let mut input = File::open(src).map_err(|e| e.to_string())?;
    let mut head = [0u8; 12];
    let n = input.read(&mut head).map_err(|e| e.to_string())?;
    if !matches_magic(&ext, &head[..n]) {
        return Err(format!("that file is not a real .{ext} image"));
    }
    drop(input);

    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let staging = dir.join(format!("{STAGING_PREFIX}-wp-{nonce}"));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let staged = staging.join("f");
        fs::copy(src, &staged).map_err(|e| e.to_string())?; // reads src; writes only inside staging
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        place(&staged, dir, &safe_name(&stem, &ext), false).map(|file| WallpaperInfo { file })
    })();
    let _ = fs::remove_dir_all(&staging); // our own staging folder
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0, 1, 2, 3,
    ];

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn copies_a_real_image_and_leaves_the_original_alone() {
        let t = TempDir::new().unwrap();
        let src = write(t.path(), "Sunset.PNG", PNG);
        let before = (
            fs::read(&src).unwrap(),
            fs::metadata(&src).unwrap().modified().unwrap(),
        );
        let dir = t.path().join("Wallpapers");
        let info = add_wallpaper(&src, &dir).unwrap();
        assert_eq!(
            info.file, "Sunset.png",
            "extension is normalized to lowercase"
        );
        assert_eq!(fs::read(dir.join("Sunset.png")).unwrap(), PNG);
        assert_eq!(
            (
                fs::read(&src).unwrap(),
                fs::metadata(&src).unwrap().modified().unwrap()
            ),
            before
        );
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "no staging leftovers"
        );
    }

    #[test]
    fn same_name_twice_never_overwrites() {
        let t = TempDir::new().unwrap();
        let a = write(t.path(), "pic.png", PNG);
        let dir = t.path().join("W");
        assert_eq!(add_wallpaper(&a, &dir).unwrap().file, "pic.png");
        assert_eq!(add_wallpaper(&a, &dir).unwrap().file, "pic (2).png");
        assert_eq!(add_wallpaper(&a, &dir).unwrap().file, "pic (3).png");
    }

    #[test]
    fn rejects_files_that_only_pretend_to_be_images() {
        let t = TempDir::new().unwrap();
        let dir = t.path().join("W");
        let fake = write(
            t.path(),
            "totally-a-photo.png",
            b"MZ\x90\x00 this is a program",
        );
        assert!(add_wallpaper(&fake, &dir)
            .unwrap_err()
            .contains("not a real .png"));
        let wrong = write(t.path(), "x.jpg", PNG); // PNG bytes under a .jpg name
        assert!(add_wallpaper(&wrong, &dir).is_err());
        assert!(!dir.exists() || fs::read_dir(&dir).unwrap().count() == 0);
    }

    #[test]
    fn rejects_bad_types_sizes_empty_and_non_files() {
        let t = TempDir::new().unwrap();
        let dir = t.path().join("W");
        assert!(add_wallpaper(&write(t.path(), "a.exe", PNG), &dir).is_err());
        assert!(add_wallpaper(&write(t.path(), "a.svg", b"<svg/>"), &dir).is_err());
        assert!(add_wallpaper(&write(t.path(), "noext", PNG), &dir).is_err());
        assert!(add_wallpaper(&write(t.path(), "e.png", b""), &dir)
            .unwrap_err()
            .contains("empty"));
        let big = t.path().join("big.png");
        let mut data = PNG.to_vec();
        data.resize(MAX_WALLPAPER_BYTES as usize + 1, 0);
        fs::write(&big, data).unwrap();
        assert!(add_wallpaper(&big, &dir).unwrap_err().contains("too large"));
        assert!(
            add_wallpaper(t.path(), &dir).is_err(),
            "a folder is not an image"
        );
        assert!(add_wallpaper(&t.path().join("missing.png"), &dir).is_err());
    }

    #[test]
    fn accepts_every_supported_format_by_its_real_header() {
        let t = TempDir::new().unwrap();
        let dir = t.path().join("W");
        let cases: &[(&str, &[u8])] = &[
            ("a.jpg", &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]),
            ("b.jpeg", &[0xFF, 0xD8, 0xFF, 0xDB, 0, 0]),
            ("c.gif", b"GIF89a\x01\x00"),
            ("d.bmp", b"BM\x00\x00\x00\x00"),
            ("e.webp", b"RIFF\x10\x00\x00\x00WEBPVP8 "),
        ];
        for (name, bytes) in cases {
            let src = write(t.path(), name, bytes);
            assert!(add_wallpaper(&src, &dir).is_ok(), "{name}");
        }
    }

    #[test]
    fn stored_names_are_always_safe() {
        assert_eq!(safe_name("My Photo (1)", "png"), "My Photo (1).png");
        assert_eq!(safe_name("bad/..\\name:*?", "jpg"), "bad_.._name___.jpg");
        assert_eq!(safe_name("日本語", "png"), "___.png");
        assert_eq!(safe_name("...", "png"), "wallpaper.png");
        assert_eq!(safe_name("", "gif"), "wallpaper.gif");
        assert_eq!(safe_name(&"a".repeat(500), "png").len(), MAX_NAME_CHARS + 4);
        for n in [safe_name("<>|\"", "png"), safe_name("con", "png")] {
            assert!(
                n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || " ._()-".contains(c)),
                "{n}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_link_is_not_followed() {
        let t = TempDir::new().unwrap();
        let real = write(t.path(), "real.png", PNG);
        let link = t.path().join("link.png");
        if std::os::windows::fs::symlink_file(&real, &link).is_ok() {
            assert!(add_wallpaper(&link, &t.path().join("W")).is_err());
        }
    }
}
