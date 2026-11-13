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

// SAFETY: the pointer is the framebuffer's kernel mapping, valid on every CPU; users
// serialise their drawing themselves (the GUI keeps its Screen behind a lock).
unsafe impl Send for Screen {}

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

    /// Copies rectangle `r` of `canvas` (0x00RRGGBB pixels, same size as the
    /// screen) to the same place on screen.
    pub fn present(&mut self, canvas: &nova_gfx::Canvas, r: nova_gfx::Rect) {
        let r = r.intersect(&nova_gfx::Rect::new(0, 0, self.width().min(canvas.width()) as i32, self.height().min(canvas.height()) as i32));
        if r.is_empty() {
            return;
        }
        let same_layout = self.fb.pack(0x12, 0x34, 0x56) == 0x0012_3456;
        let stride = self.fb.stride as usize;
        for y in r.y as usize..r.bottom() as usize {
            let src = canvas.span(r.x as usize, y, r.w as usize);
            // SAFETY: the row segment lies inside the framebuffer (clipped above).
            let dst = unsafe { self.base.add(y * stride + r.x as usize) };
            if same_layout {
                // SAFETY: `src` and the framebuffer don't overlap; both are r.w pixels long.
                unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len()) };
            } else {
                for (i, &p) in src.iter().enumerate() {
                    let px = self.fb.pack((p >> 16) as u8, (p >> 8) as u8, p as u8);
                    // SAFETY: as above.
                    unsafe { dst.add(i).write_volatile(px) };
                }
            }
        }
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
