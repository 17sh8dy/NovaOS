//! Crash reports: one text report that goes to the serial log as it is
//! written and is then drawn on the screen, so a fault is visible both in a
//! terminal and in the QEMU window (or on real hardware with no serial port).
//!
//! Everything here must work in the worst situation the kernel can be in:
//! no heap, a possibly tiny stack, and possibly a held log lock.

use crate::font;
use crate::log::SpinLock;
use crate::screen::{Line, Rgb, Screen};
use core::fmt::{self, Write};
use nova_bootinfo::Framebuffer;

/// Where to draw crash screens, once known: (framebuffer, its virtual address).
static SCREEN: SpinLock<Option<(Framebuffer, u64)>> = SpinLock::new(None);

/// Tells the crash screen where the framebuffer is.
pub fn set_framebuffer(fb: Framebuffer, virt: u64) {
    if fb.is_present() {
        SCREEN.with(|s| *s = Some((fb, virt)));
    }
}

const MAX_LINES: usize = 40;
/// Longest line that fits on the screen at the smallest text size we use.
const WRAP: usize = 78;

pub struct Report {
    title: &'static str,
    lines: [Line; MAX_LINES],
    n: usize,
    heads: usize,
}

impl Report {
    /// Starts a report; the title is printed to serial straight away.
    pub fn new(title: &'static str) -> Self {
        crate::kprintln!();
        crate::kprintln!("!!! {title} !!!");
        Self { title, lines: core::array::from_fn(|_| Line::new()), n: 0, heads: 0 }
    }

    fn push(&mut self, args: fmt::Arguments, head: bool) {
        crate::kprintln!("{args}");
        if self.n < MAX_LINES {
            let _ = self.lines[self.n].write_fmt(args);
            self.n += 1;
            if head {
                self.heads = self.n;
            }
        }
    }

    /// A headline (drawn in the accent colour). Add these before body lines.
    pub fn head(&mut self, args: fmt::Arguments) {
        self.push(args, true);
    }

    pub fn line(&mut self, args: fmt::Arguments) {
        self.push(args, false);
    }

    pub fn blank(&mut self) {
        self.push(format_args!(""), false);
    }

    /// Adds free text, wrapped to the screen width.
    pub fn wrapped(&mut self, args: fmt::Arguments) {
        let mut text = TextBuf::<512>::new();
        let _ = text.write_fmt(args);
        let mut chars = text.as_str().chars().peekable();
        while chars.peek().is_some() {
            let mut l = Line::new();
            let mut count = 0;
            while count < WRAP {
                let Some(c) = chars.next() else { break };
                if c == '\n' {
                    break;
                }
                let _ = l.write_char(c);
                count += 1;
            }
            self.line(format_args!("{}", l.as_str()));
        }
    }

    /// Draws the report, if there is a screen to draw on.
    pub fn show(self) {
        let Some((fb, virt)) = SCREEN.with(|s| *s) else { return };
        // SAFETY: the crash path never returns, so nothing else draws any more;
        // `virt` was registered by the kernel as the framebuffer's mapping.
        let mut s = unsafe { Screen::new(fb, virt) };
        let scale = (s.height() / 400).min(s.width() / (font::WIDTH + 1) / 90).max(1);
        s.gradient(Rgb(0x2E, 0x08, 0x14), Rgb(0x0A, 0x05, 0x10));
        let (ink, accent, red) = (Rgb(0xE8, 0xEC, 0xFF), Rgb(0xFF, 0xB4, 0x8A), Rgb(0xFF, 0x6B, 0x7A));
        let x = 24 * scale;
        let mut y = 20 * scale;
        s.text(x, y, scale * 2, red, self.title);
        y += (font::HEIGHT + 5) * scale * 2;
        let step = (font::HEIGHT + 3) * scale;
        for (i, l) in self.lines[..self.n].iter().enumerate() {
            let c = if i < self.heads { accent } else { ink };
            s.text(x, y, scale, c, l.as_str());
            y += step;
        }
        s.text(x, y + step, scale, Rgb(0x8C, 0x94, 0xC0), "THE SYSTEM HAS STOPPED. RESTART THE VM OR THE PC.");
    }
}

/// A fixed-capacity string buffer that silently truncates.
pub struct TextBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> TextBuf<N> {
    pub const fn new() -> Self {
        Self { buf: [0; N], len: 0 }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> Write for TextBuf<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut take = s.len().min(N - self.len);
        while !s.is_char_boundary(take) {
            take -= 1;
        }
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}
