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
//! Next stages (keyboard + console, tasks, ...) are in docs/ROADMAP.md.

#![no_std]
#![no_main]

extern crate alloc;

mod acpi;
mod apic;
mod bootdata;
mod cpu;
mod crash;
mod cursor;
mod exceptions;
mod faults;
mod font;
mod heap;
mod interrupts;
mod log;
mod pmm;
mod ps2;
mod screen;
mod vm;

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
    let mut screen = None;
    if fb.is_present() {
        kprintln!("framebuffer: {}x{}, stride {}, at {:#x} (write-combining)", fb.width, fb.height, fb.stride, fb.addr);
        crash::set_framebuffer(fb, info.phys_to_virt(fb.addr));
        // SAFETY: the framebuffer is mapped by vm::init at its direct-map
        // address and only the kernel draws to it.
        let mut s = unsafe { Screen::new(fb, info.phys_to_virt(fb.addr)) };
        draw_boot_screen(&mut s, info, free_mib);
        screen = Some(s);
    } else {
        kprintln!("framebuffer: none");
    }

    // Test builds (NOVA_FAULT=...) fault on purpose here; normal builds skip this.
    faults::run_if_requested();

    // ---- Stage 5: interrupts -----------------------------------------------
    let irq_mode = start_interrupts();

    // ---- Mouse ------------------------------------------------------------
    // SAFETY: the kernel owns the PS/2 controller.
    let mouse = match unsafe { ps2::init(irq_mode) } {
        Ok(m) => {
            kprintln!(
                "mouse: PS/2 mouse ready (device id {}{}), {}",
                m.id,
                if m.id == 3 || m.id == 4 { ", scroll wheel" } else { "" },
                if irq_mode { "interrupt driven (IRQ 12)" } else { "polled" }
            );
            true
        }
        Err(e) => {
            kprintln!("mouse: unavailable: {e}");
            false
        }
    };

    if irq_mode {
        // Interrupts have been off since the bootloader; turn them on and
        // check the timer really ticks at the rate it was set to.
        nova_x86::sti();
        tick_test();
    }

    kprintln!("{READY_MARKER}");
    match screen {
        Some(s) if mouse => desktop(s, irq_mode),
        _ => nova_x86::halt_forever(),
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
    match apic::route_isa_irq(12, interrupts::VECTOR_MOUSE) {
        Ok(gsi) => kprintln!("irq: ISA IRQ 12 (mouse) -> GSI {gsi} -> vector {:#x}", interrupts::VECTOR_MOUSE),
        Err(e) => {
            kprintln!("irq: cannot route the mouse interrupt: {e}; staying polled");
            return false;
        }
    }
    true
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

fn draw_boot_screen(s: &mut Screen, info: &BootInfo, free_mib: u64) {
    let ink = Rgb(0xE8, 0xEC, 0xFF);
    let dim = Rgb(0x8C, 0x94, 0xC0);
    let accent = Rgb(0x6E, 0xE7, 0xD8);
    s.gradient(Rgb(0x07, 0x09, 0x1C), Rgb(0x1B, 0x10, 0x3A));

    let cx = s.width() / 2;
    let unit = (s.height() / 400).max(1); // 2 at 800 px tall
    let mut y = s.height() / 6;

    let star = 30 * unit;
    s.star(cx, y + star, star, accent);
    y += 2 * star + 16 * unit;

    s.text_centered(y, 6 * unit, ink, "NOVA OS");
    y += font::HEIGHT * 6 * unit + 6 * unit;
    s.text_centered(y, 2 * unit, dim, "FULL KERNEL - STAGE 5: INTERRUPTS + TIMER");
    y += font::HEIGHT * 2 * unit + 14 * unit;

    let lines = [
        fmt_line!("BOOTED BY NOVABOOT (UEFI)  -  KERNEL {VERSION}"),
        fmt_line!("SCREEN {} X {}", info.framebuffer.width, info.framebuffer.height),
        fmt_line!("MEMORY {} MIB FREE  -  OWN PAGE TABLES", free_mib),
        fmt_line!("KERNEL AT {:#X}", info.kernel_virt),
        fmt_line!("OWN GDT + IDT  -  HEAP  -  APIC  -  MOUSE"),
    ];
    for l in &lines {
        s.text_centered(y, 2 * unit, ink, l.as_str());
        y += font::HEIGHT * 2 * unit + 5 * unit;
    }
    y += 8 * unit;
    s.text_centered(y, 2 * unit, accent, "KERNEL READY");
}

/// What the kernel does for now once it is up: show a mouse pointer, follow
/// the mouse, and show the uptime. With interrupts the CPU sleeps (`hlt`)
/// between events; without them it falls back to polling.
fn desktop(mut s: Screen, irq_mode: bool) -> ! {
    use nova_ps2::{Buttons, Pointer};

    let mut ptr = Pointer::centered(s.width(), s.height());
    let mut cursor = cursor::Cursor::new(s.height());
    let mut moved_once = false;
    let mut last_secs = 0;
    let mut wakeups: u64 = 0;
    let mut idle_reported = false;
    draw_status(&mut s, &ptr, 0);
    cursor.show(&mut s, ptr.x as usize, ptr.y as usize);

    loop {
        if !irq_mode {
            ps2::pump();
        }
        let mut redraw = false;
        while let Some(ev) = ps2::pop() {
            let change = ptr.apply(ev);
            if change.is_empty() {
                continue;
            }
            redraw = true;
            if change.moved && !moved_once {
                moved_once = true;
                kprintln!("mouse: first movement, pointer at ({}, {})", ptr.x, ptr.y);
            }
            for (bit, name) in [(Buttons::LEFT, "left"), (Buttons::RIGHT, "right"), (Buttons::MIDDLE, "middle")] {
                if change.pressed.0 & bit != 0 {
                    kprintln!("mouse: {name} button down at ({}, {})", ptr.x, ptr.y);
                }
                if change.released.0 & bit != 0 {
                    kprintln!("mouse: {name} button up at ({}, {})", ptr.x, ptr.y);
                }
            }
            if change.wheel != 0 {
                kprintln!("mouse: wheel {}", change.wheel);
            }
        }
        let secs = interrupts::ticks() / interrupts::TICK_HZ as u64;
        if secs != last_secs {
            last_secs = secs;
            redraw = true;
        }
        if redraw {
            cursor.hide(&mut s);
            draw_status(&mut s, &ptr, secs);
            cursor.show(&mut s, ptr.x as usize, ptr.y as usize);
        }
        // Proof that the CPU really sleeps: a busy loop would have spun
        // millions of times by now, an idle one wakes about once per tick.
        if irq_mode && secs >= 3 && !idle_reported {
            idle_reported = true;
            kprintln!(
                "idle: up {secs} s, {wakeups} wakeups from hlt, {} stray + {} spurious interrupts",
                interrupts::stray_count(),
                interrupts::spurious_count()
            );
        }
        if irq_mode {
            wakeups += 1;
            // Sleep until the next interrupt, unless an event arrived since we looked.
            nova_x86::cli();
            if ps2::has_events() {
                nova_x86::sti();
            } else {
                nova_x86::sti_hlt();
            }
        } else {
            core::hint::spin_loop();
        }
    }
}

/// The little pointer read-out in the bottom-left corner.
fn draw_status(s: &mut Screen, ptr: &nova_ps2::Pointer, uptime_secs: u64) {
    let unit = (s.height() / 400).max(1);
    let (w, h) = (330 * unit, 22 * unit);
    let (x, y) = (12 * unit, s.height().saturating_sub(h + 12 * unit));
    s.fill_rect(x, y, w, h, Rgb(0x10, 0x14, 0x30));
    let on = |down: bool, c: &'static str| if down { c } else { "-" };
    let line = fmt_line!(
        "MOUSE {:>4},{:>4} {}{}{}  UP {}S",
        ptr.x,
        ptr.y,
        on(ptr.buttons.left(), "L"),
        on(ptr.buttons.right(), "R"),
        on(ptr.buttons.middle(), "M"),
        uptime_secs
    );
    s.text(x + 6 * unit, y + 6 * unit, unit, Rgb(0x6E, 0xE7, 0xD8), line.as_str());
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
