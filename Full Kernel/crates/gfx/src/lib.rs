//! Drawing without a screen: a [`Canvas`] is a block of 0x00RRGGBB pixels in
//! memory with clipped drawing operations, and [`Rect`] is the rectangle maths
//! the desktop's damage tracking and hit testing are built on. The kernel
//! copies finished canvases to the framebuffer.
//!
//! No hardware access, so everything is unit-tested on the host.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod font;

use alloc::vec;
use alloc::vec::Vec;

/// A colour as 0x00RRGGBB.
pub type Color = u32;

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// Mixes `a` and `b`: `t` = 0 gives `a`, 255 gives `b`.
pub const fn mix(a: Color, b: Color, t: u32) -> Color {
    const fn ch(a: Color, b: Color, t: u32, s: u32) -> Color {
        let x = (a >> s) & 0xFF;
        let y = (b >> s) & 0xFF;
        ((x * (255 - t) + y * t) / 255) << s
    }
    ch(a, b, t, 16) | ch(a, b, t, 8) | ch(a, b, t, 0)
}

/// A rectangle in pixels; may be partly or wholly off-canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The overlap of two rectangles (empty if they don't touch).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r <= x || b <= y { Rect::default() } else { Rect::new(x, y, r - x, b - y) }
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        !self.intersect(o).is_empty()
    }

    /// The smallest rectangle holding both (an empty one is ignored).
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }

    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Shrinks by `n` on every side.
    pub fn inset(&self, n: i32) -> Rect {
        Rect::new(self.x + n, self.y + n, self.w - 2 * n, self.h - 2 * n)
    }

    pub fn area(&self) -> i64 {
        if self.is_empty() { 0 } else { self.w as i64 * self.h as i64 }
    }
}

