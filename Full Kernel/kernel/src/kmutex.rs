//! A sleeping-friendly mutex for kernel *threads*.
//!
//! Unlike `log::SpinLock` this keeps interrupts enabled while held and, when
//! contended, yields the CPU to other threads instead of spinning. That is what
//! long operations (disk I/O) need: the timer, keyboard and mouse keep working.
//! It must never be taken from an interrupt handler.

use crate::sched;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub struct KMutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: access to `value` is serialised by `locked`.
unsafe impl<T: Send> Sync for KMutex<T> {}
unsafe impl<T: Send> Send for KMutex<T> {}

pub struct KMutexGuard<'a, T> {
    mutex: &'a KMutex<T>,
}

impl<T> KMutex<T> {
    pub const fn new(value: T) -> Self {
        Self { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    pub fn lock(&self) -> KMutexGuard<'_, T> {
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            sched::yield_now();
            core::hint::spin_loop();
        }
        KMutexGuard { mutex: self }
    }
}

impl<T> Deref for KMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: we hold the lock.
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T> DerefMut for KMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: we hold the lock exclusively.
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T> Drop for KMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}
