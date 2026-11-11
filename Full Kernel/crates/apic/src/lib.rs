//! Register layouts and arithmetic for the local APIC and IO-APIC.
//!
//! No hardware access here, only the numbers, so it can be unit-tested.

#![no_std]

/// Local APIC register offsets (from the APIC's base address).
pub mod lapic {
    pub const ID: usize = 0x20;
    pub const VERSION: usize = 0x30;
    pub const TPR: usize = 0x80;
    pub const EOI: usize = 0xB0;
    pub const SPURIOUS: usize = 0xF0;
    pub const ESR: usize = 0x280;
    pub const LVT_TIMER: usize = 0x320;
    pub const LVT_LINT0: usize = 0x350;
    pub const LVT_LINT1: usize = 0x360;
    pub const LVT_ERROR: usize = 0x370;
    pub const TIMER_INITIAL: usize = 0x380;
    pub const TIMER_CURRENT: usize = 0x390;
    pub const TIMER_DIVIDE: usize = 0x3E0;

    /// Spurious-vector register: bit 8 turns the APIC on.
    pub const SPURIOUS_ENABLE: u32 = 1 << 8;
    /// Mask bit of every LVT entry.
    pub const LVT_MASKED: u32 = 1 << 16;
    /// Timer divide register value for "divide by 16".
    pub const DIVIDE_BY_16: u32 = 0x3;
    pub const DIVIDER: u64 = 16;
}

/// IA32_APIC_BASE MSR and its bits.
pub const MSR_APIC_BASE: u32 = 0x1B;
pub const APIC_BASE_ENABLE: u64 = 1 << 11;
pub const APIC_BASE_ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// How the local APIC timer counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimerMode {
    OneShot,
    Periodic,
}

/// Value for the LVT timer register.
pub const fn lvt_timer(vector: u8, mode: TimerMode, masked: bool) -> u32 {
    vector as u32 | ((matches!(mode, TimerMode::Periodic) as u32) << 17) | ((masked as u32) << 16)
}

/// Initial count that makes the (divided) timer fire `hz` times a second,
/// given that it counted down `elapsed` ticks while `window_ms` milliseconds
/// passed. Never returns 0 (which would stop the timer).
pub fn timer_initial_count(elapsed: u32, window_ms: u32, hz: u32) -> u32 {
    let per_second = elapsed as u64 * 1000 / window_ms.max(1) as u64;
    (per_second / hz.max(1) as u64).clamp(1, u32::MAX as u64) as u32
}

/// The bus frequency the APIC timer ran at, in Hz (before the divider).
pub fn timer_clock_hz(elapsed: u32, window_ms: u32) -> u64 {
    elapsed as u64 * 1000 / window_ms.max(1) as u64 * lapic::DIVIDER
}

/// One IO-APIC redirection-table entry: where an external interrupt goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Redirection {
    pub vector: u8,
    pub dest_apic_id: u8,
    pub active_low: bool,
    pub level_triggered: bool,
    pub masked: bool,
}

impl Redirection {
    /// An unmasked, fixed-delivery, physical-destination entry.
    pub const fn new(vector: u8, dest_apic_id: u8, active_low: bool, level_triggered: bool) -> Self {
        Self { vector, dest_apic_id, active_low, level_triggered, masked: false }
    }

    /// A masked entry (the state every pin should start in).
    pub const fn masked_off() -> Self {
        Self { vector: 0, dest_apic_id: 0, active_low: false, level_triggered: false, masked: true }
    }

    /// Low 32 bits: vector, delivery mode (000 = fixed), polarity, trigger, mask.
    pub const fn low(&self) -> u32 {
        self.vector as u32 | ((self.active_low as u32) << 13) | ((self.level_triggered as u32) << 15) | ((self.masked as u32) << 16)
    }

    /// High 32 bits: destination APIC ID in bits 24..32.
    pub const fn high(&self) -> u32 {
        (self.dest_apic_id as u32) << 24
    }
}

/// The interrupt command register (ICR), used to send inter-processor interrupts.
pub mod icr {
    /// Register offsets: the destination goes in the high half, and writing the
    /// low half sends the interrupt.
    pub const LOW: usize = 0x300;
    pub const HIGH: usize = 0x310;

