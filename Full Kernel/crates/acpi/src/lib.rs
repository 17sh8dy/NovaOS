//! ACPI table parsing: enough of ACPI to find the interrupt controllers.
//!
//! Starting from the RSDP (whose address the bootloader hands over) this
//! walks the root table (XSDT, or RSDT on old firmware), finds the MADT
//! ("APIC" table) and decodes it: processors, IO-APICs, and the ISA
//! interrupt-source overrides that say which IO-APIC pin a legacy IRQ lands on.
//!
//! Every table is checksum-verified before it is trusted, and every read goes
//! through the [`Memory`] trait, so the kernel can plug in "physical memory via
//! the direct map" and the tests can plug in a `Vec`.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

use alloc::vec::Vec;
use core::fmt;

/// Read access to physical memory.
pub trait Memory {
    /// `len` bytes at physical address `phys`, or `None` if any of it is not readable.
    fn read(&self, phys: u64, len: usize) -> Option<&[u8]>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Memory at this physical address is not readable.
    Unreadable(u64),
    BadSignature,
    BadChecksum,
    /// A length field is nonsense (too small, or absurdly large).
    BadLength,
    /// The MADT's entry list is malformed.
    BadEntry,
    /// The firmware has no table with that signature.
    NotFound,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(a) => write!(f, "ACPI memory at {a:#x} is not mapped"),
            Self::BadSignature => write!(f, "bad ACPI signature"),
            Self::BadChecksum => write!(f, "ACPI checksum mismatch"),
            Self::BadLength => write!(f, "bad ACPI table length"),
            Self::BadEntry => write!(f, "malformed MADT entry"),
            Self::NotFound => write!(f, "ACPI table not found"),
        }
    }
}

/// Size of the standard header every system description table starts with.
pub const HEADER_LEN: usize = 36;
/// No sane table is bigger than this; stops a corrupt length from reading the world.
const MAX_TABLE_LEN: usize = 16 << 20;

fn checksum_ok(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |a, &b| a.wrapping_add(b)) == 0
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// The Root System Description Pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rsdp {
    pub revision: u8,
    pub rsdt: u32,
    /// 0 for ACPI 1.0 firmware.
    pub xsdt: u64,
}

pub fn parse_rsdp(mem: &impl Memory, phys: u64) -> Result<Rsdp, Error> {
    let v1 = mem.read(phys, 20).ok_or(Error::Unreadable(phys))?;
    if &v1[0..8] != b"RSD PTR " {
        return Err(Error::BadSignature);
    }
    if !checksum_ok(v1) {
        return Err(Error::BadChecksum);
    }
    let revision = v1[15];
    let rsdt = u32_at(v1, 16);
    let mut xsdt = 0;
    if revision >= 2 {
        let v2 = mem.read(phys, 36).ok_or(Error::Unreadable(phys))?;
        if (u32_at(v2, 20) as usize) < 36 || !checksum_ok(&v2[..36]) {
            return Err(Error::BadChecksum);
        }
        xsdt = u64_at(v2, 24);
    }
    Ok(Rsdp { revision, rsdt, xsdt })
}

/// Reads a whole table (header + body) and verifies signature length and checksum.
fn read_table<M: Memory>(mem: &M, phys: u64) -> Result<&[u8], Error> {
    let header = mem.read(phys, HEADER_LEN).ok_or(Error::Unreadable(phys))?;
    let len = u32_at(header, 4) as usize;
    if !(HEADER_LEN..=MAX_TABLE_LEN).contains(&len) {
        return Err(Error::BadLength);
    }
    let table = mem.read(phys, len).ok_or(Error::Unreadable(phys))?;
    if !checksum_ok(table) {
        return Err(Error::BadChecksum);
    }
    Ok(table)
}

/// Physical addresses of every table the firmware lists (from the XSDT if
/// there is one, else the RSDT).
pub fn root_table_addrs(mem: &impl Memory, rsdp: &Rsdp) -> Result<Vec<u64>, Error> {
    let (addr, sig, entry) = if rsdp.revision >= 2 && rsdp.xsdt != 0 {
        (rsdp.xsdt, b"XSDT", 8)
    } else {
        (rsdp.rsdt as u64, b"RSDT", 4)
    };
    let table = read_table(mem, addr)?;
    if &table[0..4] != sig {
        return Err(Error::BadSignature);
    }
    Ok(table[HEADER_LEN..]
        .chunks_exact(entry)
        .map(|c| if entry == 8 { u64::from_le_bytes(c.try_into().unwrap()) } else { u32::from_le_bytes(c.try_into().unwrap()) as u64 })
        .collect())
}

