//! Logging for NovaBoot.
//!
//! While UEFI boot services are running, text goes to the firmware console
//! (`ConOut`), which OVMF also mirrors to the serial port. After
//! `ExitBootServices` the firmware console is gone, so text goes straight to
//! COM1 instead.

use crate::uefi::{SimpleTextOutput, SystemTable};
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use nova_x86::serial::{COM1, Serial};

static SYSTEM_TABLE: AtomicPtr<SystemTable> = AtomicPtr::new(core::ptr::null_mut());
static FIRMWARE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn init(st: *mut SystemTable) {
    SYSTEM_TABLE.store(st, Ordering::SeqCst);
    FIRMWARE_ACTIVE.store(true, Ordering::SeqCst);
}

/// Called right after `ExitBootServices` succeeds.
pub fn firmware_gone() {
    FIRMWARE_ACTIVE.store(false, Ordering::SeqCst);
}

pub fn firmware_active() -> bool {
    FIRMWARE_ACTIVE.load(Ordering::SeqCst)
}

struct ConOut(*mut SimpleTextOutput);

impl ConOut {
    fn flush(&mut self, buf: &mut [u16], len: &mut usize) {
        buf[*len] = 0;
        // SAFETY: boot services are active (checked by the caller) and the
        // buffer is NUL-terminated UCS-2.
        unsafe { ((*self.0).output_string)(self.0, buf.as_ptr()) };
        *len = 0;
    }
}

impl Write for ConOut {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut buf = [0u16; 128];
        let mut len = 0;
        for c in s.chars() {
            if c == '\n' {
                buf[len] = u16::from(b'\r');
                len += 1;
            }
            // UCS-2 only: anything outside the BMP becomes '?'.
            buf[len] = u16::try_from(u32::from(c)).unwrap_or(u16::from(b'?'));
            len += 1;
            if len >= buf.len() - 3 {
                self.flush(&mut buf, &mut len);
            }
        }
        if len > 0 {
            self.flush(&mut buf, &mut len);
        }
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    let st = SYSTEM_TABLE.load(Ordering::SeqCst);
    if firmware_active() && !st.is_null() {
        // SAFETY: the system table stays valid while boot services run.
        let out = unsafe { (*st).con_out };
        if !out.is_null() {
            let _ = ConOut(out).write_fmt(args);
            return;
        }
    }
    // SAFETY: after ExitBootServices nothing else owns COM1.
    let mut serial = unsafe { Serial::init(COM1) };
    let _ = serial.write_fmt(args);
}

/// Prints a raw NUL-terminated UCS-2 string from the firmware (e.g. the
/// firmware vendor name).
pub fn print_ucs2(s: *const u16) {
    let st = SYSTEM_TABLE.load(Ordering::SeqCst);
    if firmware_active() && !st.is_null() && !s.is_null() {
        // SAFETY: as in print(); the firmware owns the string.
        unsafe { ((*(*st).con_out).output_string)((*st).con_out, s) };
    }
}

#[macro_export]
macro_rules! println {
    () => { $crate::console::print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::console::print(format_args!("{}\n", format_args!($($arg)*))) };
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::console::print(format_args!($($arg)*)) };
}
