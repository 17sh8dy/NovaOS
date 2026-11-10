//! x86_64 4-level paging, built from a fixed pool of page tables.
//!
//! The pool is a slice of [`Table`]s whose *physical* address is known.
//! Table entries hold physical addresses, and the mapper turns them back
//! into pool indices, so the same code works in the bootloader (where the
//! pool is identity-mapped) and in host unit tests (where it is a `Vec`).

#![no_std]

pub mod dynamic;

#[cfg(test)]
extern crate std;

use core::fmt;

pub const PAGE_SIZE: u64 = 0x1000;
pub const LARGE_PAGE_SIZE: u64 = 0x20_0000; // 2 MiB

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
pub const USER: u64 = 1 << 2;
pub const HUGE: u64 = 1 << 7;
pub const GLOBAL: u64 = 1 << 8;
pub const NO_EXECUTE: u64 = 1 << 63;

/// Bits 12..52 of an entry: the physical address.
pub const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;
const TABLE_FLAGS: u64 = PRESENT | WRITABLE;

#[repr(C, align(4096))]
#[derive(Clone)]
pub struct Table(pub [u64; 512]);

impl Table {
    pub const EMPTY: Table = Table([0; 512]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// The pool has no free tables left.
    OutOfTables,
    /// An address is not aligned to the page size being mapped.
    Misaligned(u64),
    /// A virtual address is not canonical (bits 48..64 must copy bit 47).
    NonCanonical(u64),
    /// The page is already mapped.
    AlreadyMapped(u64),
    /// A 4 KiB mapping would have to go through an existing 2 MiB page.
    HugePageInTheWay(u64),
    /// An entry points outside the pool (corrupt tables).
    Corrupt(u64),
    /// Nothing is mapped at this address (unmapping).
    NotMapped(u64),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfTables => write!(f, "page table pool exhausted"),
            Self::Misaligned(a) => write!(f, "address {a:#x} is misaligned"),
            Self::NonCanonical(a) => write!(f, "address {a:#x} is not canonical"),
            Self::AlreadyMapped(a) => write!(f, "address {a:#x} is already mapped"),
            Self::HugePageInTheWay(a) => write!(f, "address {a:#x} is inside a 2 MiB page"),
            Self::Corrupt(a) => write!(f, "table entry points outside the pool ({a:#x})"),
            Self::NotMapped(a) => write!(f, "address {a:#x} is not mapped"),
        }
    }
}

pub fn is_canonical(virt: u64) -> bool {
    let top = virt >> 47;
    top == 0 || top == 0x1_FFFF
}

/// Index into the table at `level` (4 = PML4 .. 1 = PT).
pub fn index(virt: u64, level: u32) -> usize {
    ((virt >> (12 + 9 * (level - 1))) & 0x1FF) as usize
}

/// How many tables mapping `bytes` with 2 MiB pages needs below one PML4
/// entry (PDPTs + PDs), assuming the range starts 1 GiB-aligned.
pub fn tables_for_large_range(bytes: u64) -> usize {
    let gib = bytes.div_ceil(1 << 30) as usize;
    gib.div_ceil(512) + gib
}

/// An upper bound on the tables needed to map `pages` contiguous 4 KiB pages
/// anywhere (PDPT + PDs + PTs, allowing for crossing boundaries).
pub fn tables_for_small_range(pages: u64) -> usize {
    let pts = pages.div_ceil(512) as usize + 1;
    let pds = pts.div_ceil(512) + 1;
    let pdpts = pds.div_ceil(512) + 1;
    pts + pds + pdpts
}

pub struct Mapper<'a> {
    pool: &'a mut [Table],
    pool_phys: u64,
    used: usize,
}

