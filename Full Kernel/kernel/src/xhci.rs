//! USB through an xHCI host controller: keyboards and mice.
//!
//! Every PC sold in the last decade has an xHCI controller, and on many
//! (most laptops, many desktops) USB is the *only* way a keyboard or mouse
//! reaches the machine: the PS/2 "keyboard" the firmware shows is an
//! emulation that stops the moment an OS takes the controller over.
//!
//! How it works:
//! * **Take over** from the firmware (the USB Legacy Support handshake), stop
//!   and reset the controller, give it a device-context array (plus the
//!   scratchpad pages some controllers need), a *command ring* and an *event
//!   ring*, and start it.
//! * **Rings:** the driver writes TRBs (16-byte requests) into rings in memory
//!   and rings a doorbell; the controller writes results into the event ring.
//!   A cycle bit tells new entries from old ones. We poll the event ring
//!   (no interrupts): during start-up while waiting for each command, then
//!   from the `usb` kernel thread 100 times a second.
//! * **Devices:** for each connected root port: reset the port (USB 2), Enable
//!   Slot, Address Device, read the device and configuration descriptors, pick
//!   the HID *boot* keyboard and mouse interfaces, Set Configuration, switch
//!   them to the boot protocol, Configure Endpoint for their interrupt-IN
//!   endpoints, and keep a few transfers queued on each.
//! * **Input:** keyboard reports become PS/2 set 1 scancodes fed through the
//!   kernel's existing keyboard pipeline (layout, modifiers), with key repeat
//!   done here; mouse reports join the PS/2 mouse queue. The shell and the
//!   pointer cannot tell the difference.
//! * **Hot plug:** port status change events attach new devices and detach
//!   removed ones.
//!
//! Not yet: hubs (a device behind a hub is reported and ignored), USB storage,
//! interrupts/MSI, report-protocol HID (only boot-protocol keyboards and mice).
//! TRB/context/descriptor/report logic is in `crates/usb` (host-tested).

use crate::kmutex::KMutex;
use crate::{bootdata, interrupts, pci, pmm, ps2, sched, vm};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};
use nova_usb::context::{self, Layout};
use nova_usb::desc::{self, BootKind};
use nova_usb::trb::{self, Consumer, Producer, Trb};
use nova_usb::{Speed, dci, hid, xhci_interval};

// Capability registers.
const CAP_LENGTH: usize = 0x00;
const HCS_PARAMS1: usize = 0x04;
const HCS_PARAMS2: usize = 0x08;
const HCC_PARAMS1: usize = 0x10;
const DB_OFF: usize = 0x14;
const RTS_OFF: usize = 0x18;
// Operational registers.
const USBCMD: usize = 0x00;
const USBSTS: usize = 0x04;
const CRCR: usize = 0x18;
const DCBAAP: usize = 0x30;
const CONFIG: usize = 0x38;
const PORTS: usize = 0x400;
const CMD_RUN: u32 = 1 << 0;
const CMD_RESET: u32 = 1 << 1;
const STS_HALTED: u32 = 1 << 0;
const STS_NOT_READY: u32 = 1 << 11;
// Interrupter 0 (runtime registers).
const IMAN: usize = 0x20;
const ERSTSZ: usize = 0x28;
const ERSTBA: usize = 0x30;
const ERDP: usize = 0x38;
const ERDP_BUSY: u64 = 1 << 3;
// PORTSC.
const PORT_CONNECTED: u32 = 1 << 0;
const PORT_ENABLED: u32 = 1 << 1;
const PORT_RESET: u32 = 1 << 4;
const PORT_POWER: u32 = 1 << 9;
const PORT_CONNECT_CHANGE: u32 = 1 << 17;
const PORT_RESET_CHANGE: u32 = 1 << 21;
const PORT_CHANGES: u32 = 0x7F << 17;
/// Bits that are safe to write back unchanged (read-only and plain read/write ones;
/// the write-1-to-clear ones, Port Enabled and Port Reset are left out).
const PORT_NEUTRAL: u32 = 1 | 1 << 3 | 0xF << 5 | 1 << 9 | 0xF << 10 | 3 << 14 | 1 << 24 | 7 << 25 | 1 << 30;

const RING_TRBS: usize = 256;
const EVENT_TRBS: usize = 256;
/// Transfers kept queued on each keyboard/mouse endpoint.
const QUEUED_REPORTS: usize = 8;
/// Bytes of the report buffer page given to each queued transfer.
const REPORT_BYTES: u64 = 64;

fn direct() -> u64 {
    bootdata::info().physical_memory_offset
}

