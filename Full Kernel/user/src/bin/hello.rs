//! The first user-space program: says hello, proving ring 3, the address
//! space, the ELF loader and the write system call all work.

#![no_std]
#![no_main]

use nova_user::{entry, getpid, println};

fn main() -> i64 {
    println!("Hello from user space! I am process {}.", getpid());
    0
}

entry!(main);
