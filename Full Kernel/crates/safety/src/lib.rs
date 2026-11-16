//! The safety net for running Nova on a real PC.
//!
//! Nova is young, and a PC that is stuck on a frozen or crashed screen is a
//! PC you have to power-cycle by hand. The safety net hands the PC back by
//! itself:
//!
//! * a **crash** (exception or panic) shows its report, counts down, then
//!   resets the machine (any PS/2 key press during the count stops it, so the
//!   screen can be photographed);
//! * a **boot that never reaches READY** or a **desktop that stops
//!   responding** is turned into the same crash report and countdown.
//!
//! Nothing is written anywhere: no disk, no firmware variable, no CMOS. The
//! reset is the PC's ordinary reset, so the firmware starts over with the
//! boot order it already had (normally Windows first; the Nova stick is
//! only ever booted from the one-time boot menu).
//!
//! This crate is pure logic and is unit-tested on the host; the kernel
//! (`kernel/src/safety.rs`) does the port I/O.

#![no_std]

#[cfg(test)]
extern crate std;

/// Timer ticks per second (the kernel's APIC timer rate).
pub const TICK_HZ: u64 = 100;
/// How long a boot may take before it counts as stuck.
pub const DEFAULT_BOOT_SECS: u32 = 90;
/// How long the desktop loop may go without running before it counts as stuck.
pub const DEFAULT_HANG_SECS: u32 = 30;
/// Longest value accepted for any of the three times.
pub const MAX_SECS: u32 = 3600;

/// The safety net's settings. Absent (no `Config`) means it is off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Config {
    /// Seconds of crash screen before the machine is reset.
    pub crash_secs: u32,
    /// Seconds the boot may take to reach READY.
    pub boot_secs: u32,
    /// Seconds the desktop may stand still after READY.
    pub hang_secs: u32,
}

impl Config {
    /// Parses `"30"` (crash countdown only; the other two use their defaults)
    /// or `"30,90,45"` (crash, boot, hang). Empty, `"0"`, or anything that is
    /// not a number from 1 to 3600 means the safety net is off.
    pub fn parse(s: &str) -> Option<Config> {
        let mut parts = s.split(',');
        let crash_secs = secs(parts.next()?)?;
        let boot_secs = match parts.next() {
            Some(p) => secs(p)?,
            None => DEFAULT_BOOT_SECS,
        };
        let hang_secs = match parts.next() {
            Some(p) => secs(p)?,
            None => DEFAULT_HANG_SECS,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Config { crash_secs, boot_secs, hang_secs })
    }
}

fn secs(p: &str) -> Option<u32> {
    let n: u32 = p.trim().parse().ok()?;
    (1..=MAX_SECS).contains(&n).then_some(n)
}

/// What the timer-tick check found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Fine,
    /// The boot did not reach READY in time.
    BootStuck,
    /// The desktop loop has not run for too long.
    DesktopStuck,
}

/// Decides, from the clock alone, whether Nova is stuck. `now` and
/// `last_beat` are timer ticks; `last_beat` is when the desktop loop last
/// ran (0 = never, which is fine before it has started).
pub fn check(cfg: &Config, now: u64, ready: bool, last_beat: u64) -> Verdict {
    if !ready {
        return if now > u64::from(cfg.boot_secs) * TICK_HZ { Verdict::BootStuck } else { Verdict::Fine };
    }
    if last_beat != 0 && now.saturating_sub(last_beat) > u64::from(cfg.hang_secs) * TICK_HZ {
        return Verdict::DesktopStuck;
    }
    Verdict::Fine
}

/// A cycle count that is certainly longer than a second on any PC Nova runs
/// on (2 GHz and up); the fallback if the real-time clock is not ticking.
pub const FALLBACK_CYCLES: u64 = 6_000_000_000;

/// Counts whole seconds without interrupts or a timer, for use while the
/// kernel is crashed: a change of the CMOS clock's seconds register is a
/// second, and if that register never changes, so many CPU cycles are.
pub struct SecondClock {
    last_rtc: u8,
    base_tsc: u64,
}

impl SecondClock {
    pub fn new(rtc_secs: u8, tsc: u64) -> Self {
        Self { last_rtc: rtc_secs, base_tsc: tsc }
    }

    /// True once per second.
    pub fn poll(&mut self, rtc_secs: u8, tsc: u64) -> bool {
        if rtc_secs != self.last_rtc || tsc.wrapping_sub(self.base_tsc) > FALLBACK_CYCLES {
            self.last_rtc = rtc_secs;
            self.base_tsc = tsc;
            return true;
        }
        false
    }
}