fn rd(base: u64, off: usize) -> u32 {
    // SAFETY: `base` is a mapped register block of the controller (or DMA memory we own).
    unsafe { ((base + off as u64) as *const u32).read_volatile() }
}

fn wr(base: u64, off: usize, v: u32) {
    // SAFETY: as above.
    unsafe { ((base + off as u64) as *mut u32).write_volatile(v) }
}

fn wr64(base: u64, off: usize, v: u64) {
    wr(base, off, v as u32);
    wr(base, off + 4, (v >> 32) as u32);
}

fn write_trb(virt: u64, t: Trb) {
    // The control dword (with the cycle bit) goes last: the controller may be
    // looking at this slot already.
    for i in 0..3 {
        wr(virt, i * 4, t.0[i]);
    }
    fence(Ordering::SeqCst);
    wr(virt, 12, t.0[3]);
}

fn read_trb(virt: u64) -> Trb {
    Trb([rd(virt, 0), rd(virt, 4), rd(virt, 8), rd(virt, 12)])
}

fn write_context(virt: u64, c: [u32; 8]) {
    for (i, d) in c.iter().enumerate() {
        wr(virt, i * 4, *d);
    }
}

fn read_context(virt: u64) -> [u32; 8] {
    core::array::from_fn(|i| rd(virt, i * 4))
}

/// A command or transfer ring (one page).
struct Ring {
    prod: Producer,
    virt: u64,
}

impl Ring {
    fn new(frames: &mut Vec<u64>, below_4g: bool) -> Option<Self> {
        let phys = pmm::alloc_dma_frame(below_4g)?;
        frames.push(phys);
        Some(Self { prod: Producer::new(phys, RING_TRBS), virt: direct() + phys })
    }

    /// Writes a TRB; returns its physical address.
    fn push(&mut self, t: Trb) -> u64 {
        let p = self.prod.push(t);
        write_trb(self.virt + 16 * p.index as u64, p.trb);
        if let Some((i, link)) = p.link {
            write_trb(self.virt + 16 * i as u64, link);
        }
        p.address
    }

    /// Where the next TRB will go, with the cycle bit (for Set TR Dequeue Pointer).
    fn next(&self) -> (u64, bool) {
        (self.prod.base + 16 * self.prod.enqueue as u64, self.prod.cycle)
    }
}

enum Role {
    Keyboard { report: hid::Keyboard, pipeline: nova_kbd::Pipeline },
    Mouse,
}

struct Endpoint {
    dci: u8,
    ring: Ring,
    /// Page holding the report buffers (physical).
    buffers: u64,
    report_len: u32,
    role: Role,
    /// Needs a reset after an error.
    halted: bool,
    errors: u32,
    seen: bool,
}

struct Device {
    slot: u8,
    port: u8,
    speed: Speed,
    name: String,
    ep0: Ring,
    /// Data page for control transfers (physical).
    control_buf: u64,
    input: u64,  // input context (physical)
    output: u64, // device context (physical)
    endpoints: Vec<Endpoint>,
    frames: Vec<u64>,
}

struct Controller {
    address: String,
    op: u64,
    rt: u64,
    db: u64,
    max_slots: u8,
    ports: u8,
    usb3: Vec<bool>,
    layout: Layout,
    below_4g: bool,
    dcbaa: u64, // virtual
    cmd: Ring,
    events: Consumer,
    events_virt: u64,
    cmd_done: Option<Trb>,
    xfer_done: Option<Trb>,
    port_changes: Vec<u8>,
    devices: Vec<Device>,
}

static CONTROLLERS: KMutex<Vec<Controller>> = KMutex::new(Vec::new());

fn now_ms() -> u64 {
    interrupts::ticks() * 1000 / interrupts::TICK_HZ as u64
}

impl Controller {
    fn doorbell(&self, slot: u8, target: u8) {
        fence(Ordering::SeqCst);
        wr(self.db, slot as usize * 4, target as u32);
    }

    fn portsc_off(p: u8) -> usize {
        PORTS + 0x10 * (p as usize - 1)
    }

    fn portsc(&self, p: u8) -> u32 {
        rd(self.op, Self::portsc_off(p))
    }

    fn set_port(&self, p: u8, bits: u32) {
        let v = self.portsc(p);
        wr(self.op, Self::portsc_off(p), (v & PORT_NEUTRAL) | bits);
    }

    fn clear_port_changes(&self, p: u8) {
        let v = self.portsc(p);
        wr(self.op, Self::portsc_off(p), (v & PORT_NEUTRAL) | (v & PORT_CHANGES));
    }

