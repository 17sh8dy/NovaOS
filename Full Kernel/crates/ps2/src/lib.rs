//! PS/2 mouse logic that doesn't touch hardware: turning the byte stream the
//! mouse sends into movement/button events, and tracking a pointer on screen.
//!
//! The controller driver (port I/O) lives in the kernel; everything here is
//! unit-tested on the host.

#![no_std]

#[cfg(test)]
extern crate std;

/// Which buttons are down, as a bit set: left = 1, right = 2, middle = 4.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Buttons(pub u8);

impl Buttons {
    pub const LEFT: u8 = 1;
    pub const RIGHT: u8 = 2;
    pub const MIDDLE: u8 = 4;

    pub fn left(self) -> bool {
        self.0 & Self::LEFT != 0
    }
    pub fn right(self) -> bool {
        self.0 & Self::RIGHT != 0
    }
    pub fn middle(self) -> bool {
        self.0 & Self::MIDDLE != 0
    }
}

/// One decoded mouse report.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct MouseEvent {
    /// Movement in mouse counts; positive x = right.
    pub dx: i16,
    /// Movement in mouse counts; positive y = **down** (screen orientation;
    /// the wire format is up-positive and is flipped here).
    pub dy: i16,
    /// Scroll wheel: positive = towards the user (down), 0 on mice without one.
    pub wheel: i8,
    pub buttons: Buttons,
}

/// Reassembles 3- or 4-byte mouse packets from the raw byte stream.
pub struct Decoder {
    buf: [u8; 4],
    len: usize,
    packet_len: usize,
    /// Bytes thrown away to get back in step with the packet boundaries.
    pub resyncs: u32,
}

impl Decoder {
    /// `wheel` = the mouse reports 4-byte packets (IntelliMouse, device ID 3 or 4).
    pub const fn new(wheel: bool) -> Self {
        Self { buf: [0; 4], len: 0, packet_len: if wheel { 4 } else { 3 }, resyncs: 0 }
    }

    pub fn set_wheel(&mut self, wheel: bool) {
        self.packet_len = if wheel { 4 } else { 3 };
        self.len = 0;
    }

    /// Feeds one byte; returns an event when a packet is complete.
    pub fn feed(&mut self, byte: u8) -> Option<MouseEvent> {
        // The first byte of every packet has bit 3 set. If it doesn't, we are
        // out of step (a byte was lost): skip until one does.
        if self.len == 0 && byte & 0x08 == 0 {
            self.resyncs += 1;
            return None;
        }
        self.buf[self.len] = byte;
        self.len += 1;
        if self.len < self.packet_len {
            return None;
        }
        self.len = 0;
        Some(self.decode())
    }

    fn decode(&self) -> MouseEvent {
        let status = self.buf[0];
        // 9-bit two's complement: the sign bits live in the status byte.
        let mut dx = self.buf[1] as i16 - ((status as i16) << 4 & 0x100);
        let mut dy = self.buf[2] as i16 - ((status as i16) << 3 & 0x100);
        // Overflow means the counter wrapped and the value is meaningless.
        if status & 0x40 != 0 {
            dx = 0;
        }
        if status & 0x80 != 0 {
            dy = 0;
        }
        let wheel = if self.packet_len == 4 {
            // Low 4 bits, sign-extended. Positive = towards the user.
            (((self.buf[3] & 0x0F) << 4) as i8) >> 4
        } else {
            0
        };
        MouseEvent { dx, dy: -dy, wheel, buttons: Buttons(status & 0x07) }
    }
}

/// What a mouse event changed about the pointer.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Change {
    pub moved: bool,
    /// Buttons that went down / came up with this event.
    pub pressed: Buttons,
    pub released: Buttons,
    pub wheel: i8,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        !self.moved && self.pressed.0 == 0 && self.released.0 == 0 && self.wheel == 0
    }
}

/// A pointer confined to a `width` x `height` screen.
#[derive(Clone, Copy, Debug)]
pub struct Pointer {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub buttons: Buttons,
}

impl Pointer {
    /// A pointer in the middle of the screen.
    pub fn centered(width: usize, height: usize) -> Self {
        let (width, height) = (width.max(1) as i32, height.max(1) as i32);
        Self { x: width / 2, y: height / 2, width, height, buttons: Buttons(0) }
    }

