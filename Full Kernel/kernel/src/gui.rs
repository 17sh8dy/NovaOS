//! The on-screen desktop for now: a compact splash at the top, a scrolling
//! text console below it, a status bar at the bottom, and the mouse pointer
//! on top of everything.
//!
//! One lock (`GUI`) guards the framebuffer, the console grid and the pointer,
//! because the shell thread (console text) and the main thread (mouse, clock)
//! both draw. Drawing is incremental: only console rows that changed are
//! repainted, and the pointer is lifted off the screen while anything under
//! it is redrawn.

use crate::cursor::Cursor;
use crate::font;
use crate::log::SpinLock;
use crate::screen::{Rgb, Screen};
use nova_bootinfo::BootInfo;
use nova_console::Grid;
use nova_ps2::{Change, MouseEvent, Pointer};

/// Colours the console can ask for (indices stored in the grid).
pub const COLOR_TEXT: u8 = 0;
pub const COLOR_ACCENT: u8 = 1;
pub const COLOR_ERROR: u8 = 2;
pub const COLOR_DIM: u8 = 3;

const PALETTE: [Rgb; 4] = [Rgb(0xE8, 0xEC, 0xFF), Rgb(0x6E, 0xE7, 0xD8), Rgb(0xFF, 0x6B, 0x7A), Rgb(0x8C, 0x94, 0xC0)];
const PANEL_BG: Rgb = Rgb(0x0A, 0x0C, 0x1E);

struct Gui {
    screen: Screen,
    cursor: Cursor,
    pointer: Pointer,
    grid: Grid,
    /// Top-left pixel of the console area, and its cell size / scale.
    origin: (usize, usize),
    scale: usize,
    blink_on: bool,
    uptime_secs: u64,
    cpus: usize,
}

static GUI: SpinLock<Option<Gui>> = SpinLock::new(None);

impl Gui {
    fn cell_size(&self) -> (usize, usize) {
        ((font::WIDTH + 1) * self.scale, (font::HEIGHT + 2) * self.scale)
    }

    /// Repaints the console rows that changed (and the block cursor).
    fn render_console(&mut self) {
        let dirty: alloc::vec::Vec<usize> = (0..self.grid.rows()).filter(|&r| self.grid.is_dirty(r)).collect();
        if dirty.is_empty() {
            return;
        }
        self.cursor.hide(&mut self.screen);
        let (cw, ch) = self.cell_size();
        let bg = self.screen.pack(PANEL_BG);
        let (cx, cy) = self.grid.cursor();
        for row in dirty {
            for col in 0..self.grid.cols() {
                let cell = self.grid.cell(col, row);
                let fg = self.screen.pack(PALETTE[(cell.color as usize).min(PALETTE.len() - 1)]);
                let (x, y) = (self.origin.0 + col * cw, self.origin.1 + row * ch);
                if self.blink_on && (col, row) == (cx, cy) {
                    // The text cursor: a solid block with the glyph knocked out of it.
                    self.screen.draw_cell(x, y, self.scale, bg, fg, cell.ch);
                } else {
                    self.screen.draw_cell(x, y, self.scale, fg, bg, cell.ch);
                }
            }
            self.grid.clean(row);
        }
        self.cursor.show(&mut self.screen, self.pointer.x as usize, self.pointer.y as usize);
    }

    fn draw_status(&mut self) {
        let unit = self.scale / 2;
        let unit = unit.max(1);
        let h = (font::HEIGHT + 6) * unit * 2;
        let (x, y) = (self.origin.0, self.screen.height().saturating_sub(h + 6 * unit));
        let w = self.screen.width().saturating_sub(2 * self.origin.0);
        self.cursor.hide(&mut self.screen);
        self.screen.fill_rect(x, y, w, h, Rgb(0x10, 0x14, 0x30));
        let on = |down: bool, c: &'static str| if down { c } else { "-" };
        let line = crate::fmt_line!(
            "mouse {:>4},{:>4} {}{}{}   up {}s   {} cpu(s)",
            self.pointer.x,
            self.pointer.y,
            on(self.pointer.buttons.left(), "L"),
            on(self.pointer.buttons.right(), "R"),
            on(self.pointer.buttons.middle(), "M"),
            self.uptime_secs,
            self.cpus
        );
        self.screen.text(x + 6 * unit * 2, y + 4 * unit * 2, unit * 2, PALETTE[1], line.as_str());
        self.cursor.show(&mut self.screen, self.pointer.x as usize, self.pointer.y as usize);
    }
}