    /// Drains the event ring: command completions and control-transfer results
    /// are kept for whoever waits for them; keyboard/mouse reports are handled
    /// right away; port changes are queued for [`service`].
    fn poll(&mut self) {
        let mut dequeue = None;
        loop {
            let ev = read_trb(self.events_virt + 16 * self.events.dequeue as u64);
            if !self.events.is_new(&ev) {
                break;
            }
            dequeue = Some(self.events.advance());
            match ev.kind() {
                trb::COMMAND_COMPLETION => self.cmd_done = Some(ev),
                trb::TRANSFER_EVENT if ev.endpoint() == 1 => self.xfer_done = Some(ev),
                trb::TRANSFER_EVENT => self.report(ev),
                trb::PORT_STATUS_CHANGE if !self.port_changes.contains(&ev.port()) => self.port_changes.push(ev.port()),
                trb::HOST_CONTROLLER_EVENT => crate::kprintln!("xhci: {}: controller event: {}", self.address, trb::describe(ev.completion_code())),
                _ => {}
            }
        }
        // Tell the controller how far we got (and clear Event Handler Busy).
        if let Some(next) = dequeue {
            wr64(self.rt, ERDP, next | ERDP_BUSY);
        }
    }

    /// A keyboard or mouse report arrived (or failed).
    fn report(&mut self, ev: Trb) {
        let Some(dev) = self.devices.iter_mut().find(|d| d.slot == ev.slot()) else { return };
        let slot = dev.slot;
        let name = dev.name.clone();
        let Some(ep) = dev.endpoints.iter_mut().find(|e| e.dci == ev.endpoint()) else { return };
        let code = ev.completion_code();
        if code != trb::CC_SUCCESS && code != trb::CC_SHORT_PACKET {
            ep.errors += 1;
            if ep.errors <= 3 {
                crate::kprintln!("usb: {name}: transfer failed: {}", trb::describe(code));
            }
            ep.halted = true;
            return;
        }
        // The TRB that finished tells us which buffer and how much was asked for.
        let done = read_trb(direct() + ev.pointer());
        let buffer = done.pointer();
        let asked = done.0[2] & 0x1_FFFF;
        let got = asked.saturating_sub(ev.residual()) as usize;
        // SAFETY: `buffer` is inside this endpoint's report page (we queued it).
        let data = unsafe { core::slice::from_raw_parts((direct() + buffer) as *const u8, got.min(REPORT_BYTES as usize)) };
        match &mut ep.role {
            Role::Keyboard { report, pipeline } => {
                let mut typed = false;
                report.report(data, now_ms(), &mut |b| {
                    if let Some(input) = pipeline.feed(b) {
                        ps2::push_typed(input);
                        typed = true;
                    }
                });
                if typed && !ep.seen {
                    ep.seen = true;
                    crate::kprintln!("usb: first key from {name}");
                }
            }
            Role::Mouse => {
                if let Some(m) = hid::mouse_report(data) {
                    ps2::push_mouse(m);
                    if !ep.seen {
                        ep.seen = true;
                        crate::kprintln!("usb: first report from {name}");
                    }
                }
            }
        }
        // Queue the buffer again.
        let dci = ep.dci;
        ep.ring.push(Trb::normal(buffer, asked));
        self.doorbell(slot, dci);
    }

