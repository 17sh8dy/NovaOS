//! Symmetric multiprocessing: starting the other CPUs.
//!
//! A CPU other than the boot CPU wakes up in 16-bit real mode when it gets an
//! INIT followed by a STARTUP interrupt, executing at a physical address
//! below 1 MiB that the STARTUP names. So:
//!
//! 1. A small **trampoline** (below) is copied to physical `0x8000`. It walks
//!    the CPU up from real mode to protected mode to long mode, using a
//!    temporary page table that identity-maps just that page and shares the
//!    kernel's upper half.
//! 2. The trampoline loads a stack and calls [`ap_main`] in the kernel.
//! 3. `ap_main` switches to the kernel's real page tables, builds the CPU's
//!    own GDT/TSS, turns on its local APIC and timer, and idles in the
//!    scheduler, which now hands it threads like any other CPU.
//!
//! CPUs are started one at a time; the trampoline's parameter block is reused.

use crate::{apic, cpu, pmm, sched, vm};
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Physical address the trampoline is copied to (a STARTUP vector of 8).
const TRAMPOLINE_PHYS: u64 = 0x8000;
const TRAMPOLINE_PAGE: u8 = (TRAMPOLINE_PHYS >> 12) as u8;

global_asm!(
    r#"
    .section .rodata.ap_trampoline,"a"
    .balign 16
    .global nova_ap_tramp_start
    .global nova_ap_tramp_end
    .global nova_ap_cr3
    .global nova_ap_stack
    .global nova_ap_index
    .global nova_ap_entry

nova_ap_tramp_start:
    .code16
    cli
    cld
    mov ax, cs
    mov ds, ax
    lgdt [0x120]                  // nova_ap_gdt_desc (offset from the segment base)
    mov eax, cr0
    or eax, 1
    mov cr0, eax
    // far jump (operand-size prefixed) to 32-bit protected mode: selector 0x08
    .byte 0x66, 0xEA
    .long nova_ap_pm32 - nova_ap_tramp_start + 0x8000
    .word 0x08

    .code32
nova_ap_pm32:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov eax, cr4
    or eax, 0x20                  // PAE
    mov cr4, eax
    mov eax, [0x8140]             // nova_ap_cr3
    mov cr3, eax
    mov ecx, 0xC0000080           // EFER
    rdmsr
    or eax, 0x900                 // long mode enable + no-execute enable
    wrmsr
    mov eax, cr0
    or eax, 0x80000000            // paging on: now in compatibility mode
    mov cr0, eax
    // far jump to 64-bit code: selector 0x18
    .byte 0xEA
    .long nova_ap_lm64 - nova_ap_tramp_start + 0x8000
    .word 0x18

    .code64
nova_ap_lm64:
    xor eax, eax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov rsp, [0x8148]             // nova_ap_stack
    mov rdi, [0x8150]             // nova_ap_index
    mov rax, [0x8158]             // nova_ap_entry
    call rax
2:
    cli
    hlt
    jmp 2b

    // The tables below sit at fixed offsets so the code above can address
    // them with plain numbers (the assembler can't do symbol arithmetic inside
    // a memory operand). The code must stay under 0x100 bytes.
    .org nova_ap_tramp_start + 0x100
nova_ap_gdt:
    .quad 0
    .quad 0x00CF9A000000FFFF      // 0x08: 32-bit code
    .quad 0x00CF92000000FFFF      // 0x10: data
    .quad 0x00AF9A000000FFFF      // 0x18: 64-bit code
    .org nova_ap_tramp_start + 0x120
nova_ap_gdt_desc:
    .word 31
    .long 0x8000 + 0x100

    .org nova_ap_tramp_start + 0x140
nova_ap_cr3:    .quad 0
nova_ap_stack:  .quad 0
nova_ap_index:  .quad 0
nova_ap_entry:  .quad 0
nova_ap_tramp_end:
    .text
    "#
);

unsafe extern "C" {
    static nova_ap_tramp_start: u8;
    static nova_ap_tramp_end: u8;
    static nova_ap_cr3: u8;
    static nova_ap_stack: u8;
    static nova_ap_index: u8;
    static nova_ap_entry: u8;
}

/// Set by an application processor as soon as it runs on the kernel's page
/// tables (so the boot CPU can reuse the trampoline's parameter block).
static AP_ALIVE: AtomicUsize = AtomicUsize::new(0);
/// Set when a CPU is dying: the others stop when they get the NMI.
static PANICKING: AtomicBool = AtomicBool::new(false);

/// True once some CPU has started reporting a fatal error.
pub fn panicking() -> bool {
    PANICKING.load(Ordering::SeqCst)
}

/// Stops every other CPU (an NMI that they answer by halting), so a crash
/// report is not interleaved with output from CPUs that are still running.
pub fn halt_other_cpus() {
    if !PANICKING.swap(true, Ordering::SeqCst) && cpu::online_count() > 1 {
        apic::send_nmi_all_but_self();
    }
}

pub struct Started {
    pub online: usize,
    pub failed: usize,
}

