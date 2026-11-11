//! The Global Descriptor Table and Task State Segment.
//!
//! In 64-bit mode segmentation is mostly a formality: the code segment says
//! "64-bit, ring 0", the data segment exists so `ss`/`ds` hold something
//! valid, and the TSS carries the *interrupt stack table* (IST): known-good
//! stacks the CPU switches to for exceptions that must not trust the current
//! stack (double fault, NMI, machine check).
//!
//! Everything here is plain data so it can be unit-tested on the host.

use core::mem::size_of;

/// Selector of the kernel code segment (GDT index 1, ring 0).
pub const KERNEL_CS: u16 = 0x08;
/// Selector of the kernel data/stack segment (GDT index 2, ring 0).
pub const KERNEL_DS: u16 = 0x10;
/// Selector of the (unused) 32-bit user code segment. It exists because SYSRET
/// derives the 64-bit user selectors from one STAR field: user SS = base + 8,
/// user CS = base + 16, with base = 0x18.
pub const USER_CODE32: u16 = 0x18;
/// User data/stack segment (GDT index 4, ring 3).
pub const USER_DS: u16 = 0x20 | 3;
/// 64-bit user code segment (GDT index 5, ring 3).
pub const USER_CS: u16 = 0x28 | 3;
/// Selector of the TSS (GDT index 6; a 64-bit TSS descriptor takes two slots).
pub const TSS_SELECTOR: u16 = 0x30;
/// The STAR MSR's SYSRET base (see [`USER_CODE32`]).
pub const SYSRET_BASE: u16 = USER_CODE32;

/// 64-bit ring-0 code: present, code, readable, long mode, 4 KiB granularity.
pub const KERNEL_CODE: u64 = 0x00AF_9A00_0000_FFFF;
/// Ring-0 data: present, writable, 4 KiB granularity.
pub const KERNEL_DATA: u64 = 0x00CF_9200_0000_FFFF;
/// Ring-3 versions of the above (DPL 3).
pub const USER_CODE64: u64 = 0x00AF_FA00_0000_FFFF;
pub const USER_DATA: u64 = 0x00CF_F200_0000_FFFF;
pub const USER_CODE32_DESC: u64 = 0x00CF_FA00_0000_FFFF;

/// Number of interrupt stack table slots in a TSS.
pub const IST_SLOTS: usize = 7;

/// The 64-bit Task State Segment. Hardware task switching is gone in long
/// mode; the TSS only holds stack pointers and the I/O permission bitmap offset.
#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct Tss {
    _reserved0: u32,
    /// Stacks loaded on a privilege change to ring 0, 1, 2.
    pub rsp: [u64; 3],
    _reserved1: u64,
    /// Interrupt stack table: `ist[0]` is what IDT entries call IST 1.
    pub ist: [u64; IST_SLOTS],
    _reserved2: u64,
    _reserved3: u16,
    /// Offset of the I/O permission bitmap; equal to the TSS size means "none".
    pub iomap_base: u16,
}

impl Tss {
    pub const fn new() -> Self {
        Self {
            _reserved0: 0,
            rsp: [0; 3],
            _reserved1: 0,
            ist: [0; IST_SLOTS],
            _reserved2: 0,
            _reserved3: 0,
            iomap_base: size_of::<Tss>() as u16,
        }
    }
}