impl<'a> Mapper<'a> {
    /// Takes a pool of tables located at physical address `pool_phys`
    /// (4 KiB aligned). Table 0 becomes the PML4.
    pub fn new(pool: &'a mut [Table], pool_phys: u64) -> Result<Self, MapError> {
        if !pool_phys.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(pool_phys));
        }
        if pool.is_empty() {
            return Err(MapError::OutOfTables);
        }
        pool[0] = Table::EMPTY;
        Ok(Self { pool, pool_phys, used: 1 })
    }

    /// Physical address of the PML4 (the value for CR3).
    pub fn root(&self) -> u64 {
        self.pool_phys
    }

    pub fn tables_used(&self) -> usize {
        self.used
    }

    pub fn tables_total(&self) -> usize {
        self.pool.len()
    }

    fn slot(&self, phys: u64) -> Result<usize, MapError> {
        let offset = phys.wrapping_sub(self.pool_phys);
        let i = (offset / PAGE_SIZE) as usize;
        if phys < self.pool_phys || !offset.is_multiple_of(PAGE_SIZE) || i >= self.used {
            return Err(MapError::Corrupt(phys));
        }
        Ok(i)
    }

    fn alloc(&mut self) -> Result<u64, MapError> {
        if self.used == self.pool.len() {
            return Err(MapError::OutOfTables);
        }
        self.pool[self.used] = Table::EMPTY;
        let phys = self.pool_phys + self.used as u64 * PAGE_SIZE;
        self.used += 1;
        Ok(phys)
    }

    /// Returns the table that `table[idx]` points to, creating it if needed.
    fn child(&mut self, table: u64, idx: usize, virt: u64) -> Result<u64, MapError> {
        let slot = self.slot(table)?;
        let entry = self.pool[slot].0[idx];
        if entry & PRESENT != 0 {
            if entry & HUGE != 0 {
                return Err(MapError::HugePageInTheWay(virt));
            }
            return Ok(entry & ADDR_MASK);
        }
        let new = self.alloc()?;
        self.pool[slot].0[idx] = new | TABLE_FLAGS;
        Ok(new)
    }

    fn check(virt: u64, phys: u64, align: u64) -> Result<(), MapError> {
        if !is_canonical(virt) {
            return Err(MapError::NonCanonical(virt));
        }
        if !virt.is_multiple_of(align) {
            return Err(MapError::Misaligned(virt));
        }
        if !phys.is_multiple_of(align) || phys & !ADDR_MASK & !(align - 1) != 0 {
            return Err(MapError::Misaligned(phys));
        }
        Ok(())
    }

    /// Maps one 4 KiB page. `flags` are extra leaf bits (WRITABLE, NO_EXECUTE, ...).
    pub fn map_4k(&mut self, virt: u64, phys: u64, flags: u64) -> Result<(), MapError> {
        Self::check(virt, phys, PAGE_SIZE)?;
        let pdpt = self.child(self.root(), index(virt, 4), virt)?;
        let pd = self.child(pdpt, index(virt, 3), virt)?;
        let pt = self.child(pd, index(virt, 2), virt)?;
        let slot = self.slot(pt)?;
        let e = &mut self.pool[slot].0[index(virt, 1)];
        if *e & PRESENT != 0 {
            return Err(MapError::AlreadyMapped(virt));
        }
        *e = phys | (flags & !HUGE) | PRESENT;
        Ok(())
    }

    /// Maps one 2 MiB page.
    pub fn map_2m(&mut self, virt: u64, phys: u64, flags: u64) -> Result<(), MapError> {
        Self::check(virt, phys, LARGE_PAGE_SIZE)?;
        let pdpt = self.child(self.root(), index(virt, 4), virt)?;
        let pd = self.child(pdpt, index(virt, 3), virt)?;
        let slot = self.slot(pd)?;
        let e = &mut self.pool[slot].0[index(virt, 2)];
        if *e & PRESENT != 0 {
            return Err(MapError::AlreadyMapped(virt));
        }
        *e = phys | flags | HUGE | PRESENT;
        Ok(())
    }

    /// Maps `[phys, phys + len)` at `virt` with 2 MiB pages.
    pub fn map_range_2m(&mut self, virt: u64, phys: u64, len: u64, flags: u64) -> Result<(), MapError> {
        let mut off = 0;
        while off < len {
            self.map_2m(virt + off, phys + off, flags)?;
            off += LARGE_PAGE_SIZE;
        }
        Ok(())
    }

    /// Walks the tables like the CPU would. Returns the physical address
    /// and the leaf entry's flags.
    pub fn translate(&self, virt: u64) -> Option<(u64, u64)> {
        if !is_canonical(virt) {
            return None;
        }
        let mut table = self.root();
        for level in (1..=4).rev() {
            let entry = self.pool[self.slot(table).ok()?].0[index(virt, level)];
            if entry & PRESENT == 0 {
                return None;
            }
            let flags = entry & !ADDR_MASK;
            if level == 1 {
                return Some(((entry & ADDR_MASK) | (virt & 0xFFF), flags));
            }
            if level == 2 && entry & HUGE != 0 {
                let base = entry & ADDR_MASK & !(LARGE_PAGE_SIZE - 1);
                return Some((base | (virt & (LARGE_PAGE_SIZE - 1)), flags));
            }
            table = entry & ADDR_MASK;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    const POOL_PHYS: u64 = 0x10_0000;

    fn pool(n: usize) -> Vec<Table> {
        vec![Table::EMPTY; n]
    }

    #[test]
    fn indices_split_the_address() {
        let v = 0xFFFF_FFFF_8000_0000u64;
        assert_eq!(index(v, 4), 511);
        assert_eq!(index(v, 3), 510);
        assert_eq!(index(v, 2), 0);
        assert_eq!(index(v, 1), 0);
        assert_eq!(index(0xFFFF_8000_0000_0000, 4), 256);
        assert_eq!(index(0x20_1000, 2), 1);
        assert_eq!(index(0x20_1000, 1), 1);
    }

    #[test]
    fn canonical_addresses() {
        assert!(is_canonical(0));
        assert!(is_canonical(0x0000_7FFF_FFFF_FFFF));
        assert!(is_canonical(0xFFFF_8000_0000_0000));
        assert!(!is_canonical(0x0000_8000_0000_0000));
        assert!(!is_canonical(0xFFFF_7FFF_FFFF_FFFF));
    }

    #[test]
    fn maps_and_translates_4k_pages() {
        let mut p = pool(8);
        let mut m = Mapper::new(&mut p, POOL_PHYS).unwrap();
        let virt = 0xFFFF_FFFF_8000_0000;
        m.map_4k(virt, 0x40_0000, WRITABLE).unwrap();
        m.map_4k(virt + 0x1000, 0x50_0000, NO_EXECUTE).unwrap();
        assert_eq!(m.translate(virt + 0x123).unwrap().0, 0x40_0123);
        let (phys, flags) = m.translate(virt + 0x1FFF).unwrap();
        assert_eq!(phys, 0x50_0FFF);
        assert_ne!(flags & NO_EXECUTE, 0);
        assert_eq!(flags & WRITABLE, 0);
        assert_eq!(m.translate(virt + 0x2000), None);
        // PML4 + PDPT + PD + PT, shared by both pages.
        assert_eq!(m.tables_used(), 4);
    }

    #[test]
    fn maps_2m_ranges() {
        let mut p = pool(8);
        let mut m = Mapper::new(&mut p, POOL_PHYS).unwrap();
        m.map_range_2m(0, 0, 4 << 30, WRITABLE).unwrap();
        assert_eq!(m.translate(0x1234_5678).unwrap().0, 0x1234_5678);
        assert_eq!(m.translate(0xFFFF_FFFF).unwrap().0, 0xFFFF_FFFF);
        assert_eq!(m.translate(1 << 32), None);
        // PML4 + PDPT + 4 PDs.
        assert_eq!(m.tables_used(), 6);
        assert_eq!(1 + tables_for_large_range(4 << 30), 6);
    }

    #[test]
    fn same_physical_memory_at_two_addresses() {
        let mut p = pool(8);
        let mut m = Mapper::new(&mut p, POOL_PHYS).unwrap();
        let hhdm = 0xFFFF_8000_0000_0000;
        m.map_range_2m(0, 0, 1 << 30, WRITABLE).unwrap();
        m.map_range_2m(hhdm, 0, 1 << 30, WRITABLE).unwrap();
        assert_eq!(m.translate(hhdm + 0xB8000).unwrap().0, 0xB8000);
        assert_eq!(m.translate(0xB8000).unwrap().0, 0xB8000);
    }

    #[test]
    fn refuses_double_mapping() {
        let mut p = pool(8);
        let mut m = Mapper::new(&mut p, POOL_PHYS).unwrap();
        m.map_4k(0x1000, 0x2000, 0).unwrap();
        assert_eq!(m.map_4k(0x1000, 0x3000, 0), Err(MapError::AlreadyMapped(0x1000)));
        m.map_2m(0x40_0000, 0, 0).unwrap();
        assert_eq!(m.map_2m(0x40_0000, 0, 0), Err(MapError::AlreadyMapped(0x40_0000)));
        assert_eq!(m.map_4k(0x40_1000, 0, 0), Err(MapError::HugePageInTheWay(0x40_1000)));
    }

    #[test]
    fn refuses_bad_addresses() {
        let mut p = pool(8);
        let mut m = Mapper::new(&mut p, POOL_PHYS).unwrap();
        assert_eq!(m.map_4k(0x1001, 0, 0), Err(MapError::Misaligned(0x1001)));
        assert_eq!(m.map_4k(0x1000, 0x10, 0), Err(MapError::Misaligned(0x10)));
        assert_eq!(m.map_2m(0x1000, 0, 0), Err(MapError::Misaligned(0x1000)));
        assert_eq!(m.map_4k(0x8000_0000_0000, 0, 0), Err(MapError::NonCanonical(0x8000_0000_0000)));
        // A physical address beyond 52 bits.
        assert_eq!(m.map_4k(0x1000, 1 << 52, 0), Err(MapError::Misaligned(1 << 52)));
    }

    #[test]
    fn reports_pool_exhaustion_instead_of_overrunning() {
        let mut p = pool(3); // PML4 + PDPT + PD, no room for a PT
        let mut m = Mapper::new(&mut p, POOL_PHYS).unwrap();
        assert_eq!(m.map_4k(0x1000, 0x1000, 0), Err(MapError::OutOfTables));
        assert_eq!(m.tables_used(), 3);
    }

    #[test]
    fn small_range_estimate_is_enough() {
        for pages in [1u64, 2, 511, 512, 513, 2048, 10_000] {
            let need = tables_for_small_range(pages);
            let mut p = pool(1 + need);
            let mut m = Mapper::new(&mut p, POOL_PHYS).unwrap();
            // Start just below a 1 GiB boundary so every level is crossed.
            let base = 0xFFFF_FFFF_8000_0000u64 - 0x1000 * 7;
            for i in 0..pages {
                m.map_4k(base + i * 0x1000, i * 0x1000, 0).unwrap();
            }
            assert!(m.tables_used() <= 1 + need, "pages={pages}");
        }
    }

    #[test]
    fn rejects_misaligned_pool() {
        let mut p = pool(1);
        assert_eq!(Mapper::new(&mut p, 0x123).err(), Some(MapError::Misaligned(0x123)));
    }
}
