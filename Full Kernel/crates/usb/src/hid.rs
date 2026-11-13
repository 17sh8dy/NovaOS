//! HID boot-protocol reports (HID 1.11 appendix B).
//!
//! **Keyboard:** an 8-byte report: modifier bits, a reserved byte, then up to
//! six pressed keys as usage codes (keyboard page 0x07). The device sends the
//! whole state every time something changes, so presses and releases are the
//! difference between two reports. They are turned into PS/2 scancode set 1
//! bytes, the language the kernel's keyboard pipeline (`nova_kbd`) already
//! speaks. USB keyboards do not repeat a held key themselves, so [`Keyboard`]
//! also does typematic repeat (500 ms delay, then 30 per second).
//!
//! **Mouse:** buttons, then signed X and Y movement (and, on most mice, a
//! wheel byte).

use nova_ps2::{Buttons, MouseEvent};

/// Set 1 make code for a HID keyboard usage: (extended (E0 prefix), code).
pub fn usage_to_set1(usage: u8) -> Option<(bool, u8)> {
    const LETTERS: [u8; 26] = [
        0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, // a..m
        0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14, 0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C, // n..z
    ];
    let plain = |c| Some((false, c));
    let ext = |c| Some((true, c));
    match usage {
        0x04..=0x1D => plain(LETTERS[(usage - 0x04) as usize]),
        0x1E..=0x27 => plain(usage - 0x1E + 0x02), // 1..9, 0
        0x28 => plain(0x1C),                       // Enter
        0x29 => plain(0x01),                       // Escape
        0x2A => plain(0x0E),                       // Backspace
        0x2B => plain(0x0F),                       // Tab
        0x2C => plain(0x39),                       // Space
        0x2D => plain(0x0C),                       // -
        0x2E => plain(0x0D),                       // =
        0x2F => plain(0x1A),                       // [
        0x30 => plain(0x1B),                       // ]
        0x31 | 0x32 => plain(0x2B),                // \ (and the non-US # key)
        0x33 => plain(0x27),                       // ;
        0x34 => plain(0x28),                       // '
        0x35 => plain(0x29),                       // `
        0x36 => plain(0x33),                       // ,
        0x37 => plain(0x34),                       // .
        0x38 => plain(0x35),                       // /
        0x39 => plain(0x3A),                       // Caps Lock
        0x3A..=0x43 => plain(usage - 0x3A + 0x3B), // F1..F10
        0x44 => plain(0x57),                       // F11
        0x45 => plain(0x58),                       // F12
        0x47 => plain(0x46),                       // Scroll Lock
        0x49 => ext(0x52),                         // Insert
        0x4A => ext(0x47),                         // Home
        0x4B => ext(0x49),                         // Page Up
        0x4C => ext(0x53),                         // Delete
        0x4D => ext(0x4F),                         // End
        0x4E => ext(0x51),                         // Page Down
        0x4F => ext(0x4D),                         // Right
        0x50 => ext(0x4B),                         // Left
        0x51 => ext(0x50),                         // Down
        0x52 => ext(0x48),                         // Up
        0x53 => plain(0x45),                       // Num Lock
        0x54 => ext(0x35),                         // keypad /
        0x55 => plain(0x37),                       // keypad *
        0x56 => plain(0x4A),                       // keypad -
        0x57 => plain(0x4E),                       // keypad +
        0x58 => ext(0x1C),                         // keypad Enter
        0x59 => plain(0x4F),                       // keypad 1
        0x5A => plain(0x50),
        0x5B => plain(0x51),
        0x5C => plain(0x4B),
        0x5D => plain(0x4C),
        0x5E => plain(0x4D),
        0x5F => plain(0x47),
        0x60 => plain(0x48),
        0x61 => plain(0x49),                       // keypad 9
        0x62 => plain(0x52),                       // keypad 0
        0x63 => plain(0x53),                       // keypad .
        0x64 => plain(0x56),                       // the extra key next to left Shift
        _ => None,
    }
}

/// Modifier bits (report byte 0) in order: LCtrl, LShift, LAlt, LGUI, RCtrl, RShift, RAlt, RGUI.
const MODIFIERS: [(bool, u8); 8] = [(false, 0x1D), (false, 0x2A), (false, 0x38), (true, 0x5B), (true, 0x1D), (false, 0x36), (true, 0x38), (true, 0x5C)];

/// Typematic delay and rate.
pub const REPEAT_DELAY_MS: u64 = 500;
pub const REPEAT_EVERY_MS: u64 = 33;

/// A boot keyboard's state between reports.
#[derive(Debug, Default)]
pub struct Keyboard {
    modifiers: u8,
    keys: [u8; 6],
    /// The key being held for repeat, and when it next repeats.
    repeat: Option<(u8, u64)>,
}

