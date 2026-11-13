//! The Nova kernel.
//!
//! Stage 1: prove the NovaBoot handoff end to end. The kernel checks the
//! BootInfo, reports the machine over serial and draws the boot screen.
//! Stage 2: the kernel installs its own GDT, TSS and IDT, so a CPU fault is
//! reported (serial + screen register dump) instead of silently resetting.
//! Stage 3: a physical frame allocator over the boot memory map.
//! Stage 4: the kernel's own page tables (the bootloader's identity map is
//! gone), bootloader memory is reclaimed, and `alloc` works (a kernel heap).
//! Stage 5: ACPI, the local APIC + IO-APIC, a 100 Hz timer, interrupt-driven
//! input and `hlt` idling. Also: a PS/2 mouse with an on-screen cursor.
//! Stage 7: kernel threads, a preemptive scheduler, SMP.
//! Stage 6: PS/2 keyboard, a scrolling text console and a shell thread.
//! Stage 8: user space. Stage 9: PCI, SATA + NVMe disks, NovaFS.
//! Real-hardware drivers: NVMe SSDs and USB (xHCI) keyboards and mice.
//! Stage 10: the Nova desktop (windows, taskbar, Terminal / Files / System).
//! Next: the shell as a user program (docs/ROADMAP.md).

#![no_std]
#![no_main]

extern crate alloc;

mod acpi;
mod ahci;
mod apic;
mod bootdata;
mod bootlog;
mod cpu;
mod crash;
mod desktop;
mod exceptions;
mod faults;
mod files;
mod font;
mod gui;
mod heap;
mod interrupts;
mod kmutex;
mod log;
mod nvme;
mod pci;
mod pipe;
mod pmm;
mod rtc;
mod process;
mod programs;
mod ps2;
mod screen;
mod sched;
mod shell;
mod smp;
mod syscall;
mod vm;
mod xhci;

use nova_bootinfo::{BootInfo, MemoryKind, total_of};
use screen::Screen;

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
    unsafe {
        cpu::init();
        cpu::enable_syscall(syscall::entry_address());
    }
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
    // The boot info and memory map are in bootloader memory, which is given
    // back later: keep our own copies.
    // SAFETY: validated just above; the bootloader's direct map is still active.
    unsafe { bootdata::capture(info) };
    kmain(info)
}

/// A cheap fingerprint of the memory map, to notice if it gets overwritten.
fn checksum(map: &[nova_bootinfo::MemoryRegion]) -> u64 {
    map.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, r| {
        (h ^ r.start ^ r.len.rotate_left(17) ^ ((r.kind() as u64) << 56)).wrapping_mul(0x100_0000_01B3)
    })
}

fn mib(bytes: u64) -> u64 {
    bytes >> 20
}

