//! The PS/2 controller (i8042): the keyboard on its first port, the mouse on
//! its second.
//!
//! Normally interrupt driven: the keyboard raises ISA IRQ 1 and the mouse IRQ
//! 12; either handler drains the controller (one output buffer, shared by
//! both devices, flagged by a status bit) into small event queues. If the
//! machine has no APIC the same code can be polled with [`pump`].
//!
//! Scancode decoding/layout (`crates/kbd`) and mouse packet decoding
//! (`crates/ps2`) are host-tested; this file talks to the hardware.

use crate::log::SpinLock;
use core::sync::atomic::{AtomicUsize, Ordering};
use nova_kbd::Input;
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
const CMD_ENABLE_KBD_PORT: u8 = 0xAE;
const CMD_TO_MOUSE: u8 = 0xD4;
/// Pulses the CPU reset line (the classic way to reboot a PC).
const CMD_RESET_CPU: u8 = 0xFE;

const ACK: u8 = 0xFA;
const RESEND: u8 = 0xFE;

/// Config byte bits we clear: IRQ1, IRQ12, both port clock-disables, and
/// scancode translation.
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

/// Typed input waiting for the shell (oldest first; the oldest are dropped
/// if it falls behind).
struct KeyQueue {
    buf: [Option<Input>; 64],
    head: usize,
    len: usize,
}

static KEYS: SpinLock<KeyQueue> = SpinLock::new(KeyQueue { buf: [None; 64], head: 0, len: 0 });
static KBD: SpinLock<nova_kbd::Pipeline> = SpinLock::new(nova_kbd::Pipeline::new());
/// Thread to wake when typed input arrives (usize::MAX = none).
static KEY_WAITER: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Registers the thread that is woken (`sched::unpark`) whenever a key is typed.
pub fn set_key_waiter(thread: usize) {
    KEY_WAITER.store(thread, Ordering::SeqCst);
}

/// The oldest typed input, if any.
pub fn pop_key() -> Option<Input> {
    KEYS.with(|k| {
        if k.len == 0 {
            return None;
        }
        let v = k.buf[k.head].take();
        k.head = (k.head + 1) % k.buf.len();
        k.len -= 1;
        v
    })
}

fn push_key(input: Input) {
    KEYS.with(|k| {
        let cap = k.buf.len();
        if k.len == cap {
            k.head = (k.head + 1) % cap;
            k.len -= 1;
        }
        let at = (k.head + k.len) % cap;
        k.buf[at] = Some(input);
        k.len += 1;
    });
}

/// Reads whatever the keyboard and mouse have sent into their queues.
pub fn pump() {
    let mut typed = false;
    drain(
        |ev| QUEUE.with(|q| q.push(ev)),
        |input| {
            push_key(input);
            typed = true;
        },
    );
    if typed {
        let w = KEY_WAITER.load(Ordering::SeqCst);
        if w != usize::MAX {
            crate::sched::unpark(w);
        }
    }
}

/// The IRQ 1 / IRQ 12 handler.
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

/// What [`init`] found. Each device is independent: a PC may have only one.
pub struct Devices {
    pub mouse: Result<MouseInfo, &'static str>,
    pub keyboard: Result<(), &'static str>,
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

/// Resets the keyboard and starts it scanning. The controller translates the
/// keyboard's native scancodes to set 1 (config bit 6), which is what
/// `crates/kbd` decodes.
fn init_keyboard() -> Result<(), &'static str> {
    command(CMD_ENABLE_KBD_PORT)?;
    write_data(0xFF)?; // reset
    if read_data() != Some(ACK) {
        return Err("the keyboard did not acknowledge reset (no keyboard attached?)");
    }
    if read_data() != Some(0xAA) {
        return Err("the keyboard failed its self-test");
    }
    write_data(0xF4)?; // enable scanning
    if read_data() != Some(ACK) {
        return Err("the keyboard did not acknowledge enable-scanning");
    }
    Ok(())
}

/// Resets the mouse and starts it reporting.
fn init_mouse() -> Result<MouseInfo, &'static str> {
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
    Ok(MouseInfo { id })
}

/// Initialises the controller, then the keyboard and the mouse.
/// With `irq` the controller is also told to raise IRQ 1 / IRQ 12 (the caller
/// must already have routed and registered them); otherwise use [`pump`].
///
/// # Safety
/// Call once; the kernel must own the PS/2 controller (I/O ports 0x60/0x64).
pub unsafe fn init(irq: bool) -> Result<Devices, &'static str> {
    // Quiet both ports while we configure the controller.
    command(CMD_DISABLE_KBD_PORT)?;
    command(CMD_DISABLE_MOUSE_PORT)?;
    flush();

    // No interrupts yet, both clocks running; scancode translation ON (set 1).
    let cfg = (read_config()? & !CONFIG_CLEAR) | (1 << 6);
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

    let keyboard = init_keyboard();
    let mouse = init_mouse();
    flush();
    // Turn on the interrupts of the devices that came up.
    let mut final_cfg = cfg;
    if irq && keyboard.is_ok() {
        final_cfg |= 1 << 0;
    }
    if irq && mouse.is_ok() {
        final_cfg |= 1 << 1;
    }
    write_config(final_cfg)?;
    Ok(Devices { mouse, keyboard })
}

/// Resets the machine through the keyboard controller. Returns only if that failed.
pub fn reset_cpu() {
    let _ = command(CMD_RESET_CPU);
}

/// Reads everything waiting in the controller and routes each byte to the
/// keyboard or mouse decoder (the status register says which it came from).
fn drain(mut mouse: impl FnMut(MouseEvent), mut key: impl FnMut(Input)) {
    for _ in 0..256 {
        // SAFETY: reading the status/data ports of the controller we own.
        let status = unsafe { inb(STATUS) };
        if status & ST_OUTPUT_FULL == 0 {
            return;
        }
        // SAFETY: as above.
        let byte = unsafe { inb(DATA) };
        if status & ST_FROM_MOUSE != 0 {
            if let Some(ev) = DECODER.with(|d| d.feed(byte)) {
                mouse(ev);
            }
        } else if let Some(input) = KBD.with(|k| k.feed(byte)) {
            key(input);
        }
    }
}
