//! Kernel logging to COM1.
//!
//! The log sits behind a spinlock. The lock also *disables interrupts while
//! held*: with one CPU, an interrupt handler that logs while the code it
//! interrupted holds the lock would otherwise wait for it forever.

use core::cell::UnsafeCell;
use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};
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

    /// Steals the lock, for the panic handler: if the panic happened while
    /// logging, waiting for the lock would hang forever.
    ///
    /// # Safety
    /// Only call when nothing else will run again (we are about to halt).
    pub unsafe fn force_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

static SERIAL: SpinLock<Serial> = SpinLock::new(Serial::uninit(COM1));

pub fn init() -> bool {
    // SAFETY: the kernel owns COM1 from here on.
    SERIAL.with(|s| {
        *s = unsafe { Serial::init(COM1) };
        s.is_present()
    })
}

pub fn print(args: fmt::Arguments) {
    SERIAL.with(|s| {
        let _ = s.write_fmt(args);
    });
}

/// # Safety
/// See [`SpinLock::force_unlock`].
pub unsafe fn force_unlock() {
    unsafe { SERIAL.force_unlock() }
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::log::print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::log::print(format_args!("{}\n", format_args!($($arg)*))) };
}
