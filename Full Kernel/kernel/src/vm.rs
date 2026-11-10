//! Kernel virtual memory: the kernel's own page tables.
//!
//! NovaBoot's tables were only meant to get the kernel started: they contain
//! an identity map of low memory (so the bootloader survives the CR3 switch)
//! and map everything write-back. This module builds the kernel's own set:
//!
//! | Virtual range | Maps | Flags |
//! |---|---|---|
//! | `0xFFFF_8000_0000_0000 +` | all RAM-like physical memory (direct map) | RW, NX |
//! | same, over the framebuffer | the framebuffer | RW, NX, **write-combining** |
//! | `0xFFFF_FFFF_8000_0000 +` | the kernel image, per section | text R-X, rodata R--, data RW- |
//! | below `stack_top` | the boot stack (guard page below stays unmapped) | RW, NX |
//! | `0xFFFF_A000_0000_0000 +` | the kernel heap (see heap.rs) | RW, NX |
//!
//! The low half of the address space is empty: a null pointer, or any stray
//! pointer into the old identity-mapped area, is a page fault now.
//!
//! The old tables' memory (and the rest of `Loader` memory) is returned to the
//! frame allocator by [`reclaim_loader_memory`] once the switch is done.

use crate::bootdata;
use crate::log::SpinLock;
use crate::pmm;
use core::sync::atomic::{AtomicU64, Ordering};
use nova_bootinfo::{BootInfo, MemoryKind};
use nova_paging::dynamic::{AddressSpace, TableMemory};
use nova_paging::{MapError, NO_EXECUTE, PAGE_SIZE, WRITABLE};

/// Page-write-through bit. With the PAT set up below, PWT alone selects
/// write-combining (PAT entry 1), which is what a framebuffer wants.
const PWT: u64 = 1 << 3;
const PAT_ENTRY1_WC: u64 = 0x01;

unsafe extern "C" {
    static __kernel_start: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __kernel_end: u8;
}

/// Where page tables come from: the frame allocator, edited via the direct map.
pub struct KernelMemory {
    direct_map: u64,
}

impl TableMemory for KernelMemory {
    fn alloc_table(&mut self) -> Option<u64> {
        let frame = pmm::alloc_frame()?;
        // SAFETY: a frame we just allocated, inside the direct map.
        unsafe { core::ptr::write_bytes(self.table(frame).as_mut_ptr(), 0, 512) };
        Some(frame)
    }

    fn table(&mut self, phys: u64) -> &mut [u64; 512] {
        // SAFETY: table frames are RAM inside the direct map and only edited
        // through the (locked) address space.
        unsafe { &mut *((self.direct_map + phys) as *mut [u64; 512]) }
    }
}

static SPACE: SpinLock<Option<AddressSpace<KernelMemory>>> = SpinLock::new(None);
static ROOT: AtomicU64 = AtomicU64::new(0);

/// What [`init`] built, for the boot log.
pub struct Summary {
    pub root: u64,
    pub ram_ranges: usize,
    pub direct_map_bytes: u64,
    pub tables: usize,
}

/// True for memory kinds that are RAM (or RAM-like, and safe to map as
/// ordinary write-back memory): everything but holes, MMIO and bad memory.
fn is_ram_like(kind: MemoryKind) -> bool {
    !matches!(kind, MemoryKind::Reserved | MemoryKind::Mmio | MemoryKind::Bad)
}

fn addr_of<T>(sym: *const T) -> u64 {
    sym as u64
}

