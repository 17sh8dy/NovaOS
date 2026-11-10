//! Finding and reading the firmware's ACPI tables.
//!
//! The parser is `crates/acpi` (host-tested). This file supplies its window
//! onto physical memory: the direct map, with a check that every page of a
//! requested range is really mapped (a table in unmapped memory is reported,
//! not faulted on).

use crate::{bootdata, vm};
use alloc::string::String;
use alloc::vec::Vec;
use nova_acpi::{Madt, Memory};

struct DirectMap {
    offset: u64,
}

impl Memory for DirectMap {
    fn read(&self, phys: u64, len: usize) -> Option<&[u8]> {
        let end = phys.checked_add(len as u64)?;
        let mut page = phys & !0xFFF;
        while page < end {
            vm::translate(self.offset + page)?;
            page += 0x1000;
        }
        // SAFETY: every page of the range is mapped (checked above) and ACPI
        // tables are read-only data nobody writes.
        Some(unsafe { core::slice::from_raw_parts((self.offset + phys) as *const u8, len) })
    }
}

pub struct Found {
    pub rsdp_revision: u8,
    /// Four-letter signatures of every table the firmware lists.
    pub tables: Vec<[u8; 4]>,
    pub madt: Madt,
}

/// Locates the RSDP's tables and decodes the MADT.
pub fn discover() -> Result<Found, String> {
    let info = bootdata::info();
    if info.rsdp_addr == 0 {
        return Err(String::from("the firmware gave no ACPI RSDP"));
    }
    let mem = DirectMap { offset: info.physical_memory_offset };
    let rsdp = nova_acpi::parse_rsdp(&mem, info.rsdp_addr).map_err(|e| alloc::format!("RSDP: {e}"))?;
    let addrs = nova_acpi::root_table_addrs(&mem, &rsdp).map_err(|e| alloc::format!("root table: {e}"))?;
    let tables = addrs.iter().filter_map(|&a| mem.read(a, 4).map(|s| [s[0], s[1], s[2], s[3]])).collect();
    let table = nova_acpi::find_table(&mem, &addrs, b"APIC").map_err(|e| alloc::format!("MADT: {e}"))?;
    let madt = nova_acpi::parse_madt(table).map_err(|e| alloc::format!("MADT: {e}"))?;
    Ok(Found { rsdp_revision: rsdp.revision, tables, madt })
}