    pub fn apply(&mut self, ev: MouseEvent) -> Change {
        let (ox, oy) = (self.x, self.y);
        self.x = (self.x + ev.dx as i32).clamp(0, self.width - 1);
        self.y = (self.y + ev.dy as i32).clamp(0, self.height - 1);
        let old = self.buttons.0;
        self.buttons = ev.buttons;
        Change {
            moved: (ox, oy) != (self.x, self.y),
            pressed: Buttons(ev.buttons.0 & !old),
            released: Buttons(old & !ev.buttons.0),
            wheel: ev.wheel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(d: &mut Decoder, bytes: &[u8]) -> std::vec::Vec<MouseEvent> {
        bytes.iter().filter_map(|&b| d.feed(b)).collect()
    }

    #[test]
    fn decodes_a_plain_movement_packet() {
        let mut d = Decoder::new(false);
        // status: bit3 set, no buttons; dx = +5, dy = +3 (up on the wire)
        let ev = feed_all(&mut d, &[0x08, 5, 3]);
        assert_eq!(ev, [MouseEvent { dx: 5, dy: -3, wheel: 0, buttons: Buttons(0) }]);
    }

    #[test]
    fn decodes_negative_movement_using_the_sign_bits() {
        let mut d = Decoder::new(false);
        // dx = -1 (0xFF + X sign), dy = -2 (0xFE + Y sign)
        let ev = feed_all(&mut d, &[0x08 | 0x10 | 0x20, 0xFF, 0xFE]);
        assert_eq!((ev[0].dx, ev[0].dy), (-1, 2), "wire y is up-positive, so -2 flips to +2 (down)");
        // dx = -256 .. 255 range edge
        let ev = feed_all(&mut d, &[0x08 | 0x10, 0x00, 0x00]);
        assert_eq!(ev[0].dx, -256);
    }

    #[test]
    fn decodes_buttons() {
        let mut d = Decoder::new(false);
        let ev = feed_all(&mut d, &[0x08 | 0b101, 0, 0]);
        assert!(ev[0].buttons.left() && !ev[0].buttons.right() && ev[0].buttons.middle());
    }

    #[test]
    fn overflow_zeroes_that_axis() {
        let mut d = Decoder::new(false);
        let ev = feed_all(&mut d, &[0x08 | 0x40, 200, 7]);
        assert_eq!((ev[0].dx, ev[0].dy), (0, -7));
        let ev = feed_all(&mut d, &[0x08 | 0x80, 9, 200]);
        assert_eq!((ev[0].dx, ev[0].dy), (9, 0));
    }

    #[test]
    fn four_byte_packets_carry_the_wheel() {
        let mut d = Decoder::new(true);
        assert!(feed_all(&mut d, &[0x08, 1, 1]).is_empty(), "waits for the 4th byte");
        let ev = d.feed(0x01).unwrap();
        assert_eq!(ev.wheel, 1);
        let ev = feed_all(&mut d, &[0x08, 0, 0, 0x0F]);
        assert_eq!(ev[0].wheel, -1, "0xF = -1 in 4 bits");
        let ev = feed_all(&mut d, &[0x08, 0, 0, 0x08]);
        assert_eq!(ev[0].wheel, -8);
    }

    #[test]
    fn resyncs_when_a_byte_is_missing() {
        let mut d = Decoder::new(false);
        // Stray bytes with bit 3 clear are dropped until a real status byte appears.
        let ev = feed_all(&mut d, &[0x00, 0x05, 0x01, 0x08, 4, 4]);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].dx, 4);
        assert_eq!(d.resyncs, 3);
    }

    #[test]
    fn several_packets_in_a_row() {
        let mut d = Decoder::new(false);
        let ev = feed_all(&mut d, &[0x08, 1, 0, 0x09, 0, 0, 0x08, 0, 0]);
        assert_eq!(ev.len(), 3);
        assert!(ev[1].buttons.left() && !ev[2].buttons.left());
    }

    #[test]
    fn pointer_moves_clamps_and_reports_button_edges() {
        let mut p = Pointer::centered(100, 50);
        assert_eq!((p.x, p.y), (50, 25));
        let c = p.apply(MouseEvent { dx: 10, dy: -5, ..Default::default() });
        assert_eq!((p.x, p.y), (60, 20));
        assert!(c.moved);
        let c = p.apply(MouseEvent { dx: 1000, dy: 1000, ..Default::default() });
        assert_eq!((p.x, p.y), (99, 49), "clamped to the last pixel");
        assert!(c.moved);
        let c = p.apply(MouseEvent { dx: 5, dy: 5, ..Default::default() });
        assert!(!c.moved, "already in the corner");

        let c = p.apply(MouseEvent { buttons: Buttons(Buttons::LEFT), ..Default::default() });
        assert_eq!((c.pressed, c.released), (Buttons(1), Buttons(0)));
        let c = p.apply(MouseEvent { buttons: Buttons(Buttons::RIGHT), ..Default::default() });
        assert_eq!((c.pressed, c.released), (Buttons(2), Buttons(1)));
        let c = p.apply(MouseEvent::default());
        assert_eq!(c.released, Buttons(2));
        assert!(p.apply(MouseEvent::default()).is_empty());
        let c = p.apply(MouseEvent { wheel: 2, buttons: Buttons(0), ..Default::default() });
        assert!(!c.is_empty() && c.wheel == 2);
    }
}
