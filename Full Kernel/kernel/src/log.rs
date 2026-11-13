//! Kernel logging: to COM1, to a ring of recent lines (shown on the crash
//! screen), and optionally to the screen (a *mirror*: the boot log, so a real
//! PC without a serial port still shows what the kernel is doing).
//!
//! The log sits behind a spinlock. The lock also *disables interrupts while
//! held*: with one CPU, an interrupt handler that logs while the code it
//! interrupted holds the lock would otherwise wait for it forever.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use nova_x86::serial::{COM1, Serial};

pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: access to `value` is serialised by `locked`.
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let irq_were_on = nova_x86::interrupts_enabled();
        nova_x86::cli();
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        // SAFETY: we hold the lock.
        let r = f(unsafe { &mut *self.value.get() });
        self.locked.store(false, Ordering::Release);
        if irq_were_on {
            nova_x86::sti();
        }
        r
    }

    /// Like `with`, but gives up at once (returns None) if the lock is taken.
    pub fn try_with<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let irq_were_on = nova_x86::interrupts_enabled();
        nova_x86::cli();
        if self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            if irq_were_on {
                nova_x86::sti();
            }
            return None;
        }
        // SAFETY: we hold the lock.
        let r = f(unsafe { &mut *self.value.get() });
        self.locked.store(false, Ordering::Release);
        if irq_were_on {
            nova_x86::sti();
        }
        Some(r)
    }

    /// Steals the lock, for the panic handler: if the panic happened while
    /// logging, waiting for the lock would hang forever.
    ///
    /// # Safety
    /// Only call when nothing else will run again (we are about to halt).
    pub unsafe fn force_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

const RING_LINES: usize = 24;
const LINE_BYTES: usize = 120;

/// The serial port plus the last lines logged.
struct Log {
    serial: Serial,
    ring: [[u8; LINE_BYTES]; RING_LINES],
    ring_len: [usize; RING_LINES],
    /// Next slot to fill; `count` lines are valid (at most RING_LINES).
    next: usize,
    count: usize,
    /// The line being assembled (text arrives in pieces).
    cur: [u8; LINE_BYTES],
    cur_len: usize,
}

static LOG: SpinLock<Log> = SpinLock::new(Log {
    serial: Serial::uninit(COM1),
    ring: [[0; LINE_BYTES]; RING_LINES],
    ring_len: [0; RING_LINES],
    next: 0,
    count: 0,
    cur: [0; LINE_BYTES],
    cur_len: 0,
});

/// Where finished lines are also shown (0 = nowhere): a `fn(&str)`.
static MIRROR: AtomicUsize = AtomicUsize::new(0);

pub fn init() -> bool {
    // SAFETY: the kernel owns COM1 from here on.
    LOG.with(|l| {
        l.serial = unsafe { Serial::init(COM1) };
        l.serial.is_present()
    })
}

/// Also shows every finished log line with `f` (None: stop). `f` runs with the
/// log locked, so it must not log, and must not wait for a lock that someone
/// logging might hold (use `try_with`).
pub fn set_mirror(f: Option<fn(&str)>) {
    MIRROR.store(f.map_or(0, |f| f as usize), Ordering::SeqCst);
}

impl Write for Log {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let _ = self.serial.write_str(s);
        for &b in s.as_bytes() {
            if b == b'\n' {
                let n = self.next;
                self.ring[n] = self.cur;
                self.ring_len[n] = self.cur_len;
                self.next = (n + 1) % RING_LINES;
                self.count = (self.count + 1).min(RING_LINES);
                let m = MIRROR.load(Ordering::SeqCst);
                if m != 0 {
                    // SAFETY: only `set_mirror` stores here, and only `fn(&str)` addresses.
                    let f: fn(&str) = unsafe { core::mem::transmute::<usize, fn(&str)>(m) };
                    f(core::str::from_utf8(&self.cur[..self.cur_len]).unwrap_or(""));
                }
                self.cur_len = 0;
            } else if self.cur_len < LINE_BYTES && b != b'\r' {
                self.cur[self.cur_len] = b;
                self.cur_len += 1;
            }
        }
        Ok(())
    }
}

pub fn print(args: fmt::Arguments) {
    LOG.with(|l| {
        let _ = l.write_fmt(args);
    });
}

/// Calls `f` with each of the last logged lines, oldest first (for the crash screen).
pub fn recent(mut f: impl FnMut(&str)) {
    LOG.with(|l| {
        let start = (l.next + RING_LINES - l.count) % RING_LINES;
        for i in 0..l.count {
            let k = (start + i) % RING_LINES;
            f(core::str::from_utf8(&l.ring[k][..l.ring_len[k]]).unwrap_or(""));
        }
    });
}

/// # Safety
/// See [`SpinLock::force_unlock`].
pub unsafe fn force_unlock() {
    unsafe { LOG.force_unlock() }
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::log::print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::log::print(format_args!("{}\n", format_args!($($arg)*))) };
}
