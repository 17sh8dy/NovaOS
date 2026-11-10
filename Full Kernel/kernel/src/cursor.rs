//! A software mouse cursor drawn straight onto the framebuffer.
//!
//! The pixels under the cursor are saved before it is drawn and put back
//! before it moves, so it never leaves a trail. (A hardware cursor, or a
//! compositor, replaces this much later.)

use crate::screen::{Rgb, Screen};

const ARROW: [&str; 17] = [
    "X          ",
    "XX         ",
    "X.X        ",
    "X..X       ",
    "X...X      ",
    "X....X     ",
    "X.....X    ",
    "X......X   ",
    "X.......X  ",
    "X........X ",
    "X.....XXXXX",
    "X..X..X    ",
    "X.X X..X   ",
    "XX  X..X   ",
    "X    X..X  ",
    "     X..X  ",
    "      XX   ",
];
const W: usize = 11;
const H: usize = 17;
const MAX_SCALE: usize = 3;

pub struct Cursor {
    scale: usize,
    /// Where the cursor is drawn now, if it is on screen.
    at: Option<(usize, usize)>,
    saved: [u32; W * H * MAX_SCALE * MAX_SCALE],
}

impl Cursor {
    /// `screen_height` picks the size: 1x up to ~1400 px tall, then larger.
    pub fn new(screen_height: usize) -> Self {
        Self { scale: (screen_height / 700).clamp(1, MAX_SCALE), at: None, saved: [0; W * H * MAX_SCALE * MAX_SCALE] }
    }

    fn size(&self) -> (usize, usize) {
        (W * self.scale, H * self.scale)
    }

    /// Puts back the pixels under the cursor.
    pub fn hide(&mut self, s: &mut Screen) {
        let Some((x0, y0)) = self.at.take() else { return };
        let (w, h) = self.size();
        for y in 0..h {
            for x in 0..w {
                s.put_raw(x0 + x, y0 + y, self.saved[y * w + x]);
            }
        }
    }

    /// Draws the cursor with its tip at (x0, y0).
    pub fn show(&mut self, s: &mut Screen, x0: usize, y0: usize) {
        self.hide(s);
        let (w, h) = self.size();
        let (black, white) = (s.pack(Rgb(0, 0, 0)), s.pack(Rgb(255, 255, 255)));
        for y in 0..h {
            for x in 0..w {
                self.saved[y * w + x] = s.get_raw(x0 + x, y0 + y);
                match ARROW[y / self.scale].as_bytes()[x / self.scale] {
                    b'X' => s.put_raw(x0 + x, y0 + y, black),
                    b'.' => s.put_raw(x0 + x, y0 + y, white),
                    _ => {}
                }
            }
        }
        self.at = Some((x0, y0));
    }
}
