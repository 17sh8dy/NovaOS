//! The Nova desktop as pure logic: windows with title bars and close /
//! minimise buttons, stacking order and focus, dragging, a taskbar with a
//! launcher menu, window buttons and a clock, a mouse pointer, and *damage
//! tracking*: every change records the screen rectangles it affects, and
//! [`Desktop::compose`] redraws exactly such a rectangle into an off-screen
//! canvas (wallpaper, then windows bottom to top, then taskbar, menu and
//! pointer). The kernel copies the finished rectangles to the framebuffer, so
//! nothing flickers and nothing is drawn twice.
//!
//! Applications draw into their window's `content` canvas and report what they
//! changed with [`Desktop::damage_content`]. Mouse presses come back as
//! [`Event`]s (focus changes, menu choices, clicks inside a window with
//! window-local coordinates) for the kernel to act on.
//!
//! No hardware access: everything is unit-tested on the host, down to the
//! pixels of a composed screen.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

use alloc::string::String;
use alloc::vec::Vec;
pub mod apps;

use nova_gfx::{Canvas, Color, Rect, mix, rgb, text_height, text_width};

// ---- Look ---------------------------------------------------------------------

pub const INK: Color = rgb(0xE8, 0xEC, 0xFF);
pub const DIM: Color = rgb(0x8C, 0x94, 0xC0);
pub const ACCENT: Color = rgb(0x6E, 0xE7, 0xD8);
pub const DANGER: Color = rgb(0xFF, 0x6B, 0x7A);
pub const PANEL: Color = rgb(0x0A, 0x0C, 0x1E);
const SKY_TOP: Color = rgb(0x07, 0x09, 0x1C);
const SKY_BOTTOM: Color = rgb(0x1B, 0x10, 0x3A);
const BORDER: Color = rgb(0x2A, 0x31, 0x68);
const BORDER_FOCUS: Color = rgb(0x4A, 0x6A, 0xB0);
const TITLE: Color = rgb(0x12, 0x15, 0x2E);
const TITLE_FOCUS: Color = rgb(0x1C, 0x23, 0x4E);
const BAR: Color = rgb(0x0C, 0x0F, 0x26);
const BAR_LINE: Color = rgb(0x23, 0x29, 0x58);
const BUTTON: Color = rgb(0x16, 0x1B, 0x3C);
const BUTTON_ACTIVE: Color = rgb(0x24, 0x2C, 0x5E);
const MENU: Color = rgb(0x11, 0x14, 0x30);

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

/// Sizes, all derived from one UI unit `u` (1 on a 800-pixel-tall screen).
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub u: i32,
    /// Font scale for window titles, the taskbar and menus.
    pub text: i32,
    pub border: i32,
    pub title_h: i32,
    pub taskbar_h: i32,
    pub pointer_scale: i32,
}

impl Metrics {
    pub fn for_screen(h: usize) -> Self {
        let u = (h as i32 / 800).max(1);
        let text = 2 * u;
        Self { u, text, border: u, title_h: text_height(text) + 10 * u, taskbar_h: text_height(text) + 18 * u, pointer_scale: (h as i32 / 700).clamp(1, 3) }
    }
}

// ---- Windows --------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Open,
    Minimized,
    Closed,
}

pub struct Window {
    pub id: u32,
    pub title: String,
    /// Outer rectangle (frame included), screen coordinates.
    pub rect: Rect,
    pub state: State,
    /// What the application drew.
    pub content: Canvas,
}

/// What is under a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Desktop,
    Launcher,
    TaskButton(u32),
    Taskbar,
    MenuItem(usize),
    Title(u32),
    Close(u32),
    Minimize(u32),
    Frame(u32),
    /// Inside a window's content, in content coordinates.
    Content(u32, i32, i32),
}

/// What a press or release did, for the kernel to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Focused(u32),
    MenuOpened,
    MenuClosed,
    /// A launcher entry was chosen: its id.
    MenuChose(u32),
    Closed(u32),
    Minimized(u32),
    Restored(u32),
    DragStarted(u32),
    /// The window finished moving; its new top-left.
    Moved(u32, i32, i32),
    /// A click inside a window's content (content coordinates).
    Click(u32, i32, i32),
}

