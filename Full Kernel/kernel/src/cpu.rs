//! CPU tables: our own GDT, TSS and IDT, replacing the firmware's.
//!
//! Why this matters: until now the kernel ran on the *firmware's* descriptor
//! tables, which are gone (or at least not ours to keep) once we reclaim that
//! memory, and which have no exception handlers: any fault would triple-fault
//! and the machine would silently reset.
//!
//! Three exceptions run on their own known-good stacks (IST), because the
//! stack they happened on can't be trusted:
//!   IST 1  double fault   (e.g. a kernel stack overflow)
//!   IST 2  NMI            (can arrive at any instruction, even mid-handler)
//!   IST 3  machine check  (hardware error report)

use crate::exceptions;
use core::ptr::{addr_of, addr_of_mut};
use nova_x86::exception::{DOUBLE_FAULT, MACHINE_CHECK, NMI};
use nova_x86::gdt::{Gdt, KERNEL_CS, KERNEL_DS, TSS_SELECTOR, TablePointer, Tss};
use nova_x86::idt::{Idt, IdtEntry};

const IST_STACK_SIZE: usize = 16 * 1024;
const IST_COUNT: usize = 3;

#[repr(C, align(16))]
struct IstStack([u8; IST_STACK_SIZE]);

static mut TSS: Tss = Tss::new();
static mut GDT: Gdt = Gdt::new(0); // the TSS address is filled in by init()
static mut IDT: Idt = Idt::new();
// NOTE: these have no guard pages yet; the kernel owns its page tables from
// stage 4 and should add them then.
static mut IST_STACKS: [IstStack; IST_COUNT] = [const { IstStack([0; IST_STACK_SIZE]) }; IST_COUNT];

/// Which IST slot (1-based, as the IDT numbers them) a vector runs on, or 0.
fn ist_for(vector: u8) -> u8 {
    match vector {
        DOUBLE_FAULT => 1,
        NMI => 2,
        MACHINE_CHECK => 3,
        _ => 0,
    }
}

/// If `addr` lies inside one of the interrupt stacks, its 1-based IST number.
pub fn ist_stack_containing(addr: u64) -> Option<u8> {
    let base = addr_of!(IST_STACKS) as u64;
    let end = base + (IST_COUNT * IST_STACK_SIZE) as u64;
    (base..end).contains(&addr).then(|| ((addr - base) / IST_STACK_SIZE as u64) as u8 + 1)
}

/// Loads the GDT, TSS and IDT and reloads the segment registers.
///
/// # Safety
/// Call once, early, on the boot CPU, with interrupts disabled.
pub unsafe fn init() {
    unsafe {
        // TSS: point each used IST slot at the top of its stack.
        let stacks = addr_of_mut!(IST_STACKS) as u64;
        let tss = addr_of_mut!(TSS);
        for i in 0..IST_COUNT {
            (*tss).ist[i] = stacks + ((i + 1) * IST_STACK_SIZE) as u64;
        }

        // GDT: the TSS descriptor needs the TSS's final address.
        let gdt = addr_of_mut!(GDT);
        *gdt = Gdt::new(tss as u64);
        let gdt_ptr = TablePointer { limit: (size_of_val(&(*gdt).entries) - 1) as u16, base: gdt as u64 };
        nova_x86::lgdt(&gdt_ptr);
        nova_x86::reload_segments(KERNEL_CS, KERNEL_DS);
        nova_x86::ltr(TSS_SELECTOR);

        // IDT: vectors 0..32 point at the assembly stubs; the rest stay "not present".
        let idt = addr_of_mut!(IDT);
        for (v, &stub) in exceptions::stub_table().iter().enumerate() {
            (*idt).entries[v] = IdtEntry::interrupt_gate(stub, KERNEL_CS, ist_for(v as u8), 0);
        }
        let idt_ptr = TablePointer {
            limit: (size_of::<Idt>() - 1) as u16,
            base: idt as u64,
        };
        nova_x86::lidt(&idt_ptr);
    }
}