    /// Runs one command and waits for its completion event.
    fn command(&mut self, t: Trb) -> Result<Trb, &'static str> {
        self.cmd_done = None;
        let addr = self.cmd.push(t);
        self.doorbell(0, 0);
        let mut done = None;
        interrupts::wait_until(2000, || {
            self.poll();
            done = self.cmd_done.filter(|e| e.pointer() == addr);
            done.is_some()
        });
        let ev = done.ok_or("the controller did not answer a command")?;
        match ev.completion_code() {
            trb::CC_SUCCESS => Ok(ev),
            c => Err(trb::describe(c)),
        }
    }

    /// A control transfer on endpoint 0 of device `d`; data goes through its control page.
    fn control(&mut self, d: usize, request_type: u8, request: u8, value: u16, index: u16, length: u16) -> Result<(), &'static str> {
        let data_in = (length > 0).then_some(request_type & 0x80 != 0);
        let dev = &mut self.devices[d];
        let slot = dev.slot;
        dev.ep0.push(Trb::setup(request_type, request, value, index, length, data_in));
        if let Some(dir) = data_in {
            dev.ep0.push(Trb::data(dev.control_buf, length as u32, dir));
        }
        dev.ep0.push(Trb::status(data_in));
        self.xfer_done = None;
        self.doorbell(slot, 1);
        let mut done = None;
        interrupts::wait_until(1000, || {
            self.poll();
            done = self.xfer_done.filter(|e| e.slot() == slot);
            done.is_some()
        });
        let result = match done.map(|e| e.completion_code()) {
            Some(trb::CC_SUCCESS | trb::CC_SHORT_PACKET) => return Ok(()),
            Some(c) => Err(trb::describe(c)),
            None => Err("the device did not answer"),
        };
        // A stalled/failed control endpoint must be reset before it can be used again.
        self.recover(d, 1);
        result
    }

    /// Resets a halted endpoint and points it past everything that was queued on it.
    fn recover(&mut self, d: usize, ep_dci: u8) {
        let slot = self.devices[d].slot;
        let _ = self.command(Trb::reset_endpoint(slot, ep_dci));
        let dev = &self.devices[d];
        let (ptr, cycle) = if ep_dci == 1 { dev.ep0.next() } else { dev.endpoints.iter().find(|e| e.dci == ep_dci).map(|e| e.ring.next()).unwrap_or((0, true)) };
        let _ = self.command(Trb::set_tr_dequeue(slot, ep_dci, ptr, cycle));
    }

    fn control_data(&self, d: usize, len: usize) -> &[u8] {
        // SAFETY: the control page is ours, 4 KiB.
        unsafe { core::slice::from_raw_parts((direct() + self.devices[d].control_buf) as *const u8, len.min(4096)) }
    }

    fn get_descriptor(&mut self, d: usize, kind: u8, index: u8, lang: u16, len: u16) -> Result<(), &'static str> {
        self.control(d, 0x80, desc::GET_DESCRIPTOR, (kind as u16) << 8 | index as u16, lang, len)
    }

    /// Resets/enables root port `p` and returns the speed of what is attached.
    fn reset_port(&mut self, p: u8) -> Result<Speed, &'static str> {
        if self.portsc(p) & PORT_CONNECTED == 0 {
            return Err("nothing connected");
        }
        let op = self.op;
        let off = Self::portsc_off(p);
        if !self.usb3[p as usize] && self.portsc(p) & PORT_ENABLED == 0 {
            self.set_port(p, PORT_RESET);
            if !interrupts::wait_until(500, || rd(op, off) & PORT_RESET_CHANGE != 0) {
                return Err("port reset did not finish");
            }
        }
        // USB 3 ports enable themselves once the link is trained.
        if !interrupts::wait_until(500, || rd(op, off) & PORT_ENABLED != 0) {
            return Err("the port did not become enabled");
        }
        self.clear_port_changes(p);
        sched::sleep_ms(10); // reset recovery time
        Speed::from_portsc((self.portsc(p) >> 10) & 0xF).ok_or("unknown speed")
    }

    /// Brings up the device on port `p`. Returns a description for the log.
    fn attach(&mut self, p: u8) -> Result<String, &'static str> {
        let speed = self.reset_port(p)?;
        let ev = self.command(Trb::enable_slot())?;
        let slot = ev.slot();
        if slot == 0 || slot > self.max_slots {
            return Err("no device slot");
        }
        let mut frames = Vec::new();
        let b4 = self.below_4g;
        let grab = |frames: &mut Vec<u64>| {
            let f = pmm::alloc_dma_frame(b4)?;
            frames.push(f);
            Some(f)
        };
        let setup = (|| {
            let output = grab(&mut frames)?;
            let input = grab(&mut frames)?;
            let control_buf = grab(&mut frames)?;
            let ep0 = Ring::new(&mut frames, b4)?;
            Some((output, input, control_buf, ep0))
        })();
        let Some((output, input, control_buf, ep0)) = setup else {
            free_frames(&frames);
            let _ = self.command(Trb::disable_slot(slot));
            return Err("out of memory");
        };
        let d = self.devices.len();
        self.devices.push(Device { slot, port: p, speed, name: alloc::format!("port {p} device"), ep0, control_buf, input, output, endpoints: Vec::new(), frames });
        match self.configure(d) {
            Ok(text) => Ok(text),
            Err(e) => {
                self.detach_index(d, false);
                Err(e)
            }
        }
    }

    fn configure(&mut self, d: usize) -> Result<String, &'static str> {
        let (slot, port, speed, input, output) = {
            let dev = &self.devices[d];
            (dev.slot, dev.port, dev.speed, dev.input, dev.output)
        };
        let l = self.layout;
        let iv = direct() + input;
        wr64(self.dcbaa, slot as usize * 8, output);

        // Address Device.
        let mut mps0 = speed.default_control_packet();
        // SAFETY: the input context page is ours.
        unsafe { core::ptr::write_bytes(iv as *mut u8, 0, 4096) };
        write_context(iv + l.control() as u64, context::input_control(0b11));
        write_context(iv + l.input_slot() as u64, context::slot(speed, port, 1));
        write_context(iv + l.input_endpoint(1) as u64, context::control_endpoint(mps0, self.devices[d].ep0.prod.base));
        self.command(Trb::address_device(input, slot)).map_err(|_| "Address Device failed")?;

        // The first 8 bytes of the device descriptor hold the real endpoint 0 packet size.
        self.get_descriptor(d, desc::DEVICE, 0, 0, 8).map_err(|_| "could not read the device descriptor")?;
        let first = desc::Device::parse(self.control_data(d, 8)).ok_or("bad device descriptor")?;
        if first.max_packet0 != mps0 && first.max_packet0 >= 8 {
            mps0 = first.max_packet0;
            // SAFETY: as above.
            unsafe { core::ptr::write_bytes(iv as *mut u8, 0, 4096) };
            write_context(iv + l.control() as u64, context::input_control(0b10));
            write_context(iv + l.input_endpoint(1) as u64, context::control_endpoint(mps0, 0));
            self.command(Trb::evaluate_context(input, slot)).map_err(|_| "Evaluate Context failed")?;
        }
        self.get_descriptor(d, desc::DEVICE, 0, 0, 18).map_err(|_| "could not read the device descriptor")?;
        let dev_desc = desc::Device::parse(self.control_data(d, 18)).ok_or("bad device descriptor")?;

        // Name: the product string if there is one.
        let mut name = alloc::format!("{:04x}:{:04x}", dev_desc.vendor, dev_desc.product);
        if dev_desc.product_string != 0 && self.get_descriptor(d, desc::STRING, dev_desc.product_string, 0x0409, 64).is_ok() {
            let mut text = [0u8; 40];
            let n = desc::string(self.control_data(d, 64), &mut text);
            if n > 0 {
                name = String::from(core::str::from_utf8(&text[..n]).unwrap_or("?").trim());
            }
        }
        self.devices[d].name = name.clone();
        let ids = alloc::format!("{:04x}:{:04x}", dev_desc.vendor, dev_desc.product);

        // Configuration descriptor: header, then the whole thing.
        self.get_descriptor(d, desc::CONFIGURATION, 0, 0, 9).map_err(|_| "could not read the configuration")?;
        let total = desc::config_total_length(self.control_data(d, 9)).ok_or("bad configuration descriptor")?.min(4096);
        self.get_descriptor(d, desc::CONFIGURATION, 0, 0, total).map_err(|_| "could not read the configuration")?;
        let cfg = desc::parse_config(self.control_data(d, total as usize)).ok_or("bad configuration descriptor")?;
        if dev_desc.class == desc::CLASS_HUB || cfg.hub {
            return Err("USB hubs are not supported yet (plug the keyboard/mouse straight into the PC)");
        }
        let boot: Vec<_> = cfg.boot.iter().flatten().copied().collect();
        if boot.is_empty() {
            crate::kprintln!("usb: port {port}: {name} ({ids}), {}: not a keyboard or mouse, no driver", speed.name());
            return Err("no driver for this device");
        }

        self.control(d, 0x00, desc::SET_CONFIGURATION, cfg.value as u16, 0, 0).map_err(|_| "Set Configuration failed")?;
        for b in &boot {
            // Boot protocol (fixed report layout). Some devices refuse SET_IDLE; that's fine.
            let _ = self.control(d, 0x21, desc::HID_SET_PROTOCOL, 0, b.interface as u16, 0);
            if b.kind == BootKind::Keyboard {
                let _ = self.control(d, 0x21, desc::HID_SET_IDLE, 0, b.interface as u16, 0);
            }
        }

        // One interrupt-IN endpoint per boot interface.
        let b4 = self.below_4g;
        let mut add = 1u32;
        let mut max_dci = 1;
        let mut endpoints = Vec::new();
        for b in &boot {
            let ep_dci = dci(b.endpoint);
            let mut frames = Vec::new();
            let (Some(ring), Some(buffers)) = (Ring::new(&mut frames, b4), pmm::alloc_dma_frame(b4)) else {
                free_frames(&frames);
                return Err("out of memory");
            };
            frames.push(buffers);
            self.devices[d].frames.extend(frames);
            add |= 1 << ep_dci;
            max_dci = max_dci.max(ep_dci);
            let role = match b.kind {
                BootKind::Keyboard => Role::Keyboard { report: hid::Keyboard::new(), pipeline: nova_kbd::Pipeline::new() },
                BootKind::Mouse => Role::Mouse,
            };
            let report_len = (b.max_packet as u32).clamp(1, REPORT_BYTES as u32);
            endpoints.push((Endpoint { dci: ep_dci, ring, buffers, report_len, role, halted: false, errors: 0, seen: false }, b.max_packet, b.interval));
        }
        // SAFETY: as above.
        unsafe { core::ptr::write_bytes(iv as *mut u8, 0, 4096) };
        write_context(iv + l.control() as u64, context::input_control(add));
        let mut slot_ctx = read_context(direct() + output);
        slot_ctx[0] = (slot_ctx[0] & !(0x1F << 27)) | (max_dci as u32) << 27;
        slot_ctx[3] = 0;
        write_context(iv + l.input_slot() as u64, slot_ctx);
        for (ep, mps, interval) in &endpoints {
            write_context(iv + l.input_endpoint(ep.dci) as u64, context::interrupt_in_endpoint(*mps, xhci_interval(speed, *interval), ep.ring.prod.base));
        }
        self.command(Trb::configure_endpoint(input, slot)).map_err(|_| "Configure Endpoint failed")?;

        let mut kinds = Vec::new();
        for (mut ep, ..) in endpoints {
            for i in 0..QUEUED_REPORTS as u64 {
                ep.ring.push(Trb::normal(ep.buffers + i * REPORT_BYTES, ep.report_len));
            }
            kinds.push(match ep.role {
                Role::Keyboard { .. } => "keyboard",
                Role::Mouse => "mouse",
            });
            let ep_dci = ep.dci;
            self.devices[d].endpoints.push(ep);
            self.doorbell(slot, ep_dci);
        }
        Ok(alloc::format!("{name} ({ids}), {}: {}", speed.name(), kinds.join(" + ")))
    }

    /// Forgets device `d`: releases held keys, frees its slot and memory.
    fn detach_index(&mut self, d: usize, log: bool) {
        let mut dev = self.devices.remove(d);
        for ep in dev.endpoints.iter_mut() {
            if let Role::Keyboard { report, pipeline } = &mut ep.role {
                report.release_all(&mut |b| {
                    if let Some(input) = pipeline.feed(b) {
                        ps2::push_typed(input);
                    }
                });
            }
        }
        let _ = self.command(Trb::disable_slot(dev.slot));
        wr64(self.dcbaa, dev.slot as usize * 8, 0);
        free_frames(&dev.frames);
        if log {
            crate::kprintln!("usb: port {}: {} unplugged", dev.port, dev.name);
        }
    }

    fn attach_and_log(&mut self, p: u8) {
        match self.attach(p) {
            Ok(text) => crate::kprintln!("usb: port {p}: {text}"),
            Err("no driver for this device") => {}
            Err(e) => crate::kprintln!("usb: port {p}: {e}"),
        }
    }

    /// Port changes, endpoint recovery, key repeat. Called from the `usb` thread.
    fn service(&mut self) {
        self.poll();
        while let Some(p) = self.port_changes.pop() {
            if p == 0 || p > self.ports {
                continue;
            }
            let v = self.portsc(p);
            self.clear_port_changes(p);
            let known = self.devices.iter().position(|d| d.port == p);
            let connected = v & PORT_CONNECTED != 0;
            if let Some(d) = known
                && (!connected || v & PORT_CONNECT_CHANGE != 0)
            {
                self.detach_index(d, true);
            }
            if connected && (known.is_none() || v & PORT_CONNECT_CHANGE != 0) {
                self.attach_and_log(p);
            }
        }
        // Endpoints that failed: reset them and queue fresh transfers.
        for d in 0..self.devices.len() {
            for e in 0..self.devices[d].endpoints.len() {
                if !self.devices[d].endpoints[e].halted {
                    continue;
                }
                let ep_dci = self.devices[d].endpoints[e].dci;
                self.recover(d, ep_dci);
                let slot = self.devices[d].slot;
                let ep = &mut self.devices[d].endpoints[e];
                ep.halted = false;
                if ep.errors > 50 {
                    continue; // give up on a device that keeps failing
                }
                for i in 0..QUEUED_REPORTS as u64 {
                    ep.ring.push(Trb::normal(ep.buffers + i * REPORT_BYTES, ep.report_len));
                }
                self.doorbell(slot, ep_dci);
            }
        }
        // Held keys repeat.
        let now = now_ms();
        for dev in self.devices.iter_mut() {
            for ep in dev.endpoints.iter_mut() {
                if let Role::Keyboard { report, pipeline } = &mut ep.role {
                    report.tick(now, &mut |b| {
                        if let Some(input) = pipeline.feed(b) {
                            ps2::push_typed(input);
                        }
                    });
                }
            }
        }
    }
}