pub struct Desktop {
    w: i32,
    h: i32,
    pub m: Metrics,
    wallpaper: Canvas,
    /// Bottom to top.
    windows: Vec<Window>,
    focus: Option<u32>,
    drag: Option<(u32, i32, i32)>,
    /// Where the dragged window started (a click that doesn't move it is not a move).
    drag_from: (i32, i32),
    menu: Vec<(String, u32)>,
    menu_open: bool,
    pointer: (i32, i32),
    clock: String,
    note: String,
    damage: Vec<Rect>,
}

impl Desktop {
    /// A desktop for a `w` x `h` screen; `subtitle` goes under the name on the wallpaper.
    pub fn new(w: usize, h: usize, subtitle: &str) -> Self {
        let m = Metrics::for_screen(h);
        let mut d = Self {
            w: w as i32,
            h: h as i32,
            m,
            wallpaper: Canvas::new(w, h, 0),
            windows: Vec::new(),
            focus: None,
            drag: None,
            drag_from: (0, 0),
            menu: Vec::new(),
            menu_open: false,
            pointer: (w as i32 / 2, h as i32 / 2),
            clock: String::new(),
            note: String::new(),
            damage: Vec::new(),
        };
        d.paint_wallpaper(subtitle);
        d.damage_all();
        d
    }

    fn paint_wallpaper(&mut self, subtitle: &str) {
        let (w, h, u) = (self.w, self.h, self.m.u);
        let wp = &mut self.wallpaper;
        wp.gradient(SKY_TOP, SKY_BOTTOM);
        // A sprinkle of faint stars (a fixed pseudo-random pattern).
        let mut seed: u32 = 0x9E37_79B9;
        for _ in 0..(w * h / 9000).max(8) {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let x = (seed % w as u32) as i32;
            let y = ((seed >> 12) % h as u32) as i32;
            let base = wp.get(x, y).unwrap_or(0);
            wp.put(x, y, mix(base, INK, 60 + (seed >> 24) % 120));
        }
        // The emblem sits right of centre, where a freshly opened Terminal
        // (left two thirds) leaves the wallpaper visible.
        let cx = w * 82 / 100;
        let cy = (h - self.m.taskbar_h) * 52 / 100;
        let r = 40 * u;
        let glow = mix(SKY_BOTTOM, ACCENT, 70);
        wp.star(cx, cy - 30 * u, r, glow);
        let name = "NOVA OS";
        let s = 4 * u;
        let bounds = wp.bounds();
        wp.text(cx - text_width(name, s) / 2, cy + r - 10 * u, s, mix(SKY_BOTTOM, INK, 90), name, bounds);
        let s2 = 2 * u;
        wp.text(cx - text_width(subtitle, s2) / 2, cy + r + text_height(s) + 2 * u, s2, mix(SKY_BOTTOM, DIM, 120), subtitle, bounds);
    }

    pub fn width(&self) -> i32 {
        self.w
    }

    pub fn height(&self) -> i32 {
        self.h
    }

    pub fn screen(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    // ---- Layout ----

    pub fn taskbar(&self) -> Rect {
        Rect::new(0, self.h - self.m.taskbar_h, self.w, self.m.taskbar_h)
    }

    /// The space windows live in (above the taskbar).
    pub fn work_area(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h - self.m.taskbar_h)
    }

    pub fn launcher(&self) -> Rect {
        let u = self.m.u;
        let bar = self.taskbar();
        Rect::new(4 * u, bar.y + 4 * u, text_width("Nova", self.m.text) + 40 * u, bar.h - 8 * u)
    }

    /// Taskbar buttons of the windows that are open or minimised (in the order they were created).
    pub fn task_buttons(&self) -> Vec<(u32, Rect)> {
        let u = self.m.u;
        let l = self.launcher();
        let mut ids: Vec<u32> = self.windows.iter().filter(|w| w.state != State::Closed).map(|w| w.id).collect();
        ids.sort_unstable();
        let bw = 150 * u;
        ids.iter().enumerate().map(|(i, &id)| (id, Rect::new(l.right() + 8 * u + i as i32 * (bw + 4 * u), l.y, bw, l.h))).collect()
    }