fn emit(make: (bool, u8), pressed: bool, out: &mut impl FnMut(u8)) {
    if make.0 {
        out(0xE0);
    }
    out(if pressed { make.1 } else { make.1 | 0x80 });
}

impl Keyboard {
    pub const fn new() -> Self {
        Self { modifiers: 0, keys: [0; 6], repeat: None }
    }

    /// Feeds one report; `out` receives set 1 bytes for every change. `now_ms`
    /// is the time, used for key repeat.
    pub fn report(&mut self, r: &[u8], now_ms: u64, out: &mut impl FnMut(u8)) {
        if r.len() < 8 {
            return;
        }
        let keys: [u8; 6] = [r[2], r[3], r[4], r[5], r[6], r[7]];
        // "Phantom" state (too many keys at once): every slot says ErrorRollOver.
        if keys.iter().all(|&k| k == 0x01) {
            return;
        }
        let mods = r[0];
        // Releases first, so a quick "a up, b down" in one report types cleanly.
        for (bit, &make) in MODIFIERS.iter().enumerate() {
            if self.modifiers & (1 << bit) != 0 && mods & (1 << bit) == 0 {
                emit(make, false, out);
            }
        }
        for &k in self.keys.iter().filter(|&&k| k > 3 && !keys.contains(&k)) {
            if let Some(make) = usage_to_set1(k) {
                emit(make, false, out);
            }
            if self.repeat.is_some_and(|(r, _)| r == k) {
                self.repeat = None;
            }
        }
        for (bit, &make) in MODIFIERS.iter().enumerate() {
            if self.modifiers & (1 << bit) == 0 && mods & (1 << bit) != 0 {
                emit(make, true, out);
            }
        }
        for &k in keys.iter().filter(|&&k| k > 3 && !self.keys.contains(&k)) {
            if let Some(make) = usage_to_set1(k) {
                emit(make, true, out);
                // Lock keys don't repeat; anything else becomes the repeating key.
                if !matches!(k, 0x39 | 0x47 | 0x53) {
                    self.repeat = Some((k, now_ms + REPEAT_DELAY_MS));
                }
            }
        }
        self.modifiers = mods;
        self.keys = keys;
    }

    /// Call regularly: emits repeated make codes for a held key.
    pub fn tick(&mut self, now_ms: u64, out: &mut impl FnMut(u8)) {
        if let Some((k, at)) = self.repeat
            && now_ms >= at
        {
            if let Some(make) = usage_to_set1(k) {
                emit(make, true, out);
            }
            self.repeat = Some((k, now_ms.max(at) + REPEAT_EVERY_MS));
        }
    }

    /// Releases everything (the keyboard was unplugged).
    pub fn release_all(&mut self, out: &mut impl FnMut(u8)) {
        self.report(&[0; 8], 0, out);
        self.repeat = None;
    }
}

