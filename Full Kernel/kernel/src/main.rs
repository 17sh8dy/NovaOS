//! The Nova kernel.
//!
//! Stage 1: prove the NovaBoot handoff end to end. The kernel checks the
//! BootInfo, reports the machine over serial and draws the boot screen.
//! Stage 2: the kernel installs its own GDT, TSS and IDT, so a CPU fault is
//! reported (serial + screen register dump) instead of silently resetting.
//! Next stages (memory manager, heap, interrupts, ...) are in docs/ROADMAP.md.

#![no_std]
#![no_main]

mod cpu;
mod crash;
mod exceptions;
mod faults;
mod font;
mod log;
mod screen;

use nova_bootinfo::{BootInfo, MemoryKind, total_of};
use screen::{Rgb, Screen};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Printed when the kernel has finished starting. tools/boot_test.py waits for it.
const READY_MARKER: &str = "NOVA KERNEL READY";

/// The kernel entry point, jumped to by NovaBoot.
///
/// # Safety
/// Only NovaBoot may call this, with `info` pointing at a `BootInfo` in the
/// direct map (or null, which panics with a clear message).
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text._start")]
pub unsafe extern "sysv64" fn _start(info: *const BootInfo) -> ! {
    log::init();
    kprintln!();
    kprintln!("Nova kernel {VERSION} (x86_64) - entered at {:#x}", _start as *const () as usize);

    // Own the CPU tables before anything else can go wrong.
    // SAFETY: first thing on the boot CPU, interrupts are off.
    unsafe { cpu::init() };
    kprintln!(
        "cpu tables: GDT/TSS/IDT loaded (cs {:#x}, ss {:#x}, tr {:#x}); IST stacks for #DF, NMI, #MC",
        nova_x86::read_cs(),
        nova_x86::read_ss(),
        nova_x86::read_tr()
    );
    if exceptions::self_test() {
        kprintln!("exceptions: breakpoint self-test passed (handler ran, execution resumed)");
    } else {
        panic!("exceptions: breakpoint self-test FAILED");
    }

    if info.is_null() {
        panic!("no boot info (rdi was null)");
    }
    // SAFETY: NovaBoot passes a pointer to a BootInfo in the direct map.
    let info = unsafe { &*info };
    if !info.is_valid() {
        panic!(
            "boot info mismatch: magic {:#x} version {} size {} (this kernel expects NovaBoot protocol v{})",
            info.magic,
            info.version,
            info.size,
            nova_bootinfo::VERSION
        );
    }
    kmain(info)
}

fn mib(bytes: u64) -> u64 {
    bytes >> 20
}

fn kmain(info: &'static BootInfo) -> ! {
    let vendor = nova_x86::cpu_vendor();
    kprintln!("cpu: {}", core::str::from_utf8(&vendor).unwrap_or("unknown"));
    kprintln!(
        "boot info: protocol v{}, direct map at {:#x} ({} MiB), NX {}",
        info.version,
        info.physical_memory_offset,
        mib(info.physical_memory_mapped),
        if info.nx_enabled != 0 { "on" } else { "off" }
    );
    kprintln!(
        "kernel: {} KiB at {:#x} (physical {:#x}); stack top {:#x}; cr3 {:#x}",
        info.kernel_pages * 4,
        info.kernel_virt,
        info.kernel_phys,
        info.stack_top,
        nova_x86::read_cr3()
    );
    if info.rsdp_addr != 0 {
        kprintln!("acpi: rsdp at {:#x}", info.rsdp_addr);
    }

    // SAFETY: protocol guarantee; the direct map is still in place.
    let map = unsafe { info.memory_map() };
    kprintln!("memory map: {} regions", map.len());
    for r in map {
        kprintln!("  {:#014x} - {:#014x}  {:>8} KiB  {}", r.start, r.end(), r.len >> 10, r.kind().name());
    }
    let usable = total_of(map, MemoryKind::Usable);
    let reclaimable = total_of(map, MemoryKind::FirmwareReclaimable);
    kprintln!("memory: {} MiB usable now, {} MiB more after the kernel owns the GDT/IDT", mib(usable), mib(reclaimable));

    let fb = info.framebuffer;
    if fb.is_present() {
        kprintln!("framebuffer: {}x{}, stride {}, at {:#x}", fb.width, fb.height, fb.stride, fb.addr);
        crash::set_framebuffer(fb, info.phys_to_virt(fb.addr));
        // SAFETY: the framebuffer is inside the direct map (NovaBoot maps
        // at least up to its end) and only the kernel draws to it.
        let mut s = unsafe { Screen::new(fb, info.phys_to_virt(fb.addr)) };
        draw_boot_screen(&mut s, info, usable);
    } else {
        kprintln!("framebuffer: none");
    }

    // Test builds (NOVA_FAULT=...) fault on purpose here; normal builds skip this.
    faults::run_if_requested();

    kprintln!("{READY_MARKER}");
    nova_x86::halt_forever()
}

fn draw_boot_screen(s: &mut Screen, info: &BootInfo, usable: u64) {
    let ink = Rgb(0xE8, 0xEC, 0xFF);
    let dim = Rgb(0x8C, 0x94, 0xC0);
    let accent = Rgb(0x6E, 0xE7, 0xD8);
    s.gradient(Rgb(0x07, 0x09, 0x1C), Rgb(0x1B, 0x10, 0x3A));

    let cx = s.width() / 2;
    let unit = (s.height() / 400).max(1); // 2 at 800 px tall
    let mut y = s.height() / 5;

    let star = 30 * unit;
    s.star(cx, y + star, star, accent);
    y += 2 * star + 16 * unit;

    s.text_centered(y, 6 * unit, ink, "NOVA OS");
    y += font::HEIGHT * 6 * unit + 6 * unit;
    s.text_centered(y, 2 * unit, dim, "FULL KERNEL - STAGE 2: CPU TABLES + EXCEPTIONS");
    y += font::HEIGHT * 2 * unit + 14 * unit;

    let lines = [
        fmt_line!("BOOTED BY NOVABOOT (UEFI)  -  KERNEL {VERSION}"),
        fmt_line!("SCREEN {} X {}", info.framebuffer.width, info.framebuffer.height),
        fmt_line!("MEMORY {} MIB USABLE", mib(usable)),
        fmt_line!("KERNEL AT {:#X}", info.kernel_virt),
        fmt_line!("OWN GDT + TSS + IDT  -  FAULTS ARE REPORTED"),
    ];
    for l in &lines {
        s.text_centered(y, 2 * unit, ink, l.as_str());
        y += font::HEIGHT * 2 * unit + 5 * unit;
    }
    y += 8 * unit;
    s.text_centered(y, 2 * unit, accent, "KERNEL READY");
}

static PANICKING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[panic_handler]
fn panic(p: &core::panic::PanicInfo) -> ! {
    // SAFETY: we never return, so stealing the log lock is fine.
    unsafe { log::force_unlock() };
    if PANICKING.swap(true, core::sync::atomic::Ordering::SeqCst) {
        kprintln!("\nNOVA KERNEL PANIC while panicking: {p}");
        nova_x86::halt_forever()
    }
    let mut r = crash::Report::new("NOVA KERNEL PANIC");
    if let Some(l) = p.location() {
        r.head(format_args!("at {}:{}", l.file(), l.line()));
    }
    r.wrapped(format_args!("{}", p.message()));
    r.show();
    nova_x86::halt_forever()
}