/// Starts every enabled CPU the firmware lists. Returns how many came up.
///
/// # Safety
/// Call once on the boot CPU after the scheduler, APIC and heap are ready,
/// with the kernel's page tables active.
pub unsafe fn start_aps() -> Result<Started, &'static str> {
    let bsp_apic = apic::this_apic_id();
    let aps: alloc::vec::Vec<_> = apic::cpu_list().into_iter().filter(|c| c.enabled && c.apic_id != bsp_apic).collect();
    let mut started = Started { online: 1, failed: 0 };
    if aps.is_empty() {
        return Ok(started);
    }
    if aps.len() + 1 > cpu::MAX_CPUS {
        return Err("more CPUs than the kernel supports");
    }
    let direct = crate::bootdata::info().physical_memory_offset;

    // The trampoline's temporary page tables must sit below 4 GiB (it loads CR3 in 32-bit mode).
    let low_frame = || -> Result<u64, &'static str> {
        let f = pmm::alloc_frame().ok_or("no memory for the CPU start-up page tables")?;
        if f >= 1 << 32 {
            let _ = pmm::free_frame(f);
            return Err("no page frame below 4 GiB for the CPU start-up page tables");
        }
        // SAFETY: a frame we own, in the direct map.
        unsafe { core::ptr::write_bytes((direct + f) as *mut u8, 0, 4096) };
        Ok(f)
    };
    let (pml4, pdpt, pd, pt) = (low_frame()?, low_frame()?, low_frame()?, low_frame()?);
    // SAFETY: the four frames are ours and zeroed; the kernel root is a valid PML4.
    unsafe {
        let new = (direct + pml4) as *mut u64;
        let kernel = (direct + vm::root()) as *const u64;
        for i in 256..512 {
            new.add(i).write(kernel.add(i).read()); // share the kernel's upper half
        }
        new.write(pdpt | 3);
        ((direct + pdpt) as *mut u64).write(pd | 3);
        ((direct + pd) as *mut u64).write(pt | 3);
        ((direct + pt) as *mut u64).add((TRAMPOLINE_PHYS >> 12) as usize).write(TRAMPOLINE_PHYS | 3);
    }

    // Copy the trampoline to its low-memory home.
    let start = &raw const nova_ap_tramp_start as u64;
    let len = (&raw const nova_ap_tramp_end as u64 - start) as usize;
    if len > 4096 {
        return Err("the AP trampoline does not fit in one page");
    }
    // SAFETY: physical page 8 is free low memory (the frame allocator never hands out < 1 MiB).
    unsafe { core::ptr::copy_nonoverlapping(start as *const u8, (direct + TRAMPOLINE_PHYS) as *mut u8, len) };
    let field = |sym: *const u8| (direct + TRAMPOLINE_PHYS + (sym as u64 - start)) as *mut u64;

    for (n, ap) in aps.iter().enumerate() {
        let index = n + 1;
        let Some(stack) = sched::alloc_cpu_stack() else {
            started.failed += 1;
            continue;
        };
        // SAFETY: the parameter block is in the copied trampoline; the previous CPU is done with it.
        unsafe {
            field(&raw const nova_ap_cr3).write_volatile(pml4);
            field(&raw const nova_ap_stack).write_volatile(stack.top());
            field(&raw const nova_ap_index).write_volatile(index as u64);
            field(&raw const nova_ap_entry).write_volatile(ap_main as *const () as usize as u64);
        }
        apic::send_init(ap.apic_id);
        apic::pit_wait_ms(10);
        apic::send_startup(ap.apic_id, TRAMPOLINE_PAGE);
        apic::pit_wait_ms(2);
        if AP_ALIVE.load(Ordering::SeqCst) != index {
            apic::send_startup(ap.apic_id, TRAMPOLINE_PAGE); // the second STARTUP the spec asks for
        }
        // Wait (up to ~1 s) for it to reach the kernel and report in.
        let mut waited = 0;
        while cpu::cpu(index).is_none_or(|c| !c.online.load(Ordering::SeqCst)) && waited < 200 {
            apic::pit_wait_ms(5);
            waited += 1;
        }
        if cpu::cpu(index).is_some_and(|c| c.online.load(Ordering::SeqCst)) {
            started.online += 1;
        } else {
            crate::kprintln!("smp: CPU {index} (APIC id {}) did not come up", ap.apic_id);
            started.failed += 1;
            // Its parameter block is still in use if it is merely slow; stop starting more.
            break;
        }
    }

    // All started CPUs are on the kernel's tables now: the temporary ones can go.
    if started.failed == 0 {
        for f in [pml4, pdpt, pd, pt] {
            let _ = pmm::free_frame(f);
        }
    }
    Ok(started)
}

/// First Rust code an application processor runs (on its own stack, with the
/// trampoline's temporary page tables).
extern "sysv64" fn ap_main(index: u64) -> ! {
    // SAFETY: the kernel root maps everything this code uses (same upper half).
    unsafe { nova_x86::write_cr3(vm::root()) };
    AP_ALIVE.store(index as usize, Ordering::SeqCst);

    let apic_id = apic::this_apic_id();
    // SAFETY: first code on this CPU, interrupts are off, the heap works.
    let me = unsafe { cpu::init_ap(index as usize, apic_id) };
    // SAFETY: this CPU's tables are loaded; the stub is the kernel's SYSCALL entry.
    unsafe { cpu::enable_syscall(crate::syscall::entry_address()) };
    apic::init_ap();
    sched::init_ap();
    apic::start_timer_ap();
    me.online.store(true, Ordering::SeqCst);
    crate::kprintln!("smp: CPU {index} (APIC id {apic_id}) online");
    // From here this CPU is just an idle thread: the timer tick hands it work.
    loop {
        nova_x86::sti_hlt();
    }
}
