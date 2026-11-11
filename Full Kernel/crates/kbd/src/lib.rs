//! PS/2 keyboard decoding: scancode set 1 bytes -> key press/release events
//! -> characters, with a US layout and shift / caps lock / ctrl / alt tracking.
//!
//! (The PS/2 controller translates the keyboard's native set 2 into set 1 for
//! us when its translation bit is on, which is how the kernel configures it.)
//! No hardware access here, so everything is unit-tested on the host.

#![no_std]

#[cfg(test)]
extern crate std;

/// A physical key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// A key that types a character: `base` unshifted, `shifted` with Shift.
    /// `letter` keys also respond to Caps Lock.
    Printable { base: char, shifted: char, letter: bool },
    Enter,
    Backspace,
    Tab,
    Escape,
    LeftShift,
    RightShift,
    LeftCtrl,
    RightCtrl,
    LeftAlt,
    RightAlt,
    CapsLock,
    NumLock,
    ScrollLock,
    /// F1..F12.
    F(u8),
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// A numeric-keypad navigation key (set 1 make code); a digit while NumLock is on.
    Keypad(u8),
    /// A scancode we don't know.
    Unknown(u8),
}

/// A key going down or coming up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    pub key: Key,
    pub pressed: bool,
}

/// Turns the raw byte stream into key events.
#[derive(Default)]
pub struct Decoder {
    /// Saw 0xE0: the next code is an "extended" key.
    extended: bool,
    /// Bytes still to swallow of an 0xE1 sequence (the Pause key).
    skip: u8,
}

const fn p(base: char, shifted: char) -> Key {
    Key::Printable { base, shifted, letter: false }
}
const fn l(base: char, shifted: char) -> Key {
    Key::Printable { base, shifted, letter: true }
}

/// Set 1 make codes for the main block and function keys.
fn main_block(code: u8) -> Option<Key> {
    Some(match code {
        0x01 => Key::Escape,
        0x02 => p('1', '!'),
        0x03 => p('2', '@'),
        0x04 => p('3', '#'),
        0x05 => p('4', '$'),
        0x06 => p('5', '%'),
        0x07 => p('6', '^'),
        0x08 => p('7', '&'),
        0x09 => p('8', '*'),
        0x0A => p('9', '('),
        0x0B => p('0', ')'),
        0x0C => p('-', '_'),
        0x0D => p('=', '+'),
        0x0E => Key::Backspace,
        0x0F => Key::Tab,
        0x10 => l('q', 'Q'),
        0x11 => l('w', 'W'),
        0x12 => l('e', 'E'),
        0x13 => l('r', 'R'),
        0x14 => l('t', 'T'),
        0x15 => l('y', 'Y'),
        0x16 => l('u', 'U'),
        0x17 => l('i', 'I'),
        0x18 => l('o', 'O'),
        0x19 => l('p', 'P'),
        0x1A => p('[', '{'),
        0x1B => p(']', '}'),
        0x1C => Key::Enter,
        0x1D => Key::LeftCtrl,
        0x1E => l('a', 'A'),
        0x1F => l('s', 'S'),
        0x20 => l('d', 'D'),
        0x21 => l('f', 'F'),
        0x22 => l('g', 'G'),
        0x23 => l('h', 'H'),
        0x24 => l('j', 'J'),
        0x25 => l('k', 'K'),
        0x26 => l('l', 'L'),
        0x27 => p(';', ':'),
        0x28 => p('\'', '"'),
        0x29 => p('`', '~'),
        0x2A => Key::LeftShift,
        0x2B => p('\\', '|'),
        0x2C => l('z', 'Z'),
        0x2D => l('x', 'X'),
        0x2E => l('c', 'C'),
        0x2F => l('v', 'V'),
        0x30 => l('b', 'B'),
        0x31 => l('n', 'N'),
        0x32 => l('m', 'M'),
        0x33 => p(',', '<'),
        0x34 => p('.', '>'),
        0x35 => p('/', '?'),
        0x36 => Key::RightShift,
        0x37 => p('*', '*'), // keypad *
        0x38 => Key::LeftAlt,
        0x39 => p(' ', ' '),
        0x3A => Key::CapsLock,
        0x3B..=0x44 => Key::F(code - 0x3A),
        0x45 => Key::NumLock,
        0x46 => Key::ScrollLock,
        0x57 => Key::F(11),
        0x58 => Key::F(12),
        _ => return None,
    })
}