/// A block of pixels in memory.
#[derive(Clone)]
pub struct Canvas {
    w: usize,
    h: usize,
    px: Vec<Color>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, fill: Color) -> Self {
        Self { w, h, px: vec![fill; w * h] }
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w as i32, self.h as i32)
    }

    pub fn pixels(&self) -> &[Color] {
        &self.px
    }

    /// Row `y` from column `x0`, `n` pixels long (caller keeps it in bounds).
    pub fn span(&self, x0: usize, y: usize, n: usize) -> &[Color] {
        &self.px[y * self.w + x0..y * self.w + x0 + n]
    }

    pub fn get(&self, x: i32, y: i32) -> Option<Color> {
        self.bounds().contains(x, y).then(|| self.px[y as usize * self.w + x as usize])
    }

    pub fn put(&mut self, x: i32, y: i32, c: Color) {
        if self.bounds().contains(x, y) {
            self.px[y as usize * self.w + x as usize] = c;
        }
    }

    /// Fills `r` (clipped to the canvas).
    pub fn fill(&mut self, r: Rect, c: Color) {
        let r = r.intersect(&self.bounds());
        for y in r.y..r.bottom() {
            let start = y as usize * self.w + r.x as usize;
            self.px[start..start + r.w as usize].fill(c);
        }
    }

    /// Fills `r`, but only inside `clip`.
    pub fn fill_clipped(&mut self, r: Rect, c: Color, clip: Rect) {
        self.fill(r.intersect(&clip), c);
    }

    /// A one-pixel-wide frame along the inside of `r`.
    pub fn outline(&mut self, r: Rect, c: Color, clip: Rect) {
        self.fill_clipped(Rect::new(r.x, r.y, r.w, 1), c, clip);
        self.fill_clipped(Rect::new(r.x, r.bottom() - 1, r.w, 1), c, clip);
        self.fill_clipped(Rect::new(r.x, r.y, 1, r.h), c, clip);
        self.fill_clipped(Rect::new(r.right() - 1, r.y, 1, r.h), c, clip);
    }

    /// Copies `src_rect` of `src` so that its top-left lands at (`dx`, `dy`),
    /// only where it falls inside `clip` (and the canvas).
    pub fn blit(&mut self, src: &Canvas, src_rect: Rect, dx: i32, dy: i32, clip: Rect) {
        let src_rect = src_rect.intersect(&src.bounds());
        let dest = Rect::new(dx, dy, src_rect.w, src_rect.h).intersect(&clip).intersect(&self.bounds());
        if dest.is_empty() {
            return;
        }
        let (sx, sy) = (src_rect.x + dest.x - dx, src_rect.y + dest.y - dy);
        for row in 0..dest.h {
            let s = (sy + row) as usize * src.w + sx as usize;
            let d = (dest.y + row) as usize * self.w + dest.x as usize;
            self.px[d..d + dest.w as usize].copy_from_slice(&src.px[s..s + dest.w as usize]);
        }
    }

    /// A vertical gradient over the whole canvas.
    pub fn gradient(&mut self, top: Color, bottom: Color) {
        let h = self.h.max(2) - 1;
        for y in 0..self.h {
            let c = mix(top, bottom, (y * 255 / h) as u32);
            self.px[y * self.w..(y + 1) * self.w].fill(c);
        }
    }

    /// A four-pointed star centred at (cx, cy) with arm length `r`.
    pub fn star(&mut self, cx: i32, cy: i32, r: i32, c: Color) {
        let waist = (r / 5).max(1);
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() * dy.abs() <= waist * waist && dx.abs() + dy.abs() <= r {
                    self.put(cx + dx, cy + dy, c);
                }
            }
        }
    }

    /// Text in the 5x7 font, each font pixel `scale` x `scale`, clipped to `clip`.
    pub fn text(&mut self, x: i32, y: i32, scale: i32, c: Color, s: &str, clip: Rect) {
        let clip = clip.intersect(&self.bounds());
        let adv = (font::WIDTH as i32 + 1) * scale;
        for (i, ch) in s.chars().enumerate() {
            let ox = x + i as i32 * adv;
            if ox >= clip.right() {
                break;
            }
            if ox + adv <= clip.x {
                continue;
            }
            let g = font::glyph9(ch);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..font::WIDTH {
                    if bits & (0x10 >> col) != 0 {
                        self.fill_clipped(Rect::new(ox + col as i32 * scale, y + row as i32 * scale, scale, scale), c, clip);
                    }
                }
            }
        }
    }

    /// One character cell, (WIDTH+1) x (HEIGHT+2) font pixels, with its background.
    pub fn cell(&mut self, x: i32, y: i32, scale: i32, fg: Color, bg: Color, ch: char) {
        let (cw, chh) = cell_size(scale);
        self.fill(Rect::new(x, y, cw, chh), bg);
        let g = font::glyph9(ch);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..font::WIDTH {
                if bits & (0x10 >> col) != 0 {
                    self.fill(Rect::new(x + col as i32 * scale, y + row as i32 * scale, scale, scale), fg);
                }
            }
        }
    }
}

/// Width in pixels of `s` at `scale`.
pub fn text_width(s: &str, scale: i32) -> i32 {
    let n = s.chars().count() as i32;
    if n == 0 { 0 } else { (n * (font::WIDTH as i32 + 1) - 1) * scale }
}

/// Height of a line of text (with descenders) at `scale`.
pub fn text_height(scale: i32) -> i32 {
    (font::HEIGHT as i32 + 2) * scale
}

/// Size of a console cell at `scale`.
pub fn cell_size(scale: i32) -> (i32, i32) {
    ((font::WIDTH as i32 + 1) * scale, (font::HEIGHT as i32 + 2) * scale)
}