/// Builds the kernel's page tables and switches to them.
///
/// # Safety
/// Call once, after [`pmm::init`], with the bootloader's direct map active.
/// Panics (instead of risking a triple fault) if the new tables would not map
/// the code, stack, boot data or framebuffer the kernel is using.
pub unsafe fn init() -> Result<Summary, &'static str> {
    let info = bootdata::info();
    let map = bootdata::memory_map();
    let nx = if info.nx_enabled != 0 { NO_EXECUTE } else { 0 };
    let direct = info.physical_memory_offset;

    // Cache attributes: make PAT entry 1 write-combining (it is write-through by default).
    // SAFETY: PAT is a standard x86-64 MSR.
    unsafe {
        let pat = nova_x86::rdmsr(nova_x86::MSR_PAT);
        nova_x86::wrmsr(nova_x86::MSR_PAT, (pat & !0xFF00) | (PAT_ENTRY1_WC << 8));
    }

    let mut space = AddressSpace::new(KernelMemory { direct_map: direct }).map_err(|_| "no memory for the root page table")?;
    let fail = |what: &'static str| move |_: MapError| what;

    // 1. Direct map of RAM: merge neighbouring RAM-like regions, then map each run.
    let mut ranges = 0;
    let mut mapped_bytes = 0;
    let mut run: Option<(u64, u64)> = None;
    let mut flush = |space: &mut AddressSpace<KernelMemory>, run: &mut Option<(u64, u64)>| -> Result<(), &'static str> {
        if let Some((start, end)) = run.take() {
            space.map_range(direct + start, start, end - start, WRITABLE | nx).map_err(fail("direct map of RAM failed"))?;
            ranges += 1;
            mapped_bytes += end - start;
        }
        Ok(())
    };
    for r in map.iter().filter(|r| is_ram_like(r.kind())) {
        let (start, end) = (r.start & !(PAGE_SIZE - 1), r.end().next_multiple_of(PAGE_SIZE));
        match &mut run {
            Some((_, e)) if start <= *e => *e = (*e).max(end),
            _ => {
                flush(&mut space, &mut run)?;
                run = Some((start, end));
            }
        }
    }
    flush(&mut space, &mut run)?;

    // 2. Framebuffer: write-combining, at its usual direct-map address.
    let fb = info.framebuffer;
    if fb.is_present() {
        let start = fb.addr & !(PAGE_SIZE - 1);
        let end = (fb.addr + fb.stride as u64 * fb.height as u64 * 4).next_multiple_of(PAGE_SIZE);
        space
            .map_range(direct + start, start, end - start, WRITABLE | nx | PWT)
            .map_err(fail("could not map the framebuffer (does it overlap RAM?)"))?;
    }

    // 3. The kernel image, section by section.
    let (k_start, text_end, ro_end, k_end) =
        (addr_of(&raw const __kernel_start), addr_of(&raw const __text_end), addr_of(&raw const __rodata_end), addr_of(&raw const __kernel_end));
    let (text, rodata, data) = (addr_of(&raw const __text_start), addr_of(&raw const __rodata_start), addr_of(&raw const __data_start));
    if k_start != info.kernel_virt {
        return Err("kernel link address does not match the boot info");
    }
    if k_end - k_start > info.kernel_pages * PAGE_SIZE {
        return Err("kernel image is larger than the pages NovaBoot loaded");
    }
    let phys_of = |virt: u64| info.kernel_phys + (virt - k_start);
    for (from, to, flags) in [(text, text_end, 0), (rodata, ro_end, nx), (data, k_end, WRITABLE | nx)] {
        space.map_range(from, phys_of(from), to - from, flags).map_err(fail("could not map the kernel image"))?;
    }

    // 4. The boot stack (its guard page below stays unmapped).
    let stack_base = info.stack_top - info.stack_pages * PAGE_SIZE;
    space
        .map_range(stack_base, info.stack_phys, info.stack_pages * PAGE_SIZE, WRITABLE | nx)
        .map_err(fail("could not map the kernel stack"))?;

    // 5. Look before leaping: everything we are about to depend on must resolve.
    let here = init as *const () as u64;
    let sp: u64;
    // SAFETY: reading rsp has no side effects.
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags)) };
    let boot_info_addr = info as *const BootInfo as u64;
    let checks = [
        ("this code", here),
        ("the current stack", sp),
        ("the boot data copy", boot_info_addr),
        ("the memory map copy", bootdata::memory_map().as_ptr() as u64),
        ("the frame bitmap / direct map", direct + pmm::bitmap_phys()),
    ];
    for (what, addr) in checks {
        if space.translate(addr).is_none() {
            crate::kprintln!("vm: the new page tables do not map {what} ({addr:#x}); refusing to switch");
            return Err("new page tables are incomplete");
        }
    }
    if fb.is_present() && space.translate(direct + fb.addr).is_none() {
        return Err("new page tables do not map the framebuffer");
    }

    // 6. Switch. From here the identity map and the bootloader's tables are gone.
    let root = space.root();
    // SAFETY: the checks above show the new tables map the running code, stack and data.
    unsafe { nova_x86::write_cr3(root) };
    ROOT.store(root, Ordering::SeqCst);

    // Count the tables for the log, then keep the address space for later mappings.
    let tables = count_tables(&mut space);
    SPACE.with(|s| *s = Some(space));
    Ok(Summary { root, ram_ranges: ranges, direct_map_bytes: mapped_bytes, tables })
}

