//! The safety net (logic in `crates/safety`): hands the PC back after a crash,
//! a boot that never finishes, or a desktop that stops responding.
//!
//! Off unless the kernel was built with `tools/build.ps1 -Safe <seconds>`
//! (NOVA_SAFE). Never writes to a disk, to firmware variables or to CMOS: the
//! only thing it ever does to the machine is reset it.

use crate::{crash, interrupts, kprintln, rtc};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use nova_safety::{Config, SecondClock, Verdict};
use nova_x86::{cli, inb, outb, rdtsc};

/// Seconds of crash screen before the reset (0 = the safety net is off).
static CRASH_SECS: AtomicU32 = AtomicU32::new(0);
static BOOT_SECS: AtomicU32 = AtomicU32::new(0);
static HANG_SECS: AtomicU32 = AtomicU32::new(0);
/// The boot reached the READY line.
static READY: AtomicBool = AtomicBool::new(false);
/// Timer tick at which the desktop loop last ran (0 = it has not started).
static LAST_BEAT: AtomicU64 = AtomicU64::new(0);
/// The safety net has already fired; it fires once.
static TRIPPED: AtomicBool = AtomicBool::new(false);

/// Reads the build setting. Call once, early.
pub fn init() {
    let Some(setting) = option_env!("NOVA_SAFE") else { return };
    if setting.is_empty() {
        return;
    }
    match Config::parse(setting) {
        Some(c) => {
            BOOT_SECS.store(c.boot_secs, Ordering::SeqCst);
            HANG_SECS.store(c.hang_secs, Ordering::SeqCst);
            CRASH_SECS.store(c.crash_secs, Ordering::SeqCst);
            kprintln!(
                "safety: ON - a crash resets the PC after {} s; a boot not READY after {} s, or a desktop frozen for {} s, counts as a crash",
                c.crash_secs, c.boot_secs, c.hang_secs
            );
        }
        None => kprintln!("safety: NOVA_SAFE={setting:?} is not understood (want e.g. 30 or 30,90,45); the safety net is OFF"),
    }
}

fn config() -> Option<Config> {
    let crash_secs = CRASH_SECS.load(Ordering::Relaxed);
    (crash_secs != 0).then(|| Config {
        crash_secs,
        boot_secs: BOOT_SECS.load(Ordering::Relaxed),
        hang_secs: HANG_SECS.load(Ordering::Relaxed),
    })
}

/// The boot got all the way to READY.
pub fn mark_ready() {
    READY.store(true, Ordering::SeqCst);
}

/// The desktop loop is alive. Call once per pass.
pub fn heartbeat() {
    LAST_BEAT.store(interrupts::ticks().max(1), Ordering::Relaxed);
}

/// Called from the boot CPU's timer tick (interrupt context).
pub fn on_tick(now: u64) {
    let Some(cfg) = config() else { return };
    let beat = LAST_BEAT.load(Ordering::Relaxed);
    let (title, why) = match nova_safety::check(&cfg, now, READY.load(Ordering::Relaxed), beat) {
        Verdict::Fine => return,
        Verdict::BootStuck => ("NOVA WATCHDOG: BOOT STUCK", "the boot did not reach READY in time; the log lines below show how far it got"),
        Verdict::DesktopStuck => ("NOVA WATCHDOG: DESKTOP STUCK", "the desktop loop stopped running"),
    };
    if TRIPPED.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::smp::halt_other_cpus();
    // SAFETY: we never return, so stealing the log lock is fine.
    unsafe { crate::log::force_unlock() };
    let mut r = crash::Report::new(title);
    r.head(format_args!("{why}"));
    r.line(format_args!("timer ticks: {now}, last desktop pass at tick {beat}"));
    r.show();
    after_crash()
}

/// What every crash path ends with instead of `halt_forever`: with the safety
/// net on, a countdown (any PS/2 key stops it) and then a machine reset.
pub fn after_crash() -> ! {
    let Some(cfg) = config() else { nova_x86::halt_forever() };
    cli();
    // Anything left in the keyboard controller (the key press that caused the
    // crash, mouse bytes) must not count as "stop".
    for _ in 0..16 {
        // SAFETY: the kernel owns the PS/2 controller's ports; interrupts are off.
        unsafe {
            if inb(0x64) & 1 == 0 {
                break;
            }
            inb(0x60);
        }
    }
    kprintln!("safety: restarting the machine in {} s (a PS/2 key press stops this)", cfg.crash_secs);
    let mut left = cfg.crash_secs;
    draw(left);
    let mut last_secs = rtc::seconds_raw().unwrap_or(0);
    let mut clock = SecondClock::new(last_secs, rdtsc());
    loop {
        // SAFETY: as above.
        let (status, byte) = unsafe {
            let s = inb(0x64);
            (s, if s & 1 != 0 { inb(0x60) } else { 0 })
        };
        if nova_safety::cancels(status, byte) {
            kprintln!("safety: restart stopped by a key press; the machine stays on this screen");
            crash::banner("RESTART STOPPED. RESET OR POWER OFF BY HAND.");
            nova_x86::halt_forever();
        }
        // A read during the clock's own update is skipped (keeps the last value).
        last_secs = rtc::seconds_raw().unwrap_or(last_secs);
        if clock.poll(last_secs, rdtsc()) {
            left -= 1;
            if left == 0 {
                break;
            }
            draw(left);
        }
        core::hint::spin_loop();
    }
    kprintln!("safety: resetting the machine now");
    reset_machine()
}

fn draw(left: u32) {
    let mut t = crash::TextBuf::<64>::new();
    let _ = core::fmt::Write::write_fmt(&mut t, format_args!("RESTART IN {left} S. PRESS A KEY TO STOP."));
    crash::banner(t.as_str());
}

/// Resets the PC: the chipset reset register, then the keyboard controller's
/// pulse, then (if neither worked) a triple fault.
fn reset_machine() -> ! {
    for (port, value) in nova_safety::RESET_STEPS {
        // SAFETY: the ordinary PC reset ports; the machine is about to restart.
        unsafe { outb(port, value) };
        for _ in 0..30_000_000u32 {
            core::hint::spin_loop();
        }
    }
    // SAFETY: an empty IDT makes the next interrupt a triple fault, which resets the CPU.
    unsafe {
        let null_idt = [0u8; 10];
        core::arch::asm!("lidt [{0}]", "int3", in(reg) null_idt.as_ptr(), options(noreturn));
    }
}
