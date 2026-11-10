//! The PS/2 controller (i8042) and the mouse on its second port.
//!
//! Normally interrupt driven: the mouse raises ISA IRQ 12, the handler drains
//! the controller into a small event queue, and the main loop sleeps (`hlt`)
//! until something arrives. If the machine has no APIC the same code can be
//! polled with [`pump`]. The keyboard port is left disabled until the
//! keyboard driver exists (stage 6), so the only bytes that show up are mouse
//! bytes.
//!
//! Packet decoding and pointer tracking are in `crates/ps2` (host-tested);
//! this file is the part that talks to the hardware.

use crate::log::SpinLock;
use nova_ps2::{Decoder, MouseEvent};
use nova_x86::{inb, outb};

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64; // read: status, write: command

const ST_OUTPUT_FULL: u8 = 1 << 0;
const ST_INPUT_FULL: u8 = 1 << 1;
const ST_FROM_MOUSE: u8 = 1 << 5;

const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_DISABLE_MOUSE_PORT: u8 = 0xA7;
const CMD_ENABLE_MOUSE_PORT: u8 = 0xA8;
const CMD_TEST_MOUSE_PORT: u8 = 0xA9;
const CMD_SELF_TEST: u8 = 0xAA;
const CMD_DISABLE_KBD_PORT: u8 = 0xAD;
const CMD_TO_MOUSE: u8 = 0xD4;

const ACK: u8 = 0xFA;
const RESEND: u8 = 0xFE;

/// Config byte bits we clear: IRQ1, IRQ12 (we poll), both port clock-disables,
/// and scancode translation.
const CONFIG_CLEAR: u8 = (1 << 0) | (1 << 1) | (1 << 4) | (1 << 5) | (1 << 6);

/// Status polls to wait for the controller before giving up (a missing
/// controller floats the bus; this keeps boot from hanging on such a PC).
const SPIN: u32 = 200_000;

static DECODER: SpinLock<Decoder> = SpinLock::new(Decoder::new(false));

/// Events waiting for the main loop (oldest first). If it falls behind, the
/// oldest are dropped: a pointer that lags is worse than one that skips.
struct Queue {
    buf: [MouseEvent; 64],
    head: usize,
    len: usize,
}

const NO_EVENT: MouseEvent = MouseEvent { dx: 0, dy: 0, wheel: 0, buttons: nova_ps2::Buttons(0) };
static QUEUE: SpinLock<Queue> = SpinLock::new(Queue { buf: [NO_EVENT; 64], head: 0, len: 0 });

impl Queue {
    fn push(&mut self, ev: MouseEvent) {
        let cap = self.buf.len();
        if self.len == cap {
            self.head = (self.head + 1) % cap; // drop the oldest
            self.len -= 1;
        }
        self.buf[(self.head + self.len) % cap] = ev;
        self.len += 1;
    }

    fn pop(&mut self) -> Option<MouseEvent> {
        if self.len == 0 {
            return None;
        }
        let ev = self.buf[self.head];
        self.head = (self.head + 1) % self.buf.len();
        self.len -= 1;
        Some(ev)
    }
}

/// Reads what the mouse has sent into the event queue.
pub fn pump() {
    poll(|ev| QUEUE.with(|q| q.push(ev)));
}

/// The IRQ 12 handler.
pub fn irq_handler() {
    pump();
}

/// The oldest queued mouse event, if any.
pub fn pop() -> Option<MouseEvent> {
    QUEUE.with(|q| q.pop())
}

pub fn has_events() -> bool {
    QUEUE.with(|q| q.len > 0)
}

pub struct MouseInfo {
    /// Device ID: 0 = plain mouse, 3 = wheel, 4 = wheel + 5 buttons.
    pub id: u8,
}

fn wait_input_ready() -> bool {
    // SAFETY: reading the controller's status port; the kernel owns it.
    (0..SPIN).any(|_| unsafe { inb(STATUS) } & ST_INPUT_FULL == 0)
}

fn wait_output_ready() -> bool {
    // SAFETY: as above.
    (0..SPIN).any(|_| unsafe { inb(STATUS) } & ST_OUTPUT_FULL != 0)
}

fn command(cmd: u8) -> Result<(), &'static str> {
    if !wait_input_ready() {
        return Err("PS/2 controller not ready for a command");
    }
    // SAFETY: the kernel owns the PS/2 controller.
    unsafe { outb(STATUS, cmd) };
    Ok(())
}

fn write_data(byte: u8) -> Result<(), &'static str> {
    if !wait_input_ready() {
        return Err("PS/2 controller not ready for data");
    }
    // SAFETY: as above.
    unsafe { outb(DATA, byte) };
    Ok(())
}

fn read_data() -> Option<u8> {
    // SAFETY: as above.
    wait_output_ready().then(|| unsafe { inb(DATA) })
}