/// Counts the page-table pages reachable from the root (for the boot log).
fn count_tables(space: &mut AddressSpace<KernelMemory>) -> usize {
    let mut n = 1;
    let root = space.root();
    let mem = space.memory();
    for i in 0..512 {
        let l4 = mem.table(root)[i];
        if l4 & 1 == 0 {
            continue;
        }
        n += 1;
        let pdpt = l4 & nova_paging::ADDR_MASK;
        for j in 0..512 {
            let l3 = mem.table(pdpt)[j];
            if l3 & 1 == 0 {
                continue;
            }
            n += 1;
            let pd = l3 & nova_paging::ADDR_MASK;
            for k in 0..512 {
                let l2 = mem.table(pd)[k];
                if l2 & 1 != 0 && l2 & nova_paging::HUGE == 0 {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Gives back the memory the bootloader was using: its page tables, the
/// boot info and memory-map arrays, its own code and data. Only the kernel
/// image and the boot stack (both `Loader` memory too) are kept.
/// Returns the number of frames freed.
pub fn reclaim_loader_memory() -> usize {
    let info = bootdata::info();
    let mut freed = 0;
    for r in bootdata::memory_map().iter().filter(|r| r.kind() == MemoryKind::Loader) {
        freed += pmm::release_range(r.start.max(nova_pmm::LOW_MEMORY_LIMIT), r.end());
    }
    // The kernel image and boot stack sit inside Loader regions: take them back.
    pmm::reserve_range(info.kernel_phys, info.kernel_phys + info.kernel_pages * PAGE_SIZE);
    pmm::reserve_range(info.stack_phys, info.stack_phys + info.stack_pages * PAGE_SIZE);
    // (The two reserve calls re-reserve frames that release_range just freed; don't count those.)
    freed.saturating_sub(info.kernel_pages as usize + info.stack_pages as usize)
}

/// Page-cache-disable + write-through: with the default PAT this is
/// *uncached*, which is what device registers need.
const UNCACHED: u64 = (1 << 4) | PWT;

/// Maps device registers (MMIO) uncached, at their usual direct-map address
/// (`physical offset + phys`). Returns the virtual address of `phys`.
/// Mapping the same page twice is fine.
pub fn map_mmio(phys: u64, bytes: u64) -> Result<u64, MapError> {
    let direct = bootdata::info().physical_memory_offset;
    let first = phys & !(PAGE_SIZE - 1);
    let end = (phys + bytes).next_multiple_of(PAGE_SIZE);
    let nx = if bootdata::info().nx_enabled != 0 { NO_EXECUTE } else { 0 };
    let mut page = first;
    while page < end {
        match map_page(direct + page, page, WRITABLE | nx | UNCACHED) {
            Ok(()) | Err(MapError::AlreadyMapped(_)) => {}
            Err(e) => return Err(e),
        }
        page += PAGE_SIZE;
    }
    Ok(direct + phys)
}

/// Maps one 4 KiB page in the kernel's address space.
pub fn map_page(virt: u64, phys: u64, flags: u64) -> Result<(), MapError> {
    let r = SPACE.with(|s| s.as_mut().ok_or(MapError::Corrupt(0))?.map_4k(virt, phys, flags));
    if r.is_ok() {
        nova_x86::invlpg(virt);
    }
    r
}

#[allow(dead_code)] // used by later stages (user space, stack guards)
/// Unmaps one 4 KiB page; returns its frame (the caller decides whether to free it).
pub fn unmap_page(virt: u64) -> Result<u64, MapError> {
    let r = SPACE.with(|s| s.as_mut().ok_or(MapError::Corrupt(0))?.unmap_4k(virt));
    if r.is_ok() {
        nova_x86::invlpg(virt);
    }
    r
}

/// (physical address, leaf flags) for a virtual address, if mapped.
pub fn translate(virt: u64) -> Option<(u64, u64)> {
    SPACE.with(|s| s.as_mut()?.translate(virt))
}

pub fn root() -> u64 {
    ROOT.load(Ordering::SeqCst)
}
