//! CPU tables and per-CPU state: each CPU has its own GDT, TSS (with its own
//! interrupt stacks) and a `PerCpu` record reached through the `gs` segment
//! base; the IDT is shared by all CPUs.
//!
//! Why this matters: until the kernel loads its own tables it runs on the
//! *firmware's*, which are gone once that memory is reclaimed and which have no
//! exception handlers: any fault would triple-fault and the machine would
//! silently reset.
//!
//! Three exceptions run on their own known-good stacks (IST), because the
//! stack they happened on can't be trusted:
//!   IST 1  double fault   (e.g. a kernel stack overflow)
//!   IST 2  NMI            (can arrive at any instruction, even mid-handler)
//!   IST 3  machine check  (hardware error report)
//!
//! The boot CPU's tables are statics (they are needed before the heap exists);
//! every other CPU's are allocated when it is started.

use crate::exceptions;
use alloc::boxed::Box;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use nova_x86::exception::{DOUBLE_FAULT, MACHINE_CHECK, NMI};
use nova_x86::gdt::{Gdt, KERNEL_CS, KERNEL_DS, TSS_SELECTOR, TablePointer, Tss};
use nova_x86::idt::{Idt, IdtEntry};

const IST_STACK_SIZE: usize = 16 * 1024;
const IST_COUNT: usize = 3;

/// The most CPUs the kernel will run.
pub const MAX_CPUS: usize = 64;

/// IA32_GS_BASE: where `gs:0` points.
const MSR_GS_BASE: u32 = 0xC000_0101;

#[repr(C, align(16))]
struct IstStack([u8; IST_STACK_SIZE]);

/// Everything one CPU needs to own privately in its descriptor tables.
#[repr(C, align(16))]
pub struct CpuTables {
    gdt: Gdt,
    tss: Tss,
    ist: [IstStack; IST_COUNT],
}

impl CpuTables {
    const fn new() -> Self {
        Self { gdt: Gdt::new(0), tss: Tss::new(), ist: [const { IstStack([0; IST_STACK_SIZE]) }; IST_COUNT] }
    }

    fn ist_range(&self) -> (u64, u64) {
        let base = addr_of!(self.ist) as u64;
        (base, base + (IST_COUNT * IST_STACK_SIZE) as u64)
    }
}

/// Per-CPU state, reached with [`this_cpu`]. `self_ptr`, `user_rsp` and
/// `kernel_rsp` must stay first and in this order: the SYSCALL entry stub
/// addresses them as `gs:[0]`, `gs:[8]` and `gs:[16]`.
#[repr(C)]
pub struct PerCpu {
    self_ptr: AtomicPtr<PerCpu>,
    /// Scratch for the SYSCALL stub: the user stack pointer.
    pub user_rsp: AtomicU64,
    /// Top of the kernel stack of the thread running here (for entries from user mode).
    pub kernel_rsp: AtomicU64,
    /// The page-table root loaded on this CPU.
    pub cr3: AtomicU64,
    /// Dense index: 0 = boot CPU.
    pub id: usize,
    pub apic_id: AtomicU32,
    tables: AtomicPtr<CpuTables>,
    /// Scheduler: the thread running here, this CPU's idle thread, and the
    /// thread that was just switched away from (see sched.rs).
    pub current: AtomicUsize,
    pub idle: AtomicUsize,
    pub prev: AtomicUsize,
    /// Timer ticks the current thread has used up.
    pub slice: AtomicU32,
    pub online: AtomicBool,
}

impl PerCpu {
    const fn new(id: usize, apic_id: u32) -> Self {
        Self {
            self_ptr: AtomicPtr::new(core::ptr::null_mut()),
            user_rsp: AtomicU64::new(0),
            kernel_rsp: AtomicU64::new(0),
            cr3: AtomicU64::new(0),
            id,
            apic_id: AtomicU32::new(apic_id),
            tables: AtomicPtr::new(core::ptr::null_mut()),
            current: AtomicUsize::new(usize::MAX),
            idle: AtomicUsize::new(usize::MAX),
            prev: AtomicUsize::new(usize::MAX),
            slice: AtomicU32::new(0),
            online: AtomicBool::new(false),
        }
    }
}