/// Finds the table with `signature` among `addrs`, returning its verified bytes.
/// Tables that cannot be read or fail their checksum are skipped.
pub fn find_table<'m, M: Memory>(mem: &'m M, addrs: &[u64], signature: &[u8; 4]) -> Result<&'m [u8], Error> {
    for &a in addrs {
        if let Some(h) = mem.read(a, HEADER_LEN)
            && &h[0..4] == signature
            && let Ok(t) = read_table(mem, a)
        {
            return Ok(t);
        }
    }
    Err(Error::NotFound)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cpu {
    pub acpi_id: u32,
    pub apic_id: u32,
    /// Enabled, or at least able to be brought online.
    pub usable: bool,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApic {
    pub id: u8,
    pub address: u32,
    /// The first global system interrupt this IO-APIC handles.
    pub gsi_base: u32,
}

/// "Legacy IRQ `source` is wired to global system interrupt `gsi`, with these flags".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrqOverride {
    pub bus: u8,
    pub source: u8,
    pub gsi: u32,
    pub flags: u16,
}

/// Where a legacy ISA IRQ ends up, and how its line behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub gsi: u32,
    pub active_low: bool,
    pub level_triggered: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Madt {
    /// Physical address of the local APIC registers.
    pub lapic_addr: u64,
    /// Bit 0: the PC also has legacy 8259 PICs.
    pub flags: u32,
    pub cpus: Vec<Cpu>,
    pub ioapics: Vec<IoApic>,
    pub overrides: Vec<IrqOverride>,
}

pub fn parse_madt(table: &[u8]) -> Result<Madt, Error> {
    if table.len() < HEADER_LEN + 8 {
        return Err(Error::BadLength);
    }
    if &table[0..4] != b"APIC" {
        return Err(Error::BadSignature);
    }
    let mut madt = Madt { lapic_addr: u32_at(table, HEADER_LEN) as u64, flags: u32_at(table, HEADER_LEN + 4), ..Default::default() };
    let mut off = HEADER_LEN + 8;
    while off < table.len() {
        if off + 2 > table.len() {
            return Err(Error::BadEntry);
        }
        let (kind, len) = (table[off], table[off + 1] as usize);
        if len < 2 || off + len > table.len() {
            return Err(Error::BadEntry);
        }
        let e = &table[off..off + len];
        match kind {
            // Processor local APIC
            0 if len >= 8 => {
                let flags = u32_at(e, 4);
                madt.cpus.push(Cpu { acpi_id: e[2] as u32, apic_id: e[3] as u32, enabled: flags & 1 != 0, usable: flags & 3 != 0 });
            }
            // IO-APIC
            1 if len >= 12 => madt.ioapics.push(IoApic { id: e[2], address: u32_at(e, 4), gsi_base: u32_at(e, 8) }),
            // Interrupt source override
            2 if len >= 10 => madt.overrides.push(IrqOverride { bus: e[2], source: e[3], gsi: u32_at(e, 4), flags: u16_at(e, 8) }),
            // Local APIC address override (64-bit)
            5 if len >= 12 => madt.lapic_addr = u64_at(e, 4),
            // Processor local x2APIC (CPUs with APIC IDs >= 255)
            9 if len >= 16 => {
                let flags = u32_at(e, 8);
                madt.cpus.push(Cpu { acpi_id: u32_at(e, 12), apic_id: u32_at(e, 4), enabled: flags & 1 != 0, usable: flags & 3 != 0 });
            }
            // NMI sources, x2APIC NMIs, GIC, ... : not needed yet.
            _ => {}
        }
        off += len;
    }
    Ok(madt)
}

impl Madt {
    /// Where legacy ISA IRQ `irq` lands. Without an override an ISA IRQ is
    /// GSI `irq`, active high, edge triggered.
    pub fn isa_route(&self, irq: u8) -> Route {
        let Some(o) = self.overrides.iter().find(|o| o.bus == 0 && o.source == irq) else {
            return Route { gsi: irq as u32, active_low: false, level_triggered: false };
        };
        Route {
            gsi: o.gsi,
            // 0 = "as the bus does it" (ISA: active high, edge); 1 = high/edge; 3 = low/level.
            active_low: o.flags & 3 == 3,
            level_triggered: (o.flags >> 2) & 3 == 3,
        }
    }

    /// The IO-APIC that handles `gsi` and the pin on it. The IO-APIC with
    /// the highest base not above `gsi` is chosen (the table doesn't say how
    /// many pins each has; the caller checks against the chip's own count).
    pub fn ioapic_for_gsi(&self, gsi: u32) -> Option<(&IoApic, u32)> {
        self.ioapics.iter().filter(|a| a.gsi_base <= gsi).max_by_key(|a| a.gsi_base).map(|a| (a, gsi - a.gsi_base))
    }

