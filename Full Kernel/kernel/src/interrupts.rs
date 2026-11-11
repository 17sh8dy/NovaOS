//! External interrupts: which handler runs for which vector, the timer tick
//! counter, and the bookkeeping for interrupts nobody asked for.
//!
//! Vector map (0-31 are CPU exceptions, see exceptions.rs):
//!
//! | Vector | What |
//! |---|---|
//! | `0x20` | local APIC timer (100 Hz tick, every CPU) |
//! | `0x2C` | PS/2 mouse (ISA IRQ 12, via the IO-APIC) |
//! | `0xFF` | APIC spurious interrupt (needs no EOI) |
//! | anything else | stray: counted and logged, never silently ignored |

use crate::apic;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub const VECTOR_TIMER: u8 = 0x20;
pub const VECTOR_MOUSE: u8 = 0x2C;
pub const VECTOR_SPURIOUS: u8 = 0xFF;

/// Timer interrupts per second.
pub const TICK_HZ: u32 = 100;

/// Handler per vector, stored as a function address (0 = none).
static HANDLERS: [AtomicUsize; 256] = [const { AtomicUsize::new(0) }; 256];

static TICKS: AtomicU64 = AtomicU64::new(0);
static STRAY: AtomicU32 = AtomicU32::new(0);
static SPURIOUS: AtomicU32 = AtomicU32::new(0);

/// How many stray interrupts get a log line each (then it is just counted).
const STRAY_LOG_LIMIT: u32 = 8;

/// Installs `handler` for `vector`. It runs with interrupts disabled, on the
/// kernel stack, and must not block. The EOI is sent for it afterwards.
pub fn register(vector: u8, handler: fn()) {
    HANDLERS[vector as usize].store(handler as usize, Ordering::SeqCst);
}

/// Called from the entry stub for every vector >= 32.
pub fn dispatch(vector: u8) {
    if vector == VECTOR_SPURIOUS {
        // Spurious interrupts are not in-service: sending an EOI would
        // wrongly retire a real one.
        SPURIOUS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // Acknowledge first: the timer handler may switch to another thread and
    // not come back for a while, and an unacknowledged interrupt would block
    // every later one of the same or lower priority on this CPU. Interrupts
    // stay disabled inside the handler, so nothing can slip in meanwhile.
    apic::eoi();
    let h = HANDLERS[vector as usize].load(Ordering::SeqCst);
    if h != 0 {
        // SAFETY: only `register` stores here, and only valid `fn()` addresses.
        let f: fn() = unsafe { core::mem::transmute::<usize, fn()>(h) };
        f();
    } else {
        let n = STRAY.fetch_add(1, Ordering::Relaxed);
        if n < STRAY_LOG_LIMIT {
            crate::kprintln!("irq: stray interrupt on vector {vector:#x} (no handler registered)");
        }
    }
}

/// The timer's handler, run on every CPU. The boot CPU keeps the global
/// clock and wakes sleepers; all CPUs then let the scheduler end time slices.
pub fn timer_tick() {
    if crate::cpu::this_cpu().id == 0 {
        let now = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
        crate::sched::wake_sleepers(now);
    }
    crate::sched::tick();
}

/// Timer ticks since the timer was started.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

pub fn stray_count() -> u32 {
    STRAY.load(Ordering::Relaxed)
}

pub fn spurious_count() -> u32 {
    SPURIOUS.load(Ordering::Relaxed)
}
