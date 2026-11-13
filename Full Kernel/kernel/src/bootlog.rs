//! The boot log on screen, from the kernel's first line until the desktop
//! takes over: every log line is drawn straight onto the framebuffer. On a
//! real PC there is usually no serial port, so this (and the crash screen) is
//! how anyone sees where a boot got to.

use crate::font;
use crate::log::SpinLock;
use crate::screen::{Rgb, Screen};
use nova_bootinfo::Framebuffer;

struct Early {
    screen: Screen,
    scale: usize,
    y: usize,
}

static EARLY: SpinLock<Option<Early>> = SpinLock::new(None);

const BG: Rgb = Rgb(0x07, 0x09, 0x1C);
const INK: Rgb = Rgb(0x8C, 0x94, 0xC0);
const TITLE: Rgb = Rgb(0x6E, 0xE7, 0xD8);

fn top(scale: usize) -> usize {
    (font::HEIGHT + 6) * scale * 2 + 8 * scale
}

fn clear(e: &mut Early) {
    let (w, h, s) = (e.screen.width(), e.screen.height(), e.scale);
    e.screen.fill_rect(0, 0, w, h, BG);
    e.screen.text(8 * s, 6 * s, s * 2, TITLE, "NOVA IS STARTING - BOOT LOG");
    e.y = top(s);
}

fn line(text: &str) {
    // try_with: never wait while the log is locked (this runs inside it).
    EARLY.try_with(|e| {
        let Some(e) = e.as_mut() else { return };
        let s = e.scale;
        let step = (font::HEIGHT + 3) * s;
        if e.y + step > e.screen.height() {
            clear(e);
        }
        let max = e.screen.width() / ((font::WIDTH + 1) * s);
        let shown = &text[..text.char_indices().nth(max.saturating_sub(2)).map_or(text.len(), |(i, _)| i)];
        e.screen.text(8 * s, e.y, s, INK, shown);
        e.y += step;
    });
}

/// Starts drawing log lines on the screen.
///
/// # Safety
/// `virt` must be the framebuffer's mapping, and nothing else may draw until [`stop`].
pub unsafe fn start(fb: Framebuffer, virt: u64) {
    if !fb.is_present() {
        return;
    }
    // SAFETY: as the caller promises.
    let screen = unsafe { Screen::new(fb, virt) };
    let scale = (screen.height() / 700).max(1);
    let mut e = Early { screen, scale, y: 0 };
    clear(&mut e);
    EARLY.with(|slot| *slot = Some(e));
    crate::log::set_mirror(Some(line));
}

/// The desktop is taking over the screen.
pub fn stop() {
    crate::log::set_mirror(None);
    EARLY.with(|slot| *slot = None);
}