    /// Processors that are enabled.
    pub fn enabled_cpus(&self) -> usize {
        self.cpus.iter().filter(|c| c.enabled).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    /// Sparse fake physical memory: (base, bytes) blocks.
    struct Fake(Vec<(u64, Vec<u8>)>);

    impl Memory for Fake {
        fn read(&self, phys: u64, len: usize) -> Option<&[u8]> {
            self.0.iter().find(|(b, v)| phys >= *b && phys + len as u64 <= *b + v.len() as u64).map(|(b, v)| &v[(phys - b) as usize..][..len])
        }
    }

    fn fix_checksum(t: &mut [u8], at: usize) {
        t[at] = 0;
        let sum = t.iter().fold(0u8, |a, &b| a.wrapping_add(b));
        t[at] = 0u8.wrapping_sub(sum);
    }

    fn table(sig: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut t = vec![0u8; HEADER_LEN];
        t[0..4].copy_from_slice(sig);
        t[8] = 1;
        t.extend_from_slice(body);
        let len = t.len() as u32;
        t[4..8].copy_from_slice(&len.to_le_bytes());
        fix_checksum(&mut t, 9);
        t
    }

    fn madt_body() -> Vec<u8> {
        let mut b = vec![];
        b.extend(0xFEE0_0000u32.to_le_bytes()); // local APIC address
        b.extend(1u32.to_le_bytes()); // flags: PCAT_COMPAT
        // CPU 0: enabled, CPU 1: disabled but online-capable, CPU 2: unusable
        for (id, flags) in [(0u8, 1u32), (1, 2), (2, 0)] {
            b.extend([0, 8, id, id]);
            b.extend(flags.to_le_bytes());
        }
        // IO-APIC id 4 at 0xFEC00000, GSI base 0
        b.extend([1, 12, 4, 0]);
        b.extend(0xFEC0_0000u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        // Override: ISA IRQ 0 -> GSI 2
        b.extend([2, 10, 0, 0]);
        b.extend(2u32.to_le_bytes());
        b.extend(0u16.to_le_bytes());
        // Override: ISA IRQ 9 -> GSI 9, active low, level triggered
        b.extend([2, 10, 0, 9]);
        b.extend(9u32.to_le_bytes());
        b.extend(0b1111u16.to_le_bytes());
        // A NMI entry we ignore
        b.extend([4, 6, 0xFF, 0, 0, 1]);
        b
    }

    /// RSDP -> XSDT -> [FADT-ish, MADT].
    fn machine(revision: u8) -> (Fake, u64) {
        let madt = table(b"APIC", &madt_body());
        let other = table(b"FACP", &[0; 16]);
        let (madt_at, other_at, root_at, rsdp_at) = (0x2000u64, 0x3000u64, 0x4000u64, 0xE0000u64);
        let mut root_body = vec![];
        for a in [other_at, madt_at] {
            if revision >= 2 {
                root_body.extend(a.to_le_bytes());
            } else {
                root_body.extend((a as u32).to_le_bytes());
            }
        }
        let root = table(if revision >= 2 { b"XSDT" } else { b"RSDT" }, &root_body);
        let mut rsdp = vec![0u8; if revision >= 2 { 36 } else { 20 }];
        rsdp[0..8].copy_from_slice(b"RSD PTR ");
        rsdp[15] = revision;
        rsdp[16..20].copy_from_slice(&(root_at as u32).to_le_bytes());
        if revision >= 2 {
            rsdp[20..24].copy_from_slice(&36u32.to_le_bytes());
            rsdp[24..32].copy_from_slice(&root_at.to_le_bytes());
            fix_checksum(&mut rsdp[..20], 8);
            fix_checksum(&mut rsdp, 32);
        } else {
            fix_checksum(&mut rsdp, 8);
        }
        (Fake(vec![(madt_at, madt), (other_at, other), (root_at, root), (rsdp_at, rsdp)]), rsdp_at)
    }

    #[test]
    fn finds_and_parses_the_madt_through_an_xsdt() {
        let (mem, rsdp_at) = machine(2);
        let rsdp = parse_rsdp(&mem, rsdp_at).unwrap();
        assert_eq!(rsdp.revision, 2);
        let addrs = root_table_addrs(&mem, &rsdp).unwrap();
        assert_eq!(addrs, [0x3000, 0x2000]);
        let madt = parse_madt(find_table(&mem, &addrs, b"APIC").unwrap()).unwrap();
        assert_eq!(madt.lapic_addr, 0xFEE0_0000);
        assert_eq!(madt.flags & 1, 1);
        assert_eq!(madt.cpus.len(), 3);
        assert_eq!(madt.enabled_cpus(), 1);
        assert!(madt.cpus[1].usable && !madt.cpus[1].enabled && !madt.cpus[2].usable);
        assert_eq!(madt.ioapics, [IoApic { id: 4, address: 0xFEC0_0000, gsi_base: 0 }]);
        assert_eq!(madt.overrides.len(), 2);
    }

    #[test]
    fn works_with_an_old_rsdt_too() {
        let (mem, rsdp_at) = machine(0);
        let rsdp = parse_rsdp(&mem, rsdp_at).unwrap();
        assert_eq!(rsdp.xsdt, 0);
        let addrs = root_table_addrs(&mem, &rsdp).unwrap();
        assert_eq!(addrs, [0x3000, 0x2000]);
    }

    #[test]
    fn isa_irqs_follow_overrides_and_default_to_identity() {
        let (mem, rsdp_at) = machine(2);
        let rsdp = parse_rsdp(&mem, rsdp_at).unwrap();
        let addrs = root_table_addrs(&mem, &rsdp).unwrap();
        let madt = parse_madt(find_table(&mem, &addrs, b"APIC").unwrap()).unwrap();
        // The classic QEMU/PC quirk: the timer (IRQ 0) is on GSI 2.
        assert_eq!(madt.isa_route(0), Route { gsi: 2, active_low: false, level_triggered: false });
        assert_eq!(madt.isa_route(9), Route { gsi: 9, active_low: true, level_triggered: true });
        // No override: the PS/2 mouse is IRQ 12 = GSI 12, active high, edge.
        assert_eq!(madt.isa_route(12), Route { gsi: 12, active_low: false, level_triggered: false });
    }

    #[test]
    fn picks_the_io_apic_by_gsi_base() {
        let m = Madt {
            ioapics: vec![IoApic { id: 1, address: 0xFEC0_0000, gsi_base: 0 }, IoApic { id: 2, address: 0xFEC1_0000, gsi_base: 24 }],
            ..Default::default()
        };
        assert_eq!(m.ioapic_for_gsi(12).map(|(a, p)| (a.id, p)), Some((1, 12)));
        assert_eq!(m.ioapic_for_gsi(30).map(|(a, p)| (a.id, p)), Some((2, 6)));
        assert!(Madt::default().ioapic_for_gsi(0).is_none());
    }

    #[test]
    fn rejects_corrupt_tables() {
        let (mut mem, rsdp_at) = machine(2);
        // Flip a byte in the MADT: its checksum no longer matches, so it is skipped.
        mem.0[0].1[40] ^= 0xFF;
        let rsdp = parse_rsdp(&mem, rsdp_at).unwrap();
        let addrs = root_table_addrs(&mem, &rsdp).unwrap();
        assert_eq!(find_table(&mem, &addrs, b"APIC"), Err(Error::NotFound));

        // Bad RSDP signature / checksum.
        let (mut mem, rsdp_at) = machine(2);
        mem.0[3].1[0] = b'X';
        assert_eq!(parse_rsdp(&mem, rsdp_at), Err(Error::BadSignature));
        let (mut mem, rsdp_at) = machine(2);
        mem.0[3].1[16] ^= 1;
        assert_eq!(parse_rsdp(&mem, rsdp_at), Err(Error::BadChecksum));

        // Unmapped memory.
        let (mem, _) = machine(2);
        assert_eq!(parse_rsdp(&mem, 0x9999_0000), Err(Error::Unreadable(0x9999_0000)));
    }

    #[test]
    fn rejects_absurd_lengths_and_malformed_entries() {
        let (mut mem, rsdp_at) = machine(2);
        // XSDT length field set to 4 GiB: must not try to read it.
        mem.0[2].1[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let rsdp = parse_rsdp(&mem, rsdp_at).unwrap();
        assert_eq!(root_table_addrs(&mem, &rsdp), Err(Error::BadLength));

        // An entry whose length runs past the end of the table.
        let mut body = madt_body();
        body.extend([1, 200, 0, 0]);
        let t = table(b"APIC", &body);
        assert_eq!(parse_madt(&t), Err(Error::BadEntry));
        // An entry with length 0 would loop forever if not rejected.
        let mut body = madt_body();
        body.extend([1, 0]);
        assert_eq!(parse_madt(&table(b"APIC", &body)), Err(Error::BadEntry));
        // Wrong signature.
        assert_eq!(parse_madt(&table(b"FACP", &madt_body())), Err(Error::BadSignature));
    }

    #[test]
    fn local_apic_address_override_wins() {
        let mut body = madt_body();
        body.extend([5, 12, 0, 0]);
        body.extend(0x1_FEE0_0000u64.to_le_bytes());
        assert_eq!(parse_madt(&table(b"APIC", &body)).unwrap().lapic_addr, 0x1_FEE0_0000);
    }
}
