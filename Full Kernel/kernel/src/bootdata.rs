//! The boot information, copied into kernel memory.
//!
//! NovaBoot's `BootInfo` and memory map live in `Loader` memory, which the
//! kernel gives back to the frame allocator once it has its own page tables.
//! So the very first thing the kernel does is copy both here.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use nova_bootinfo::{BootInfo, MemoryKind, MemoryRegion};

/// More regions than any real firmware map has (this PC: ~35).
pub const MAX_REGIONS: usize = 512;

struct Store {
    info: UnsafeCell<MaybeUninit<BootInfo>>,
    map: UnsafeCell<[MemoryRegion; MAX_REGIONS]>,
    len: UnsafeCell<usize>,
}

// SAFETY: written once, during early single-CPU boot, before anything reads it.
unsafe impl Sync for Store {}

static STORE: Store = Store {
    info: UnsafeCell::new(MaybeUninit::uninit()),
    map: UnsafeCell::new([MemoryRegion::new(0, 0, MemoryKind::Reserved); MAX_REGIONS]),
    len: UnsafeCell::new(0),
};

/// Copies the boot info and memory map into the kernel.
///
/// # Safety
/// Call once, early, on the boot CPU, with the bootloader's direct map still
/// in place and `info` already validated.
pub unsafe fn capture(info: &BootInfo) {
    // SAFETY: the direct map is in place (caller); we are the only writer.
    unsafe {
        let map = info.memory_map();
        if map.len() > MAX_REGIONS {
            panic!("memory map has {} regions; the kernel keeps at most {}", map.len(), MAX_REGIONS);
        }
        let dst: &mut [MemoryRegion; MAX_REGIONS] = &mut *STORE.map.get();
        dst[..map.len()].copy_from_slice(map);
        *STORE.len.get() = map.len();
        (*STORE.info.get()).write(*info);
    }
}

/// The boot info (valid after [`capture`]).
pub fn info() -> &'static BootInfo {
    // SAFETY: initialised by `capture` before anything calls this; never mutated after.
    unsafe { (*STORE.info.get()).assume_init_ref() }
}

/// The memory map (valid after [`capture`]), sorted by address.
pub fn memory_map() -> &'static [MemoryRegion] {
    // SAFETY: as above.
    unsafe {
        let all: &'static [MemoryRegion; MAX_REGIONS] = &*STORE.map.get();
        &all[..*STORE.len.get()]
    }
}
