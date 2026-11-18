//! Bits the apps share: the keys they understand and how a button looks.

use crate::{DIM, INK};
use nova_gfx::{Canvas, Color, Rect, mix, rgb, text_height, text_width};

/// A key as the apps see it (the kernel maps real keyboard input onto this).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyIn {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    Escape,
    /// Ctrl + a letter (lower case).
    Ctrl(char),
}

pub const FACE: Color = rgb(0x18, 0x1D, 0x40);
pub const FACE_STRONG: Color = rgb(0x24, 0x5A, 0x63);
pub const FACE_SOFT: Color = rgb(0x12, 0x16, 0x34);
pub const EDGE: Color = rgb(0x2A, 0x31, 0x68);

/// A flat button with a centred label.
pub fn button(c: &mut Canvas, r: Rect, label: &str, scale: i32, u: i32, face: Color, ink: Color) {
    let clip = c.bounds();
    c.fill_clipped(r, mix(face, EDGE, 120), clip);
    c.fill_clipped(r.inset(u), face, clip);
    let s = if text_width(label, scale) > r.w - 4 * u { (scale / 2).max(1) } else { scale };
    let lx = r.x + (r.w - text_width(label, s)) / 2;
    let ly = r.y + (r.h - text_height(s)) / 2 + u;
    c.text(lx, ly, s, ink, label, r);
}

/// A button whose label is dimmed (not available right now).
pub fn button_off(c: &mut Canvas, r: Rect, label: &str, scale: i32, u: i32) {
    button(c, r, label, scale, u, FACE_SOFT, mix(FACE_SOFT, DIM, 150));
}

/// The usual button.
pub fn button_on(c: &mut Canvas, r: Rect, label: &str, scale: i32, u: i32) {
    button(c, r, label, scale, u, FACE, INK);
}
