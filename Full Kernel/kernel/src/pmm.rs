//! The kernel's physical memory manager: the global frame allocator.
//!
//! Wraps `nova_pmm::FrameAllocator` (bitmap, unit-tested on the host) with
//! the kernel-side parts: finding RAM for the bitmap, reading the boot memory
//! map, a lock, and a self-test that actually *touches* the memory.

use crate::log::SpinLock;
use core::sync::atomic::{AtomicU64, Ordering};
use nova_bootinfo::{BootInfo, MemoryRegion};
use nova_pmm::{FRAME_SIZE, FrameAllocator, FreeError, Stats};

static PMM: SpinLock<Option<FrameAllocator<'static>>> = SpinLock::new(None);
/// Virtual address of physical address 0 (the direct map), for touching frames.
static DIRECT_MAP: AtomicU64 = AtomicU64::new(0);

pub struct InitInfo {
    pub stats: Stats,
    /// Frames freed from `Usable` + reclaimed firmware memory.
    pub bitmap_phys: u64,
    pub bitmap_bytes: u64,
}

/// Builds the allocator from the boot memory map.
///
/// # Safety
/// Call once, with `map` being the real memory map and the direct map in
/// place. Everything the allocator later hands out must really be free RAM.
pub unsafe fn init(info: &BootInfo, map: &[MemoryRegion]) -> Result<InitInfo, &'static str> {
    let frames = nova_pmm::tracked_frames(map);
    if frames == 0 {
        return Err("memory map has no usable memory");
    }
    let words = nova_pmm::bitmap_words(frames);
    let bytes = (words * 8) as u64;
    let home = nova_pmm::find_home(map, bytes, info.physical_memory_mapped)
        .ok_or("no usable memory (inside the direct map) to hold the frame bitmap")?;

    DIRECT_MAP.store(info.physical_memory_offset, Ordering::SeqCst);
    // SAFETY: `home..home+bytes` is usable RAM inside the direct map that
    // nothing else owns; we reserve it below so it is never handed out.
    let bitmap = unsafe { core::slice::from_raw_parts_mut(info.phys_to_virt(home) as *mut u64, words) };
    let mut alloc = FrameAllocator::new(bitmap, frames);
    nova_pmm::populate(&mut alloc, map);
    alloc.reserve_range(home, home + bytes);
    let stats = alloc.stats();
    PMM.with(|p| *p = Some(alloc));
    Ok(InitInfo { stats, bitmap_phys: home, bitmap_bytes: bytes })
}

pub fn stats() -> Option<Stats> {
    PMM.with(|p| p.as_ref().map(|a| a.stats()))
}

/// Allocates one 4 KiB frame (physical address). Contents are unspecified.
pub fn alloc_frame() -> Option<u64> {
    PMM.with(|p| p.as_mut()?.alloc())
}

/// Allocates `count` contiguous frames aligned to `align_frames` (a power of two).
pub fn alloc_contiguous(count: usize, align_frames: usize) -> Option<u64> {
    PMM.with(|p| p.as_mut()?.alloc_contiguous(count, align_frames))
}

pub fn free_frame(addr: u64) -> Result<(), FreeError> {
    PMM.with(|p| p.as_mut().ok_or(FreeError::OutOfRange)?.free(addr))
}

pub fn free_contiguous(addr: u64, count: usize) -> Result<(), FreeError> {
    PMM.with(|p| p.as_mut().ok_or(FreeError::OutOfRange)?.free_contiguous(addr, count))
}

/// Direct-map pointer to the frame at physical address `phys`.
fn frame_ptr(phys: u64) -> *mut u64 {
    (DIRECT_MAP.load(Ordering::SeqCst) + phys) as *mut u64
}

const WORDS: usize = (FRAME_SIZE / 8) as usize;
/// The self-test stops after this many frames (512 MiB) so that booting with
/// many GiB of RAM stays quick.
const SELFTEST_MAX_FRAMES: usize = 128 * 1024;

pub struct SelfTest {
    pub frames_touched: usize,
    /// True if the test ran until memory was exhausted (rather than hitting the cap).
    pub exhausted: bool,
}

/// Checks the allocator against real memory.
///
/// 1. API behaviour: distinct frames, double-free refused, contiguous runs.
/// 2. Allocates frames (up to the cap), writes a pattern derived from each
///    frame's own address, then re-reads every one. Any frame that belonged
///    to something else (the bootloader's tables, the boot info, our stack,
///    firmware memory still in use) would be corrupted, or would corrupt that
///    other thing and crash the kernel shortly after.
/// 3. Frees everything and checks the free count is exactly what it was.
///
/// Frames are chained into a list through their first word, so the test
/// needs no memory of its own.
pub fn self_test() -> Result<SelfTest, &'static str> {
    let before = stats().ok_or("pmm not initialised")?;

    // 1. API behaviour.
    let a = alloc_frame().ok_or("alloc failed")?;
    let b = alloc_frame().ok_or("alloc failed")?;
    if a == b || a % FRAME_SIZE != 0 || b % FRAME_SIZE != 0 {
        return Err("alloc returned a duplicate or unaligned frame");
    }
    if a < nova_pmm::LOW_MEMORY_LIMIT {
        return Err("alloc returned memory below 1 MiB");
    }
    free_frame(a).map_err(|_| "free of a valid frame failed")?;
    if free_frame(a) != Err(FreeError::NotAllocated) {
        return Err("double free was not refused");
    }
    free_frame(b).map_err(|_| "free of a valid frame failed")?;
    let run = alloc_contiguous(16, 16).ok_or("contiguous alloc of 16 frames failed")?;
    if run % (16 * FRAME_SIZE) != 0 {
        return Err("contiguous run is not aligned");
    }
    free_contiguous(run, 16).map_err(|_| "free of a contiguous run failed")?;
    if stats() != Some(before) {
        return Err("free count drifted during the API checks");
    }

    // 2. Fill real memory with address-derived patterns.
    let mut head = 0u64; // physical address of the most recent frame; 0 = end of list
    let mut touched = 0usize;
    let mut exhausted = false;
    while touched < SELFTEST_MAX_FRAMES {
        let Some(f) = alloc_frame() else {
            exhausted = true;
            break;
        };
        let p = frame_ptr(f);
        // SAFETY: `f` is a frame we own, inside the direct map.
        unsafe {
            p.write_volatile(head);
            for i in 1..WORDS {
                p.add(i).write_volatile(pattern(f, i));
            }
        }
        head = f;
        touched += 1;
    }
    if exhausted && stats().map(|s| s.free_frames) != Some(0) {
        return Err("alloc failed but the allocator reports free frames");
    }

    // Verify and free, newest first.
    let mut cursor = head;
    let mut freed = 0usize;
    while cursor != 0 {
        let p = frame_ptr(cursor);
        // SAFETY: still our frame, nothing else wrote to it.
        let next = unsafe {
            for i in 1..WORDS {
                if p.add(i).read_volatile() != pattern(cursor, i) {
                    return Err("a frame handed out by the allocator was overwritten (it is in use by something else)");
                }
            }
            p.read_volatile()
        };
        free_frame(cursor).map_err(|_| "free failed while releasing the test frames")?;
        cursor = next;
        freed += 1;
    }
    if freed != touched {
        return Err("the frame list was corrupted");
    }

    // 3. Nothing leaked.
    if stats() != Some(before) {
        return Err("free count differs after the memory test");
    }
    Ok(SelfTest { frames_touched: touched, exhausted })
}

fn pattern(frame: u64, word: usize) -> u64 {
    (frame ^ 0x9E37_79B9_7F4A_7C15).wrapping_add((word as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93))
}
