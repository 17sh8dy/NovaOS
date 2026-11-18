//! The login password.
//!
//! Where it comes from, first match wins:
//! 1. `/system/password` on the root file system (set with the lock screen's
//!    "Password" card or the `passwd` command; on a NovaFS disk it survives a
//!    restart, on the RAM root of a `-NoDisks` build it lasts until restart);
//! 2. the build: `tools\build.ps1 -Password` asks for one and bakes only its
//!    salted PBKDF2 record into the kernel (`NOVA_PASSWORD`), so a USB-stick
//!    kernel can be locked without any disk;
//! 3. none: the desktop opens without a login, as before.
//!
//! Only the record (`crates/auth`) is ever stored, never the password.
//! Checking costs ~100,000 HMACs (a tenth of a second), which, with the lock
//! screen's growing waits, makes guessing slow.

use crate::log::SpinLock;
use crate::{files, kprintln};
use alloc::string::String;
use nova_auth::{DEFAULT_ITERATIONS, Record, SALT_LEN};

pub const PATH: &str = "/system/password";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Build,
    File,
}

static RECORD: SpinLock<Option<(Record, Source)>> = SpinLock::new(None);

/// Loads the password record (after the file systems are mounted).
pub fn init() {
    let from_file = files::VFS.lock().read_file(PATH).ok();
    let found = match from_file {
        Some(bytes) => match core::str::from_utf8(&bytes).ok().and_then(Record::parse) {
            Some(r) => Some((r, Source::File)),
            None => {
                kprintln!("login: {PATH} is not a valid password record; ignoring it");
                None
            }
        },
        None => None,
    };
    let found = found.or_else(|| option_env!("NOVA_PASSWORD").and_then(Record::parse).map(|r| (r, Source::Build)));
    match &found {
        Some((r, s)) => kprintln!("login: a password is set ({}, {} iterations)", if *s == Source::File { PATH } else { "built in" }, r.iterations),
        None => kprintln!("login: no password set (the desktop opens without one)"),
    }
    RECORD.with(|g| *g = found);
}

pub fn is_set() -> bool {
    RECORD.with(|g| g.is_some())
}

/// Is this the password? (The work happens outside the lock.)
pub fn verify(password: &str) -> bool {
    let Some((record, _)) = RECORD.with(|g| g.clone()) else { return true };
    record.verify(password)
}

/// 64 bits from the CPU's random number generator, if it has one.
fn rdrand() -> Option<u64> {
    let ecx = core::arch::x86_64::__cpuid(1).ecx;
    if ecx & (1 << 30) == 0 {
        return None;
    }
    for _ in 0..10 {
        let (v, ok): (u64, u8);
        // SAFETY: RDRAND exists (checked above); it only writes the register and CF.
        unsafe { core::arch::asm!("rdrand {0}", "setc {1}", out(reg) v, out(reg_byte) ok, options(nomem, nostack)) };
        if ok != 0 {
            return Some(v);
        }
    }
    None
}

/// A fresh salt: RDRAND when there is one, mixed with the time stamp counter
/// and the clock (a salt must be unique, not secret).
fn salt() -> [u8; SALT_LEN] {
    let mut h = nova_auth::Sha256::new();
    for _ in 0..4 {
        h.update(&rdrand().unwrap_or(0).to_le_bytes());
        h.update(&nova_x86::rdtsc().to_le_bytes());
    }
    if let Some(t) = crate::rtc::now() {
        h.update(&[t.second, t.minute, t.hour, t.day, t.month]);
    }
    h.update(&crate::interrupts::ticks().to_le_bytes());
    let d = h.finish();
    let mut s = [0u8; SALT_LEN];
    s.copy_from_slice(&d[..SALT_LEN]);
    s
}

/// Sets a new password and writes its record to `/system/password`.
/// Returns what to tell the user.
pub fn set(password: &str) -> Result<String, String> {
    let record = Record::new(password, salt(), DEFAULT_ITERATIONS);
    let line = record.to_line() + "\n";
    let written = {
        let mut vfs = files::VFS.lock();
        let _ = vfs.mkdir("/system");
        vfs.write_file(PATH, line.as_bytes())
    };
    RECORD.with(|g| *g = Some((record, Source::File)));
    match written {
        Ok(()) => {
            let lasting = files::root_disk().is_some();
            let _ = files::sync();
            kprintln!("login: new password saved to {PATH}{}", if lasting { "" } else { " (RAM: until restart)" });
            Ok(String::from(if lasting { "Password saved" } else { "Saved until restart (no Nova disk)" }))
        }
        Err(e) => {
            kprintln!("login: could not save {PATH}: {e} (the new password works until restart)");
            Err(String::from("Not saved: works until restart"))
        }
    }
}