static mut BSP_TABLES: CpuTables = CpuTables::new();
static BSP_PERCPU: PerCpu = PerCpu::new(0, 0);
static mut IDT: Idt = Idt::new();

/// All CPUs' records, by dense id (null = not started).
static CPUS: [AtomicPtr<PerCpu>; MAX_CPUS] = [const { AtomicPtr::new(core::ptr::null_mut()) }; MAX_CPUS];

/// The record of the CPU executing this code.
pub fn this_cpu() -> &'static PerCpu {
    let p: *const PerCpu;
    // SAFETY: `gs:0` holds the address of this CPU's (never freed) PerCpu once
    // `init`/`init_ap` has run, which is before anything calls this.
    unsafe {
        core::arch::asm!("mov {}, gs:0", out(reg) p, options(nostack, preserves_flags, readonly));
        &*p
    }
}

/// The record of CPU `id`, if it has been started.
pub fn cpu(id: usize) -> Option<&'static PerCpu> {
    // SAFETY: records are leaked (never freed), so any non-null pointer stays valid.
    unsafe { CPUS.get(id)?.load(Ordering::SeqCst).as_ref() }
}

/// Number of CPUs that have finished starting up.
pub fn online_count() -> usize {
    CPUS.iter().filter_map(|c| unsafe { c.load(Ordering::SeqCst).as_ref() }).filter(|c| c.online.load(Ordering::SeqCst)).count()
}

/// Which IST slot (1-based, as the IDT numbers them) a vector runs on, or 0.
fn ist_for(vector: u8) -> u8 {
    match vector {
        DOUBLE_FAULT => 1,
        NMI => 2,
        MACHINE_CHECK => 3,
        _ => 0,
    }
}

/// If `addr` lies inside one of *this CPU's* interrupt stacks, its 1-based IST number.
pub fn ist_stack_containing(addr: u64) -> Option<u8> {
    // SAFETY: the tables of a started CPU are never freed.
    let t = unsafe { this_cpu().tables.load(Ordering::SeqCst).as_ref()? };
    let (base, end) = t.ist_range();
    (base..end).contains(&addr).then(|| ((addr - base) / IST_STACK_SIZE as u64) as u8 + 1)
}

/// Loads `t`'s GDT and TSS on this CPU and reloads the segment registers.
///
/// # Safety
/// `t` must stay in place forever; interrupts must be off.
unsafe fn load_tables(t: *mut CpuTables) {
    unsafe {
        (*t).tss = Tss::new();
        let stacks = addr_of_mut!((*t).ist) as u64;
        for i in 0..IST_COUNT {
            (*t).tss.ist[i] = stacks + ((i + 1) * IST_STACK_SIZE) as u64;
        }
        // The TSS descriptor needs the TSS's final address.
        (*t).gdt = Gdt::new(addr_of_mut!((*t).tss) as u64);
        let gdt_ptr = TablePointer { limit: (size_of_val(&(*t).gdt.entries) - 1) as u16, base: addr_of_mut!((*t).gdt) as u64 };
        nova_x86::lgdt(&gdt_ptr);
        nova_x86::reload_segments(KERNEL_CS, KERNEL_DS);
        nova_x86::ltr(TSS_SELECTOR);
    }
}

/// Points `gs` at `percpu` and registers it.
///
/// # Safety
/// `percpu` must never be freed.
unsafe fn install_percpu(percpu: &'static PerCpu, tables: *mut CpuTables) {
    percpu.self_ptr.store(percpu as *const PerCpu as *mut PerCpu, Ordering::SeqCst);
    percpu.tables.store(tables, Ordering::SeqCst);
    // SAFETY: standard MSR; the record is valid forever.
    unsafe { nova_x86::wrmsr(MSR_GS_BASE, percpu as *const PerCpu as u64) };
    CPUS[percpu.id].store(percpu as *const PerCpu as *mut PerCpu, Ordering::SeqCst);
}