/// Draws the splash, the console frame and the status bar, and starts the GUI.
pub fn init(mut screen: Screen, info: &BootInfo, free_mib: u64, cpus: usize) {
    let (w, h) = (screen.width(), screen.height());
    let u = (h / 800).max(1);
    screen.gradient(Rgb(0x07, 0x09, 0x1C), Rgb(0x1B, 0x10, 0x3A));

    // Compact splash.
    let (ink, dim, accent) = (PALETTE[0], PALETTE[3], PALETTE[1]);
    let cx = w / 2;
    let mut y = 10 * u;
    let star = 22 * u;
    screen.star(cx, y + star, star, accent);
    y += 2 * star + 8 * u;
    screen.text_centered(y, 4 * u, ink, "NOVA OS");
    y += font::HEIGHT * 4 * u + 6 * u;
    screen.text_centered(y, 2 * u, dim, "FULL KERNEL - STAGE 8: USER SPACE");
    y += font::HEIGHT * 2 * u + 6 * u;
    let lines = [
        crate::fmt_line!("{}x{}  -  {} MIB FREE  -  KERNEL AT {:#X}", info.framebuffer.width, info.framebuffer.height, free_mib, info.kernel_virt),
    ];
    for l in &lines {
        screen.text_centered(y, u, ink, l.as_str());
        y += font::HEIGHT * u + 4 * u;
    }

    // Console panel.
    let scale = (2 * u).min(3);
    let margin = 16 * u;
    let top = y + 8 * u;
    let status_h = (font::HEIGHT + 6) * u * 4 + 12 * u;
    let avail_h = h.saturating_sub(top + status_h);
    let cols = (w - 2 * margin) / ((font::WIDTH + 1) * scale);
    let rows = avail_h / ((font::HEIGHT + 2) * scale);
    screen.fill_rect(margin - 4 * u, top - 4 * u, cols * (font::WIDTH + 1) * scale + 8 * u, rows * (font::HEIGHT + 2) * scale + 8 * u, PANEL_BG);

    let gui = Gui {
        cursor: Cursor::new(h),
        pointer: Pointer::centered(w, h),
        grid: Grid::new(cols, rows),
        origin: (margin, top),
        scale,
        blink_on: true,
        uptime_secs: 0,
        cpus,
        screen,
    };
    GUI.with(|g| *g = Some(gui));
    with_gui(|g| {
        g.draw_status();
        g.render_console();
        g.cursor.show(&mut g.screen, g.pointer.x as usize, g.pointer.y as usize);
    });
}

fn with_gui<R>(f: impl FnOnce(&mut Gui) -> R) -> Option<R> {
    GUI.with(|g| g.as_mut().map(f))
}

/// Columns and rows of the console, if the GUI is up.
pub fn console_size() -> Option<(usize, usize)> {
    with_gui(|g| (g.grid.cols(), g.grid.rows()))
}

/// Runs `f` on the console grid, then repaints whatever it changed.
pub fn with_console<R>(f: impl FnOnce(&mut Grid) -> R) -> Option<R> {
    with_gui(|g| {
        let r = f(&mut g.grid);
        g.render_console();
        r
    })
}

/// Writes text to the console in a colour.
pub fn console_write(text: &str, color: u8) {
    with_console(|grid| {
        grid.set_color(color);
        grid.write_str(text);
        grid.set_color(COLOR_TEXT);
    });
}

/// Applies a mouse event: moves the pointer and refreshes the status bar.
pub fn mouse(ev: MouseEvent) -> Option<(Change, i32, i32)> {
    with_gui(|g| {
        let change = g.pointer.apply(ev);
        if !change.is_empty() {
            g.draw_status();
        }
        (change, g.pointer.x, g.pointer.y)
    })
}

/// Once-a-second housekeeping from the main loop: the clock in the status bar.
pub fn set_uptime(secs: u64) {
    with_gui(|g| {
        g.uptime_secs = secs;
        g.draw_status();
    });
}

/// Updates the CPU count shown in the status bar.
pub fn set_cpus(n: usize) {
    with_gui(|g| {
        g.cpus = n;
        g.draw_status();
    });
}

/// Toggles the text cursor's blink phase and repaints its row.
pub fn blink() {
    with_gui(|g| {
        g.blink_on = !g.blink_on;
        let (_, cy) = g.grid.cursor();
        g.grid.mark_dirty(cy);
        g.render_console();
    });
}
