//! Deliberate faults, for testing the exception machinery.
//!
//! A kernel built with the environment variable `NOVA_FAULT=<kind>` triggers
//! that fault right after the boot screen is drawn; a normal build contains
//! none of this (the match on `None` folds away). tools\build.ps1 -Fault <kind>
//! builds one, and tools\test.ps1 builds and boots every kind.
//!
//!   div    divide by zero                                  -> #DE
//!   ud     undefined instruction                           -> #UD
//!   gp     memory access at a non-canonical address        -> #GP
//!   pf     read of an unmapped address                     -> #PF (+ CR2)
//!   df     stack pointer zeroed, then a trap               -> #DF on IST 1
//!   stack  unbounded recursion into the stack guard page   -> #DF on IST 1
//!   nmi    software `int 2`                                -> NMI on IST 2
//!   panic  a Rust panic                                    -> panic screen
//!   null   read of address 0                               -> #PF (the low half is unmapped)
//!   lowmem read of 1 MiB, which the bootloader identity-mapped -> #PF (that map is gone)
//!   ro     write to the kernel's own code                  -> #PF, protection violation (W^X)
//!   nx     jump into the heap                              -> #PF, instruction fetch (NX)

use core::arch::asm;

pub const KINDS: [&str; 12] = ["div", "ud", "gp", "pf", "df", "stack", "nmi", "panic", "null", "lowmem", "ro", "nx"];

unsafe extern "C" {
    static __text_start: u8;
}

pub fn run_if_requested() {
    let Some(kind) = option_env!("NOVA_FAULT") else { return };
    crate::kprintln!("fault-test: deliberately triggering '{kind}'");
    // SAFETY: every arm is a deliberate fault that the kernel must report.
    unsafe {
        match kind {
            "div" => asm!(
                "div {z}",
                z = in(reg) 0u64,
                in("rax") 1u64,
                in("rdx") 0u64,
                lateout("rax") _,
                lateout("rdx") _,
            ),
            "ud" => asm!("ud2"),
            "gp" => asm!(
                "mov {t}, [{a}]",
                a = in(reg) 0x8000_0000_0000_0000u64,
                t = out(reg) _,
            ),
            "pf" => {
                let p = 0xFFFF_C000_0000_0000u64 as *const u8;
                core::hint::black_box(p.read_volatile());
            }
            "df" => asm!("mov rsp, 0", "int3", options(noreturn)),
            "stack" => {
                core::hint::black_box(recurse(0));
            }
            "nmi" => asm!("int 2"),
            "null" => {
                core::hint::black_box(core::ptr::null::<u8>().read_volatile());
            }
            "lowmem" => {
                core::hint::black_box((0x10_0000usize as *const u8).read_volatile());
            }
            "ro" => (&raw const __text_start as *mut u8).write_volatile(0xCC),
            "nx" => {
                // A `ret` instruction in heap memory: the heap is mapped no-execute.
                let code = alloc::boxed::Box::new([0xC3u8; 16]);
                let f: extern "C" fn() = core::mem::transmute(code.as_ptr());
                f();
            }
            "panic" => panic!("deliberate panic from NOVA_FAULT=panic (value {})", 42),
            other => crate::kprintln!("fault-test: unknown kind '{other}' (known: {KINDS:?})"),
        }
    }
    crate::kprintln!("fault-test: '{kind}' returned without a fault (this is a bug)");
}

/// Recurses without end (on purpose); `limit` is never reached, but the
/// compiler can't know that, so it doesn't see this as plain infinite recursion.
#[inline(never)]
fn recurse(depth: u64) -> u64 {
    let pad = [depth; 64];
    if core::hint::black_box(depth) == u64::MAX {
        return 0;
    }
    let r = recurse(core::hint::black_box(depth + 1));
    r + core::hint::black_box(pad)[(depth % 64) as usize]
}