    pub const DELIVERY_NMI: u32 = 0b100 << 8;
    pub const DELIVERY_INIT: u32 = 0b101 << 8;
    pub const DELIVERY_STARTUP: u32 = 0b110 << 8;
    /// Set by the hardware while an IPI is still being delivered.
    pub const PENDING: u32 = 1 << 12;
    pub const LEVEL_ASSERT: u32 = 1 << 14;
    pub const TRIGGER_LEVEL: u32 = 1 << 15;
    pub const SHORTHAND_ALL_BUT_SELF: u32 = 0b11 << 18;

    /// INIT IPI (puts the target CPU into the wait-for-SIPI state).
    pub const INIT: u32 = DELIVERY_INIT | LEVEL_ASSERT | TRIGGER_LEVEL;

    /// STARTUP IPI: the target begins real-mode execution at physical
    /// address `page * 0x1000` (so the code must sit below 1 MiB).
    pub const fn startup(page: u8) -> u32 {
        DELIVERY_STARTUP | LEVEL_ASSERT | page as u32
    }

    /// High half for a physical destination APIC id.
    pub const fn destination(apic_id: u8) -> u32 {
        (apic_id as u32) << 24
    }

    /// NMI to every CPU except the sender.
    pub const NMI_ALL_BUT_SELF: u32 = DELIVERY_NMI | LEVEL_ASSERT | SHORTHAND_ALL_BUT_SELF;

    /// Fixed-vector IPI to every CPU except the sender.
    pub const fn fixed_all_but_self(vector: u8) -> u32 {
        LEVEL_ASSERT | SHORTHAND_ALL_BUT_SELF | vector as u32
    }
}

/// IO-APIC register selectors.
pub mod ioapic {
    /// Index register offset and data window offset, from the IO-APIC base.
    pub const REGSEL: usize = 0x00;
    pub const WINDOW: usize = 0x10;
    pub const REG_ID: u32 = 0x00;
    pub const REG_VERSION: u32 = 0x01;

    /// Register index of the low half of redirection entry `pin`.
    pub const fn redirection_low(pin: u32) -> u32 {
        0x10 + 2 * pin
    }

    /// Highest pin number, from the version register.
    pub const fn max_pin(version_register: u32) -> u32 {
        (version_register >> 16) & 0xFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirection_entries_encode_per_the_datasheet() {
        let e = Redirection::new(0x2C, 3, false, false);
        assert_eq!(e.low(), 0x2C);
        assert_eq!(e.high(), 3 << 24);
        let e = Redirection::new(0x30, 0, true, true);
        assert_eq!(e.low(), 0x30 | 1 << 13 | 1 << 15);
        assert_eq!(Redirection::masked_off().low() & (1 << 16), 1 << 16);
    }

    #[test]
    fn timer_register_encoding() {
        assert_eq!(lvt_timer(0x20, TimerMode::Periodic, false), 0x20 | 1 << 17);
        assert_eq!(lvt_timer(0x20, TimerMode::OneShot, true), 0x20 | 1 << 16);
        assert_eq!(lvt_timer(0xFF, TimerMode::OneShot, false), 0xFF);
    }

    #[test]
    fn calibration_maths() {
        // 20 ms window, timer counted 1_000_000 ticks: 50 MHz after the divider.
        assert_eq!(timer_initial_count(1_000_000, 20, 100), 500_000, "100 Hz needs 500k ticks per interrupt");
        assert_eq!(timer_clock_hz(1_000_000, 20), 800_000_000, "x16 divider = 800 MHz bus clock");
        assert_eq!(timer_initial_count(0, 20, 100), 1, "never zero");
        assert_eq!(timer_initial_count(u32::MAX, 1, 1), u32::MAX, "clamped, no overflow");
        assert_eq!(timer_initial_count(10, 0, 0), 10_000, "zero arguments don't divide by zero");
    }

    #[test]
    fn ipi_encodings_match_the_intel_manual() {
        assert_eq!(icr::INIT, 0xC500, "INIT, level, assert");
        assert_eq!(icr::startup(0x08), 0x4608, "SIPI to page 8 = physical 0x8000");
        assert_eq!(icr::destination(3), 0x0300_0000);
        assert_eq!(icr::NMI_ALL_BUT_SELF, 0xC4400);
        assert_eq!(icr::fixed_all_but_self(0xFD), 0xC40FD);
    }

    #[test]
    fn ioapic_register_helpers() {
        assert_eq!(ioapic::redirection_low(0), 0x10);
        assert_eq!(ioapic::redirection_low(12), 0x28);
        assert_eq!(ioapic::max_pin(0x0017_0011), 23);
    }
}
