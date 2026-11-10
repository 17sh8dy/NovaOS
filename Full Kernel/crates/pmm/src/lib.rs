//! The physical memory manager: a bitmap allocator for 4 KiB page frames.
//!
//! One bit per frame, 1 = in use. Everything starts "in use"; the boot memory
//! map then *releases* the ranges that are really free RAM, so anything the
//! map doesn't mention (holes, MMIO, firmware) can never be handed out.
//!
//! The bitmap itself is plain memory supplied by the caller, so the same code
//! runs in the kernel (bitmap in the direct map) and in host unit tests
//! (a `Vec`). Frames are returned as *physical addresses*; the allocator never
//! touches the frames themselves.

#![no_std]

#[cfg(test)]
extern crate std;

use nova_bootinfo::{MemoryKind, MemoryRegion};

pub const FRAME_SIZE: u64 = 4096;
/// Memory below 1 MiB (BIOS data, option ROMs, the future SMP start-up code)
/// is never given out by [`populate`].
pub const LOW_MEMORY_LIMIT: u64 = 1 << 20;

const BITS: usize = 64;

/// Why a free was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeError {
    /// The address is not a multiple of the frame size.
    Unaligned,
    /// The address is beyond the memory this allocator tracks.
    OutOfRange,
    /// The frame is already free (a double free, or it never came from here).
    NotAllocated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    /// Frames the allocator tracks (everything below the highest usable address).
    pub tracked_frames: usize,
    pub free_frames: usize,
}

impl Stats {
    pub fn free_bytes(&self) -> u64 {
        self.free_frames as u64 * FRAME_SIZE
    }
}

pub struct FrameAllocator<'a> {
    bitmap: &'a mut [u64],
    frames: usize,
    free: usize,
    /// Word index where the next search starts (next-fit).
    hint: usize,
}

/// How many `u64` words of bitmap are needed to track `frames` frames.
pub const fn bitmap_words(frames: usize) -> usize {
    frames.div_ceil(BITS)
}

impl<'a> FrameAllocator<'a> {
    /// Creates an allocator with every frame in use. `bitmap` must hold at
    /// least [`bitmap_words`]`(frames)` words; its contents are overwritten.
    pub fn new(bitmap: &'a mut [u64], frames: usize) -> Self {
        assert!(bitmap.len() >= bitmap_words(frames), "bitmap too small");
        let bitmap = &mut bitmap[..bitmap_words(frames)];
        bitmap.fill(!0);
        Self { bitmap, frames, free: 0, hint: 0 }
    }

    pub fn stats(&self) -> Stats {
        Stats { tracked_frames: self.frames, free_frames: self.free }
    }

    fn is_used(&self, frame: usize) -> bool {
        self.bitmap[frame / BITS] & (1 << (frame % BITS)) != 0
    }

    fn set_free(&mut self, frame: usize) {
        self.bitmap[frame / BITS] &= !(1 << (frame % BITS));
        self.free += 1;
        self.hint = self.hint.min(frame / BITS);
    }

    fn set_used(&mut self, frame: usize) {
        self.bitmap[frame / BITS] |= 1 << (frame % BITS);
        self.free -= 1;
    }

    /// True if the frame containing `addr` is tracked and free.
    pub fn is_free(&self, addr: u64) -> bool {
        let f = (addr / FRAME_SIZE) as usize;
        f < self.frames && !self.is_used(f)
    }

    /// Marks every frame that lies *entirely inside* `start..end` as free.
    /// Partial frames at the edges stay as they were. Returns how many
    /// frames changed from used to free.
    pub fn release_range(&mut self, start: u64, end: u64) -> usize {
        let first = start.div_ceil(FRAME_SIZE) as usize;
        let last = ((end / FRAME_SIZE) as usize).min(self.frames);
        let mut n = 0;
        for f in first..last {
            if self.is_used(f) {
                self.set_free(f);
                n += 1;
            }
        }
        n
    }

    /// Marks every frame that *overlaps* `start..end` as used. Returns how
    /// many frames changed from free to used.
    pub fn reserve_range(&mut self, start: u64, end: u64) -> usize {
        let first = (start / FRAME_SIZE) as usize;
        let last = (end.div_ceil(FRAME_SIZE) as usize).min(self.frames);
        let mut n = 0;
        for f in first..last {
            if !self.is_used(f) {
                self.set_used(f);
                n += 1;
            }
        }
        n
    }

    /// Allocates one frame; returns its physical address. The contents are
    /// whatever was there before.
    pub fn alloc(&mut self) -> Option<u64> {
        if self.free == 0 {
            return None;
        }
        let words = self.bitmap.len();
        for step in 0..words {
            let w = (self.hint + step) % words;
            let word = self.bitmap[w];
            if word != !0 {
                let frame = w * BITS + (!word).trailing_zeros() as usize;
                // Padding bits past `frames` are always set, so this is in range.
                debug_assert!(frame < self.frames);
                self.set_used(frame);
                self.hint = w;
                return Some(frame as u64 * FRAME_SIZE);
            }
        }
        None
    }

