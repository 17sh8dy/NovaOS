//! Grows its heap with sbrk, fills it with a pattern derived from its process
//! id, and checks the pattern while other processes do the same: proof that
//! every process has its own memory.

#![no_std]
#![no_main]

use nova_user::{entry, errno, getpid, println, sbrk, sleep_ms, yield_now};

const SIZE: usize = 256 * 1024;

fn main() -> i64 {
    let pid = getpid();
    let base = sbrk(SIZE as u64);
    if base < 0 {
        println!("[{pid}] sbrk failed: {}", -base);
        return 1;
    }
    // SAFETY: sbrk just mapped SIZE zeroed bytes at `base` for us.
    let mem = unsafe { core::slice::from_raw_parts_mut(base as *mut u8, SIZE) };
    if mem.iter().any(|&b| b != 0) {
        println!("[{pid}] new memory was not zeroed");
        return 1;
    }
    let tag = (pid as u8).wrapping_mul(37).wrapping_add(1);
    for (i, b) in mem.iter_mut().enumerate() {
        *b = tag ^ (i as u8);
    }
    // Let the other processes run and write their own patterns.
    for _ in 0..5 {
        sleep_ms(20);
        yield_now();
    }
    if mem.iter().enumerate().any(|(i, &b)| b != tag ^ (i as u8)) {
        println!("[{pid}] memory was overwritten by someone else!");
        return 1;
    }
    // A huge request must fail cleanly rather than crash anything.
    if sbrk(1 << 40) != -errno::ENOMEM {
        println!("[{pid}] absurd sbrk did not fail with ENOMEM");
        return 1;
    }
    println!("[{pid}] memtest ok: {} KiB private and intact", SIZE / 1024);
    0
}

entry!(main);
