//! The kernel heap: `Box`, `Vec`, `String` and friends.
//!
//! The allocator itself is `nova_heap::Heap` (host-tested). This file adds
//! the kernel parts: a lock, the global-allocator hook, and *growth*: when
//! the heap runs out, it maps more frames at the next free address of its
//! virtual range (a fixed window starting at [`HEAP_BASE`]) and carries on.
//!
//! Lock order is always heap -> vm -> pmm, and none of those allocate from
//! the heap, so growing from inside `alloc` cannot recurse.

use crate::log::SpinLock;
use crate::{pmm, vm};
use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use nova_heap::Heap;
use nova_paging::{NO_EXECUTE, PAGE_SIZE, WRITABLE};

/// Start of the heap's virtual window.
pub const HEAP_BASE: u64 = 0xFFFF_A000_0000_0000;
/// The heap may grow to this size.
pub const HEAP_MAX: u64 = 1 << 30;
/// Heap growth happens in chunks of at least this size.
const GROW_CHUNK: u64 = 1 << 20;

static HEAP: SpinLock<Heap> = SpinLock::new(Heap::new());
/// First virtual address not yet mapped for the heap.
static HEAP_END: AtomicUsize = AtomicUsize::new(HEAP_BASE as usize);

pub struct KernelHeap;

#[global_allocator]
static GLOBAL: KernelHeap = KernelHeap;

/// Maps at least `min_bytes` more heap pages and hands them to `heap`.
/// Returns false if it could not grow (out of frames or out of window).
fn grow(heap: &mut Heap, min_bytes: u64) -> bool {
    let start = HEAP_END.load(Ordering::SeqCst) as u64;
    let want = min_bytes.max(GROW_CHUNK).next_multiple_of(PAGE_SIZE);
    let want = want.min(HEAP_BASE + HEAP_MAX - start);
    let mut mapped = 0;
    while mapped < want {
        let Some(frame) = pmm::alloc_frame() else { break };
        if vm::map_page(start + mapped, frame, WRITABLE | NO_EXECUTE).is_err() {
            let _ = pmm::free_frame(frame);
            break;
        }
        mapped += PAGE_SIZE;
    }
    if mapped == 0 {
        return false;
    }
    HEAP_END.store((start + mapped) as usize, Ordering::SeqCst);
    // SAFETY: freshly mapped, writable, unused pages that stay mapped for good.
    unsafe { heap.add_region(start as usize, mapped as usize) };
    true
}

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HEAP.with(|heap| {
            if let Some(p) = heap.alloc(layout) {
                return p.as_ptr();
            }
            // Out of room: grow (enough for this request plus alignment slack) and retry.
            let need = layout.size() as u64 + layout.align() as u64 + 64;
            if grow(heap, need)
                && let Some(p) = heap.alloc(layout)
            {
                return p.as_ptr();
            }
            ptr::null_mut()
        })
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if let Some(p) = ptr::NonNull::new(p) {
            // SAFETY: GlobalAlloc contract: `p` came from `alloc` with this layout.
            HEAP.with(|heap| unsafe { heap.dealloc(p, layout) });
        }
    }
}

/// (bytes ever mapped for the heap, bytes currently free).
pub fn stats() -> (usize, usize) {
    HEAP.with(|h| (h.total_bytes(), h.free_bytes()))
}

/// Maps the first chunk of heap so the first allocation is cheap.
pub fn init() -> Result<(), &'static str> {
    if HEAP.with(|h| grow(h, GROW_CHUNK)) { Ok(()) } else { Err("could not map the first heap pages") }
}

/// Exercises the heap with the standard collections.
pub fn self_test() -> Result<(), &'static str> {
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;

    let (total_before, free_before) = stats();
    let used_before = total_before - free_before;
    {
        // Box and a trait object.
        let b: Box<dyn Fn(u32) -> u32> = Box::new(|x| x * 3 + 1);
        if b(13) != 40 {
            return Err("boxed closure gave the wrong answer");
        }

        // A growing Vec (more than the first chunk, so the heap must grow).
        let mut v: Vec<u64> = Vec::new();
        for i in 0..300_000u64 {
            v.push(i * i);
        }
        if v.iter().enumerate().any(|(i, &x)| x != (i as u64) * (i as u64)) {
            return Err("Vec contents were corrupted");
        }

        // Many small allocations of different sizes, freed in a scrambled order.
        let mut parts: Vec<String> = Vec::new();
        for i in 0..2_000 {
            parts.push(alloc::format!("nova-{i}-{}", "x".repeat(i % 97)));
        }
        let mut i = 0;
        while !parts.is_empty() {
            i = (i * 7 + 3) % parts.len();
            let s = parts.swap_remove(i);
            if !s.starts_with("nova-") {
                return Err("a String was corrupted");
            }
        }

        // Over-aligned allocation.
        #[repr(align(4096))]
        struct Page([u8; 4096]);
        let p = Box::new(Page([7; 4096]));
        if !(&*p as *const Page as usize).is_multiple_of(4096) || p.0[4095] != 7 {
            return Err("over-aligned Box is misaligned");
        }
    }
    let (total_after, free_after) = stats();
    if total_after - free_after != used_before {
        return Err("heap leaked: bytes in use differ after dropping everything");
    }
    if total_after <= total_before {
        return Err("the heap never had to grow (test is not exercising growth)");
    }
    Ok(())
}
