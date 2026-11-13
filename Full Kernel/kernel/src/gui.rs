//! The screen: the Nova desktop (`crates/desktop`) drawn into an off-screen
//! canvas and copied to the framebuffer one damaged rectangle at a time.
//!
//! Three windows: **Terminal** (the shell's console: same API as before, so
//! the shell and user programs did not change), **Files** and **System**
//! (their data is fetched by `desktop.rs`, outside this module's lock).
//!
//! One lock (`GUI`) guards the framebuffer, the back buffer, the desktop and
//! the console grid, because the shell thread (console text), user processes
//! (their output) and the main thread (mouse, clock, apps) all draw. Every
//! change records damage; [`Gui::flush`] composes and presents just that.

use crate::log::SpinLock;
use crate::screen::Screen;
use alloc::string::String;
use alloc::vec::Vec;
use nova_bootinfo::BootInfo;
use nova_console::Grid;
use nova_desktop::apps::FilesView;
use nova_desktop::{Desktop, Event, PANEL};
use nova_gfx::{Canvas, Color, Rect, cell_size, rgb};
use nova_ps2::{Change, MouseEvent, Pointer};

/// Colours the console can ask for (indices stored in the grid).
pub const COLOR_TEXT: u8 = 0;
pub const COLOR_ACCENT: u8 = 1;
pub const COLOR_ERROR: u8 = 2;
pub const COLOR_DIM: u8 = 3;

const PALETTE: [Color; 4] = [rgb(0xE8, 0xEC, 0xFF), rgb(0x6E, 0xE7, 0xD8), rgb(0xFF, 0x6B, 0x7A), rgb(0x8C, 0x94, 0xC0)];

pub const TERMINAL: u32 = 1;
pub const FILES: u32 = 2;
pub const SYSTEM: u32 = 3;

/// The line under the name on the wallpaper.
pub const SUBTITLE: &str = "Full Kernel - Stage 10";

pub struct Gui {
    screen: Screen,
    back: Canvas,
    pub desk: Desktop,
    pointer: Pointer,
    grid: Grid,
    term_scale: i32,
    term_pad: i32,
    blink_on: bool,
    pub files: FilesView,
    /// Font scale for app content.
    pub scale: i32,
    pub boot_lines: Vec<(String, String)>,
}

static GUI: SpinLock<Option<Gui>> = SpinLock::new(None);

impl Gui {
    /// Composes and presents everything that changed.
    pub fn flush(&mut self) {
        for r in self.desk.take_damage() {
            self.desk.compose(&mut self.back, r);
            self.screen.present(&self.back, r);
        }
        crate::ps2::set_terminal_focus(self.desk.focused() == Some(TERMINAL));
    }

    /// Repaints the console rows that changed (and the block cursor) into the Terminal window.
    fn render_console(&mut self) {
        let dirty: Vec<usize> = (0..self.grid.rows()).filter(|&r| self.grid.is_dirty(r)).collect();
        if dirty.is_empty() {
            return;
        }
        let (cw, ch) = cell_size(self.term_scale);
        let (pad, scale) = (self.term_pad, self.term_scale);
        let (cx, cy) = self.grid.cursor();
        let show_cursor = self.blink_on && self.desk.focused() == Some(TERMINAL);
        let cols = self.grid.cols();
        let Some(canvas) = self.desk.content_mut(TERMINAL) else { return };
        for &row in &dirty {
            for col in 0..cols {
                let cell = self.grid.cell(col, row);
                let fg = PALETTE[(cell.color as usize).min(PALETTE.len() - 1)];
                let (x, y) = (pad + col as i32 * cw, pad + row as i32 * ch);
                if show_cursor && (col, row) == (cx, cy) {
                    canvas.cell(x, y, scale, PANEL, fg, cell.ch);
                } else {
                    canvas.cell(x, y, scale, fg, PANEL, cell.ch);
                }
            }
        }
        for row in dirty {
            self.grid.clean(row);
            self.desk.damage_content(TERMINAL, Rect::new(pad, pad + row as i32 * ch, cols as i32 * cw, ch));
        }
    }

    /// Redraws the Files window from its view.
    pub fn render_files(&mut self) {
        let Some(c) = self.desk.content_mut(FILES) else { return };
        self.files.render(c);
        let all = c.bounds();
        self.desk.damage_content(FILES, all);
    }

    /// Redraws the System window with these lines.
    pub fn render_system(&mut self, lines: &[(String, String)]) {
        let (scale, u) = (self.scale, self.desk.m.u);
        let Some(c) = self.desk.content_mut(SYSTEM) else { return };
        nova_desktop::apps::render_info(c, scale, u, lines);
        let all = c.bounds();
        self.desk.damage_content(SYSTEM, all);
    }

    /// Size of a window's content.
    pub fn content_size(&self, id: u32) -> (i32, i32) {
        self.desk.window(id).map_or((0, 0), |w| (w.content.width() as i32, w.content.height() as i32))
    }

    /// Opens (or brings back) a window.
    pub fn show(&mut self, id: u32) {
        self.desk.show(id);
        self.refresh_cursor();
    }

    /// The terminal's cursor shows only while it has focus: repaint its row.
    pub fn refresh_cursor(&mut self) {
        let (_, cy) = self.grid.cursor();
        self.grid.mark_dirty(cy);
        self.render_console();
    }
}

