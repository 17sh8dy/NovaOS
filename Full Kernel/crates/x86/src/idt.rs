//! The Interrupt Descriptor Table.

/// Number of vectors the CPU can raise.
pub const VECTORS: usize = 256;

/// One 16-byte gate. A gate with the present bit clear raises #NP when used.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    _reserved: u32,
}

impl IdtEntry {
    /// A gate that is not present.
    pub const MISSING: Self = Self { offset_low: 0, selector: 0, ist: 0, type_attr: 0, offset_mid: 0, offset_high: 0, _reserved: 0 };

    /// A 64-bit interrupt gate (clears IF on entry) to `handler`, run with
    /// code segment `selector`. `ist` is 0 (stay on the current stack) or
    /// 1..=7 (switch to that TSS interrupt stack). `dpl` is the least
    /// privileged ring allowed to reach it with a software `int`.
    pub const fn interrupt_gate(handler: u64, selector: u16, ist: u8, dpl: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector,
            ist: ist & 7,
            type_attr: 0x80 | ((dpl & 3) << 5) | 0x0E,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved: 0,
        }
    }

    pub fn handler(&self) -> u64 {
        self.offset_low as u64 | (self.offset_mid as u64) << 16 | (self.offset_high as u64) << 32
    }

    pub fn is_present(&self) -> bool {
        self.type_attr & 0x80 != 0
    }

    pub fn ist(&self) -> u8 {
        self.ist
    }
}

#[repr(C, align(16))]
pub struct Idt {
    pub entries: [IdtEntry; VECTORS],
}

impl Idt {
    pub const fn new() -> Self {
        Self { entries: [IdtEntry::MISSING; VECTORS] }
    }
}

impl Default for Idt {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn gate_is_sixteen_bytes_and_table_is_4k() {
        assert_eq!(size_of::<IdtEntry>(), 16);
        assert_eq!(size_of::<Idt>(), 4096);
    }

    #[test]
    fn gate_round_trips_the_handler_address() {
        let g = IdtEntry::interrupt_gate(0xFFFF_FFFF_8001_2345, 0x08, 0, 0);
        assert_eq!(g.handler(), 0xFFFF_FFFF_8001_2345);
        assert!(g.is_present());
    }

    #[test]
    fn gate_encodes_type_dpl_and_ist() {
        let g = IdtEntry::interrupt_gate(0, 0x08, 1, 0);
        assert_eq!(g.type_attr, 0x8E, "present, DPL 0, interrupt gate");
        assert_eq!(g.ist(), 1);
        let user = IdtEntry::interrupt_gate(0, 0x08, 0, 3);
        assert_eq!(user.type_attr, 0xEE);
        assert_eq!(IdtEntry::interrupt_gate(0, 0x08, 9, 0).ist(), 1, "ist is masked to 3 bits");
    }

    #[test]
    fn missing_gate_is_not_present() {
        assert!(!IdtEntry::MISSING.is_present());
        assert!(!Idt::new().entries[200].is_present());
    }
}