/// Throws away anything waiting in the controller's output buffer.
fn flush() {
    for _ in 0..64 {
        // SAFETY: as above.
        unsafe {
            if inb(STATUS) & ST_OUTPUT_FULL == 0 {
                return;
            }
            let _ = inb(DATA);
        }
    }
}

fn read_config() -> Result<u8, &'static str> {
    command(CMD_READ_CONFIG)?;
    read_data().ok_or("no reply to read-configuration")
}

fn write_config(cfg: u8) -> Result<(), &'static str> {
    command(CMD_WRITE_CONFIG)?;
    write_data(cfg)
}

/// Sends a byte to the mouse and returns its reply (retrying on "resend").
fn mouse_send(byte: u8) -> Result<u8, &'static str> {
    for _ in 0..3 {
        command(CMD_TO_MOUSE)?;
        write_data(byte)?;
        match read_data() {
            Some(RESEND) => continue,
            Some(b) => return Ok(b),
            None => return Err("the mouse did not answer"),
        }
    }
    Err("the mouse kept asking for a resend")
}

/// Sends a mouse command and requires an ACK.
fn mouse_cmd(byte: u8) -> Result<(), &'static str> {
    if mouse_send(byte)? == ACK { Ok(()) } else { Err("the mouse did not acknowledge a command") }
}

fn mouse_cmd_arg(cmd: u8, arg: u8) -> Result<(), &'static str> {
    mouse_cmd(cmd)?;
    mouse_cmd(arg)
}

/// Initialises the controller and the mouse and starts mouse reporting.
/// With `irq` the controller is also told to raise IRQ 12 (the caller must
/// already have routed and registered it); otherwise use [`pump`].
///
/// # Safety
/// Call once; the kernel must own the PS/2 controller (I/O ports 0x60/0x64).
pub unsafe fn init(irq: bool) -> Result<MouseInfo, &'static str> {
    // Quiet both ports while we configure the controller.
    command(CMD_DISABLE_KBD_PORT)?;
    command(CMD_DISABLE_MOUSE_PORT)?;
    flush();

    // No interrupts (we poll), no scancode translation, both clocks running.
    let cfg = read_config()? & !CONFIG_CLEAR;
    write_config(cfg)?;

    // Controller self-test. Some controllers reset their configuration here,
    // so the configuration is written again afterwards.
    command(CMD_SELF_TEST)?;
    match read_data() {
        Some(0x55) => {}
        Some(_) => return Err("PS/2 controller failed its self-test"),
        None => return Err("no PS/2 controller answered (this PC may be USB-only)"),
    }
    write_config(cfg)?;

    // Is there a second (mouse) port at all?
    command(CMD_ENABLE_MOUSE_PORT)?;
    if read_config()? & (1 << 5) != 0 {
        return Err("this controller has no mouse port");
    }
    command(CMD_TEST_MOUSE_PORT)?;
    if read_data() != Some(0x00) {
        return Err("the mouse port failed its test");
    }
    flush();

    // Reset the mouse: ACK, then 0xAA (self-test passed), then its device ID.
    if mouse_send(0xFF)? != ACK {
        return Err("the mouse did not acknowledge reset (no mouse attached?)");
    }
    if read_data() != Some(0xAA) {
        return Err("the mouse failed its self-test");
    }
    let _ = read_data(); // device ID (0x00)
    mouse_cmd(0xF6)?; // defaults

    // Try to switch on the scroll wheel (the "magic" sample-rate sequence),
    // then read the device ID to see if it took.
    for rate in [200, 100, 80] {
        mouse_cmd_arg(0xF3, rate)?;
    }
    mouse_cmd(0xF2)?;
    let id = read_data().unwrap_or(0);
    DECODER.with(|d| d.set_wheel(id == 3 || id == 4));

    mouse_cmd_arg(0xF3, 100)?; // 100 reports per second
    mouse_cmd(0xF4)?; // start reporting
    flush();
    if irq {
        write_config(cfg | (1 << 1))?; // let the controller raise IRQ 12
    }
    Ok(MouseInfo { id })
}

/// Reads whatever the mouse has sent and calls `f` with each complete event.
/// Returns immediately if there is nothing to read.
pub fn poll(mut f: impl FnMut(MouseEvent)) {
    for _ in 0..256 {
        // SAFETY: reading the status/data ports of the controller we own.
        let status = unsafe { inb(STATUS) };
        if status & ST_OUTPUT_FULL == 0 {
            return;
        }
        // SAFETY: as above.
        let byte = unsafe { inb(DATA) };
        if status & ST_FROM_MOUSE != 0
            && let Some(ev) = DECODER.with(|d| d.feed(byte))
        {
            f(ev);
        }
    }
}