fn free_frames(frames: &[u64]) {
    for &f in frames {
        let _ = pmm::free_frame(f);
    }
}

/// The firmware may be using the controller (to pretend a USB keyboard is a
/// PS/2 one); ask it to let go, and turn off its SMIs.
fn take_over_from_firmware(cap: u64, hcc1: u32, ports: u8, usb3: &mut [bool]) {
    let mut off = ((hcc1 >> 16) as usize) * 4;
    let mut guard = 0;
    while off != 0 && guard < 64 {
        guard += 1;
        let v = rd(cap, off);
        match v & 0xFF {
            1 => {
                if v & 1 << 16 != 0 {
                    wr(cap, off, v | 1 << 24);
                    if !interrupts::wait_until(1000, || rd(cap, off) & 1 << 16 == 0) {
                        crate::kprintln!("xhci: the firmware did not let go of the controller; taking it anyway");
                        wr(cap, off, (rd(cap, off) & !(1 << 16)) | 1 << 24);
                    }
                }
                let ctl = rd(cap, off + 4);
                wr(cap, off + 4, (ctl & !((7 << 1) | (0xFF << 5) | (7 << 17))) | 7 << 29);
            }
            2 => {
                let major = v >> 24;
                let ports_word = rd(cap, off + 8);
                let first = (ports_word & 0xFF) as usize;
                let count = ((ports_word >> 8) & 0xFF) as usize;
                for flag in usb3.iter_mut().take(ports as usize + 1).skip(first.max(1)).take(count) {
                    *flag = major >= 3;
                }
            }
            _ => {}
        }
        let next = ((v >> 8) & 0xFF) as usize;
        if next == 0 {
            break;
        }
        off += next * 4;
    }
}

