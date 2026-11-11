//! A page-table builder for a *running* kernel: tables are allocated on
//! demand from the physical memory manager and edited through whatever
//! window the kernel has onto physical memory (the direct map).
//!
//! The boot-time [`crate::Mapper`] works from a fixed pool; this one is
//! generic over [`TableMemory`], so the kernel plugs in the frame allocator +
//! direct map and host tests plug in a `Vec`.

use crate::{
    ADDR_MASK, HUGE, LARGE_PAGE_SIZE, MapError, NO_EXECUTE, PAGE_SIZE, PRESENT, USER, WRITABLE, index, is_canonical,
};

/// Where page tables come from and how to read and write them.
pub trait TableMemory {
    /// Allocates one zeroed, 4 KiB-aligned table; returns its physical address.
    fn alloc_table(&mut self) -> Option<u64>;
    /// Access to a table previously returned by `alloc_table` (or the root).
    fn table(&mut self, phys: u64) -> &mut [u64; 512];
}

const TABLE_FLAGS: u64 = PRESENT | WRITABLE;

pub struct AddressSpace<M: TableMemory> {
    mem: M,
    root: u64,
}

impl<M: TableMemory> AddressSpace<M> {
    /// Creates an empty address space (just a PML4).
    pub fn new(mut mem: M) -> Result<Self, MapError> {
        let root = mem.alloc_table().ok_or(MapError::OutOfTables)?;
        Ok(Self { mem, root })
    }

    /// Physical address of the PML4 (the value for CR3).
    pub fn root(&self) -> u64 {
        self.root
    }

    pub fn memory(&mut self) -> &mut M {
        &mut self.mem
    }

    /// Returns the next-level table behind `table[idx]`, creating it if absent.
    /// `user` marks the path as reachable from ring 3 (every level of a user
    /// mapping needs the USER bit, or the CPU refuses the access).
    fn child(&mut self, table: u64, idx: usize, virt: u64, user: bool) -> Result<u64, MapError> {
        let entry = self.mem.table(table)[idx];
        let user_bit = if user { USER } else { 0 };
        if entry & PRESENT != 0 {
            if entry & HUGE != 0 {
                return Err(MapError::HugePageInTheWay(virt));
            }
            self.mem.table(table)[idx] = entry | user_bit;
            return Ok(entry & ADDR_MASK);
        }
        let new = self.mem.alloc_table().ok_or(MapError::OutOfTables)?;
        self.mem.table(table)[idx] = new | TABLE_FLAGS | user_bit;
        Ok(new)
    }

    fn check(virt: u64, phys: u64, align: u64) -> Result<(), MapError> {
        if !is_canonical(virt) {
            return Err(MapError::NonCanonical(virt));
        }
        if !virt.is_multiple_of(align) {
            return Err(MapError::Misaligned(virt));
        }
        if !phys.is_multiple_of(align) || phys & !ADDR_MASK != 0 {
            return Err(MapError::Misaligned(phys));
        }
        Ok(())
    }

    /// Maps one 4 KiB page. `flags` are leaf bits (WRITABLE, NO_EXECUTE, cache bits...).
    pub fn map_4k(&mut self, virt: u64, phys: u64, flags: u64) -> Result<(), MapError> {
        Self::check(virt, phys, PAGE_SIZE)?;
        let user = flags & USER != 0;
        let pdpt = self.child(self.root, index(virt, 4), virt, user)?;
        let pd = self.child(pdpt, index(virt, 3), virt, user)?;
        let pt = self.child(pd, index(virt, 2), virt, user)?;
        let e = &mut self.mem.table(pt)[index(virt, 1)];
        if *e & PRESENT != 0 {
            return Err(MapError::AlreadyMapped(virt));
        }
        *e = phys | (flags & !HUGE) | PRESENT;
        Ok(())
    }

    /// Maps one 2 MiB page.
    pub fn map_2m(&mut self, virt: u64, phys: u64, flags: u64) -> Result<(), MapError> {
        Self::check(virt, phys, LARGE_PAGE_SIZE)?;
        let user = flags & USER != 0;
        let pdpt = self.child(self.root, index(virt, 4), virt, user)?;
        let pd = self.child(pdpt, index(virt, 3), virt, user)?;
        let e = &mut self.mem.table(pd)[index(virt, 2)];
        if *e & PRESENT != 0 {
            return Err(MapError::AlreadyMapped(virt));
        }
        *e = phys | flags | HUGE | PRESENT;
        Ok(())
    }