    /// Allocates `count` physically contiguous frames whose first frame is a
    /// multiple of `align_frames` (a power of two; 1 = no constraint).
    pub fn alloc_contiguous(&mut self, count: usize, align_frames: usize) -> Option<u64> {
        if count == 0 || !align_frames.is_power_of_two() || self.free < count {
            return None;
        }
        let mut start = 0;
        while start + count <= self.frames {
            match (start..start + count).find(|&f| self.is_used(f)) {
                None => {
                    for f in start..start + count {
                        self.set_used(f);
                    }
                    return Some(start as u64 * FRAME_SIZE);
                }
                // Jump past the used frame, back onto the alignment grid.
                Some(used) => start = (used + 1).next_multiple_of(align_frames),
            }
        }
        None
    }

    /// Returns one frame to the allocator.
    pub fn free(&mut self, addr: u64) -> Result<(), FreeError> {
        if !addr.is_multiple_of(FRAME_SIZE) {
            return Err(FreeError::Unaligned);
        }
        let f = (addr / FRAME_SIZE) as usize;
        if f >= self.frames {
            return Err(FreeError::OutOfRange);
        }
        if !self.is_used(f) {
            return Err(FreeError::NotAllocated);
        }
        self.set_free(f);
        Ok(())
    }

    /// Returns `count` contiguous frames (all or nothing: nothing changes on error).
    pub fn free_contiguous(&mut self, addr: u64, count: usize) -> Result<(), FreeError> {
        if !addr.is_multiple_of(FRAME_SIZE) {
            return Err(FreeError::Unaligned);
        }
        let first = (addr / FRAME_SIZE) as usize;
        if first.checked_add(count).is_none_or(|end| end > self.frames) {
            return Err(FreeError::OutOfRange);
        }
        if (first..first + count).any(|f| !self.is_used(f)) {
            return Err(FreeError::NotAllocated);
        }
        for f in first..first + count {
            self.set_free(f);
        }
        Ok(())
    }
}

/// True for the memory kinds the allocator may hand out once the kernel owns
/// its CPU tables: free RAM and the firmware's boot-time scratch memory.
/// (ACPI tables stay reserved until the kernel has parsed them; `Loader`
/// memory holds the page tables, boot info and kernel stack in use right now.)
pub fn is_allocatable(kind: MemoryKind) -> bool {
    matches!(kind, MemoryKind::Usable | MemoryKind::FirmwareReclaimable)
}

/// How many frames the allocator must track for this memory map: everything
/// up to the end of the highest allocatable region.
pub fn tracked_frames(map: &[MemoryRegion]) -> usize {
    let top = map.iter().filter(|r| is_allocatable(r.kind())).map(|r| r.end()).max().unwrap_or(0);
    (top / FRAME_SIZE) as usize
}

/// Finds a physical home for `bytes` of bookkeeping: the first *usable*
/// region above 1 MiB that fits it and lies inside the mapped memory
/// (`mapped_limit` = how much physical memory the direct map covers).
/// Returns a frame-aligned physical address.
pub fn find_home(map: &[MemoryRegion], bytes: u64, mapped_limit: u64) -> Option<u64> {
    map.iter().filter(|r| r.kind() == MemoryKind::Usable).find_map(|r| {
        let start = r.start.max(LOW_MEMORY_LIMIT).next_multiple_of(FRAME_SIZE);
        let end = r.end().min(mapped_limit);
        (start.checked_add(bytes)? <= end).then_some(start)
    })
}

