//! A 16550 UART driver (COM1 by default). Used for logging: QEMU forwards
//! COM1 to a file or the terminal, which is how the boot test reads output.

use crate::{inb, outb};
use core::fmt;

pub const COM1: u16 = 0x3F8;

/// How many status polls to wait for the transmitter before giving up. On a
/// machine with no UART the line-status register reads 0xFF or never becomes
/// ready; giving up keeps logging from hanging the boot.
const SPIN_LIMIT: u32 = 100_000;

pub struct Serial {
    base: u16,
    present: bool,
}

impl Serial {
    /// A port that has not been initialised yet; writes are dropped.
    pub const fn uninit(base: u16) -> Self {
        Self { base, present: false }
    }

    /// Initialises the UART at 115200 baud, 8 data bits, no parity, 1 stop bit.
    ///
    /// # Safety
    /// The caller must own the UART at `base`.
    pub unsafe fn init(base: u16) -> Self {
        unsafe {
            outb(base + 1, 0x00); // no interrupts
            outb(base + 3, 0x80); // DLAB on: next two bytes are the divisor
            outb(base, 0x01); // divisor 1 = 115200 baud
            outb(base + 1, 0x00);
            outb(base + 3, 0x03); // 8N1, DLAB off
            outb(base + 2, 0xC7); // enable + clear FIFOs, 14-byte threshold
            outb(base + 4, 0x0B); // DTR, RTS, OUT2

            // Loopback self-test: a missing UART will not echo the byte.
            outb(base + 4, 0x1E);
            outb(base, 0xAE);
            let present = inb(base) == 0xAE;
            outb(base + 4, 0x0F); // normal operation
            Self { base, present }
        }
    }

    pub fn is_present(&self) -> bool {
        self.present
    }

    pub fn write_byte(&mut self, byte: u8) {
        if !self.present {
            return;
        }
        for _ in 0..SPIN_LIMIT {
            // SAFETY: we own this UART (see init).
            if unsafe { inb(self.base + 5) } & 0x20 != 0 {
                unsafe { outb(self.base, byte) };
                return;
            }
            core::hint::spin_loop();
        }
        // Transmitter never became ready: stop trying so logging can't stall boot.
        self.present = false;
    }
}

impl fmt::Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(b);
        }
        Ok(())
    }
}