fn start(dev: &pci::PciDevice) -> Result<Controller, &'static str> {
    let (bar, size) = pci::bar_memory_size(dev, 0).ok_or("no register BAR")?;
    pci::enable_dma(dev);
    pci::disable_intx(dev);
    let cap = vm::map_mmio(bar, size.min(1 << 20)).map_err(|_| "could not map the registers")?;
    let caplen = rd(cap, CAP_LENGTH) & 0xFF;
    let hcs1 = rd(cap, HCS_PARAMS1);
    let hcs2 = rd(cap, HCS_PARAMS2);
    let hcc1 = rd(cap, HCC_PARAMS1);
    let op = cap + caplen as u64;
    let rt = cap + (rd(cap, RTS_OFF) & !0x1F) as u64;
    let db = cap + (rd(cap, DB_OFF) & !0x3) as u64;
    let max_slots = (hcs1 & 0xFF) as u8;
    let ports = (hcs1 >> 24) as u8;
    let below_4g = hcc1 & 1 == 0;
    let mut usb3 = alloc::vec![false; ports as usize + 1];

    take_over_from_firmware(cap, hcc1, ports, &mut usb3);

    // Stop, then reset.
    wr(op, USBCMD, rd(op, USBCMD) & !CMD_RUN);
    if !interrupts::wait_until(100, || rd(op, USBSTS) & STS_HALTED != 0) {
        return Err("would not stop");
    }
    wr(op, USBCMD, CMD_RESET);
    sched::sleep_ms(2); // some Intel controllers must not be touched right after a reset
    if !interrupts::wait_until(1000, || rd(op, USBCMD) & CMD_RESET == 0 && rd(op, USBSTS) & STS_NOT_READY == 0) {
        return Err("would not reset");
    }

    let mut frames = Vec::new();
    let mut page = || -> Result<u64, &'static str> {
        let f = pmm::alloc_dma_frame(below_4g).ok_or("out of DMA memory")?;
        frames.push(f);
        Ok(f)
    };
    // Device context base address array (+ scratchpad pages for the controller's own use).
    let dcbaa = page()?;
    let scratch = ((hcs2 >> 21) & 0x1F) << 5 | (hcs2 >> 27) & 0x1F;
    if scratch > 0 {
        if scratch > 512 {
            return Err("wants too many scratchpad pages");
        }
        let list = page()?;
        for i in 0..scratch as usize {
            let p = page()?;
            wr64(direct() + list, i * 8, p);
        }
        wr64(direct() + dcbaa, 0, list);
    }
    let event_ring = page()?;
    let erst = page()?;
    let cmd = Ring::new(&mut frames, below_4g).ok_or("out of DMA memory")?;

    wr(op, CONFIG, max_slots as u32);
    wr64(op, DCBAAP, dcbaa);
    wr64(op, CRCR, cmd.prod.base | 1);
    // One event ring segment.
    let ev = direct() + erst;
    wr64(ev, 0, event_ring);
    wr(ev, 8, EVENT_TRBS as u32);
    wr(rt, IMAN, rd(rt, IMAN) & !2); // interrupter 0: no interrupts (we poll)
    wr(rt, ERSTSZ, 1);
    wr64(rt, ERDP, event_ring);
    wr64(rt, ERSTBA, erst);

    wr(op, USBCMD, CMD_RUN);
    if !interrupts::wait_until(100, || rd(op, USBSTS) & STS_HALTED == 0) {
        return Err("would not start");
    }

    let c = Controller {
        address: dev.address(),
        op,
        rt,
        db,
        max_slots,
        ports,
        usb3,
        layout: Layout::new(hcc1 & 1 << 2 != 0),
        below_4g,
        dcbaa: direct() + dcbaa,
        cmd,
        events: Consumer::new(event_ring, EVENT_TRBS),
        events_virt: direct() + event_ring,
        cmd_done: None,
        xfer_done: None,
        port_changes: Vec::new(),
        devices: Vec::new(),
    };
    // Port power (only matters on controllers with port power control).
    if hcc1 & 1 << 3 != 0 {
        for p in 1..=ports {
            if c.portsc(p) & PORT_POWER == 0 {
                c.set_port(p, PORT_POWER);
            }
        }
        sched::sleep_ms(20);
    }
    Ok(c)
}