impl Default for Tss {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds the two 8-byte halves of a 64-bit "available TSS" descriptor.
pub const fn tss_descriptor(base: u64, limit: u32) -> [u64; 2] {
    let low = (limit as u64 & 0xFFFF)
        | ((base & 0xFF_FFFF) << 16)
        | (0x89u64 << 40) // present, DPL 0, type 9 = available 64-bit TSS
        | (((limit as u64 >> 16) & 0xF) << 48)
        | (((base >> 24) & 0xFF) << 56);
    [low, base >> 32]
}

/// Our GDT: null, kernel code, kernel data, the three user segments, and the
/// (two-slot) TSS descriptor.
#[repr(C, align(16))]
pub struct Gdt {
    pub entries: [u64; 8],
}

impl Gdt {
    pub const fn new(tss_base: u64) -> Self {
        let t = tss_descriptor(tss_base, size_of::<Tss>() as u32 - 1);
        Self { entries: [0, KERNEL_CODE, KERNEL_DATA, USER_CODE32_DESC, USER_DATA, USER_CODE64, t[0], t[1]] }
    }
}

/// The operand of `lgdt` / `lidt`.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct TablePointer {
    pub limit: u16,
    pub base: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    #[test]
    fn tss_has_the_architectural_layout() {
        assert_eq!(size_of::<Tss>(), 104);
        assert_eq!(offset_of!(Tss, rsp), 4);
        assert_eq!(offset_of!(Tss, ist), 36);
        assert_eq!(offset_of!(Tss, iomap_base), 102);
        assert_eq!(Tss::new().iomap_base, 104);
    }

    #[test]
    fn table_pointer_is_ten_bytes() {
        assert_eq!(size_of::<TablePointer>(), 10);
    }

    #[test]
    fn code_descriptor_is_64_bit_ring0() {
        let d = KERNEL_CODE;
        assert_ne!(d & (1 << 53), 0, "L (long mode) bit");
        assert_eq!(d & (1 << 54), 0, "D bit must be clear in long mode");
        assert_ne!(d & (1 << 47), 0, "present");
        assert_eq!((d >> 45) & 3, 0, "DPL 0");
        assert_ne!(d & (1 << 43), 0, "executable");
    }

    #[test]
    fn data_descriptor_is_writable_ring0() {
        let d = KERNEL_DATA;
        assert_ne!(d & (1 << 47), 0, "present");
        assert_eq!((d >> 45) & 3, 0, "DPL 0");
        assert_eq!(d & (1 << 43), 0, "not executable");
        assert_ne!(d & (1 << 41), 0, "writable");
    }

    #[test]
    fn tss_descriptor_splits_the_base_correctly() {
        let [lo, hi] = tss_descriptor(0xFFFF_FFFF_8012_3456, 103);
        assert_eq!(lo & 0xFFFF, 103);
        assert_eq!((lo >> 16) & 0xFF_FFFF, 0x12_3456);
        assert_eq!((lo >> 40) & 0xFF, 0x89);
        assert_eq!((lo >> 48) & 0xF, 0);
        assert_eq!(lo >> 56, 0x80);
        assert_eq!(hi, 0xFFFF_FFFF);
    }

    #[test]
    fn user_segments_are_ring3_and_sysret_compatible() {
        for d in [USER_CODE64, USER_DATA, USER_CODE32_DESC] {
            assert_eq!((d >> 45) & 3, 3, "DPL 3");
            assert_ne!(d & (1 << 47), 0, "present");
        }
        assert_ne!(USER_CODE64 & (1 << 53), 0, "64-bit code has L set");
        assert_eq!(USER_DS, (SYSRET_BASE + 8) | 3, "SYSRET loads SS from base + 8");
        assert_eq!(USER_CS, (SYSRET_BASE + 16) | 3, "SYSRET loads CS from base + 16");
        let g = Gdt::new(0x1000);
        assert_eq!(g.entries[(USER_DS >> 3) as usize], USER_DATA);
        assert_eq!(g.entries[(USER_CS >> 3) as usize], USER_CODE64);
    }

    #[test]
    fn gdt_layout_matches_the_selectors() {
        let g = Gdt::new(0x1000);
        assert_eq!(g.entries[0], 0);
        assert_eq!(g.entries[(KERNEL_CS / 8) as usize], KERNEL_CODE);
        assert_eq!(g.entries[(KERNEL_DS / 8) as usize], KERNEL_DATA);
        assert_eq!(g.entries[(TSS_SELECTOR / 8) as usize] & 0xFFFF, 103);
        assert_eq!(g.entries.len() * 8, (TSS_SELECTOR as usize) + 16);
    }
}
