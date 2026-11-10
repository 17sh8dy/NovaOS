//! Drawing on the boot framebuffer.

use crate::font;
use core::fmt;
use nova_bootinfo::Framebuffer;

#[derive(Clone, Copy)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub struct Screen {
    base: *mut u32,
    fb: Framebuffer,
}

impl Screen {
    /// # Safety
    /// `virt` must be the framebuffer's mapped virtual address and nothing
    /// else may draw to it at the same time.
    pub unsafe fn new(fb: Framebuffer, virt: u64) -> Self {
        Self { base: virt as *mut u32, fb }
    }

    pub fn width(&self) -> usize {
        self.fb.width as usize
    }

    pub fn height(&self) -> usize {
        self.fb.height as usize
    }

    #[inline]
    pub fn put(&mut self, x: usize, y: usize, c: Rgb) {
        if x < self.width() && y < self.height() {
            let px = self.fb.pack(c.0, c.1, c.2);
            // SAFETY: (x, y) is inside the framebuffer (checked above).
            unsafe { self.base.add(y * self.fb.stride as usize + x).write_volatile(px) };
        }
    }

    /// The raw pixel at (x, y); 0 if outside the screen.
    pub fn get_raw(&self, x: usize, y: usize) -> u32 {
        if x < self.width() && y < self.height() {
            // SAFETY: (x, y) is inside the framebuffer (checked above).
            unsafe { self.base.add(y * self.fb.stride as usize + x).read_volatile() }
        } else {
            0
        }
    }

    /// Writes a raw (already packed) pixel; ignored outside the screen.
    pub fn put_raw(&mut self, x: usize, y: usize, px: u32) {
        if x < self.width() && y < self.height() {
            // SAFETY: (x, y) is inside the framebuffer (checked above).
            unsafe { self.base.add(y * self.fb.stride as usize + x).write_volatile(px) };
        }
    }

    pub fn pack(&self, c: Rgb) -> u32 {
        self.fb.pack(c.0, c.1, c.2)
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: Rgb) {
        for yy in y..(y + h).min(self.height()) {
            for xx in x..(x + w).min(self.width()) {
                self.put(xx, yy, c);
            }
        }
    }

    /// A vertical gradient over the whole screen.
    pub fn gradient(&mut self, top: Rgb, bottom: Rgb) {
        let h = self.height().max(1);
        for y in 0..self.height() {
            let mix = |a: u8, b: u8| ((a as usize * (h - y) + b as usize * y) / h) as u8;
            let row = Rgb(mix(top.0, bottom.0), mix(top.1, bottom.1), mix(top.2, bottom.2));
            let px = self.fb.pack(row.0, row.1, row.2);
            for x in 0..self.width() {
                // SAFETY: inside the framebuffer.
                unsafe { self.base.add(y * self.fb.stride as usize + x).write_volatile(px) };
            }
        }
    }

    /// A four-pointed star centred at (cx, cy) with arm length `r`.
    pub fn star(&mut self, cx: usize, cy: usize, r: usize, c: Rgb) {
        let r = r as i64;
        let waist = (r / 5).max(1);
        for dy in -r..=r {
            for dx in -r..=r {
                // Inside the star where |dx|·|dy| is small: concave sides
                // that meet in four sharp points.
                if dx.abs() * dy.abs() <= waist * waist && dx.abs() + dy.abs() <= r {
                    self.put((cx as i64 + dx) as usize, (cy as i64 + dy) as usize, c);
                }
            }
        }
    }

    pub fn text(&mut self, x: usize, y: usize, scale: usize, c: Rgb, s: &str) {
        for (i, ch) in s.chars().enumerate() {
            let g = font::glyph(ch);
            let ox = x + i * (font::WIDTH + 1) * scale;
            for (row, bits) in g.iter().enumerate() {
                for col in 0..font::WIDTH {
                    if bits & (0x10 >> col) != 0 {
                        self.fill_rect(ox + col * scale, y + row * scale, scale, scale, c);
                    }
                }
            }
        }
    }

    /// Width in pixels of `s` drawn at `scale`.
    pub fn text_width(s: &str, scale: usize) -> usize {
        let n = s.chars().count();
        if n == 0 { 0 } else { (n * (font::WIDTH + 1) - 1) * scale }
    }

    /// Draws `s` centred horizontally, shrinking it if it would not fit.
    pub fn text_centered(&mut self, y: usize, scale: usize, c: Rgb, s: &str) {
        let mut scale = scale.max(1);
        while scale > 1 && Self::text_width(s, scale) + 32 > self.width() {
            scale -= 1;
        }
        let w = Self::text_width(s, scale);
        let x = self.width().saturating_sub(w) / 2;
        self.text(x, y, scale, c, s);
    }
}

/// A fixed-capacity string for formatting text without a heap.
pub struct Line {
    buf: [u8; 96],
    len: usize,
}

impl Line {
    pub fn new() -> Self {
        Self { buf: [0; 96], len: 0 }
    }

    pub fn as_str(&self) -> &str {
        // Only whole &str pieces are ever copied in, so this is valid UTF-8
        // unless a piece was cut; fall back to "" in that case.
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = self.buf.len() - self.len;
        let mut take = s.len().min(room);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

#[macro_export]
macro_rules! fmt_line {
    ($($arg:tt)*) => {{
        let mut l = $crate::screen::Line::new();
        let _ = core::fmt::Write::write_fmt(&mut l, format_args!($($arg)*));
        l
    }};
}