fn kmain(boot_copy: &'static BootInfo) -> ! {
    let info = bootdata::info();
    let map = bootdata::memory_map();
    // The screen first: the crash screen and the on-screen boot log need nothing
    // else (the framebuffer is in the direct map from the bootloader on).
    if info.framebuffer.is_present() {
        crash::set_framebuffer(info.framebuffer, info.phys_to_virt(info.framebuffer.addr));
        // SAFETY: the bootloader mapped the framebuffer at its direct-map address;
        // nothing else draws until the desktop starts (which stops the boot log).
        unsafe { bootlog::start(info.framebuffer, info.phys_to_virt(info.framebuffer.addr)) };
    }
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

    kprintln!("memory map: {} regions", map.len());
    for r in map {
        kprintln!("  {:#014x} - {:#014x}  {:>8} KiB  {}", r.start, r.end(), r.len >> 10, r.kind().name());
    }
    let usable = total_of(map, MemoryKind::Usable);
    let reclaimable = total_of(map, MemoryKind::FirmwareReclaimable);
    kprintln!("memory: {} MiB usable, {} MiB firmware scratch (reclaimable: the kernel now owns its GDT/IDT)", mib(usable), mib(reclaimable));

    // ---- Stage 3: physical memory --------------------------------------
    // SAFETY: first and only call; `map` is the real memory map.
    match unsafe { pmm::init(info, map) } {
        Ok(p) => kprintln!(
            "pmm: tracking {} frames; bitmap {} KiB at {:#x}; {} MiB free",
            p.stats.tracked_frames,
            p.bitmap_bytes.div_ceil(1024),
            p.bitmap_phys,
            mib(p.stats.free_bytes())
        ),
        Err(e) => panic!("pmm init failed: {e}"),
    }
    pmm_self_test("");
    // The test wrote over every frame it was given: if any of them was still
    // in use (the bootloader's own copy of the boot info, ...) it shows up here.
    // SAFETY: the bootloader's direct map is still active.
    if !boot_copy.is_valid() || checksum(unsafe { boot_copy.memory_map() }) != checksum(map) {
        panic!("pmm: the bootloader's boot info or memory map changed during the memory test");
    }

    // ---- Stage 4: the kernel's own page tables -------------------------
    // SAFETY: after pmm::init, with the bootloader's tables still active.
    match unsafe { vm::init() } {
        Ok(v) => kprintln!(
            "vm: kernel page tables active (cr3 {:#x}): {} MiB RAM direct-mapped in {} ranges, {} tables; identity map dropped",
            v.root,
            mib(v.direct_map_bytes),
            v.ram_ranges,
            v.tables
        ),
        Err(e) => panic!("vm init failed: {e}"),
    }
    if nova_x86::read_cr3() != vm::root() {
        panic!("vm: cr3 does not hold the new root");
    }
    if vm::translate(nova_pmm::LOW_MEMORY_LIMIT).is_some() || vm::translate(0).is_some() {
        panic!("vm: low memory is still mapped (the identity map survived)");
    }
    let free_before = pmm::stats().map_or(0, |s| s.free_bytes());
    let freed = vm::reclaim_loader_memory();
    kprintln!(
        "pmm: reclaimed {} KiB of bootloader memory (old page tables, boot data, loader image); {} MiB free",
        freed * 4,
        mib(free_before + freed as u64 * 4096)
    );
    // The real proof that reclaimed memory is really free: write to all of it.
    pmm_self_test(" after reclaim");
    if exceptions::self_test() {
        kprintln!("exceptions: still working on the kernel's page tables");
    } else {
        panic!("exceptions: breakpoint self-test FAILED after the page table switch");
    }

    // ---- Stage 4: heap --------------------------------------------------
    if let Err(e) = heap::init() {
        panic!("heap init failed: {e}");
    }
    match heap::self_test() {
        Ok(()) => {
            let (total, free) = heap::stats();
            kprintln!("heap: self-test passed (Box, Vec, String, aligned allocations; heap grew to {} KiB, {} KiB free)", total / 1024, free / 1024);
        }
        Err(e) => panic!("heap self-test FAILED: {e}"),
    }
    let free_mib = pmm::stats().map_or(0, |s| mib(s.free_bytes()));

    // ---- Screen ----------------------------------------------------------
    let fb = info.framebuffer;
    let mut have_gui = false;
    if fb.is_present() {
        kprintln!("framebuffer: {}x{}, stride {}, at {:#x} (write-combining)", fb.width, fb.height, fb.stride, fb.addr);
        bootlog::stop();
        // SAFETY: the framebuffer is mapped by vm::init at its direct-map
        // address and only the kernel draws to it (through gui.rs from here on).
        let s = unsafe { Screen::new(fb, info.phys_to_virt(fb.addr)) };
        gui::init(s, info, free_mib, 1);
        have_gui = true;
    } else {
        kprintln!("framebuffer: none");
    }

    // Test builds (NOVA_FAULT=...) fault on purpose here; normal builds skip this.
    faults::run_if_requested();

    // ---- Stage 5: interrupts -----------------------------------------------
    let irq_mode = start_interrupts();

    // ---- Keyboard + mouse ---------------------------------------------------
    // SAFETY: the kernel owns the PS/2 controller.
    let (mouse, keyboard) = match unsafe { ps2::init(irq_mode) } {
        Ok(d) => {
            let how = if irq_mode { "interrupt driven" } else { "polled" };
            let mouse = match d.mouse {
                Ok(m) => {
                    kprintln!("mouse: PS/2 mouse ready (device id {}{}), {how}", m.id, if m.id == 3 || m.id == 4 { ", scroll wheel" } else { "" });
                    true
                }
                Err(e) => {
                    kprintln!("mouse: unavailable: {e}");
                    false
                }
            };
            let keyboard = match d.keyboard {
                Ok(()) => {
                    kprintln!("keyboard: PS/2 keyboard ready (scancode set 1 via controller translation), {how}");
                    true
                }
                Err(e) => {
                    kprintln!("keyboard: unavailable: {e}");
                    false
                }
            };
            (mouse, keyboard)
        }
        Err(e) => {
            kprintln!("ps/2: unavailable: {e}");
            (false, false)
        }
    };

    let mut usb_input = false;
    if irq_mode {
        // Threads and the scheduler exist before interrupts come on, because
        // the timer tick drives them.
        sched::init();
        // Interrupts have been off since the bootloader; turn them on and
        // check the timer really ticks at the rate it was set to.
        nova_x86::sti();
        tick_test();

        // ---- Stage 7: more CPUs, then prove threads and the scheduler work --
        start_other_cpus();
        run_scheduler_tests();
        faults::run_late_if_requested();

        // ---- Devices on the PCI bus ------------------------------------------
        let devices = pci::enumerate();
        for d in &devices {
            // Bridges and the like are noise; show the devices that matter for storage and I/O.
            if matches!(d.class, 0x01 | 0x02 | 0x03 | 0x0C) {
                kprintln!("pci: {} {:04x}:{:04x} {}", d.address(), d.vendor, d.device_id, d.class_name());
            }
        }

        // ---- Stage 9: storage (SATA + NVMe) and files ----------------------
        files::init(&devices);

        // ---- USB keyboards and mice (xHCI) -----------------------------------
        let usb = xhci::init(&devices);
        if usb.controllers > 0 {
            kprintln!("usb: {} controller(s), {} keyboard(s), {} mouse/mice at boot", usb.controllers, usb.keyboards, usb.mice);
            xhci::start_service();
        }
        // A USB keyboard may be plugged in later, so a USB controller is enough for the shell.
        usb_input = usb.controllers > 0;

        if have_gui {
            gui::set_cpus(cpu::online_count());
            if keyboard || usb_input {
                if sched::spawn("terminal", shell::terminal).is_none() {
                    kprintln!("shell: could not start");
                }
            } else {
                kprintln!("shell: no keyboard, not started");
            }
        }
    } else if keyboard {
        kprintln!("shell: needs interrupts and the scheduler; not started in polled mode");
    }

    kprintln!("{READY_MARKER}");
    if have_gui && (mouse || keyboard || usb_input) {
        desktop::run(irq_mode)
    } else {
        nova_x86::halt_forever()
    }
}