/// A boot mouse report: buttons, dx, dy (positive = down, as on screen) and
/// an optional wheel byte (positive = away from the user, flipped here to
/// match the PS/2 convention).
pub fn mouse_report(r: &[u8]) -> Option<MouseEvent> {
    if r.len() < 3 {
        return None;
    }
    Some(MouseEvent {
        dx: r[1] as i8 as i16,
        dy: r[2] as i8 as i16,
        wheel: r.get(3).map_or(0, |&w| (w as i8).saturating_neg()),
        buttons: Buttons(r[0] & 7),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_kbd::{Action, Key, Pipeline};
    use std::string::String;
    use std::vec::Vec;

    fn bytes(kb: &mut Keyboard, reports: &[[u8; 8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for r in reports {
            kb.report(r, 0, &mut |b| out.push(b));
        }
        out
    }

    fn typed(set1: &[u8]) -> String {
        let mut p = Pipeline::new();
        set1.iter().filter_map(|&b| p.feed(b)).filter_map(|i| if let Action::Char(c) = i.action { Some(c) } else { None }).collect()
    }

    #[test]
    fn press_and_release() {
        let mut kb = Keyboard::new();
        let out = bytes(&mut kb, &[[0, 0, 0x04, 0, 0, 0, 0, 0], [0; 8]]);
        assert_eq!(out, [0x1E, 0x9E]);
    }

    #[test]
    fn shifted_text_through_the_real_pipeline() {
        // "Hi!" = Shift+h, i, Shift+1.
        let mut kb = Keyboard::new();
        let out = bytes(&mut kb, &[
            [0x02, 0, 0x0B, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0],
            [0, 0, 0x0C, 0, 0, 0, 0, 0],
            [0, 0, 0, 0, 0, 0, 0, 0],
            [0x20, 0, 0x1E, 0, 0, 0, 0, 0], // right shift + 1
            [0, 0, 0, 0, 0, 0, 0, 0],
        ]);
        assert_eq!(typed(&out), "Hi!");
    }

    #[test]
    fn rollover_and_extended_keys() {
        let mut kb = Keyboard::new();
        // a down, then a + b down, then only b.
        let out = bytes(&mut kb, &[[0, 0, 4, 0, 0, 0, 0, 0], [0, 0, 4, 5, 0, 0, 0, 0], [0, 0, 5, 0, 0, 0, 0, 0]]);
        assert_eq!(out, [0x1E, 0x30, 0x9E]);
        // Up arrow is E0 48 / E0 C8.
        let out = bytes(&mut kb, &[[0, 0, 0x52, 0, 0, 0, 0, 0], [0; 8]]);
        // (b, still held from above, is released first.)
        assert_eq!(out, [0xB0, 0xE0, 0x48, 0xE0, 0xC8]);
        let mut p = Pipeline::new();
        let keys: Vec<_> = [0xE0, 0x48].iter().filter_map(|&b| p.feed(b)).collect();
        assert_eq!(keys[0].action, Action::Key(Key::Up));
        // Too many keys: ignored, state kept.
        let mut kb = Keyboard::new();
        assert!(bytes(&mut kb, &[[0, 0, 1, 1, 1, 1, 1, 1]]).is_empty());
    }

    #[test]
    fn ctrl_c() {
        let mut kb = Keyboard::new();
        let out = bytes(&mut kb, &[[0x01, 0, 0x06, 0, 0, 0, 0, 0], [0; 8]]);
        let mut p = Pipeline::new();
        let inputs: Vec<_> = out.iter().filter_map(|&b| p.feed(b)).collect();
        assert_eq!(inputs.len(), 1);
        assert!(inputs[0].ctrl);
        assert_eq!(inputs[0].action, Action::Char('c'));
    }

    #[test]
    fn key_repeat() {
        let mut kb = Keyboard::new();
        let mut out = Vec::new();
        kb.report(&[0, 0, 0x04, 0, 0, 0, 0, 0], 1000, &mut |b| out.push(b));
        kb.tick(1400, &mut |b| out.push(b));
        assert_eq!(out, [0x1E], "no repeat before the delay");
        kb.tick(1500, &mut |b| out.push(b));
        kb.tick(1520, &mut |b| out.push(b));
        kb.tick(1533, &mut |b| out.push(b));
        assert_eq!(out, [0x1E, 0x1E, 0x1E]);
        kb.report(&[0; 8], 1540, &mut |b| out.push(b));
        kb.tick(5000, &mut |b| out.push(b));
        assert_eq!(out, [0x1E, 0x1E, 0x1E, 0x9E], "released keys stop repeating");
        assert_eq!(typed(&out), "aaa");
        // Caps Lock never repeats.
        let mut kb = Keyboard::new();
        let mut out = Vec::new();
        kb.report(&[0, 0, 0x39, 0, 0, 0, 0, 0], 0, &mut |b| out.push(b));
        kb.tick(10_000, &mut |b| out.push(b));
        assert_eq!(out, [0x3A]);
    }

    #[test]
    fn unplug_releases_modifiers() {
        let mut kb = Keyboard::new();
        let mut out = Vec::new();
        kb.report(&[0x02, 0, 0x04, 0, 0, 0, 0, 0], 0, &mut |b| out.push(b));
        kb.release_all(&mut |b| out.push(b));
        assert_eq!(out, [0x2A, 0x1E, 0xAA, 0x9E]);
    }

    #[test]
    fn every_mapped_usage_decodes_to_a_known_key() {
        let mut p = Pipeline::new();
        for u in 0x04..=0x64u8 {
            if let Some((ext, code)) = usage_to_set1(u) {
                if ext {
                    p.feed(0xE0);
                }
                let got = p.feed(code);
                if let Some(i) = got {
                    assert!(!matches!(i.action, Action::Key(Key::Unknown(_))), "usage {u:#x} decoded as unknown");
                }
                if ext {
                    p.feed(0xE0);
                }
                p.feed(code | 0x80);
            }
        }
    }

    #[test]
    fn mouse_reports() {
        let e = mouse_report(&[0x01, 0xFB, 10, 0x01]).unwrap();
        assert_eq!((e.dx, e.dy, e.wheel, e.buttons.left()), (-5, 10, -1, true));
        let e = mouse_report(&[0x06, 1, 0x80]).unwrap();
        assert_eq!((e.dy, e.wheel, e.buttons.right(), e.buttons.middle()), (-128, 0, true, true));
        assert!(mouse_report(&[0, 1]).is_none());
    }
}
