//! The real-time clock: the date and time the PC's battery-backed CMOS clock
//! keeps. (It holds whatever the firmware set: UTC on most Linux machines and
//! in QEMU, local time on PCs that run Windows. Nova shows it as it is.)

use nova_x86::rtc::{self, DateTime};
use nova_x86::{inb, outb};

fn reg(r: u8) -> u8 {
    // SAFETY: the kernel owns the CMOS index/data ports; only the desktop thread reads them.
    unsafe {
        outb(0x70, r);
        inb(0x71)
    }
}

fn raw() -> [u8; 6] {
    [reg(rtc::SECONDS), reg(rtc::MINUTES), reg(rtc::HOURS), reg(rtc::DAY), reg(rtc::MONTH), reg(rtc::YEAR)]
}

/// The current date and time, or None if the clock can't be read sensibly.
pub fn now() -> Option<DateTime> {
    // Read until two reads agree and no update was in progress: the clock
    // ticks while we read it.
    for _ in 0..10 {
        let mut spins = 0;
        while reg(rtc::STATUS_A) & rtc::UPDATING != 0 && spins < 100_000 {
            spins += 1;
            core::hint::spin_loop();
        }
        let a = raw();
        let b = raw();
        if a == b {
            return rtc::decode(a, reg(rtc::STATUS_B));
        }
    }
    None
}