/// ACPI -> APIC -> timer -> mouse routing. Returns true if interrupts are
/// ready to be enabled (false: no usable APIC; input falls back to polling).
fn start_interrupts() -> bool {
    let found = match acpi::discover() {
        Ok(f) => f,
        Err(e) => {
            kprintln!("acpi: unavailable: {e}; no interrupts, staying polled");
            return false;
        }
    };
    let mut names = alloc::string::String::new();
    for t in &found.tables {
        names.push_str(core::str::from_utf8(t).unwrap_or("????"));
        names.push(' ');
    }
    kprintln!("acpi: RSDP revision {}; tables: {}", found.rsdp_revision, names.trim_end());
    let m = &found.madt;
    kprintln!(
        "acpi: MADT: local APIC at {:#x}, {} CPU(s) ({} enabled), {} IO-APIC(s), {} IRQ override(s)",
        m.lapic_addr,
        m.cpus.len(),
        m.enabled_cpus(),
        m.ioapics.len(),
        m.overrides.len()
    );
    for o in &m.overrides {
        kprintln!("acpi:   ISA IRQ {} -> GSI {} (flags {:#x})", o.source, o.gsi, o.flags);
    }

    // SAFETY: boot CPU, interrupts still off, kernel page tables active.
    let info = match unsafe { apic::init(found.madt) } {
        Ok(i) => i,
        Err(e) => {
            kprintln!("apic: unavailable: {e}; no interrupts, staying polled");
            return false;
        }
    };
    kprintln!(
        "apic: legacy PIC off; local APIC id {} (version {:#x}) at {:#x}; {} IO-APIC(s) with {} pins, all masked",
        info.apic_id,
        info.version,
        info.lapic_phys,
        info.ioapics,
        info.io_pins
    );
    match apic::start_timer() {
        Ok(hz) => kprintln!(
            "timer: APIC timer clock {} MHz (measured against the PIT), ticking {} Hz on vector {:#x}",
            hz / 1_000_000,
            interrupts::TICK_HZ,
            interrupts::VECTOR_TIMER
        ),
        Err(e) => {
            kprintln!("timer: unavailable: {e}; no interrupts, staying polled");
            return false;
        }
    }
    interrupts::register(interrupts::VECTOR_MOUSE, ps2::irq_handler);
    interrupts::register(interrupts::VECTOR_KEYBOARD, ps2::irq_handler);
    for (irq, vector, name) in [(12u8, interrupts::VECTOR_MOUSE, "mouse"), (1, interrupts::VECTOR_KEYBOARD, "keyboard")] {
        match apic::route_isa_irq(irq, vector) {
            Ok(gsi) => kprintln!("irq: ISA IRQ {irq} ({name}) -> GSI {gsi} -> vector {vector:#x}"),
            Err(e) => {
                kprintln!("irq: cannot route the {name} interrupt: {e}; staying polled");
                return false;
            }
        }
    }
    true
}