/// What was found at boot.
pub struct Found {
    pub controllers: usize,
    pub keyboards: usize,
    pub mice: usize,
}

/// Starts every xHCI controller and the devices on its ports. Needs the
/// scheduler (it sleeps while ports reset).
pub fn init(devices: &[pci::PciDevice]) -> Found {
    let mut found = Found { controllers: 0, keyboards: 0, mice: 0 };
    let mut list = CONTROLLERS.lock();
    for dev in devices.iter().filter(|d| d.class == 0x0C && d.subclass == 0x03 && d.prog_if == 0x30) {
        let mut c = match start(dev) {
            Ok(c) => c,
            Err(e) => {
                crate::kprintln!("xhci: {}: {e}", dev.address());
                continue;
            }
        };
        let usb3 = c.usb3.iter().filter(|&&b| b).count();
        crate::kprintln!("xhci: {}: running, {} ports ({} USB 3), {} device slots{}", c.address, c.ports, usb3, c.max_slots, if c.below_4g { ", 32-bit DMA" } else { "" });
        for p in 1..=c.ports {
            if c.portsc(p) & PORT_CONNECTED != 0 {
                c.attach_and_log(p);
            }
        }
        // The start-up attaches answered their own port change events.
        c.poll();
        let pending: Vec<u8> = c.port_changes.iter().copied().filter(|&p| c.portsc(p) & PORT_CHANGES != 0).collect();
        c.port_changes = pending;
        for d in &c.devices {
            for e in &d.endpoints {
                match e.role {
                    Role::Keyboard { .. } => found.keyboards += 1,
                    Role::Mouse => found.mice += 1,
                }
            }
        }
        found.controllers += 1;
        list.push(c);
    }
    found
}

/// Starts the thread that watches the controllers (reports, plugging, key repeat).
pub fn start_service() {
    if CONTROLLERS.lock().is_empty() {
        return;
    }
    let spawned = sched::spawn("usb", || {
        loop {
            for c in CONTROLLERS.lock().iter_mut() {
                c.service();
            }
            sched::sleep_ms(10);
        }
    });
    if spawned.is_none() {
        crate::kprintln!("usb: could not start the usb thread");
    }
}

/// Lines describing the attached devices (for the shell).
pub fn describe() -> Vec<String> {
    let mut out = Vec::new();
    for c in CONTROLLERS.lock().iter() {
        out.push(alloc::format!("controller {}: {} ports", c.address, c.ports));
        if c.devices.is_empty() {
            out.push(String::from("  (no keyboards or mice attached)"));
        }
        for d in &c.devices {
            let kinds: Vec<&str> = d.endpoints.iter().map(|e| if matches!(e.role, Role::Keyboard { .. }) { "keyboard" } else { "mouse" }).collect();
            out.push(alloc::format!("  port {}: {}, {}, {}", d.port, d.name, d.speed.name(), kinds.join(" + ")));
        }
    }
    out
}
