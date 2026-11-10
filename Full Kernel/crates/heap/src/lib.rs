//! The kernel heap: a first-fit allocator over an address-ordered free list.
//!
//! Free blocks store their own size and next pointer inside the free memory,
//! so the allocator needs no memory of its own. Freed blocks are merged with
//! their neighbours, so alloc/free cycles do not fragment the heap forever.
//!
//! Every block (address and size) is a multiple of 16 bytes, which makes
//! splitting trivial: a leftover piece is either empty or big enough to hold
//! a free-list node. Memory is added with [`Heap::add_region`]; the kernel
//! calls it whenever it maps more pages for the heap.

#![no_std]

#[cfg(test)]
extern crate std;

use core::alloc::Layout;
use core::mem::size_of;
use core::ptr::{self, NonNull};

/// Granularity and minimum alignment of every allocation.
pub const GRANULE: usize = 16;

#[repr(C)]
struct FreeNode {
    size: usize,
    next: *mut FreeNode,
}

const _: () = assert!(size_of::<FreeNode>() <= GRANULE);

pub struct Heap {
    head: *mut FreeNode,
    free_bytes: usize,
    total_bytes: usize,
}

// SAFETY: the heap owns the memory its pointers refer to; moving it between
// threads is fine (callers serialise access with a lock).
unsafe impl Send for Heap {}

fn align_up(v: usize, a: usize) -> usize {
    (v + a - 1) & !(a - 1)
}

/// The size actually reserved for a request.
fn block_size(layout: Layout) -> usize {
    align_up(layout.size().max(1), GRANULE)
}

impl Heap {
    pub const fn new() -> Self {
        Self { head: ptr::null_mut(), free_bytes: 0, total_bytes: 0 }
    }

    pub fn free_bytes(&self) -> usize {
        self.free_bytes
    }

    /// Bytes ever added to the heap.
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Adds `[start, start + len)` to the heap (rounded inwards to 16 bytes).
    ///
    /// # Safety
    /// The range must be valid, writable memory that nothing else uses and
    /// that stays mapped for the life of the heap.
    pub unsafe fn add_region(&mut self, start: usize, len: usize) {
        let from = align_up(start, GRANULE);
        let to = (start + len) & !(GRANULE - 1);
        if to <= from || to - from < GRANULE {
            return;
        }
        self.total_bytes += to - from;
        // SAFETY: guaranteed by the caller.
        unsafe { self.insert(from, to - from) };
    }

    /// Inserts a block into the address-ordered list, merging neighbours.
    ///
    /// # Safety
    /// `addr..addr+size` must be unused heap memory, 16-byte aligned.
    unsafe fn insert(&mut self, addr: usize, size: usize) {
        self.free_bytes += size;
        let mut prev: *mut FreeNode = ptr::null_mut();
        let mut cur = self.head;
        // SAFETY: list nodes live in free heap memory that we own.
        unsafe {
            while !cur.is_null() && (cur as usize) < addr {
                prev = cur;
                cur = (*cur).next;
            }
            let node = addr as *mut FreeNode;
            let mut size = size;
            // Merge with the following block if adjacent.
            if !cur.is_null() && addr + size == cur as usize {
                size += (*cur).size;
                cur = (*cur).next;
            }
            // Merge with the preceding block if adjacent.
            if !prev.is_null() && prev as usize + (*prev).size == addr {
                (*prev).size += size;
                (*prev).next = cur;
                return;
            }
            node.write(FreeNode { size, next: cur });
            if prev.is_null() {
                self.head = node;
            } else {
                (*prev).next = node;
            }
        }
    }

    /// Allocates a block for `layout`, or `None` if no free block fits.
    pub fn alloc(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        let size = block_size(layout);
        let align = layout.align().max(GRANULE);
        let mut prev: *mut FreeNode = ptr::null_mut();
        let mut cur = self.head;
        // SAFETY: list nodes live in free heap memory that we own.
        unsafe {
            while !cur.is_null() {
                let start = cur as usize;
                let end = start + (*cur).size;
                let at = align_up(start, align);
                if at + size <= end {
                    let next = (*cur).next;
                    // Unlink the whole block, then give back the unused head and tail.
                    if prev.is_null() {
                        self.head = next;
                    } else {
                        (*prev).next = next;
                    }
                    self.free_bytes -= end - start;
                    if at > start {
                        self.insert(start, at - start);
                    }
                    if at + size < end {
                        self.insert(at + size, end - (at + size));
                    }
                    return NonNull::new(at as *mut u8);
                }
                prev = cur;
                cur = (*cur).next;
            }
        }
        None
    }

