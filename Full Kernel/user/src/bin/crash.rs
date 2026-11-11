//! Writes to address 0. The kernel must kill this process (and only this one).

#![no_std]
#![no_main]

use nova_user::{entry, println};

fn main() -> i64 {
    println!("about to write to address 0...");
    // SAFETY: deliberately invalid.
    unsafe { (core::ptr::null_mut::<u8>()).write_volatile(1) };
    println!("this line must never appear");
    0
}

entry!(main);