/// Whether a PS/2 controller status + data byte is a key press that should
/// stop the countdown. Mouse bytes (status bit 5) and the controller's own
/// replies (self-test passed, ack, resend, error) are not.
pub fn cancels(status: u8, byte: u8) -> bool {
    const OUTPUT_FULL: u8 = 0x01;
    const FROM_MOUSE: u8 = 0x20;
    status & OUTPUT_FULL != 0 && status & FROM_MOUSE == 0 && !matches!(byte, 0x00 | 0xAA | 0xFA | 0xFC | 0xFE | 0xFF)
}

/// The writes that reset a PC, strongest first: the chipset reset-control
/// register (0xCF9: reset the CPU + the system), then the keyboard
/// controller's CPU-reset pulse. The kernel finishes with a triple fault if
/// both fall through.
pub const RESET_STEPS: [(u16, u8); 3] = [(0xCF9, 0x02), (0xCF9, 0x06), (0x64, 0xFE)];

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: Config = Config { crash_secs: 30, boot_secs: 90, hang_secs: 30 };

    #[test]
    fn parse_one_number_uses_defaults() {
        assert_eq!(Config::parse("45"), Some(Config { crash_secs: 45, boot_secs: 90, hang_secs: 30 }));
    }

    #[test]
    fn parse_three_numbers() {
        assert_eq!(Config::parse("3,8,6"), Some(Config { crash_secs: 3, boot_secs: 8, hang_secs: 6 }));
        assert_eq!(Config::parse(" 3 , 8 "), Some(Config { crash_secs: 3, boot_secs: 8, hang_secs: 30 }));
    }

    #[test]
    fn parse_rejects_off_and_junk() {
        for bad in ["", "0", "-1", "abc", "3601", "3,0", "3,4,5,6", "3,,5", "3.5"] {
            assert_eq!(Config::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn boot_stuck_only_after_the_limit_and_only_before_ready() {
        assert_eq!(check(&CFG, 90 * TICK_HZ, false, 0), Verdict::Fine);
        assert_eq!(check(&CFG, 90 * TICK_HZ + 1, false, 0), Verdict::BootStuck);
        // Once ready, a slow boot is history.
        assert_eq!(check(&CFG, 500 * TICK_HZ, true, 0), Verdict::Fine);
    }

    #[test]
    fn desktop_stuck_needs_a_heartbeat_gone_quiet() {
        let beat = 10 * TICK_HZ;
        assert_eq!(check(&CFG, beat + 30 * TICK_HZ, true, beat), Verdict::Fine);
        assert_eq!(check(&CFG, beat + 30 * TICK_HZ + 1, true, beat), Verdict::DesktopStuck);
        // No heartbeat yet (desktop not started, e.g. no input devices): never "stuck".
        assert_eq!(check(&CFG, 9999 * TICK_HZ, true, 0), Verdict::Fine);
    }

    #[test]
    fn a_beat_in_the_future_does_not_underflow() {
        assert_eq!(check(&CFG, 5, true, 50), Verdict::Fine);
    }

    #[test]
    fn second_clock_counts_rtc_changes() {
        let mut c = SecondClock::new(7, 1000);
        assert!(!c.poll(7, 2000));
        assert!(c.poll(8, 3000));
        assert!(!c.poll(8, 4000));
        assert!(c.poll(9, 5000));
        // Seconds wrap 59 -> 0 like any other change.
        let mut w = SecondClock::new(59, 0);
        assert!(w.poll(0, 1));
    }

    #[test]
    fn second_clock_falls_back_to_cycles_when_the_rtc_is_stuck() {
        let mut c = SecondClock::new(5, 0);
        assert!(!c.poll(5, FALLBACK_CYCLES));
        assert!(c.poll(5, FALLBACK_CYCLES + 1));
        // The baseline moved: the next second needs the full count again.
        assert!(!c.poll(5, FALLBACK_CYCLES + 2));
    }

    #[test]
    fn only_real_key_bytes_cancel() {
        assert!(cancels(0x01, 0x1C)); // a key
        assert!(!cancels(0x00, 0x1C)); // nothing waiting
        assert!(!cancels(0x21, 0x1C)); // that byte is from the mouse
        for noise in [0x00, 0xAA, 0xFA, 0xFC, 0xFE, 0xFF] {
            assert!(!cancels(0x01, noise), "{noise:#x}");
        }
    }

    #[test]
    fn reset_goes_through_the_chipset_first() {
        assert_eq!(RESET_STEPS[0], (0xCF9, 0x02));
        assert_eq!(RESET_STEPS[1], (0xCF9, 0x06));
        assert_eq!(RESET_STEPS[2], (0x64, 0xFE));
    }
}