    /// Returns a block to the heap.
    ///
    /// # Safety
    /// `ptr` must have come from [`Heap::alloc`] on this heap with the same
    /// `layout`, and must not be used afterwards.
    pub unsafe fn dealloc(&mut self, ptr: NonNull<u8>, layout: Layout) {
        // SAFETY: guaranteed by the caller.
        unsafe { self.insert(ptr.as_ptr() as usize, block_size(layout)) };
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::boxed::Box;
    use std::vec::Vec;

    /// An aligned arena to use as heap memory.
    fn arena(bytes: usize) -> (Box<[u128]>, usize, usize) {
        let mut b = std::vec![0u128; bytes / 16].into_boxed_slice();
        let p = b.as_mut_ptr() as usize;
        (b, p, bytes)
    }

    fn heap_of(bytes: usize) -> (Heap, Box<[u128]>) {
        let (b, p, n) = arena(bytes);
        let mut h = Heap::new();
        unsafe { h.add_region(p, n) };
        (h, b)
    }

    fn l(size: usize, align: usize) -> Layout {
        Layout::from_size_align(size, align).unwrap()
    }

    #[test]
    fn allocations_are_aligned_and_do_not_overlap() {
        let (mut h, _keep) = heap_of(64 * 1024);
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        for (size, align) in [(1, 1), (24, 8), (100, 16), (64, 64), (4096, 4096), (7, 2), (500, 256)] {
            let p = h.alloc(l(size, align)).unwrap().as_ptr() as usize;
            assert_eq!(p % align.max(16), 0, "size {size} align {align}");
            for &(q, qs) in &blocks {
                assert!(p + block_size(l(size, align)) <= q || q + qs <= p, "overlap");
            }
            blocks.push((p, block_size(l(size, align))));
        }
    }

    #[test]
    fn exhaustion_returns_none_and_free_restores_everything() {
        let (mut h, _keep) = heap_of(4096);
        let total = h.free_bytes();
        let a = h.alloc(l(4096, 16)).unwrap();
        assert_eq!(h.free_bytes(), total - 4096);
        assert!(h.alloc(l(16, 16)).is_none());
        unsafe { h.dealloc(a, l(4096, 16)) };
        assert_eq!(h.free_bytes(), total);
        assert!(h.alloc(l(4096, 16)).is_some(), "the merged block is whole again");
    }

    #[test]
    fn neighbours_merge_whichever_order_they_are_freed() {
        for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2], [0, 2, 1]] {
            let (mut h, _keep) = heap_of(3 * 1024);
            let ps: Vec<_> = (0..3).map(|_| h.alloc(l(1024, 16)).unwrap()).collect();
            assert!(h.alloc(l(16, 16)).is_none());
            for i in order {
                unsafe { h.dealloc(ps[i], l(1024, 16)) };
            }
            assert!(h.alloc(l(3 * 1024, 16)).is_some(), "order {order:?} left the heap fragmented");
        }
    }

    #[test]
    fn alignment_gaps_are_returned_to_the_heap() {
        let (mut h, _keep) = heap_of(16 * 1024);
        let total = h.free_bytes();
        let a = h.alloc(l(8, 4096)).unwrap();
        assert_eq!(a.as_ptr() as usize % 4096, 0);
        assert_eq!(h.free_bytes(), total - 16, "only the 16-byte block is used; the gap went back");
        unsafe { h.dealloc(a, l(8, 4096)) };
        assert_eq!(h.free_bytes(), total);
    }

    #[test]
    fn regions_can_be_added_later() {
        let (mut h, _keep) = heap_of(1024);
        assert!(h.alloc(l(2048, 16)).is_none());
        let (b2, p2, n2) = arena(4096);
        unsafe { h.add_region(p2, n2) };
        assert_eq!(h.total_bytes(), 1024 + 4096);
        assert!(h.alloc(l(2048, 16)).is_some());
        drop(b2);
    }

    #[test]
    fn unaligned_regions_are_trimmed_inwards() {
        let (b, p, _) = arena(256);
        let mut h = Heap::new();
        unsafe { h.add_region(p + 3, 100) };
        assert_eq!(h.total_bytes(), 80, "from p+16 to p+96");
        assert_eq!(h.free_bytes(), 80);
        drop(b);
        let mut tiny = Heap::new();
        unsafe { tiny.add_region(8, 8) };
        assert_eq!(tiny.total_bytes(), 0);
    }

    /// Random alloc/free with every block filled with a recognisable pattern;
    /// corruption of a live block, overlap, or a leak all fail this.
    #[test]
    fn random_workout_keeps_data_intact_and_leaks_nothing() {
        let (mut h, _keep) = heap_of(256 * 1024);
        let total = h.free_bytes();
        let mut live: Vec<(NonNull<u8>, Layout, u8)> = Vec::new();
        let mut rng = 0x1234_5678_9ABC_DEF1u64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for step in 0..20_000u32 {
            if live.len() < 200 && next() % 5 < 3 {
                let size = (next() % 700 + 1) as usize;
                let align = 1usize << (next() % 8); // 1..=128
                let lay = l(size, align);
                if let Some(p) = h.alloc(lay) {
                    let tag = (step % 251) as u8 + 1;
                    unsafe { ptr::write_bytes(p.as_ptr(), tag, size) };
                    live.push((p, lay, tag));
                }
            } else if !live.is_empty() {
                let i = (next() % live.len() as u64) as usize;
                let (p, lay, tag) = live.swap_remove(i);
                let bytes = unsafe { core::slice::from_raw_parts(p.as_ptr(), lay.size()) };
                assert!(bytes.iter().all(|&b| b == tag), "block was corrupted");
                unsafe { h.dealloc(p, lay) };
            }
        }
        for (p, lay, tag) in live {
            let bytes = unsafe { core::slice::from_raw_parts(p.as_ptr(), lay.size()) };
            assert!(bytes.iter().all(|&b| b == tag));
            unsafe { h.dealloc(p, lay) };
        }
        assert_eq!(h.free_bytes(), total, "leak");
        assert!(h.alloc(l(total, 16)).is_some(), "everything merged back into one block");
    }
}