/// Builds the desktop: wallpaper, the Terminal window (focused), and the
/// Files and System windows (closed until opened from the launcher).
pub fn init(screen: Screen, info: &BootInfo, free_mib: u64, cpus: usize) {
    let (w, h) = (screen.width(), screen.height());
    let mut desk = Desktop::new(w, h, SUBTITLE);
    let u = desk.m.u;
    let work = desk.work_area();
    let scale = 2 * u;

    // Files and System exist from the start but stay closed until launched.
    let (fw, fh) = ((w as i32 * 53 / 100).max(420) as usize, (work.h * 55 / 100).max(300) as usize);
    desk.add_window(FILES, "Files", (fw, fh), (w as i32 - fw as i32 - 40 * u, 70 * u), PANEL);
    let (sw, sh) = ((w as i32 * 42 / 100).max(380) as usize, (work.h * 62 / 100).max(300) as usize);
    desk.add_window(SYSTEM, "System", (sw, sh), (w as i32 / 2 - sw as i32 / 2 + 60 * u, 60 * u), PANEL);
    desk.close(FILES);
    desk.close(SYSTEM);

    // The Terminal: about two thirds of the screen.
    let term_scale = (2 * u).min(4);
    let (cw, ch) = cell_size(term_scale);
    let pad = 6 * u;
    let cols = ((w as i32 * 64 / 100) / cw).max(40) as usize;
    let rows = ((work.h * 68 / 100) / ch).max(12) as usize;
    let (tx, ty) = (32 * u, 28 * u);
    desk.add_window(TERMINAL, "Terminal", (cols * cw as usize + 2 * pad as usize, rows * ch as usize + 2 * pad as usize), (tx, ty), PANEL);
    desk.set_menu(alloc::vec![(String::from("Terminal"), TERMINAL), (String::from("Files"), FILES), (String::from("System"), SYSTEM)]);
    desk.set_note(&alloc::format!("{cpus} CPU(s)"));

    let boot_lines = alloc::vec![
        (String::from("Screen"), alloc::format!("{}x{}", info.framebuffer.width, info.framebuffer.height)),
        (String::from("Kernel at"), alloc::format!("{:#x}", info.kernel_virt)),
        (String::from("Free at boot"), alloc::format!("{free_mib} MiB")),
    ];
    let term = desk.content_rect(TERMINAL).unwrap_or_default();
    crate::kprintln!("desktop: {w}x{h}, terminal content at ({}, {}) {}x{} ({cols}x{rows} cells)", term.x, term.y, term.w, term.h);
    let mut gui = Gui {
        back: Canvas::new(w, h, 0),
        pointer: Pointer::centered(w, h),
        grid: Grid::new(cols, rows),
        term_scale,
        term_pad: pad,
        blink_on: true,
        files: FilesView::new(scale, u),
        scale,
        boot_lines,
        desk,
        screen,
    };
    let (px, py) = (gui.pointer.x, gui.pointer.y);
    gui.desk.pointer_moved(px, py);
    // The boot log so far, then every new line, in the Terminal until the shell starts.
    crate::log::recent(|l| {
        gui.grid.set_color(COLOR_DIM);
        gui.grid.write_str(l);
        gui.grid.write_str("\n");
    });
    gui.grid.set_color(COLOR_TEXT);
    gui.render_console();
    gui.flush();
    GUI.with(|g| *g = Some(gui));
    crate::log::set_mirror(Some(boot_line));
}

/// A log line during boot, shown in the Terminal. Runs inside the log's lock:
/// gives up rather than wait if the GUI is busy.
fn boot_line(text: &str) {
    GUI.try_with(|g| {
        let Some(g) = g.as_mut() else { return };
        g.grid.set_color(COLOR_DIM);
        g.grid.write_str(text);
        g.grid.write_str("\n");
        g.grid.set_color(COLOR_TEXT);
        g.render_console();
        g.flush();
    });
}

/// The shell is starting: stop showing log lines in the Terminal.
pub fn end_boot_log() {
    crate::log::set_mirror(None);
}

/// Runs `f` on the GUI, then composes and presents what it changed.
pub fn with<R>(f: impl FnOnce(&mut Gui) -> R) -> Option<R> {
    GUI.with(|g| {
        g.as_mut().map(|g| {
            let r = f(g);
            g.flush();
            r
        })
    })
}

/// Columns and rows of the console, if the GUI is up.
pub fn console_size() -> Option<(usize, usize)> {
    with(|g| (g.grid.cols(), g.grid.rows()))
}

/// Runs `f` on the console grid, then repaints whatever it changed.
pub fn with_console<R>(f: impl FnOnce(&mut Grid) -> R) -> Option<R> {
    with(|g| {
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

/// Applies a mouse event: moves the pointer, and presses/releases the left
/// button on the desktop. Returns what changed, where the pointer is, and
/// what the desktop did.
pub fn mouse(ev: MouseEvent) -> Option<(Change, i32, i32, Vec<Event>)> {
    with(|g| {
        let change = g.pointer.apply(ev);
        let (x, y) = (g.pointer.x, g.pointer.y);
        let mut events = Vec::new();
        if change.moved {
            g.desk.pointer_moved(x, y);
        }
        if change.pressed.left() {
            events.extend(g.desk.press());
        }
        if change.released.left() {
            events.extend(g.desk.release());
        }
        if events.iter().any(|e| matches!(e, Event::Focused(_) | Event::Closed(_) | Event::Minimized(_) | Event::Restored(_))) {
            g.refresh_cursor();
        }
        (change, x, y, events)
    })
}

/// The clock at the right of the taskbar.
pub fn set_clock(text: &str) {
    with(|g| g.desk.set_clock(text));
}

/// Updates the CPU count shown in the taskbar.
pub fn set_cpus(n: usize) {
    with(|g| g.desk.set_note(&alloc::format!("{n} CPU(s)")));
}

/// Toggles the text cursor's blink phase and repaints its row.
pub fn blink() {
    with(|g| {
        g.blink_on = !g.blink_on;
        g.refresh_cursor();
    });
}