/// Keypad keys (no 0xE0 prefix).
fn keypad(code: u8) -> Option<Key> {
    Some(match code {
        0x4A => p('-', '-'),
        0x4E => p('+', '+'),
        0x47 | 0x48 | 0x49 | 0x4B | 0x4C | 0x4D | 0x4F | 0x50 | 0x51 | 0x52 | 0x53 => Key::Keypad(code),
        _ => return None,
    })
}

/// The digit a keypad key types with NumLock on.
fn keypad_digit(code: u8) -> Option<char> {
    Some(match code {
        0x47 => '7',
        0x48 => '8',
        0x49 => '9',
        0x4B => '4',
        0x4C => '5',
        0x4D => '6',
        0x4F => '1',
        0x50 => '2',
        0x51 => '3',
        0x52 => '0',
        0x53 => '.',
        _ => return None,
    })
}

/// The navigation key a keypad key acts as with NumLock off.
fn keypad_nav(code: u8) -> Option<Key> {
    Some(match code {
        0x47 => Key::Home,
        0x48 => Key::Up,
        0x49 => Key::PageUp,
        0x4B => Key::Left,
        0x4D => Key::Right,
        0x4F => Key::End,
        0x50 => Key::Down,
        0x51 => Key::PageDown,
        0x52 => Key::Insert,
        0x53 => Key::Delete,
        _ => return None,
    })
}

/// 0xE0-prefixed keys.
fn extended(code: u8) -> Option<Key> {
    Some(match code {
        0x1C => Key::Enter, // keypad enter
        0x1D => Key::RightCtrl,
        0x35 => p('/', '/'), // keypad /
        0x38 => Key::RightAlt,
        0x47 => Key::Home,
        0x48 => Key::Up,
        0x49 => Key::PageUp,
        0x4B => Key::Left,
        0x4D => Key::Right,
        0x4F => Key::End,
        0x50 => Key::Down,
        0x51 => Key::PageDown,
        0x52 => Key::Insert,
        0x53 => Key::Delete,
        _ => return None,
    })
}

impl Decoder {
    pub const fn new() -> Self {
        Self { extended: false, skip: 0 }
    }

    /// Feeds one byte; returns an event when it completes a key press/release.
    pub fn feed(&mut self, byte: u8) -> Option<KeyEvent> {
        if self.skip > 0 {
            self.skip -= 1;
            return None;
        }
        match byte {
            0xE0 => {
                self.extended = true;
                return None;
            }
            0xE1 => {
                self.skip = 5; // Pause sends E1 1D 45 E1 9D C5
                return None;
            }
            // ACK / resend / overrun / error bytes from the keyboard itself. (0xAA, the
            // self-test result, is also "left shift released": the kernel consumes
            // the real one during initialisation, so it is left to mean shift here.)
            0xFA | 0xFE | 0xFF | 0x00 => {
                self.extended = false;
                return None;
            }
            _ => {}
        }
        let pressed = byte & 0x80 == 0;
        let code = byte & 0x7F;
        let ext = core::mem::take(&mut self.extended);
        // "Fake shift" bytes that Print Screen / Insert etc. wrap around the real key.
        if ext && (code == 0x2A || code == 0x36) {
            return None;
        }
        let key = if ext { extended(code) } else { main_block(code).or_else(|| keypad(code)) };
        Some(KeyEvent { key: key.unwrap_or(Key::Unknown(code)), pressed })
    }
}