/// Starts the other CPUs the firmware lists.
fn start_other_cpus() {
    // SAFETY: boot CPU, scheduler/APIC/heap ready, kernel page tables active.
    match unsafe { smp::start_aps() } {
        Ok(s) if s.failed == 0 => kprintln!("smp: all {} CPUs online", s.online),
        Ok(s) => kprintln!("smp: {} CPUs online, {} failed to start", s.online, s.failed),
        Err(e) => kprintln!("smp: could not start the other CPUs: {e}"),
    }
}

/// Threads, preemption, sleeping, locks, and (with several CPUs) parallelism.
fn run_scheduler_tests() {
    let cpus = cpu::online_count();
    match sched::self_test(cpus) {
        Ok(t) => {
            let s = sched::stats();
            kprintln!(
                "sched: self-test passed ({} busy threads time-sliced over {} CPU(s), {} used; sleep(50 ms) = {} ticks; {} context switches; {} stacks created, {} pooled, {} threads left)",
                t.spinners,
                cpus,
                t.cpus_used,
                t.sleep_ticks,
                t.switches,
                t.stacks_created,
                s.stacks_pooled,
                s.threads
            );
        }
        Err(e) => panic!("scheduler self-test FAILED: {e}"),
    }
}

/// With interrupts on, the timer must advance at its set rate: count ticks
/// over 100 ms of the (independent) PIT.
fn tick_test() {
    let t0 = interrupts::ticks();
    if !(apic::pit_wait_ms(50) && apic::pit_wait_ms(50)) {
        panic!("timer self-test: the PIT did not finish");
    }
    let n = interrupts::ticks() - t0;
    let expected = interrupts::TICK_HZ as u64 / 10;
    if n < expected - 2 || n > expected + 2 {
        panic!("timer self-test FAILED: {n} ticks in 100 ms, expected about {expected}");
    }
    kprintln!("timer: tick test passed ({n} ticks in 100 ms by the PIT, expected about {expected})");
}

/// Runs the pmm self-test and panics with a clear message if it fails.
fn pmm_self_test(when: &str) {
    match pmm::self_test() {
        Ok(t) => kprintln!(
            "pmm: self-test passed{when} ({} frames = {} MiB written, verified and freed{})",
            t.frames_touched,
            (t.frames_touched as u64 * 4096) >> 20,
            if t.exhausted { "; ran until memory was exhausted" } else { "" }
        ),
        Err(e) => panic!("pmm self-test FAILED{when}: {e}"),
    }
}

static PANICKING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[panic_handler]
fn panic(p: &core::panic::PanicInfo) -> ! {
    smp::halt_other_cpus();
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
