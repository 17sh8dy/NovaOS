//! Misbehaves in ways the kernel must survive: bad pointers to system calls,
//! an unknown system call, then a privileged instruction.

#![no_std]
#![no_main]

use core::arch::asm;
use nova_user::{entry, println, syscall3};

fn main() -> i64 {
    // A pointer into kernel memory.
    let r = unsafe { syscall3(nova_user::nr::WRITE, 1, 0xFFFF_8000_0000_0000, 16) };
    println!("write from a kernel address returned {r}");
    // A length that wraps around the address space.
    let r = unsafe { syscall3(nova_user::nr::WRITE, 1, u64::MAX - 4, 100) };
    println!("write with a wrapping range returned {r}");
    // An address that is simply not mapped.
    let r = unsafe { syscall3(nova_user::nr::WRITE, 1, 0x1000, 8) };
    println!("write from an unmapped address returned {r}");
    // A system call that does not exist.
    let r = unsafe { syscall3(9999, 0, 0, 0) };
    println!("unknown system call returned {r}");
    println!("now trying a privileged instruction (cli)...");
    // SAFETY: deliberately illegal in ring 3: the kernel must kill us.
    unsafe { asm!("cli") };
    println!("this line must never appear");
    0
}

entry!(main);
