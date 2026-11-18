//! HID report descriptors, as far as a mouse needs them.
//!
//! The *boot* mouse report (buttons, dx, dy as bytes) is only what a mouse
//! promises when it is switched to the boot protocol. Many gaming mice ignore
//! that, or put other data first: the Razer Viper V3 Pro sends 1 byte of
//! buttons, 2 vendor bytes, a wheel byte and then 16-bit X and Y. So the
//! right way is to read the device's own **report descriptor** and find where
//! the buttons, X, Y and the wheel sit in each report ([`parse_mouse`]), then
//! pull them out of every report with [`MouseLayout::decode`].
//!
//! Handles short and long items, report IDs, signed and unsigned fields of
//! 1 to 32 bits and extended usages. No allocation; garbage input gives `None`.
//!
//! Reference: Device Class Definition for HID 1.11, section 6.2.2.

use nova_ps2::{Buttons, MouseEvent};

/// Where one value sits in a report (bits counted from the start of the data after any report ID).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub bit: u16,
    pub size: u8,
    pub signed: bool,
}

/// Where a mouse's fields are in its input reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseLayout {
    /// The report ID byte that starts every report (None: the device has no report IDs).
    pub report_id: Option<u8>,
    /// First button's bit and how many buttons there are.
    pub buttons: Option<(u16, u8)>,
    pub x: Field,
    pub y: Field,
    pub wheel: Option<Field>,
}

const MAX_USAGES: usize = 16;
const MAX_IDS: usize = 8;

const PAGE_DESKTOP: u32 = 0x01;
const PAGE_BUTTON: u32 = 0x09;
const USAGE_X: u32 = 0x30;
const USAGE_Y: u32 = 0x31;
const USAGE_WHEEL: u32 = 0x38;

/// Bits at `field` in `data` (little endian, sign-extended if the field is signed).
fn get(data: &[u8], f: Field) -> Option<i32> {
    let (bit, size) = (f.bit as usize, f.size as usize);
    if size == 0 || size > 32 || (bit + size).div_ceil(8) > data.len() {
        return None;
    }
    let mut v: u64 = 0;
    for i in 0..size {
        let b = bit + i;
        if data[b / 8] >> (b % 8) & 1 != 0 {
            v |= 1 << i;
        }
    }
    if f.signed && size < 64 && v >> (size - 1) & 1 != 0 {
        v |= !0u64 << size;
    }
    Some(v as i64 as i32)
}

impl MouseLayout {
    /// Decodes one input report. `None` if it is another report ID or too short for X and Y.
    pub fn decode(&self, report: &[u8]) -> Option<MouseEvent> {
        let data = match self.report_id {
            Some(id) => {
                if report.first() != Some(&id) {
                    return None;
                }
                &report[1..]
            }
            None => report,
        };
        let dx = get(data, self.x)?;
        let dy = get(data, self.y)?;
        let wheel = self.wheel.and_then(|f| get(data, f)).unwrap_or(0);
        let mut buttons = 0u8;
        if let Some((bit, count)) = self.buttons {
            for i in 0..count.min(3) {
                if get(data, Field { bit: bit + i as u16, size: 1, signed: false }) == Some(1) {
                    buttons |= 1 << i;
                }
            }
        }
        Some(MouseEvent {
            dx: dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            dy: dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            // HID: positive wheel = away from the user; Nova (like PS/2): towards the user.
            wheel: (wheel.clamp(-127, 127) as i8).saturating_neg(),
            buttons: Buttons(buttons),
        })
    }

    /// A short description for the log, e.g. `id none, buttons 5 @0, x 16 bits @32, ...`.
    pub fn summary(&self, out: &mut impl core::fmt::Write) {
        let _ = match self.report_id {
            Some(i) => write!(out, "id {i}"),
            None => write!(out, "no id"),
        };
        if let Some((b, n)) = self.buttons {
            let _ = write!(out, ", {n} buttons @{b}");
        }
        let _ = write!(out, ", x {}b @{}, y {}b @{}", self.x.size, self.x.bit, self.y.size, self.y.bit);
        if let Some(w) = self.wheel {
            let _ = write!(out, ", wheel {}b @{}", w.size, w.bit);
        }
    }
}