/// Cuts `s` to at most `max_px` wide at `scale`, ending in ".." if shortened.
pub fn fit(s: &str, scale: i32, max_px: i32) -> alloc::string::String {
    if text_width(s, scale) <= max_px {
        return alloc::string::String::from(s);
    }
    let per = (font::WIDTH as i32 + 1) * scale;
    let n = ((max_px / per) - 2).max(0) as usize;
    let mut out: alloc::string::String = s.chars().take(n).collect();
    out.push_str("..");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_maths() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Rect::new(5, 5, 5, 5));
        assert!(a.intersect(&Rect::new(10, 0, 5, 5)).is_empty(), "touching edges don't overlap");
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert_eq!(Rect::default().union(&b), b);
        assert!(a.contains(9, 9) && !a.contains(10, 9));
        assert_eq!(a.inset(2), Rect::new(2, 2, 6, 6));
        assert_eq!(b.offset(-5, 1), Rect::new(0, 6, 10, 10));
        assert_eq!(Rect::new(0, 0, -1, 5).area(), 0);
    }

    #[test]
    fn fill_is_clipped() {
        let mut c = Canvas::new(4, 4, 0);
        c.fill(Rect::new(-2, -2, 4, 4), 7);
        assert_eq!(c.get(0, 0), Some(7));
        assert_eq!(c.get(1, 1), Some(7));
        assert_eq!(c.get(2, 2), Some(0));
        c.fill(Rect::new(3, 3, 100, 100), 9);
        assert_eq!(c.get(3, 3), Some(9));
        assert_eq!(c.get(4, 4), None);
        c.fill_clipped(Rect::new(0, 0, 4, 4), 5, Rect::new(0, 3, 1, 1));
        assert_eq!(c.pixels().iter().filter(|&&p| p == 5).count(), 1);
    }

    #[test]
    fn blit_with_offsets_and_clip() {
        let mut src = Canvas::new(3, 3, 0);
        for y in 0..3 {
            for x in 0..3 {
                src.put(x, y, (y * 3 + x) as u32 + 1);
            }
        }
        let mut dst = Canvas::new(5, 5, 0);
        // Whole source at (-1, -1): only its bottom-right 2x2 lands.
        dst.blit(&src, src.bounds(), -1, -1, dst.bounds());
        assert_eq!(dst.get(0, 0), Some(5));
        assert_eq!(dst.get(1, 1), Some(9));
        assert_eq!(dst.get(2, 2), Some(0));
        // A clip lets through one pixel.
        let mut dst = Canvas::new(5, 5, 0);
        dst.blit(&src, src.bounds(), 2, 2, Rect::new(3, 3, 1, 1));
        assert_eq!(dst.pixels().iter().filter(|&&p| p != 0).count(), 1);
        assert_eq!(dst.get(3, 3), Some(5));
        // Part of the source.
        let mut dst = Canvas::new(5, 5, 0);
        dst.blit(&src, Rect::new(1, 1, 2, 1), 0, 4, dst.bounds());
        assert_eq!((dst.get(0, 4), dst.get(1, 4)), (Some(5), Some(6)));
    }

    #[test]
    fn colours() {
        assert_eq!(rgb(0x12, 0x34, 0x56), 0x123456);
        assert_eq!(mix(0, 0xFFFFFF, 0), 0);
        assert_eq!(mix(0, 0xFFFFFF, 255), 0xFFFFFF);
        assert_eq!(mix(0x000000, 0x0000FF, 128), 0x80);
        let mut c = Canvas::new(1, 3, 0);
        c.gradient(0x000000, 0xFF0000);
        assert_eq!((c.get(0, 0), c.get(0, 2)), (Some(0), Some(0xFF0000)));
    }

    #[test]
    fn text_draws_and_clips() {
        let mut c = Canvas::new(40, 12, 0);
        c.text(0, 0, 1, 1, "I", c.bounds());
        // 'I' is a vertical bar in column 2 with serifs: (2, 3) is lit.
        assert_eq!(c.get(2, 3), Some(1));
        let lit = c.pixels().iter().filter(|&&p| p == 1).count();
        assert!(lit > 5);
        let mut c2 = Canvas::new(40, 12, 0);
        c2.text(0, 0, 1, 1, "II", Rect::new(0, 0, 6, 12));
        assert_eq!(c2.pixels().iter().filter(|&&p| p == 1).count(), lit, "second glyph clipped away");
        assert_eq!(text_width("ab", 2), 22);
        assert_eq!(cell_size(2), (12, 18));
        assert_eq!(fit("hello world", 1, 30), "hel..");
        assert_eq!(fit("hi", 1, 30), "hi");
    }

    #[test]
    fn cell_paints_background() {
        let mut c = Canvas::new(6, 9, 0);
        c.cell(0, 0, 1, 1, 2, ' ');
        assert!(c.pixels().iter().all(|&p| p == 2));
    }
}