/// What a key press means after the modifiers are applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// A character to type (Ctrl+letter is delivered as the letter with `ctrl` set).
    Char(char),
    /// A non-character key (arrows, Enter, F-keys...).
    Key(Key),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Input {
    pub action: Action,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// Tracks modifier and lock state and turns key events into [`Input`]s.
pub struct Keyboard {
    shift_l: bool,
    shift_r: bool,
    ctrl_l: bool,
    ctrl_r: bool,
    alt_l: bool,
    alt_r: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyboard {
    pub const fn new() -> Self {
        Self { shift_l: false, shift_r: false, ctrl_l: false, ctrl_r: false, alt_l: false, alt_r: false, caps_lock: false, num_lock: true }
    }

    pub fn shift(&self) -> bool {
        self.shift_l || self.shift_r
    }
    pub fn ctrl(&self) -> bool {
        self.ctrl_l || self.ctrl_r
    }
    pub fn alt(&self) -> bool {
        self.alt_l || self.alt_r
    }

    /// Applies a key event. Returns an input for presses that produce one
    /// (modifier and lock keys only change state).
    pub fn handle(&mut self, ev: KeyEvent) -> Option<Input> {
        match ev.key {
            Key::LeftShift => self.shift_l = ev.pressed,
            Key::RightShift => self.shift_r = ev.pressed,
            Key::LeftCtrl => self.ctrl_l = ev.pressed,
            Key::RightCtrl => self.ctrl_r = ev.pressed,
            Key::LeftAlt => self.alt_l = ev.pressed,
            Key::RightAlt => self.alt_r = ev.pressed,
            Key::CapsLock if ev.pressed => self.caps_lock = !self.caps_lock,
            Key::NumLock if ev.pressed => self.num_lock = !self.num_lock,
            Key::CapsLock | Key::NumLock | Key::ScrollLock => {}
            _ if !ev.pressed => {}
            Key::Keypad(code) => {
                return if self.num_lock && !self.shift() {
                    keypad_digit(code).map(|d| self.input(Action::Char(d)))
                } else {
                    keypad_nav(code).map(|k| self.input(Action::Key(k)))
                };
            }
            Key::Printable { base, shifted, letter } => {
                let upper = if letter { self.shift() != self.caps_lock } else { self.shift() };
                let c = if upper { shifted } else { base };
                return Some(self.input(Action::Char(c)));
            }
            key => return Some(self.input(Action::Key(key))),
        }
        None
    }

    fn input(&self, action: Action) -> Input {
        Input { action, shift: self.shift(), ctrl: self.ctrl(), alt: self.alt() }
    }
}

/// Convenience for the kernel: bytes in, inputs out.
pub struct Pipeline {
    pub decoder: Decoder,
    pub keyboard: Keyboard,
}

impl Pipeline {
    pub const fn new() -> Self {
        Self { decoder: Decoder::new(), keyboard: Keyboard::new() }
    }

    pub fn feed(&mut self, byte: u8) -> Option<Input> {
        let ev = self.decoder.feed(byte)?;
        self.keyboard.handle(ev)
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn type_bytes(bytes: &[u8]) -> Vec<Input> {
        let mut p = Pipeline::new();
        bytes.iter().filter_map(|&b| p.feed(b)).collect()
    }

    fn chars(inputs: &[Input]) -> std::string::String {
        inputs.iter().filter_map(|i| if let Action::Char(c) = i.action { Some(c) } else { None }).collect()
    }

    #[test]
    fn decodes_make_and_break_codes() {
        let mut d = Decoder::new();
        assert_eq!(d.feed(0x1E), Some(KeyEvent { key: l('a', 'A'), pressed: true }));
        assert_eq!(d.feed(0x9E), Some(KeyEvent { key: l('a', 'A'), pressed: false }));
    }

    #[test]
    fn extended_keys_use_the_e0_prefix() {
        let mut d = Decoder::new();
        assert_eq!(d.feed(0xE0), None);
        assert_eq!(d.feed(0x48), Some(KeyEvent { key: Key::Up, pressed: true }));
        assert_eq!(d.feed(0xE0), None);
        assert_eq!(d.feed(0xC8), Some(KeyEvent { key: Key::Up, pressed: false }));
        // The prefix applies to one code only.
        assert_eq!(d.feed(0x1E).map(|e| e.key), Some(l('a', 'A')));
    }

    #[test]
    fn types_text_with_shift_and_caps_lock() {
        // h, shift+i, 1
        let s = type_bytes(&[0x23, 0xA3, 0x2A, 0x17, 0x97, 0xAA, 0x02, 0x82]);
        assert_eq!(chars(&s), "hI1");
        // shift+1 = '!'
        let s = type_bytes(&[0x2A, 0x02, 0x82, 0xAA]);
        assert_eq!(chars(&s), "!");
        // caps lock: letters shift, digits do not; shift cancels caps for letters.
        let s = type_bytes(&[0x3A, 0xBA, 0x1E, 0x9E, 0x02, 0x82, 0x2A, 0x1E, 0x9E, 0xAA]);
        assert_eq!(chars(&s), "A1a");
    }

    #[test]
    fn right_shift_and_both_shifts_released_independently() {
        let mut p = Pipeline::new();
        p.feed(0x2A);
        p.feed(0x36);
        p.feed(0xAA); // left released, right still down
        assert!(p.keyboard.shift());
        p.feed(0xB6);
        assert!(!p.keyboard.shift());
    }

    #[test]
    fn ctrl_and_alt_are_reported_with_the_key() {
        let s = type_bytes(&[0x1D, 0x2E, 0xAE, 0x9D]); // ctrl+c
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].action, Action::Char('c'));
        assert!(s[0].ctrl && !s[0].alt);
        let s = type_bytes(&[0x38, 0x1E]); // alt+a
        assert!(s[0].alt);
    }

    #[test]
    fn special_keys_pass_through_as_keys() {
        let s = type_bytes(&[0x1C, 0x0E, 0x0F, 0x01, 0x3B, 0x58]);
        let keys: Vec<Action> = s.iter().map(|i| i.action).collect();
        assert_eq!(
            keys,
            [Action::Key(Key::Enter), Action::Key(Key::Backspace), Action::Key(Key::Tab), Action::Key(Key::Escape), Action::Key(Key::F(1)), Action::Key(Key::F(12))]
        );
    }

    #[test]
    fn keyboard_noise_bytes_are_ignored() {
        // ACK, error, resend, and a full Pause key sequence
        let s = type_bytes(&[0xFA, 0xFF, 0xFE, 0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5, 0x1E]);
        assert_eq!(chars(&s), "a");
    }

    #[test]
    fn fake_shift_around_print_screen_is_ignored() {
        let mut p = Pipeline::new();
        p.feed(0xE0);
        p.feed(0x2A); // fake shift down
        assert!(!p.keyboard.shift());
    }

    #[test]
    fn unknown_scancodes_do_not_break_the_stream() {
        let s = type_bytes(&[0x7E, 0x1E]);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].action, Action::Key(Key::Unknown(0x7E)));
        assert_eq!(s[1].action, Action::Char('a'));
    }

    #[test]
    fn the_whole_us_layout_is_reachable() {
        let mut seen = std::collections::BTreeSet::new();
        for code in 0x01u8..=0x58 {
            let mut p = Pipeline::new();
            for shift in [false, true] {
                if shift {
                    p.feed(0x2A);
                }
                if let Some(Input { action: Action::Char(c), .. }) = p.feed(code) {
                    seen.insert(c);
                }
                if shift {
                    p.feed(0xAA);
                }
            }
        }
        for c in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 !@#$%^&*()-_=+[]{};:'\"`~\\|,<.>/?".chars() {
            assert!(seen.contains(&c), "cannot type {c:?}");
        }
    }

    #[test]
    fn keypad_digits_follow_numlock() {
        let mut p = Pipeline::new();
        assert_eq!(p.feed(0x48).map(|i| i.action), Some(Action::Char('8')));
        assert_eq!(p.feed(0x4C).map(|i| i.action), Some(Action::Char('5')));
        p.feed(0x45); // NumLock off
        assert_eq!(p.feed(0x48).map(|i| i.action), Some(Action::Key(Key::Up)));
        assert_eq!(p.feed(0x4C), None, "keypad 5 does nothing without NumLock");
        // Real arrow keys (0xE0 prefix) never type digits.
        p.feed(0x45); // NumLock on again
        p.feed(0xE0);
        assert_eq!(p.feed(0x48).map(|i| i.action), Some(Action::Key(Key::Up)));
        // Shift turns the keypad back into navigation.
        p.feed(0x2A);
        assert_eq!(p.feed(0x4B).map(|i| i.action), Some(Action::Key(Key::Left)));
    }

    #[test]
    fn num_lock_and_caps_lock_toggle_on_press_only() {
        let mut p = Pipeline::new();
        assert!(p.keyboard.num_lock);
        p.feed(0x45);
        assert!(!p.keyboard.num_lock);
        p.feed(0xC5); // release: no change
        assert!(!p.keyboard.num_lock);
        p.feed(0x3A);
        assert!(p.keyboard.caps_lock);
    }
}