    fn clock_rect(&self) -> Rect {
        let u = self.m.u;
        let bar = self.taskbar();
        let text = alloc::format!("{}    {}", self.note, self.clock);
        let tw = text_width(&text, self.m.text) + 16 * u;
        Rect::new(self.w - tw - 4 * u, bar.y, tw + 4 * u, bar.h)
    }

    fn item_h(&self) -> i32 {
        text_height(self.m.text) + 14 * self.m.u
    }

    pub fn menu_rect(&self) -> Rect {
        let u = self.m.u;
        let n = self.menu.len() as i32;
        let h = n * self.item_h() + 8 * u;
        Rect::new(self.launcher().x, self.taskbar().y - h - 4 * u, 200 * u, h)
    }

    /// Screen rectangle of launcher entry `i`.
    pub fn menu_item(&self, i: usize) -> Rect {
        let r = self.menu_rect();
        let u = self.m.u;
        Rect::new(r.x + 4 * u, r.y + 4 * u + i as i32 * self.item_h(), r.w - 8 * u, self.item_h())
    }

    fn title_rect(&self, w: &Window) -> Rect {
        let b = self.m.border;
        Rect::new(w.rect.x + b, w.rect.y + b, w.rect.w - 2 * b, self.m.title_h)
    }

    fn button_size(&self) -> i32 {
        self.m.title_h - 8 * self.m.u
    }

    fn close_rect(&self, w: &Window) -> Rect {
        let t = self.title_rect(w);
        let s = self.button_size();
        Rect::new(t.right() - s - 4 * self.m.u, t.y + 4 * self.m.u, s, s)
    }

    fn minimize_rect(&self, w: &Window) -> Rect {
        let c = self.close_rect(w);
        c.offset(-(c.w + 4 * self.m.u), 0)
    }

    /// A window's title bar, close button and minimise button (screen coordinates).
    pub fn chrome(&self, id: u32) -> Option<(Rect, Rect, Rect)> {
        let w = self.find(id)?;
        Some((self.title_rect(w), self.close_rect(w), self.minimize_rect(w)))
    }

    /// Where a window's content sits on screen.
    pub fn content_rect(&self, id: u32) -> Option<Rect> {
        let w = self.find(id)?;
        let b = self.m.border;
        Some(Rect::new(w.rect.x + b, w.rect.y + b + self.m.title_h, w.content.width() as i32, w.content.height() as i32))
    }

    fn pointer_rect(&self) -> Rect {
        let s = self.m.pointer_scale;
        Rect::new(self.pointer.0, self.pointer.1, 11 * s, 17 * s)
    }

    fn shadow(&self, r: Rect) -> Rect {
        let o = 6 * self.m.u;
        Rect::new(r.x + o, r.y + o, r.w, r.h)
    }

    // ---- Windows ----