    /// Maps `[phys, phys + len)` at `virt`, using 2 MiB pages wherever both
    /// addresses line up and 4 KiB pages at the ragged edges. All three must
    /// be 4 KiB aligned.
    pub fn map_range(&mut self, virt: u64, phys: u64, len: u64, flags: u64) -> Result<(), MapError> {
        if !len.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(len));
        }
        let mut off = 0;
        while off < len {
            let (v, p) = (virt + off, phys + off);
            if v.is_multiple_of(LARGE_PAGE_SIZE) && p.is_multiple_of(LARGE_PAGE_SIZE) && len - off >= LARGE_PAGE_SIZE {
                self.map_2m(v, p, flags)?;
                off += LARGE_PAGE_SIZE;
            } else {
                self.map_4k(v, p, flags)?;
                off += PAGE_SIZE;
            }
        }
        Ok(())
    }

    /// Unmaps one 4 KiB page and returns the physical frame it pointed at.
    /// (The caller must flush the TLB entry; empty tables are kept.)
    pub fn unmap_4k(&mut self, virt: u64) -> Result<u64, MapError> {
        if !is_canonical(virt) {
            return Err(MapError::NonCanonical(virt));
        }
        if !virt.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(virt));
        }
        let mut table = self.root;
        for level in (2..=4).rev() {
            let entry = self.mem.table(table)[index(virt, level)];
            if entry & PRESENT == 0 {
                return Err(MapError::NotMapped(virt));
            }
            if level == 2 && entry & HUGE != 0 {
                return Err(MapError::HugePageInTheWay(virt));
            }
            table = entry & ADDR_MASK;
        }
        let e = &mut self.mem.table(table)[index(virt, 1)];
        if *e & PRESENT == 0 {
            return Err(MapError::NotMapped(virt));
        }
        let phys = *e & ADDR_MASK;
        *e = 0;
        Ok(phys)
    }

    /// Walks the tables like the CPU would: (physical address, leaf flags).
    pub fn translate(&mut self, virt: u64) -> Option<(u64, u64)> {
        if !is_canonical(virt) {
            return None;
        }
        let mut table = self.root;
        for level in (1..=4).rev() {
            let entry = self.mem.table(table)[index(virt, level)];
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

    /// True if `virt` is mapped executable (present, NX clear).
    pub fn is_executable(&mut self, virt: u64) -> bool {
        self.translate(virt).is_some_and(|(_, f)| f & NO_EXECUTE == 0)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::boxed::Box;
    use std::vec::Vec;

    /// Tables live in a Vec; "physical addresses" are table index * 4096 + 0x10000.
    struct FakeMemory {
        tables: Vec<Box<[u64; 512]>>,
        limit: usize,
    }

    const BASE: u64 = 0x10000;

    impl FakeMemory {
        fn new(limit: usize) -> Self {
            Self { tables: Vec::new(), limit }
        }
    }

    impl TableMemory for FakeMemory {
        fn alloc_table(&mut self) -> Option<u64> {
            if self.tables.len() >= self.limit {
                return None;
            }
            self.tables.push(Box::new([0; 512]));
            Some(BASE + (self.tables.len() as u64 - 1) * PAGE_SIZE)
        }

        fn table(&mut self, phys: u64) -> &mut [u64; 512] {
            &mut self.tables[((phys - BASE) / PAGE_SIZE) as usize]
        }
    }

    fn space(limit: usize) -> AddressSpace<FakeMemory> {
        AddressSpace::new(FakeMemory::new(limit)).unwrap()
    }

    const KERNEL: u64 = 0xFFFF_FFFF_8000_0000;

    #[test]
    fn maps_and_translates_a_4k_page() {
        let mut a = space(16);
        a.map_4k(KERNEL, 0x20_0000, WRITABLE).unwrap();
        assert_eq!(a.translate(KERNEL + 0x123), Some((0x20_0123, WRITABLE | PRESENT)));
        assert_eq!(a.translate(KERNEL + PAGE_SIZE), None);
    }

    #[test]
    fn maps_and_translates_a_2m_page() {
        let mut a = space(16);
        a.map_2m(0xFFFF_8000_0000_0000, 0x4000_0000, WRITABLE | NO_EXECUTE).unwrap();
        let (p, f) = a.translate(0xFFFF_8000_0000_0000 + 0x1F_FFFF).unwrap();
        assert_eq!(p, 0x4000_0000 + 0x1F_FFFF);
        assert_ne!(f & NO_EXECUTE, 0);
        assert!(!a.is_executable(0xFFFF_8000_0000_0000));
    }

    #[test]
    fn refuses_to_remap_or_cross_a_huge_page() {
        let mut a = space(16);
        a.map_2m(0x4000_0000, 0, WRITABLE).unwrap();
        assert_eq!(a.map_2m(0x4000_0000, 0x20_0000, WRITABLE), Err(MapError::AlreadyMapped(0x4000_0000)));
        assert_eq!(a.map_4k(0x4000_1000, 0x5000, WRITABLE), Err(MapError::HugePageInTheWay(0x4000_1000)));
        a.map_4k(0x8000_0000, 0x5000, WRITABLE).unwrap();
        assert_eq!(a.map_4k(0x8000_0000, 0x6000, WRITABLE), Err(MapError::AlreadyMapped(0x8000_0000)));
    }

    #[test]
    fn rejects_bad_addresses() {
        let mut a = space(16);
        assert_eq!(a.map_4k(0x0000_8000_0000_0000, 0, 0), Err(MapError::NonCanonical(0x0000_8000_0000_0000)));
        assert_eq!(a.map_4k(0x1001, 0, 0), Err(MapError::Misaligned(0x1001)));
        assert_eq!(a.map_4k(0x1000, 0x1234, 0), Err(MapError::Misaligned(0x1234)));
        assert_eq!(a.map_2m(0x1000, 0, 0), Err(MapError::Misaligned(0x1000)));
        assert_eq!(a.map_range(0x1000, 0x1000, 100, 0), Err(MapError::Misaligned(100)));
    }

    #[test]
    fn map_range_uses_large_pages_in_the_middle_and_small_at_the_edges() {
        let mut a = space(32);
        // 0x1FF000 .. 0x601000: ragged head (1 page), two 2 MiB pages, ragged tail (1 page).
        a.map_range(0x1F_F000, 0x1F_F000, 0x40_2000, WRITABLE).unwrap();
        for off in (0..0x40_2000u64).step_by(PAGE_SIZE as usize) {
            assert_eq!(a.translate(0x1F_F000 + off).map(|t| t.0), Some(0x1F_F000 + off), "offset {off:#x}");
        }
        assert_eq!(a.translate(0x1F_F000 + 0x40_2000), None, "one past the end is unmapped");
        // PML4 + PDPT + PD + one PT for the head + one PT for the tail: the middle used no PTs.
        assert_eq!(a.mem.tables.len(), 5);
    }

    #[test]
    fn map_range_large_ranges_need_few_tables() {
        let mut a = space(8);
        // 1 GiB of 2 MiB pages: PML4 + PDPT + PD = 3 tables.
        a.map_range(0xFFFF_8000_0000_0000, 0, 1 << 30, WRITABLE).unwrap();
        assert_eq!(a.mem.tables.len(), 3);
    }

    #[test]
    fn unmap_returns_the_frame_and_clears_the_mapping() {
        let mut a = space(16);
        a.map_4k(0xFFFF_A000_0000_0000, 0x7000, WRITABLE).unwrap();
        assert_eq!(a.unmap_4k(0xFFFF_A000_0000_0000), Ok(0x7000));
        assert_eq!(a.translate(0xFFFF_A000_0000_0000), None);
        assert_eq!(a.unmap_4k(0xFFFF_A000_0000_0000), Err(MapError::NotMapped(0xFFFF_A000_0000_0000)));
        assert_eq!(a.unmap_4k(0x1234_5000), Err(MapError::NotMapped(0x1234_5000)));
        a.map_4k(0xFFFF_A000_0000_0000, 0x8000, WRITABLE).unwrap();
        assert_eq!(a.translate(0xFFFF_A000_0000_0000).map(|t| t.0), Some(0x8000), "can be mapped again");
    }

    #[test]
    fn user_mappings_mark_every_level_user_accessible() {
        let mut a = space(16);
        a.map_4k(0x40_0000, 0x7000, WRITABLE | USER).unwrap();
        // Walk by hand: all of PML4/PDPT/PD/PT entries on the path carry USER.
        let mut table = a.root();
        for level in (1..=4).rev() {
            let e = a.memory().table(table)[index(0x40_0000, level)];
            assert_ne!(e & USER, 0, "level {level} must allow user access");
            table = e & ADDR_MASK;
        }
        // A later kernel-only mapping in a *different* subtree stays supervisor-only.
        a.map_4k(0xFFFF_8000_0000_0000, 0x8000, WRITABLE).unwrap();
        let root = a.root();
        let e = a.memory().table(root)[index(0xFFFF_8000_0000_0000, 4)];
        assert_eq!(e & USER, 0);
        // Mapping a user page next to a kernel page in the same table upgrades the path
        // but not the kernel leaf.
        a.map_4k(0x40_1000, 0x9000, WRITABLE).unwrap();
        assert_eq!(a.translate(0x40_1000).unwrap().1 & USER, 0);
        assert_ne!(a.translate(0x40_0000).unwrap().1 & USER, 0);
    }

    #[test]
    fn out_of_tables_is_reported_not_panicked() {
        let mut a = space(2); // PML4 + one more
        assert_eq!(a.map_4k(0x1000, 0x1000, 0), Err(MapError::OutOfTables));
    }
}
