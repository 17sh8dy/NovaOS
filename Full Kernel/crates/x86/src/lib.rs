//! x86_64 CPU helpers shared by NovaBoot and the kernel.

#![no_std]

pub mod exception;
pub mod gdt;
pub mod idt;
pub mod serial;

use core::arch::asm;

/// Loads the GDT register.
///
/// # Safety
/// `ptr` must describe a valid GDT that stays in place (and mapped) for good.
#[inline]
pub unsafe fn lgdt(ptr: &gdt::TablePointer) {
    unsafe { asm!("lgdt [{}]", in(reg) ptr, options(readonly, nostack, preserves_flags)) }
}

/// Loads the IDT register.
///
/// # Safety
/// `ptr` must describe a valid IDT that stays in place (and mapped) for good.
#[inline]
pub unsafe fn lidt(ptr: &gdt::TablePointer) {
    unsafe { asm!("lidt [{}]", in(reg) ptr, options(readonly, nostack, preserves_flags)) }
}

/// Loads the task register with a TSS selector.
///
/// # Safety
/// `selector` must name an available TSS descriptor in the loaded GDT.
#[inline]
pub unsafe fn ltr(selector: u16) {
    unsafe { asm!("ltr {0:x}", in(reg) selector, options(nostack, preserves_flags)) }
}

/// Reloads `cs` (via a far return) and the data segment registers from the
/// freshly loaded GDT. `fs` and `gs` are set to the null selector.
///
/// # Safety
/// The loaded GDT must hold a 64-bit code segment at `cs` and a data segment
/// at `ds`.
pub unsafe fn reload_segments(cs: u16, ds: u16) {
    unsafe {
        asm!(
            "push {cs}",
            "lea {tmp}, [2f + rip]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {ds:x}",
            "mov es, {ds:x}",
            "mov ss, {ds:x}",
            "xor {ds:e}, {ds:e}",
            "mov fs, {ds:x}",
            "mov gs, {ds:x}",
            cs = in(reg) cs as u64,
            tmp = out(reg) _,
            ds = inout(reg) ds as u32 => _,
            options(preserves_flags),
        )
    }
}

#[inline]
pub fn read_cs() -> u16 {
    let v: u16;
    unsafe { asm!("mov {0:x}, cs", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

#[inline]
pub fn read_ss() -> u16 {
    let v: u16;
    unsafe { asm!("mov {0:x}, ss", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

/// The task register (the selector of the loaded TSS).
#[inline]
pub fn read_tr() -> u16 {
    let v: u16;
    unsafe { asm!("str {0:x}", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

/// Loads a new top-level page table (flushes the non-global TLB entries).
///
/// # Safety
/// `root` must be the physical address of a valid PML4 that maps the code,
/// stack and data the CPU is using right now.
#[inline]
pub unsafe fn write_cr3(root: u64) {
    unsafe { asm!("mov cr3, {}", in(reg) root, options(nostack, preserves_flags)) }
}

/// Invalidates the TLB entry for one page.
#[inline]
pub fn invlpg(addr: u64) {
    // SAFETY: invlpg only drops a cached translation.
    unsafe { asm!("invlpg [{}]", in(reg) addr, options(nostack, preserves_flags)) }
}

/// The page-attribute-table MSR (cache types selectable by PAT/PCD/PWT bits).
pub const MSR_PAT: u32 = 0x277;
pub const EFER_NXE_BIT: u64 = EFER_NXE;

/// The faulting address of the last page fault.
#[inline]
pub fn read_cr2() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr2", out(reg) v, options(nomem, nostack, preserves_flags)) }
    v
}

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