    fn find(&self, id: u32) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    fn index(&self, id: u32) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }

    pub fn window(&self, id: u32) -> Option<&Window> {
        self.find(id)
    }

    pub fn state(&self, id: u32) -> Option<State> {
        self.find(id).map(|w| w.state)
    }

    pub fn focused(&self) -> Option<u32> {
        self.focus
    }

    /// Window ids from bottom to top.
    pub fn stacking(&self) -> Vec<u32> {
        self.windows.iter().map(|w| w.id).collect()
    }

    /// Creates a window whose content is `cw` x `ch` pixels, top-left at (`x`, `y`),
    /// opened on top and focused.
    pub fn add_window(&mut self, id: u32, title: &str, (cw, ch): (usize, usize), (x, y): (i32, i32), fill: Color) {
        let b = self.m.border;
        let rect = Rect::new(x, y, cw as i32 + 2 * b, ch as i32 + self.m.title_h + 2 * b);
        self.windows.push(Window { id, title: String::from(title), rect, state: State::Open, content: Canvas::new(cw, ch, fill) });
        self.set_focus(Some(id));
        self.damage_window(id);
        self.damage_taskbar();
    }

    /// Mutable access to what a window shows (report changes with `damage_content`).
    pub fn content_mut(&mut self, id: u32) -> Option<&mut Canvas> {
        self.windows.iter_mut().find(|w| w.id == id).map(|w| &mut w.content)
    }

    /// Records that `local` (content coordinates) of window `id` changed.
    pub fn damage_content(&mut self, id: u32, local: Rect) {
        let Some(w) = self.find(id) else { return };
        if w.state != State::Open {
            return;
        }
        let Some(c) = self.content_rect(id) else { return };
        let r = local.offset(c.x, c.y).intersect(&c);
        self.add_damage(r);
    }

    /// Opens (or restores) a window, raises and focuses it.
    pub fn show(&mut self, id: u32) {
        let Some(i) = self.index(id) else { return };
        self.windows[i].state = State::Open;
        self.raise(id);
        self.set_focus(Some(id));
        self.damage_window(id);
        self.damage_taskbar();
    }

    pub fn close(&mut self, id: u32) {
        self.hide(id, State::Closed);
    }

    pub fn minimize(&mut self, id: u32) {
        self.hide(id, State::Minimized);
    }

    fn hide(&mut self, id: u32, state: State) {
        let Some(i) = self.index(id) else { return };
        self.damage_window(id);
        self.windows[i].state = state;
        if self.drag.is_some_and(|d| d.0 == id) {
            self.drag = None;
        }
        if self.focus == Some(id) {
            let next = self.windows.iter().rev().find(|w| w.state == State::Open).map(|w| w.id);
            self.set_focus(next);
        }
        self.damage_taskbar();
    }

    fn raise(&mut self, id: u32) {
        if let Some(i) = self.index(id)
            && i + 1 != self.windows.len()
        {
            let w = self.windows.remove(i);
            self.windows.push(w);
            self.damage_window(id);
        }
    }

    fn set_focus(&mut self, id: Option<u32>) {
        if self.focus == id {
            return;
        }
        if let Some(old) = self.focus {
            self.damage_window(old);
        }
        self.focus = id;
        if let Some(new) = id {
            self.damage_window(new);
        }
        self.damage_taskbar();
    }

    /// Moves a window so its top-left is at (x, y), kept where its title bar can still be grabbed.
    pub fn move_window(&mut self, id: u32, x: i32, y: i32) {
        let Some(i) = self.index(id) else { return };
        let keep = 60 * self.m.u;
        let r = self.windows[i].rect;
        let work = self.work_area();
        let x = x.clamp(keep - r.w, work.right() - keep);
        let y = y.clamp(0, work.bottom() - self.m.title_h);
        if (x, y) == (r.x, r.y) {
            return;
        }
        self.damage_window(id);
        self.windows[i].rect.x = x;
        self.windows[i].rect.y = y;
        self.damage_window(id);
    }

    // ---- Bits of chrome ----

    pub fn set_menu(&mut self, items: Vec<(String, u32)>) {
        self.menu = items;
    }

    pub fn menu_is_open(&self) -> bool {
        self.menu_open
    }

    fn set_menu_open(&mut self, open: bool) {
        if self.menu_open != open {
            self.add_damage(self.menu_rect().union(&self.shadow(self.menu_rect())));
            self.menu_open = open;
            self.damage_taskbar();
        }
    }

    /// The clock text on the right of the taskbar.
    pub fn set_clock(&mut self, clock: &str) {
        if self.clock != clock {
            self.damage_taskbar();
            self.clock = String::from(clock);
            self.damage_taskbar();
        }
    }

    /// A short status text left of the clock.
    pub fn set_note(&mut self, note: &str) {
        if self.note != note {
            self.damage_taskbar();
            self.note = String::from(note);
            self.damage_taskbar();
        }
    }

    // ---- Input ----

    pub fn pointer(&self) -> (i32, i32) {
        self.pointer
    }

    /// What is at (x, y).
    pub fn hit(&self, x: i32, y: i32) -> Hit {
        if self.menu_open && self.menu_rect().contains(x, y) {
            return (0..self.menu.len()).find(|&i| self.menu_item(i).contains(x, y)).map_or(Hit::Desktop, Hit::MenuItem);
        }
        if self.taskbar().contains(x, y) {
            if self.launcher().contains(x, y) {
                return Hit::Launcher;
            }
            return self.task_buttons().iter().find(|(_, r)| r.contains(x, y)).map_or(Hit::Taskbar, |(id, _)| Hit::TaskButton(*id));
        }
        for w in self.windows.iter().rev().filter(|w| w.state == State::Open) {
            if !w.rect.contains(x, y) {
                continue;
            }
            if self.close_rect(w).contains(x, y) {
                return Hit::Close(w.id);
            }
            if self.minimize_rect(w).contains(x, y) {
                return Hit::Minimize(w.id);
            }
            if self.title_rect(w).contains(x, y) {
                return Hit::Title(w.id);
            }
            if let Some(c) = self.content_rect(w.id)
                && c.contains(x, y)
            {
                return Hit::Content(w.id, x - c.x, y - c.y);
            }
            return Hit::Frame(w.id);
        }
        Hit::Desktop
    }

    /// The pointer moved to (x, y). Drags the window being dragged.
    pub fn pointer_moved(&mut self, x: i32, y: i32) {
        if (x, y) == self.pointer {
            return;
        }
        self.add_damage(self.pointer_rect());
        self.pointer = (x, y);
        self.add_damage(self.pointer_rect());
        if let Some((id, gx, gy)) = self.drag {
            self.move_window(id, x - gx, y - gy);
        }
    }

    /// The left button went down at the pointer.
    pub fn press(&mut self) -> Vec<Event> {
        let (x, y) = self.pointer;
        let mut ev = Vec::new();
        let hit = self.hit(x, y);
        if self.menu_open && !matches!(hit, Hit::MenuItem(_) | Hit::Launcher) {
            self.set_menu_open(false);
            ev.push(Event::MenuClosed);
        }
        let focus = |d: &mut Self, id: u32, ev: &mut Vec<Event>| {
            d.raise(id);
            if d.focus != Some(id) {
                d.set_focus(Some(id));
                ev.push(Event::Focused(id));
            }
        };
        match hit {
            Hit::Launcher => {
                let open = !self.menu_open;
                self.set_menu_open(open);
                ev.push(if open { Event::MenuOpened } else { Event::MenuClosed });
            }
            Hit::MenuItem(i) => {
                self.set_menu_open(false);
                if let Some(&(_, id)) = self.menu.get(i) {
                    ev.push(Event::MenuChose(id));
                }
            }
            Hit::TaskButton(id) => match self.state(id) {
                Some(State::Minimized) => {
                    self.show(id);
                    ev.push(Event::Restored(id));
                }
                Some(State::Open) if self.focus == Some(id) && self.windows.last().map(|w| w.id) == Some(id) => {
                    self.minimize(id);
                    ev.push(Event::Minimized(id));
                }
                Some(State::Open) => focus(self, id, &mut ev),
                _ => {}
            },
            Hit::Close(id) => {
                self.close(id);
                ev.push(Event::Closed(id));
            }
            Hit::Minimize(id) => {
                self.minimize(id);
                ev.push(Event::Minimized(id));
            }
            Hit::Title(id) => {
                focus(self, id, &mut ev);
                let r = self.find(id).map(|w| w.rect).unwrap_or_default();
                self.drag = Some((id, x - r.x, y - r.y));
                self.drag_from = (r.x, r.y);
                ev.push(Event::DragStarted(id));
            }
            Hit::Frame(id) => focus(self, id, &mut ev),
            Hit::Content(id, cx, cy) => {
                focus(self, id, &mut ev);
                ev.push(Event::Click(id, cx, cy));
            }
            Hit::Desktop | Hit::Taskbar => {}
        }
        ev
    }

    /// The left button came up.
    pub fn release(&mut self) -> Vec<Event> {
        match self.drag.take() {
            Some((id, ..)) => {
                let r = self.find(id).map(|w| w.rect).unwrap_or_default();
                if (r.x, r.y) == self.drag_from { Vec::new() } else { alloc::vec![Event::Moved(id, r.x, r.y)] }
            }
            None => Vec::new(),
        }
    }

    pub fn dragging(&self) -> Option<u32> {
        self.drag.map(|d| d.0)
    }

    // ---- Damage ----

    pub fn add_damage(&mut self, r: Rect) {
        let r = r.intersect(&self.screen());
        if !r.is_empty() {
            self.damage.push(r);
        }
    }

    pub fn damage_all(&mut self) {
        self.add_damage(self.screen());
    }

    fn damage_window(&mut self, id: u32) {
        if let Some(w) = self.find(id) {
            let r = w.rect.union(&self.shadow(w.rect));
            self.add_damage(r);
        }
    }

    fn damage_taskbar(&mut self) {
        self.add_damage(self.taskbar());
    }

    /// The rectangles to redraw since last time, merged so that overlapping
    /// ones (and floods of small ones) don't get drawn twice.
    pub fn take_damage(&mut self) -> Vec<Rect> {
        let mut rs = core::mem::take(&mut self.damage);
        let mut merged = true;
        while merged {
            merged = false;
            'outer: for i in 0..rs.len() {
                for j in i + 1..rs.len() {
                    let u = rs[i].union(&rs[j]);
                    // Merge when they overlap, or when one box costs little more than both.
                    if rs[i].intersects(&rs[j]) || u.area() <= (rs[i].area() + rs[j].area()) * 5 / 4 {
                        rs[i] = u;
                        rs.swap_remove(j);
                        merged = true;
                        break 'outer;
                    }
                }
            }
        }
        if rs.len() > 24 {
            let all = rs.iter().fold(Rect::default(), |a, r| a.union(r));
            return alloc::vec![all];
        }
        rs
    }

    // ---- Drawing ----

    /// Redraws `area` of the screen into `back` (a canvas the size of the screen).
    pub fn compose(&self, back: &mut Canvas, area: Rect) {
        let area = area.intersect(&self.screen());
        if area.is_empty() {
            return;
        }
        back.blit(&self.wallpaper, area, area.x, area.y, area);
        for w in self.windows.iter().filter(|w| w.state == State::Open) {
            if w.rect.union(&self.shadow(w.rect)).intersects(&area) {
                self.draw_window(back, w, area);
            }
        }
        if self.taskbar().intersects(&area) {
            self.draw_taskbar(back, area);
        }
        if self.menu_open && self.menu_rect().union(&self.shadow(self.menu_rect())).intersects(&area) {
            self.draw_menu(back, area);
        }
        if self.pointer_rect().intersects(&area) {
            self.draw_pointer(back, area);
        }
    }

    fn darken(back: &mut Canvas, r: Rect, clip: Rect, amount: u32) {
        let r = r.intersect(&clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                if let Some(p) = back.get(x, y) {
                    back.put(x, y, mix(p, 0, amount));
                }
            }
        }
    }

    fn draw_window(&self, back: &mut Canvas, w: &Window, clip: Rect) {
        let u = self.m.u;
        let focused = self.focus == Some(w.id);
        // Soft shadow: the parts of the offset rectangle not under the window.
        let s = self.shadow(w.rect);
        Self::darken(back, Rect::new(w.rect.right(), s.y, s.right() - w.rect.right(), s.h), clip, 110);
        Self::darken(back, Rect::new(s.x, w.rect.bottom(), w.rect.right() - s.x, s.bottom() - w.rect.bottom()), clip, 110);

        back.fill_clipped(w.rect, if focused { BORDER_FOCUS } else { BORDER }, clip);
        let t = self.title_rect(w);
        back.fill_clipped(t, if focused { TITLE_FOCUS } else { TITLE }, clip);
        if focused {
            back.fill_clipped(Rect::new(t.x, t.bottom() - u, t.w, u), mix(TITLE_FOCUS, ACCENT, 90), clip);
        }
        let ts = self.m.text;
        let tx = t.x + 10 * u;
        let ty = t.y + (t.h - text_height(ts)) / 2 + u;
        let room = self.minimize_rect(w).x - tx - 6 * u;
        let title = nova_gfx::fit(&w.title, ts, room);
        back.text(tx, ty, ts, if focused { INK } else { DIM }, &title, clip.intersect(&t));

        // Minimise and close buttons.
        for (r, glyph, color) in [(self.minimize_rect(w), "_", DIM), (self.close_rect(w), "x", if focused { DANGER } else { DIM })] {
            back.fill_clipped(r, mix(if focused { TITLE_FOCUS } else { TITLE }, INK, 18), clip);
            let gx = r.x + (r.w - text_width(glyph, ts)) / 2;
            let gy = r.y + (r.h - nova_gfx::font::HEIGHT as i32 * ts) / 2 - if glyph == "_" { 2 * u } else { 0 };
            back.text(gx, gy, ts, color, glyph, clip.intersect(&r));
        }

        if let Some(c) = self.content_rect(w.id) {
            back.blit(&w.content, w.content.bounds(), c.x, c.y, clip.intersect(&c));
        }
    }

    fn draw_taskbar(&self, back: &mut Canvas, clip: Rect) {
        let u = self.m.u;
        let ts = self.m.text;
        let bar = self.taskbar();
        back.fill_clipped(bar, BAR, clip);
        back.fill_clipped(Rect::new(bar.x, bar.y, bar.w, u), BAR_LINE, clip);

        let l = self.launcher();
        back.fill_clipped(l, if self.menu_open { BUTTON_ACTIVE } else { BUTTON }, clip);
        let star_r = l.h / 2 - 4 * u;
        let sc = (l.x + 8 * u + star_r, l.y + l.h / 2);
        let mut star = Canvas::new((2 * star_r + 1) as usize, (2 * star_r + 1) as usize, if self.menu_open { BUTTON_ACTIVE } else { BUTTON });
        star.star(star_r, star_r, star_r, ACCENT);
        back.blit(&star, star.bounds(), sc.0 - star_r, sc.1 - star_r, clip.intersect(&l));
        let ty = l.y + (l.h - text_height(ts)) / 2 + u;
        back.text(sc.0 + star_r + 8 * u, ty, ts, INK, "Nova", clip.intersect(&l));

        for (id, r) in self.task_buttons() {
            let Some(w) = self.find(id) else { continue };
            let focused = self.focus == Some(id) && w.state == State::Open;
            back.fill_clipped(r, if focused { BUTTON_ACTIVE } else { BUTTON }, clip);
            if focused {
                back.fill_clipped(Rect::new(r.x, r.bottom() - 2 * u, r.w, 2 * u), ACCENT, clip);
            }
            let color = match w.state {
                State::Minimized => DIM,
                _ => INK,
            };
            let label = nova_gfx::fit(&w.title, ts, r.w - 16 * u);
            back.text(r.x + 8 * u, ty, ts, color, &label, clip.intersect(&r));
        }

        let c = self.clock_rect();
        let note_w = text_width(&self.note, ts);
        let right = c.right() - 10 * u;
        let clock_x = right - text_width(&self.clock, ts);
        back.text(clock_x, ty, ts, INK, &self.clock, clip);
        back.text(clock_x - 24 * u - note_w, ty, ts, DIM, &self.note, clip);
    }

    fn draw_menu(&self, back: &mut Canvas, clip: Rect) {
        let u = self.m.u;
        let r = self.menu_rect();
        let s = self.shadow(r);
        Self::darken(back, Rect::new(r.right(), s.y, s.right() - r.right(), s.h), clip, 110);
        Self::darken(back, Rect::new(s.x, r.bottom(), r.right() - s.x, s.bottom() - r.bottom()), clip, 110);
        back.fill_clipped(r, BORDER_FOCUS, clip);
        back.fill_clipped(r.inset(u), MENU, clip);
        let ts = self.m.text;
        for (i, (label, _)) in self.menu.iter().enumerate() {
            let it = self.menu_item(i);
            if i > 0 {
                back.fill_clipped(Rect::new(it.x + 6 * u, it.y, it.w - 12 * u, u), BAR_LINE, clip);
            }
            back.fill_clipped(Rect::new(it.x + 8 * u, it.y + it.h / 2 - 3 * u, 6 * u, 6 * u), ACCENT, clip);
            back.text(it.x + 22 * u, it.y + (it.h - text_height(ts)) / 2 + u, ts, INK, label, clip.intersect(&it));
        }
    }

    fn draw_pointer(&self, back: &mut Canvas, clip: Rect) {
        let s = self.m.pointer_scale;
        let (px, py) = self.pointer;
        for (row, line) in ARROW.iter().enumerate() {
            for (col, b) in line.bytes().enumerate() {
                let c = match b {
                    b'X' => rgb(0, 0, 0),
                    b'.' => rgb(0xFF, 0xFF, 0xFF),
                    _ => continue,
                };
                back.fill_clipped(Rect::new(px + col as i32 * s, py + row as i32 * s, s, s), c, clip);
            }
        }
    }
}

#[cfg(test)]
mod tests;