/// Frees every allocatable region of the map (see [`is_allocatable`]) except
/// memory below [`LOW_MEMORY_LIMIT`]. Returns the frames freed.
pub fn populate(alloc: &mut FrameAllocator, map: &[MemoryRegion]) -> usize {
    let mut n = 0;
    for r in map.iter().filter(|r| is_allocatable(r.kind())) {
        n += alloc.release_range(r.start.max(LOW_MEMORY_LIMIT), r.end());
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    const MIB: u64 = 1 << 20;

    fn region(start: u64, len: u64, kind: MemoryKind) -> MemoryRegion {
        MemoryRegion::new(start, len, kind)
    }

    /// A 16 MiB test machine: low memory, usable RAM, a hole, and a reserved block.
    fn map() -> Vec<MemoryRegion> {
        vec![
            region(0, 640 * 1024, MemoryKind::Usable),         // below 1 MiB: never allocated
            region(MIB, 7 * MIB, MemoryKind::Usable),          // 1..8 MiB
            region(8 * MIB, MIB, MemoryKind::Loader),          // 8..9 MiB in use
            region(9 * MIB, 2 * MIB, MemoryKind::FirmwareReclaimable), // 9..11 MiB
            region(11 * MIB, MIB, MemoryKind::AcpiNvs),        // 11..12 MiB never
            region(14 * MIB, 2 * MIB, MemoryKind::Usable),     // 14..16 MiB (hole 12..14)
        ]
    }

    fn make(bitmap: &mut Vec<u64>) -> FrameAllocator<'_> {
        let m = map();
        let frames = tracked_frames(&m);
        *bitmap = vec![0; bitmap_words(frames)];
        let mut a = FrameAllocator::new(bitmap, frames);
        populate(&mut a, &m);
        a
    }

    #[test]
    fn tracks_up_to_the_highest_allocatable_address() {
        assert_eq!(tracked_frames(&map()), (16 * MIB / FRAME_SIZE) as usize);
        assert_eq!(tracked_frames(&[]), 0);
        // A reserved region above the RAM does not make us track more.
        let mut m = map();
        m.push(region(1 << 40, MIB, MemoryKind::Reserved));
        assert_eq!(tracked_frames(&m), (16 * MIB / FRAME_SIZE) as usize);
    }

    #[test]
    fn populate_frees_only_allocatable_memory_above_1_mib() {
        let mut b = Vec::new();
        let a = make(&mut b);
        // 7 MiB + 2 MiB + 2 MiB
        assert_eq!(a.stats().free_bytes(), 11 * MIB);
        assert!(!a.is_free(0));
        assert!(!a.is_free(512 * 1024), "low memory stays reserved");
        assert!(a.is_free(MIB));
        assert!(!a.is_free(8 * MIB), "loader memory stays reserved");
        assert!(a.is_free(9 * MIB));
        assert!(!a.is_free(11 * MIB), "ACPI NVS stays reserved");
        assert!(!a.is_free(13 * MIB), "holes stay reserved");
        assert!(a.is_free(15 * MIB));
        assert!(!a.is_free(100 * MIB), "beyond the tracked range");
    }

    #[test]
    fn partial_frames_at_region_edges_are_not_released() {
        let mut bits = vec![0u64; 1];
        let mut a = FrameAllocator::new(&mut bits, 8);
        // 0x1800..0x4800 fully contains frames 2 and 3 only (0x2000..0x4000).
        assert_eq!(a.release_range(0x1800, 0x4800), 2);
        assert!(!a.is_free(0x1000) && a.is_free(0x2000) && a.is_free(0x3000) && !a.is_free(0x4000));
    }

    #[test]
    fn reserve_range_rounds_outwards() {
        let mut bits = vec![0u64; 1];
        let mut a = FrameAllocator::new(&mut bits, 8);
        a.release_range(0, 8 * FRAME_SIZE);
        assert_eq!(a.reserve_range(0x1800, 0x2800), 2, "touches frames 1 and 2");
        assert!(a.is_free(0) && !a.is_free(0x1000) && !a.is_free(0x2000) && a.is_free(0x3000));
    }

    #[test]
    fn alloc_returns_distinct_frames_from_free_memory_only() {
        let mut b = Vec::new();
        let mut a = make(&mut b);
        let total = a.stats().free_frames;
        let mut seen = Vec::new();
        for _ in 0..total {
            let f = a.alloc().expect("memory left");
            assert_eq!(f % FRAME_SIZE, 0);
            assert!(f >= MIB, "never below 1 MiB");
            assert!(!(8 * MIB..9 * MIB).contains(&f), "never loader memory");
            assert!(!(11 * MIB..14 * MIB).contains(&f), "never ACPI or the hole");
            seen.push(f);
        }
        assert_eq!(a.alloc(), None, "exhausted");
        assert_eq!(a.stats().free_frames, 0);
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), total, "every frame handed out exactly once");
    }

    #[test]
    fn free_makes_a_frame_available_again() {
        let mut b = Vec::new();
        let mut a = make(&mut b);
        let before = a.stats().free_frames;
        let f = a.alloc().unwrap();
        assert_eq!(a.stats().free_frames, before - 1);
        assert_eq!(a.free(f), Ok(()));
        assert_eq!(a.stats().free_frames, before);
        assert_eq!(a.alloc(), Some(f), "the lowest free frame is found again");
    }

    #[test]
    fn free_rejects_bad_addresses_and_double_frees() {
        let mut b = Vec::new();
        let mut a = make(&mut b);
        let f = a.alloc().unwrap();
        assert_eq!(a.free(f + 8), Err(FreeError::Unaligned));
        assert_eq!(a.free(64 * MIB), Err(FreeError::OutOfRange));
        assert_eq!(a.free(f), Ok(()));
        assert_eq!(a.free(f), Err(FreeError::NotAllocated), "double free");
        let before = a.stats().free_frames;
        assert_eq!(a.free(f), Err(FreeError::NotAllocated));
        assert_eq!(a.stats().free_frames, before, "a refused free changes nothing");
    }

    #[test]
    fn free_hint_moves_back_so_low_frames_are_reused() {
        let mut b = Vec::new();
        let mut a = make(&mut b);
        let first = a.alloc().unwrap();
        for _ in 0..1000 {
            a.alloc().unwrap();
        }
        a.free(first).unwrap();
        assert_eq!(a.alloc(), Some(first));
    }

    #[test]
    fn contiguous_alloc_respects_alignment_and_skips_used_frames() {
        let mut b = Vec::new();
        let mut a = make(&mut b);
        let one = a.alloc().unwrap(); // occupies the first frame at 1 MiB
        assert_eq!(one, MIB);
        let run = a.alloc_contiguous(16, 16).unwrap();
        assert_eq!(run % (16 * FRAME_SIZE), 0, "aligned to 16 frames");
        assert!(run > one);
        for i in 0..16 {
            assert!(!a.is_free(run + i * FRAME_SIZE));
        }
        assert_eq!(a.free_contiguous(run, 16), Ok(()));
        assert!(a.is_free(run));
    }

    #[test]
    fn contiguous_alloc_never_spans_a_hole() {
        let mut b = Vec::new();
        let mut a = make(&mut b);
        // The 1..8 MiB region is 7 MiB = 1792 frames; 2048 cannot fit anywhere.
        assert_eq!(a.alloc_contiguous(2048, 1), None);
        let big = a.alloc_contiguous(1792, 1).unwrap();
        assert_eq!(big, MIB);
        assert_eq!(a.alloc_contiguous(0, 1), None);
        assert_eq!(a.alloc_contiguous(1, 3), None, "alignment must be a power of two");
    }

    #[test]
    fn free_contiguous_is_all_or_nothing() {
        let mut b = Vec::new();
        let mut a = make(&mut b);
        let run = a.alloc_contiguous(4, 1).unwrap();
        a.free(run + FRAME_SIZE).unwrap(); // free one in the middle
        let free_before = a.stats().free_frames;
        assert_eq!(a.free_contiguous(run, 4), Err(FreeError::NotAllocated));
        assert_eq!(a.stats().free_frames, free_before, "nothing was freed");
    }

    #[test]
    fn find_home_picks_usable_memory_above_1_mib_inside_the_mapping() {
        let m = map();
        assert_eq!(find_home(&m, 4096, 16 * MIB), Some(MIB));
        // Nothing is big enough for 7 MiB + 1 byte.
        assert_eq!(find_home(&m, 7 * MIB + 1, 16 * MIB), None, "no usable region is that large");
        assert_eq!(find_home(&m, 2 * MIB, 16 * MIB), Some(MIB));
        // Not mapped far enough: the last region is out of reach.
        assert_eq!(find_home(&[region(14 * MIB, 2 * MIB, MemoryKind::Usable)], 4096, 8 * MIB), None);
        // Region that starts below 1 MiB is clipped.
        assert_eq!(find_home(&[region(0, 2 * MIB, MemoryKind::Usable)], 4096, 16 * MIB), Some(MIB));
        // Only genuinely usable memory qualifies.
        assert_eq!(find_home(&[region(2 * MIB, MIB, MemoryKind::FirmwareReclaimable)], 4096, 16 * MIB), None);
    }

    #[test]
    fn the_bitmap_home_can_be_reserved_from_the_allocator() {
        let mut b = Vec::new();
        let mut a = make(&mut b);
        let before = a.stats().free_frames;
        let home = find_home(&map(), 3 * FRAME_SIZE, 16 * MIB).unwrap();
        assert_eq!(a.reserve_range(home, home + 3 * FRAME_SIZE), 3);
        assert_eq!(a.stats().free_frames, before - 3);
        assert_ne!(a.alloc().unwrap(), home);
    }

    #[test]
    fn padding_bits_past_the_last_frame_are_never_allocated() {
        // 70 frames use 2 words; bits 70..128 are padding.
        let mut bits = vec![0u64; 2];
        let mut a = FrameAllocator::new(&mut bits, 70);
        a.release_range(0, 70 * FRAME_SIZE);
        for _ in 0..70 {
            assert!(a.alloc().unwrap() < 70 * FRAME_SIZE);
        }
        assert_eq!(a.alloc(), None);
    }

    #[test]
    fn bitmap_size_rounds_up() {
        assert_eq!(bitmap_words(0), 0);
        assert_eq!(bitmap_words(1), 1);
        assert_eq!(bitmap_words(64), 1);
        assert_eq!(bitmap_words(65), 2);
    }
}