/// Boot CPU: loads the GDT, TSS and IDT and reloads the segment registers.
///
/// # Safety
/// Call once, early, on the boot CPU, with interrupts disabled.
pub unsafe fn init() {
    unsafe {
        let tables = addr_of_mut!(BSP_TABLES);
        load_tables(tables);

        // IDT: every vector points at its assembly stub; the rest of the
        // setup (handlers) is in exceptions.rs and interrupts.rs.
        let idt = addr_of_mut!(IDT);
        for (v, &stub) in exceptions::stub_table().iter().enumerate() {
            (*idt).entries[v] = IdtEntry::interrupt_gate(stub, KERNEL_CS, ist_for(v as u8), 0);
        }
        load_idt();
        install_percpu(&BSP_PERCPU, tables);
        BSP_PERCPU.online.store(true, Ordering::SeqCst);
    }
}

/// Loads the shared IDT on this CPU.
///
/// # Safety
/// The IDT must be built (`init` ran on the boot CPU).
pub unsafe fn load_idt() {
    let idt = addr_of_mut!(IDT);
    let idt_ptr = TablePointer { limit: (size_of::<Idt>() - 1) as u16, base: idt as u64 };
    unsafe { nova_x86::lidt(&idt_ptr) };
}

/// Application processor: gives it its own tables and per-CPU record.
///
/// # Safety
/// Call once per CPU, on that CPU, with interrupts disabled, after the heap works.
pub unsafe fn init_ap(id: usize, apic_id: u32) -> &'static PerCpu {
    // Allocated zeroed straight on the heap: building a CpuTables on the (small) stack first
    // would not fit. All-zero is a valid CpuTables; load_tables fills in the real values.
    // SAFETY: non-zero-sized layout; checked for null below.
    let raw = unsafe { alloc::alloc::alloc_zeroed(core::alloc::Layout::new::<CpuTables>()) } as *mut CpuTables;
    assert!(!raw.is_null(), "out of memory for a CPU's descriptor tables");
    let tables: &'static mut CpuTables = unsafe { &mut *raw };
    let percpu: &'static PerCpu = Box::leak(Box::new(PerCpu::new(id, apic_id)));
    unsafe {
        load_tables(tables);
        load_idt();
        install_percpu(percpu, tables);
    }
    percpu
}

/// Tells the CPU where to switch to when user mode traps into the kernel:
/// the TSS's ring-0 stack (interrupts and exceptions) and the per-CPU slot
/// the SYSCALL stub reads. Call whenever a different thread starts running.
pub fn set_kernel_stack(top: u64) {
    let cpu = this_cpu();
    cpu.kernel_rsp.store(top, Ordering::SeqCst);
    // SAFETY: the tables of a started CPU are never freed; only this CPU writes its own TSS.
    unsafe {
        if let Some(t) = cpu.tables.load(Ordering::SeqCst).as_mut() {
            core::ptr::write_volatile(addr_of_mut!(t.tss.rsp[0]), top);
        }
    }
}

/// Turns on SYSCALL/SYSRET on this CPU, entering the kernel at `entry`.
///
/// # Safety
/// `entry` must be the SYSCALL entry stub; call once per CPU after `init`/`init_ap`.
pub unsafe fn enable_syscall(entry: u64) {
    use nova_x86::gdt::{KERNEL_CS, SYSRET_BASE};
    // Entering via SYSCALL clears these RFLAGS bits: IF (the stub decides when to
    // re-enable interrupts), DF, TF, NT and AC.
    const SFMASK: u64 = 0x200 | 0x400 | 0x100 | 0x4000 | 0x4_0000;
    unsafe {
        let efer = nova_x86::rdmsr(nova_x86::MSR_EFER);
        nova_x86::wrmsr(nova_x86::MSR_EFER, efer | nova_x86::EFER_SCE);
        nova_x86::wrmsr(nova_x86::MSR_STAR, (SYSRET_BASE as u64) << 48 | (KERNEL_CS as u64) << 32);
        nova_x86::wrmsr(nova_x86::MSR_LSTAR, entry);
        nova_x86::wrmsr(nova_x86::MSR_SFMASK, SFMASK);
    }
}

/// Records the boot CPU's APIC id (known once the APIC is up).
pub fn set_bsp_apic_id(apic_id: u32) {
    BSP_PERCPU.apic_id.store(apic_id, Ordering::SeqCst);
}
