//! The interrupt controllers: local APIC (per CPU, includes a timer), IO-APIC
//! (routes device interrupts to CPUs), and switching the legacy 8259 PICs off.
//!
//! Register encodings and the timer arithmetic are in `crates/apic`
//! (host-tested); this file does the MMIO. Only the boot CPU is used until
//! stage 7. The PIT (a fixed-rate legacy timer present on every PC) is used
//! once, to measure how fast the APIC timer runs.

use crate::interrupts::{self, TICK_HZ, VECTOR_SPURIOUS, VECTOR_TIMER};
use crate::log::SpinLock;
use crate::vm;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use nova_acpi::Madt;
use nova_apic::{APIC_BASE_ENABLE, MSR_APIC_BASE, Redirection, TimerMode, ioapic, lapic};
use nova_x86::{inb, outb, rdmsr, wrmsr};

/// Virtual address of the local APIC registers (0 until `init`).
static LAPIC: AtomicU64 = AtomicU64::new(0);
/// The boot CPU's APIC id.
static BSP_ID: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy)]
struct IoApicHw {
    gsi_base: u32,
    virt: u64,
    max_pin: u32,
}

const MAX_IOAPICS: usize = 8;
static IOAPICS: SpinLock<[Option<IoApicHw>; MAX_IOAPICS]> = SpinLock::new([None; MAX_IOAPICS]);
static MADT: SpinLock<Option<Madt>> = SpinLock::new(None);

pub struct Info {
    pub lapic_phys: u64,
    pub apic_id: u32,
    pub version: u32,
    pub ioapics: usize,
    pub io_pins: u32,
}

fn lapic_read(reg: usize) -> u32 {
    let base = LAPIC.load(Ordering::Relaxed);
    // SAFETY: `base` is the mapped local APIC page (set in `init`).
    unsafe { ((base + reg as u64) as *const u32).read_volatile() }
}

fn lapic_write(reg: usize, value: u32) {
    let base = LAPIC.load(Ordering::Relaxed);
    // SAFETY: as above.
    unsafe { ((base + reg as u64) as *mut u32).write_volatile(value) }
}

/// Signals end-of-interrupt to the local APIC.
pub fn eoi() {
    if LAPIC.load(Ordering::Relaxed) == 0 {
        return; // no APIC yet (an interrupt before init): nothing to acknowledge
    }
    lapic_write(lapic::EOI, 0);
}

fn io_read(virt: u64, reg: u32) -> u32 {
    // SAFETY: `virt` is a mapped IO-APIC register window.
    unsafe {
        (virt as *mut u32).write_volatile(reg);
        ((virt + ioapic::WINDOW as u64) as *const u32).read_volatile()
    }
}

fn io_write(virt: u64, reg: u32, value: u32) {
    // SAFETY: as above.
    unsafe {
        (virt as *mut u32).write_volatile(reg);
        ((virt + ioapic::WINDOW as u64) as *mut u32).write_volatile(value);
    }
}

fn io_wait() {
    // SAFETY: port 0x80 is the traditional "POST code" port; writing it is a short delay.
    unsafe { outb(0x80, 0) };
}

/// Remaps the two 8259 PICs out of the way and masks every line, so they
/// can never raise an interrupt (the APICs take over).
fn disable_legacy_pic() {
    // SAFETY: the kernel owns the PICs.
    unsafe {
        for (cmd, data, vector, wiring) in [(0x20u16, 0x21u16, 0xE0u8, 4u8), (0xA0, 0xA1, 0xE8, 2)] {
            outb(cmd, 0x11); // initialise, expect ICW4
            io_wait();
            outb(data, vector); // vector offset (out of the way, but valid)
            io_wait();
            outb(data, wiring); // master: slave on IRQ2; slave: cascade identity 2
            io_wait();
            outb(data, 0x01); // 8086 mode
            io_wait();
            outb(data, 0xFF); // mask everything
        }
    }
}