/// Everything one pass over the descriptor learns.
#[derive(Default)]
struct Found {
    buttons: Option<(u8, u16, u8)>,
    x: Option<(u8, Field)>,
    y: Option<(u8, Field)>,
    wheel: Option<(u8, Field)>,
}

fn signed_value(v: u32, size: usize) -> i32 {
    match size {
        1 => v as u8 as i8 as i32,
        2 => v as u16 as i16 as i32,
        _ => v as i32,
    }
}

/// Finds where the mouse fields are in a report descriptor.
pub fn parse_mouse(desc: &[u8]) -> Option<MouseLayout> {
    let mut found = Found::default();
    // Globals.
    let (mut page, mut lmin, mut report_size, mut report_count, mut id) = (0u32, 0i32, 0u32, 0u32, 0u8);
    // Locals (cleared after every main item).
    let mut usages = [0u32; MAX_USAGES];
    let (mut n_usages, mut umin, mut umax) = (0usize, None::<u32>, None::<u32>);
    // Where the next input bit goes, per report ID.
    let mut pos = [(0u8, 0u32); MAX_IDS];
    let mut n_pos = 0usize;
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        i += 1;
        if prefix == 0xFE {
            // A long item: length, tag, data (unused by any mouse).
            let len = *desc.get(i)? as usize;
            i += 2 + len;
            continue;
        }
        let size = match prefix & 3 {
            3 => 4,
            n => n as usize,
        };
        let (kind, tag) = ((prefix >> 2) & 3, prefix >> 4);
        let bytes = desc.get(i..i + size)?;
        i += size;
        let mut value = 0u32;
        for (k, b) in bytes.iter().enumerate() {
            value |= (*b as u32) << (8 * k);
        }
        match (kind, tag) {
            // Global items.
            (1, 0) => page = value,
            (1, 1) => lmin = signed_value(value, size),
            (1, 7) => report_size = value,
            (1, 8) => id = value as u8,
            (1, 9) => report_count = value,
            // Local items.
            (2, 0) if n_usages < MAX_USAGES => {
                usages[n_usages] = value;
                n_usages += 1;
            }
            (2, 1) => umin = Some(value),
            (2, 2) => umax = Some(value),
            // Main items: Input is the only one that takes space in input reports.
            (0, 8) => {
                let slot = match pos[..n_pos].iter().position(|p| p.0 == id) {
                    Some(s) => s,
                    None if n_pos < MAX_IDS => {
                        pos[n_pos] = (id, 0);
                        n_pos += 1;
                        n_pos - 1
                    }
                    None => return None,
                };
                let constant = value & 1 != 0;
                let variable = value & 2 != 0;
                if report_size == 0 || report_size > 32 || report_count > 256 {
                    return None;
                }
                if !constant && variable {
                    for k in 0..report_count as usize {
                        let usage = if n_usages > 0 {
                            usages[k.min(n_usages - 1)]
                        } else if let (Some(lo), Some(_)) = (umin, umax) {
                            lo + k as u32
                        } else {
                            continue;
                        };
                        // An extended usage carries its own page.
                        let (up, u) = if usage >> 16 != 0 { (usage >> 16, usage & 0xFFFF) } else { (page, usage) };
                        let bit = pos[slot].1 + k as u32 * report_size;
                        if bit > u16::MAX as u32 {
                            return None;
                        }
                        let field = Field { bit: bit as u16, size: report_size as u8, signed: lmin < 0 };
                        match (up, u) {
                            (PAGE_DESKTOP, USAGE_X) if found.x.is_none() => found.x = Some((id, field)),
                            (PAGE_DESKTOP, USAGE_Y) if found.y.is_none() => found.y = Some((id, field)),
                            (PAGE_DESKTOP, USAGE_WHEEL) if found.wheel.is_none() => found.wheel = Some((id, field)),
                            (PAGE_BUTTON, 1) if found.buttons.is_none() && report_size == 1 => found.buttons = Some((id, field.bit, 1)),
                            (PAGE_BUTTON, 2..=32) if report_size == 1 => {
                                if let Some((bid, b0, n)) = &mut found.buttons
                                    && *bid == id
                                    && *b0 as u32 + *n as u32 == field.bit as u32
                                {
                                    *n += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                pos[slot].1 = pos[slot].1.saturating_add(report_size * report_count);
            }
            _ => {}
        }
        // Local items last for one main item only.
        if kind == 0 {
            n_usages = 0;
            umin = None;
            umax = None;
        }
    }
    let (xid, x) = found.x?;
    let (yid, y) = found.y?;
    if xid != yid {
        return None;
    }
    // Reports carry an ID byte only if the descriptor uses report IDs at all.
    let uses_ids = xid != 0 || pos[..n_pos].iter().any(|p| p.0 != 0);
    Some(MouseLayout {
        report_id: uses_ids.then_some(xid),
        buttons: found.buttons.filter(|b| b.0 == xid).map(|b| (b.1, b.2)),
        x,
        y,
        wheel: found.wheel.filter(|w| w.0 == xid).map(|w| w.1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Razer Viper V3 Pro's mouse interface, copied from a photo of Nova's own boot log on the
    /// real PC: 5 buttons, 2 vendor bytes, wheel, then 16-bit X and Y.
    const RAZER: [u8; 94] = [
        0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x09, 0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29, 0x05, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x05,
        0x81, 0x02, 0x75, 0x01, 0x95, 0x03, 0x81, 0x03, 0x06, 0x00, 0xff, 0x09, 0x40, 0x75, 0x08, 0x95, 0x02, 0x15, 0x81, 0x25, 0x7f, 0x81, 0x02, 0x05,
        0x01, 0x09, 0x38, 0x15, 0x81, 0x25, 0x7f, 0x75, 0x08, 0x95, 0x01, 0x81, 0x06, 0x09, 0x30, 0x09, 0x31, 0x16, 0x01, 0x80, 0x26, 0xff, 0x7f, 0x75,
        0x10, 0x95, 0x02, 0x81, 0x06, 0xc0, 0x06, 0x00, 0xff, 0x09, 0x02, 0x15, 0x00, 0x25, 0x01, 0x75, 0x08, 0x95, 0x5a, 0xb1, 0x01, 0xc0,
    ];

    /// QEMU's USB mouse (and the classic boot layout).
    const BOOT: [u8; 52] = [
        0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x09, 0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29, 0x05, 0x15, 0x00, 0x25, 0x01, 0x95, 0x05, 0x75, 0x01,
        0x81, 0x02, 0x95, 0x01, 0x75, 0x03, 0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x09, 0x38, 0x15, 0x81, 0x25, 0x7f, 0x75, 0x08, 0x95, 0x03,
        0x81, 0x06, 0xc0, 0xc0,
    ];

    #[test]
    fn the_razers_real_descriptor() {
        // The descriptor is 70 bytes of which the last 22 (a vendor feature report) are the tail.
        let l = parse_mouse(&RAZER[..RAZER.len()]).expect("a mouse");
        assert_eq!(l.report_id, None);
        assert_eq!(l.buttons, Some((0, 5)));
        assert_eq!(l.x, Field { bit: 32, size: 16, signed: true });
        assert_eq!(l.y, Field { bit: 48, size: 16, signed: true });
        assert_eq!(l.wheel, Some(Field { bit: 24, size: 8, signed: true }));
        // Left button down, moving right 10 and up 10, wheel one notch away from the user.
        let e = l.decode(&[0x01, 0, 0, 0x01, 0x0a, 0x00, 0xf6, 0xff]).unwrap();
        assert_eq!((e.dx, e.dy, e.wheel, e.buttons), (10, -10, -1, Buttons(Buttons::LEFT)));
        // The vendor bytes in the middle must not look like movement.
        let e = l.decode(&[0, 0x7f, 0x7f, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!((e.dx, e.dy), (0, 0));
        // Large fast movement keeps its sign and size.
        let e = l.decode(&[0x06, 0, 0, 0, 0x00, 0x80, 0xff, 0x7f]).unwrap();
        assert_eq!((e.dx, e.dy, e.buttons), (i16::MIN, i16::MAX, Buttons(Buttons::RIGHT | Buttons::MIDDLE)));
        assert!(l.decode(&[0, 0, 0, 0]).is_none(), "too short for X and Y");
        let mut text = std::string::String::new();
        l.summary(&mut text);
        assert_eq!(text, "no id, 5 buttons @0, x 16b @32, y 16b @48, wheel 8b @24");
    }

    #[test]
    fn the_classic_boot_layout() {
        let l = parse_mouse(&BOOT).unwrap();
        assert_eq!((l.report_id, l.buttons, l.x.bit, l.y.bit, l.wheel.map(|w| w.bit)), (None, Some((0, 5)), 8, 16, Some(24)));
        let e = l.decode(&[0x02, 5, 0xfb, 0xff]).unwrap();
        assert_eq!((e.dx, e.dy, e.wheel, e.buttons), (5, -5, 1, Buttons(Buttons::RIGHT)));
        // It agrees with the boot-protocol decoder on every report.
        for r in [[0u8, 0, 0, 0], [1, 1, 255, 0], [7, 127, 128, 255], [4, 10, 20, 2]] {
            assert_eq!(l.decode(&r), crate::hid::mouse_report(&r));
        }
    }

    #[test]
    fn report_ids_prefix_every_report() {
        // Report ID 1: 3 buttons, 5 padding bits, 8-bit X and Y.
        let d = [
            0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x01, 0x09, 0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03,
            0x75, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x05, 0x81, 0x03, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7f, 0x75, 0x08, 0x95, 0x02,
            0x81, 0x06, 0xc0, 0xc0,
        ];
        let l = parse_mouse(&d).unwrap();
        assert_eq!((l.report_id, l.x.bit, l.y.bit, l.buttons, l.wheel), (Some(1), 8, 16, Some((0, 3)), None));
        let e = l.decode(&[1, 0x01, 5, 0xfb]).unwrap();
        assert_eq!((e.dx, e.dy, e.buttons), (5, -5, Buttons(Buttons::LEFT)));
        assert!(l.decode(&[2, 0x01, 5, 0xfb]).is_none(), "another report ID is not the mouse");
        assert!(l.decode(&[]).is_none());
    }

    #[test]
    fn things_that_are_not_mice_give_nothing() {
        // A boot keyboard.
        let kb = [
            0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01,
            0x75, 0x08, 0x81, 0x01, 0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x25, 0x65, 0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x81, 0x00, 0xc0,
        ];
        assert_eq!(parse_mouse(&kb), None);
        assert_eq!(parse_mouse(&[]), None);
    }

    #[test]
    fn garbage_never_panics() {
        // Every prefix and every truncation of real descriptors, plus pseudo-random bytes.
        for d in [&RAZER[..], &BOOT[..]] {
            for n in 0..=d.len() {
                let _ = parse_mouse(&d[..n]);
            }
        }
        let mut seed: u32 = 0x1234_5678;
        for len in 0..200 {
            let mut buf = [0u8; 200];
            for b in buf.iter_mut().take(len) {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *b = (seed >> 8) as u8;
            }
            let _ = parse_mouse(&buf[..len]);
        }
        // Absurd sizes are refused, not trusted.
        assert_eq!(parse_mouse(&[0x75, 0x40, 0x95, 0x02, 0x81, 0x02]), None);
        // A long item is skipped.
        assert_eq!(parse_mouse(&[0xFE, 0x02, 0x10, 0xAA, 0xBB]), None);
        // Fields read the right bits at odd offsets and sizes.
        let f = Field { bit: 3, size: 5, signed: true };
        assert_eq!(get(&[0b1111_1000], f), Some(-1));
        assert_eq!(get(&[0b0111_1000], f), Some(15));
        assert_eq!(get(&[0xff], Field { bit: 4, size: 8, signed: false }), None);
    }
}
