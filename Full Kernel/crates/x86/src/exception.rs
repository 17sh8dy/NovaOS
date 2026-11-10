//! What the CPU's exception vectors mean, and how to read their error codes.
//! Pure functions so the crash report can be unit-tested on the host.

use core::fmt::{self, Write};

pub const DIVIDE_ERROR: u8 = 0;
pub const NMI: u8 = 2;
pub const BREAKPOINT: u8 = 3;
pub const DOUBLE_FAULT: u8 = 8;
pub const GENERAL_PROTECTION: u8 = 13;
pub const PAGE_FAULT: u8 = 14;
pub const MACHINE_CHECK: u8 = 18;

/// (mnemonic, description) for an exception vector.
pub fn name(vector: u8) -> (&'static str, &'static str) {
    match vector {
        0 => ("#DE", "divide error"),
        1 => ("#DB", "debug"),
        2 => ("NMI", "non-maskable interrupt"),
        3 => ("#BP", "breakpoint"),
        4 => ("#OF", "overflow"),
        5 => ("#BR", "bound range exceeded"),
        6 => ("#UD", "invalid opcode"),
        7 => ("#NM", "device not available"),
        8 => ("#DF", "double fault"),
        9 => ("---", "coprocessor segment overrun"),
        10 => ("#TS", "invalid TSS"),
        11 => ("#NP", "segment not present"),
        12 => ("#SS", "stack-segment fault"),
        13 => ("#GP", "general protection fault"),
        14 => ("#PF", "page fault"),
        16 => ("#MF", "x87 floating-point exception"),
        17 => ("#AC", "alignment check"),
        18 => ("#MC", "machine check"),
        19 => ("#XM", "SIMD floating-point exception"),
        20 => ("#VE", "virtualization exception"),
        21 => ("#CP", "control protection"),
        28 => ("#HV", "hypervisor injection"),
        29 => ("#VC", "VMM communication"),
        30 => ("#SX", "security exception"),
        15 | 22..=27 | 31 => ("---", "reserved vector"),
        _ => ("INT", "interrupt"),
    }
}

/// True if the CPU pushes an error code for this vector.
pub fn has_error_code(vector: u8) -> bool {
    matches!(vector, 8 | 10..=14 | 17 | 21 | 29 | 30)
}

/// Writes the meaning of a page-fault error code, e.g.
/// `not-present, write, supervisor`.
pub fn write_page_fault_error(w: &mut impl Write, code: u64) -> fmt::Result {
    w.write_str(if code & 1 != 0 { "protection violation" } else { "page not present" })?;
    w.write_str(if code & 2 != 0 { ", write" } else { ", read" })?;
    w.write_str(if code & 4 != 0 { ", user mode" } else { ", supervisor mode" })?;
    if code & 8 != 0 {
        w.write_str(", reserved bit set in page table")?;
    }
    if code & 16 != 0 {
        w.write_str(", instruction fetch")?;
    }
    if code & 32 != 0 {
        w.write_str(", protection-key violation")?;
    }
    if code & (1 << 15) != 0 {
        w.write_str(", SGX")?;
    }
    Ok(())
}

/// Writes the meaning of a selector-style error code (#TS, #NP, #SS, #GP),
/// e.g. `GDT index 3` or `IDT index 32, external`. A zero code means the
/// fault was not about a specific selector.
pub fn write_selector_error(w: &mut impl Write, code: u64) -> fmt::Result {
    if code == 0 {
        return w.write_str("no selector involved");
    }
    let table = match (code >> 1) & 3 {
        0 => "GDT",
        1 | 3 => "IDT",
        _ => "LDT",
    };
    write!(w, "{table} index {}", (code >> 3) & 0x1FFF)?;
    if code & 1 != 0 {
        w.write_str(", external event")?;
    }
    Ok(())
}

/// Writes the names of the set bits of RFLAGS, e.g. `ZF PF IF`.
pub fn write_rflags(w: &mut impl Write, flags: u64) -> fmt::Result {
    const BITS: [(u32, &str); 10] = [
        (0, "CF"),
        (2, "PF"),
        (4, "AF"),
        (6, "ZF"),
        (7, "SF"),
        (8, "TF"),
        (9, "IF"),
        (10, "DF"),
        (11, "OF"),
        (14, "NT"),
    ];
    let mut first = true;
    for (bit, name) in BITS {
        if flags & (1 << bit) != 0 {
            if !first {
                w.write_char(' ')?;
            }
            w.write_str(name)?;
            first = false;
        }
    }
    if first {
        w.write_str("-")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;

    fn render(f: impl FnOnce(&mut String) -> fmt::Result) -> String {
        let mut s = String::new();
        f(&mut s).unwrap();
        s
    }

    #[test]
    fn error_code_vectors_are_the_architectural_set() {
        let with: std::vec::Vec<u8> = (0..32).filter(|&v| has_error_code(v)).collect();
        assert_eq!(with, [8, 10, 11, 12, 13, 14, 17, 21, 29, 30]);
        assert!(!has_error_code(BREAKPOINT));
        assert!(!has_error_code(200));
    }

    #[test]
    fn every_exception_vector_has_a_name() {
        for v in 0..32u8 {
            assert!(!name(v).1.is_empty());
        }
        assert_eq!(name(14), ("#PF", "page fault"));
        assert_eq!(name(8).0, "#DF");
    }

    #[test]
    fn decodes_page_fault_codes() {
        assert_eq!(render(|s| write_page_fault_error(s, 0)), "page not present, read, supervisor mode");
        assert_eq!(render(|s| write_page_fault_error(s, 0b00111)), "protection violation, write, user mode");
        assert!(render(|s| write_page_fault_error(s, 0x10)).contains("instruction fetch"));
    }

    #[test]
    fn decodes_selector_error_codes() {
        assert_eq!(render(|s| write_selector_error(s, 0)), "no selector involved");
        assert_eq!(render(|s| write_selector_error(s, 0x18)), "GDT index 3");
        assert_eq!(render(|s| write_selector_error(s, (32 << 3) | 0b011)), "IDT index 32, external event");
    }

    #[test]
    fn decodes_rflags() {
        assert_eq!(render(|s| write_rflags(s, 0x202)), "IF");
        assert_eq!(render(|s| write_rflags(s, 0x246)), "PF ZF IF");
        assert_eq!(render(|s| write_rflags(s, 0x2)), "-");
    }
}