/// Switches from the legacy PICs to the APICs.
///
/// # Safety
/// Call once on the boot CPU, with interrupts disabled and the kernel's page
/// tables active.
pub unsafe fn init(madt: Madt) -> Result<Info, &'static str> {
    disable_legacy_pic();

    // Local APIC: make sure it is on, map it, enable it with our spurious vector.
    // SAFETY: standard MSR on every CPU with an APIC.
    unsafe {
        let base = rdmsr(MSR_APIC_BASE);
        if base & APIC_BASE_ENABLE == 0 {
            wrmsr(MSR_APIC_BASE, base | APIC_BASE_ENABLE);
        }
    }
    let virt = vm::map_mmio(madt.lapic_addr, 0x1000).map_err(|_| "could not map the local APIC")?;
    LAPIC.store(virt, Ordering::SeqCst);
    lapic_write(lapic::TPR, 0); // accept every priority
    lapic_write(lapic::LVT_LINT0, lapic::LVT_MASKED); // the legacy PIC "virtual wire": off
    lapic_write(lapic::LVT_ERROR, lapic::LVT_MASKED);
    lapic_write(lapic::SPURIOUS, lapic::SPURIOUS_ENABLE | VECTOR_SPURIOUS as u32);
    let apic_id = lapic_read(lapic::ID) >> 24;
    BSP_ID.store(apic_id, Ordering::SeqCst);

    // IO-APICs: map each and mask every pin.
    let mut total_pins = 0;
    let mut count = 0;
    IOAPICS.with(|slots| -> Result<(), &'static str> {
        for (slot, io) in slots.iter_mut().zip(madt.ioapics.iter()) {
            let v = vm::map_mmio(io.address as u64, 0x20).map_err(|_| "could not map an IO-APIC")?;
            let max_pin = ioapic::max_pin(io_read(v, ioapic::REG_VERSION));
            for pin in 0..=max_pin {
                let masked = Redirection::masked_off();
                io_write(v, ioapic::redirection_low(pin) + 1, masked.high());
                io_write(v, ioapic::redirection_low(pin), masked.low());
            }
            *slot = Some(IoApicHw { gsi_base: io.gsi_base, virt: v, max_pin });
            total_pins += max_pin + 1;
            count += 1;
        }
        Ok(())
    })?;
    if count == 0 {
        return Err("the MADT lists no IO-APIC");
    }

    let info = Info {
        lapic_phys: madt.lapic_addr,
        apic_id,
        version: lapic_read(lapic::VERSION) & 0xFF,
        ioapics: count,
        io_pins: total_pins,
    };
    MADT.with(|m| *m = Some(madt));
    Ok(info)
}

/// Waits `ms` milliseconds (at most 54) using PIT channel 2. Returns false
/// if the PIT never finished (so a missing/broken PIT can't hang boot).
pub fn pit_wait_ms(ms: u32) -> bool {
    const PIT_HZ: u32 = 1_193_182;
    let count = (PIT_HZ * ms.clamp(1, 54) / 1000) as u16;
    // SAFETY: the kernel owns the PIT and the speaker/gate port.
    unsafe {
        let gate = inb(0x61) & 0xFC; // gate off, speaker off
        outb(0x61, gate);
        outb(0x43, 0xB0); // channel 2, lo/hi byte, mode 0 (interrupt on terminal count)
        outb(0x42, count as u8);
        outb(0x42, (count >> 8) as u8);
        outb(0x61, gate | 1); // gate on: start counting
        for _ in 0..50_000_000u32 {
            if inb(0x61) & 0x20 != 0 {
                outb(0x61, gate);
                return true;
            }
            core::hint::spin_loop();
        }
        outb(0x61, gate);
    }
    false
}

/// Measures the APIC timer against the PIT and starts it ticking at
/// [`TICK_HZ`]. Returns the timer's input clock in Hz.
pub fn start_timer() -> Result<u64, &'static str> {
    const WINDOW_MS: u32 = 20;
    lapic_write(lapic::TIMER_DIVIDE, lapic::DIVIDE_BY_16);
    lapic_write(lapic::LVT_TIMER, nova_apic::lvt_timer(VECTOR_TIMER, TimerMode::OneShot, true));
    lapic_write(lapic::TIMER_INITIAL, u32::MAX);
    if !pit_wait_ms(WINDOW_MS) {
        lapic_write(lapic::TIMER_INITIAL, 0);
        return Err("the PIT did not finish its countdown (cannot calibrate the APIC timer)");
    }
    let elapsed = u32::MAX - lapic_read(lapic::TIMER_CURRENT);
    lapic_write(lapic::TIMER_INITIAL, 0);
    if elapsed == 0 {
        return Err("the APIC timer did not count");
    }
    interrupts::register(VECTOR_TIMER, interrupts::timer_tick);
    lapic_write(lapic::LVT_TIMER, nova_apic::lvt_timer(VECTOR_TIMER, TimerMode::Periodic, false));
    lapic_write(lapic::TIMER_INITIAL, nova_apic::timer_initial_count(elapsed, WINDOW_MS, TICK_HZ));
    Ok(nova_apic::timer_clock_hz(elapsed, WINDOW_MS))
}

/// Routes legacy ISA IRQ `irq` to `vector` on the boot CPU, honouring the
/// firmware's overrides. Returns the global system interrupt it landed on.
pub fn route_isa_irq(irq: u8, vector: u8) -> Result<u32, &'static str> {
    let route = MADT.with(|m| m.as_ref().map(|m| m.isa_route(irq))).ok_or("APIC not initialised")?;
    let hw = IOAPICS.with(|slots| {
        slots.iter().flatten().filter(|a| a.gsi_base <= route.gsi).max_by_key(|a| a.gsi_base).copied()
    });
    let hw = hw.ok_or("no IO-APIC handles this interrupt")?;
    let pin = route.gsi - hw.gsi_base;
    if pin > hw.max_pin {
        return Err("the interrupt is beyond the IO-APIC's last pin");
    }
    let e = Redirection::new(vector, BSP_ID.load(Ordering::SeqCst) as u8, route.active_low, route.level_triggered);
    // High half first, so the entry is never live with a stale destination.
    io_write(hw.virt, ioapic::redirection_low(pin) + 1, e.high());
    io_write(hw.virt, ioapic::redirection_low(pin), e.low());
    Ok(route.gsi)
}
