//! `/system`: the folder where Nova keeps what it needs to run (like the
//! Windows folder on a Windows PC), and `/documents`, where Notes saves.
//!
//! ```text
//! /system/README.txt     what lives here
//! /system/settings       Settings app (plain "key = value" text)
//! /system/password       the login password's salted hash (auth.rs)
//! /system/logs/boot.log  the log of the latest boot
//! /documents/            your files (Notes saves here by default)
//! ```
//!
//! Created at boot when missing. On a Nova disk it survives a restart; on the
//! RAM root of a `-NoDisks` build it lasts until restart. Nova only writes to a
//! disk that already holds a NovaFS (one the user formatted for Nova).

use crate::log::SpinLock;
use crate::{files, kprintln};
use alloc::string::String;
use nova_desktop::settings::Settings;

pub const SETTINGS: &str = "/system/settings";
pub const BOOT_LOG: &str = "/system/logs/boot.log";

const README: &str = "\
Nova OS system folder
=====================

Nova keeps the files it needs to run here (like the Windows folder on a PC).

  settings        your Settings (lock after idle, time zone, wallpaper);
                  plain text, safe to edit, wrong values are ignored
  password        the login password as a salted PBKDF2-SHA-256 hash
                  (the password itself is never stored); set or change it
                  in Settings or with the 'passwd' command
  logs/boot.log   everything the kernel printed during the latest boot

Your own files go in /documents (Notes saves there by default).
";

static CURRENT: SpinLock<Option<Settings>> = SpinLock::new(None);

/// Makes sure the folders exist and loads the settings (after the file systems are mounted).
pub fn init() {
    let loaded = {
        let mut vfs = files::VFS.lock();
        for dir in ["/system", "/system/logs", "/documents"] {
            if vfs.stat(dir).is_err() {
                let _ = vfs.mkdir(dir);
            }
        }
        if vfs.stat("/system/README.txt").is_err() {
            let _ = vfs.write_file("/system/README.txt", README.as_bytes());
        }
        vfs.read_file(SETTINGS).ok()
    };
    let s = match loaded {
        Some(bytes) => Settings::parse(&String::from_utf8_lossy(&bytes)),
        None => {
            let s = Settings::default();
            let _ = files::VFS.lock().write_file(SETTINGS, s.to_text().as_bytes());
            s
        }
    };
    kprintln!(
        "system: /system ready; settings: lock after {} min, {}, wallpaper {}",
        s.lock_minutes,
        nova_desktop::settings::offset_text(s.utc_offset_min),
        nova_desktop::settings::THEMES[s.theme].name
    );
    CURRENT.with(|c| *c = Some(s));
}

pub fn settings() -> Settings {
    CURRENT.with(|c| c.unwrap_or_default())
}

/// Stores new settings in memory and in `/system/settings`. Returns where they went.
pub fn save(s: Settings) -> Result<&'static str, String> {
    CURRENT.with(|c| *c = Some(s));
    let r = files::VFS.lock().write_file(SETTINGS, s.to_text().as_bytes());
    match r {
        Ok(()) => {
            let _ = files::sync();
            Ok(stored_where())
        }
        Err(e) => Err(alloc::format!("{e}")),
    }
}

/// Where settings live, in words, for the Settings window.
pub fn stored_where() -> &'static str {
    if files::root_disk().is_some() { "Saved on the Nova disk (/system/settings)" } else { "Kept until restart (no Nova disk)" }
}

/// Writes the boot log so far to `/system/logs/boot.log`.
pub fn save_boot_log() {
    let mut text = String::new();
    crate::log::recent(|l| {
        text.push_str(l);
        text.push('\n');
    });
    let r = files::VFS.lock().write_file(BOOT_LOG, text.as_bytes());
    match r {
        Ok(()) => kprintln!("system: boot log saved to {BOOT_LOG} ({} bytes)", text.len()),
        Err(e) => kprintln!("system: could not save {BOOT_LOG}: {e}"),
    }
}
