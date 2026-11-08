//! x86_64 CPU helpers shared by NovaBoot and the kernel.

#![no_std]

pub mod serial;

use core::arch::asm;

/// Writes a byte to an I/O port.
///
/// # Safety
/// Port writes can reconfigure hardware; the caller must own the device.
#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags)) }
}

/// Reads a byte from an I/O port.
///
/// # Safety
/// Some port reads have side effects (e.g. acknowledging data).
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags)) }
    value
}

/// Reads a model-specific register.
///
/// # Safety
/// Reading a non-existent MSR raises #GP.
#[inline]
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) }
    (hi as u64) << 32 | lo as u64
}

/// Writes a model-specific register.
///
/// # Safety
/// Writing MSRs changes CPU behaviour; invalid values raise #GP.
#[inline]
pub unsafe fn wrmsr(msr: u32, value: u64) {
    let (lo, hi) = (value as u32, (value >> 32) as u32);
    unsafe { asm!("wrmsr", in("ecx") msr, in("eax") lo, in("edx") hi, options(nostack, preserves_flags)) }
}

pub const MSR_EFER: u32 = 0xC000_0080;
pub const EFER_NXE: u64 = 1 << 11;

/// CPUID. Returns (eax, ebx, ecx, edx).
pub fn cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    #[allow(unused_unsafe)]
    let r = unsafe { core::arch::x86_64::__cpuid_count(leaf, subleaf) };
    (r.eax, r.ebx, r.ecx, r.edx)
}

/// True if the CPU supports the no-execute page bit.
pub fn has_nx() -> bool {
    let (max_ext, ..) = cpuid(0x8000_0000, 0);
    max_ext >= 0x8000_0001 && cpuid(0x8000_0001, 0).3 & (1 << 20) != 0
}

/// The 12-character CPU vendor string, e.g. "GenuineIntel" or "AuthenticAMD".
pub fn cpu_vendor() -> [u8; 12] {
    let (_, b, c, d) = cpuid(0, 0);
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&b.to_le_bytes());
    out[4..8].copy_from_slice(&d.to_le_bytes());
    out[8..12].copy_from_slice(&c.to_le_bytes());
    out
}

#[inline]
pub fn read_cr3() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

#[inline]
pub fn read_cr0() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr0", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

#[inline]
pub fn read_cr4() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr4", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

/// Disables maskable interrupts.
#[inline]
pub fn cli() {
    unsafe { asm!("cli", options(nomem, nostack)) }
}

/// Stops the CPU for good: interrupts off, halt forever.
pub fn halt_forever() -> ! {
    loop {
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) }
    }
}
