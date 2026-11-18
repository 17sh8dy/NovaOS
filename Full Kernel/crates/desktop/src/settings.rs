//! Settings: what Nova remembers in `/system/settings` and the window that changes it.
//!
//! * Lock after idle: off, 1, 5, 10, 30 or 60 minutes without mouse or keyboard
//!   (only matters when a password is set).
//! * Time zone: the CMOS clock is usually UTC; this offset (in half hours) is
//!   added for the taskbar clock and the lock screen.
//! * Wallpaper: one of four colour themes.
//! * Password: opens the create/change card.
//!
//! The file is plain `key = value` lines, so it can be read with `cat` and fixed by hand;
//! unknown keys and bad values are ignored (the default is kept).

use crate::ui::{self, KeyIn};
use crate::{ACCENT, DIM, INK, PANEL};
use alloc::string::String;
use alloc::vec::Vec;
use nova_gfx::{Canvas, Color, Rect, fit, mix, rgb, text_height, text_width};

pub const LOCK_CHOICES: [u32; 6] = [0, 1, 5, 10, 30, 60];
/// Time zone limits, minutes.
pub const MIN_OFFSET: i32 = -12 * 60;
pub const MAX_OFFSET: i32 = 14 * 60;

pub struct Theme {
    pub name: &'static str,
    pub top: Color,
    pub bottom: Color,
}

pub const THEMES: [Theme; 4] = [
    Theme { name: "Night", top: rgb(0x07, 0x09, 0x1C), bottom: rgb(0x1B, 0x10, 0x3A) },
    Theme { name: "Ocean", top: rgb(0x03, 0x12, 0x22), bottom: rgb(0x06, 0x3A, 0x4E) },
    Theme { name: "Forest", top: rgb(0x05, 0x14, 0x10), bottom: rgb(0x14, 0x36, 0x24) },
    Theme { name: "Ember", top: rgb(0x14, 0x07, 0x0C), bottom: rgb(0x46, 0x16, 0x1A) },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// 0 = never.
    pub lock_minutes: u32,
    pub utc_offset_min: i32,
    pub theme: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { lock_minutes: 10, utc_offset_min: 0, theme: 0 }
    }
}

/// "UTC+05:30".
pub fn offset_text(min: i32) -> String {
    let sign = if min < 0 { '-' } else { '+' };
    let a = min.unsigned_abs();
    alloc::format!("UTC{sign}{:02}:{:02}", a / 60, a % 60)
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "lock_minutes" => {
                    if let Some(n) = v.parse::<u32>().ok().filter(|n| LOCK_CHOICES.contains(n)) {
                        s.lock_minutes = n;
                    }
                }
                "utc_offset_minutes" => {
                    if let Some(n) = v.parse::<i32>().ok().filter(|n| (MIN_OFFSET..=MAX_OFFSET).contains(n) && n % 30 == 0) {
                        s.utc_offset_min = n;
                    }
                }
                "wallpaper" => {
                    if let Some(i) = THEMES.iter().position(|t| t.name.eq_ignore_ascii_case(v)) {
                        s.theme = i;
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        alloc::format!(
            "# Nova settings (written by the Settings app; plain text, safe to edit)\nlock_minutes = {}\nutc_offset_minutes = {}\nwallpaper = {}\n",
            self.lock_minutes,
            self.utc_offset_min,
            THEMES[self.theme.min(THEMES.len() - 1)].name
        )
    }

    pub fn lock_text(&self) -> String {
        match self.lock_minutes {
            0 => String::from("Never"),
            1 => String::from("1 minute"),
            n => alloc::format!("{n} minutes"),
        }
    }

    /// Moves setting `row` one step (`dir` = -1 or +1). Returns true if it changed.
    fn step(&mut self, row: usize, dir: i32) -> bool {
        let before = *self;
        match row {
            0 => {
                let i = LOCK_CHOICES.iter().position(|&n| n == self.lock_minutes).unwrap_or(3) as i32;
                self.lock_minutes = LOCK_CHOICES[(i + dir).clamp(0, LOCK_CHOICES.len() as i32 - 1) as usize];
            }
            1 => self.utc_offset_min = (self.utc_offset_min + 30 * dir).clamp(MIN_OFFSET, MAX_OFFSET),
            2 => self.theme = (self.theme as i32 + dir).rem_euclid(THEMES.len() as i32) as usize,
            _ => {}
        }
        *self != before
    }
}

/// What a key or click in Settings did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsOut {
    None,
    /// A setting changed: save it and apply it.
    Changed,
    /// Open the password card.
    Password,
}

const ROWS: usize = 4;
const LABELS: [&str; ROWS] = ["Lock after idle", "Time zone", "Wallpaper", "Password"];

pub struct SettingsView {
    pub s: Settings,
    pub password_set: bool,
    /// Where the settings are kept, and whether they survive a restart.
    pub stored: String,
    pub about: Vec<(String, String)>,
    sel: usize,
    scale: i32,
    u: i32,
}

impl SettingsView {
    pub fn new(s: Settings, scale: i32, u: i32) -> Self {
        Self { s, password_set: false, stored: String::new(), about: Vec::new(), sel: 0, scale, u }
    }

    pub fn selected(&self) -> usize {
        self.sel
    }

    fn row_h(&self) -> i32 {
        text_height(self.scale) + 22 * self.u
    }

    fn row_rect(&self, i: usize, w: i32) -> Rect {
        let u = self.u;
        Rect::new(10 * u, 16 * u + text_height(self.scale) + 12 * u + i as i32 * (self.row_h() + 6 * u), w - 20 * u, self.row_h())
    }

    /// The `<` and `>` buttons of a row (or the single button of the Password row).
    fn controls(&self, i: usize, w: i32) -> (Rect, Rect) {
        let u = self.u;
        let r = self.row_rect(i, w);
        let bw = text_width(">", self.scale) + 20 * u;
        let right = Rect::new(r.right() - bw - 6 * u, r.y + 5 * u, bw, r.h - 10 * u);
        let value_w = 150 * u;
        let left = Rect::new(right.x - value_w - bw, right.y, bw, right.h);
        if i == 3 {
            let label = self.password_button();
            let pw = text_width(label, self.scale) + 24 * u;
            let b = Rect::new(r.right() - pw - 6 * u, r.y + 5 * u, pw, r.h - 10 * u);
            return (b, b);
        }
        (left, right)
    }

    fn password_button(&self) -> &'static str {
        if self.password_set { "Change..." } else { "Set a password..." }
    }

    /// Centre of a row's `<` / `>` (or Password) button, for tests and logs.
    pub fn control_center(&self, row: usize, right: bool, w: i32) -> (i32, i32) {
        let (l, r) = self.controls(row, w);
        let b = if right { r } else { l };
        (b.x + b.w / 2, b.y + b.h / 2)
    }

    pub fn click(&mut self, x: i32, y: i32, w: i32) -> SettingsOut {
        for i in 0..ROWS {
            if !self.row_rect(i, w).contains(x, y) {
                continue;
            }
            self.sel = i;
            let (l, r) = self.controls(i, w);
            if i == 3 {
                return if l.contains(x, y) { SettingsOut::Password } else { SettingsOut::None };
            }
            let dir = if l.contains(x, y) {
                -1
            } else if r.contains(x, y) {
                1
            } else {
                return SettingsOut::None;
            };
            return if self.s.step(i, dir) { SettingsOut::Changed } else { SettingsOut::None };
        }
        SettingsOut::None
    }

    pub fn key(&mut self, k: KeyIn) -> SettingsOut {
        match k {
            KeyIn::Up => self.sel = self.sel.saturating_sub(1),
            KeyIn::Down | KeyIn::Tab => self.sel = (self.sel + 1).min(ROWS - 1),
            KeyIn::Left | KeyIn::Right => {
                let dir = if k == KeyIn::Left { -1 } else { 1 };
                if self.sel < 3 && self.s.step(self.sel, dir) {
                    return SettingsOut::Changed;
                }
            }
            KeyIn::Enter | KeyIn::Char(' ') => {
                if self.sel == 3 {
                    return SettingsOut::Password;
                }
                if self.s.step(self.sel, 1) {
                    return SettingsOut::Changed;
                }
            }
            _ => {}
        }
        SettingsOut::None
    }

    fn value(&self, i: usize) -> String {
        match i {
            0 => self.s.lock_text(),
            1 => offset_text(self.s.utc_offset_min),
            2 => String::from(THEMES[self.s.theme.min(THEMES.len() - 1)].name),
            _ => String::from(if self.password_set { "Set" } else { "Not set" }),
        }
    }

    pub fn render(&self, c: &mut Canvas) {
        let (w, h) = (c.width() as i32, c.height() as i32);
        let (u, s) = (self.u, self.scale);
        let all = c.bounds();
        c.fill(all, PANEL);
        c.text(14 * u, 16 * u, s, ACCENT, "Settings", all);
        for (i, label) in LABELS.iter().enumerate() {
            let r = self.row_rect(i, w);
            let on = i == self.sel;
            c.fill(r, if on { mix(PANEL, ACCENT, 26) } else { ui::FACE_SOFT });
            if on {
                c.fill(Rect::new(r.x, r.y, 3 * u, r.h), ACCENT);
            }
            let ty = r.y + (r.h - text_height(s)) / 2 + u;
            c.text(r.x + 14 * u, ty, s, INK, label, r);
            let (l, rt) = self.controls(i, w);
            if i == 3 {
                ui::button_on(c, l, self.password_button(), s, u);
                let v = self.value(i);
                c.text(l.x - text_width(&v, s) - 14 * u, ty, s, DIM, &v, r);
                continue;
            }
            ui::button_on(c, l, "<", s, u);
            ui::button_on(c, rt, ">", s, u);
            let v = self.value(i);
            let mid = l.right() + (rt.x - l.right()) / 2;
            c.text(mid - text_width(&v, s) / 2, ty, s, INK, &v, r);
            if i == 2 {
                // A swatch of the chosen wallpaper.
                let t = &THEMES[self.s.theme.min(THEMES.len() - 1)];
                let sw = Rect::new(l.x - 40 * u, r.y + 6 * u, 28 * u, r.h - 12 * u);
                c.fill_clipped(Rect::new(sw.x, sw.y, sw.w, sw.h / 2), t.top, r);
                c.fill_clipped(Rect::new(sw.x, sw.y + sw.h / 2, sw.w, sw.h - sw.h / 2), t.bottom, r);
            }
        }
        let mut y = self.row_rect(ROWS - 1, w).bottom() + 12 * u;
        let lh = text_height(s) + 6 * u;
        let hint = if self.password_set { "Up/Down choose, Left/Right change" } else { "Locking after idle needs a password" };
        c.text(14 * u, y, s, DIM, &fit(hint, s, w - 28 * u), all);
        y += lh;
        if !self.stored.is_empty() {
            c.text(14 * u, y, s, DIM, &fit(&self.stored, s, w - 28 * u), all);
            y += lh;
        }
        y += 6 * u;
        if !self.about.is_empty() && y + lh < h {
            c.text(14 * u, y, s, ACCENT, "About this PC", all);
            y += lh + 2 * u;
            let label_w = self.about.iter().map(|(l, _)| text_width(l, s)).max().unwrap_or(0) + 20 * u;
            for (l, v) in &self.about {
                if y + lh > h {
                    break;
                }
                c.text(14 * u, y, s, DIM, l, all);
                c.text(14 * u + label_w, y, s, INK, &fit(v, s, w - label_w - 28 * u), all);
                y += lh;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trip_and_bad_values() {
        let s = Settings { lock_minutes: 5, utc_offset_min: 330, theme: 2 };
        let text = s.to_text();
        assert!(text.contains("wallpaper = Forest"));
        assert_eq!(Settings::parse(&text), s);
        assert_eq!(Settings::parse(""), Settings::default());
        let bad = "lock_minutes = 7\nutc_offset_minutes = 9999\nwallpaper = Plaid\nnonsense\n# c = 1\nutc_offset_minutes=-45";
        assert_eq!(Settings::parse(bad), Settings::default(), "invalid values keep the defaults");
        assert_eq!(Settings::parse("utc_offset_minutes = -210").utc_offset_min, -210);
        assert_eq!(offset_text(330), "UTC+05:30");
        assert_eq!(offset_text(-210), "UTC-03:30");
        assert_eq!(offset_text(0), "UTC+00:00");
    }

    #[test]
    fn keys_change_the_selected_row() {
        let mut v = SettingsView::new(Settings::default(), 2, 1);
        assert_eq!(v.key(KeyIn::Right), SettingsOut::Changed);
        assert_eq!(v.s.lock_minutes, 30);
        assert_eq!(v.key(KeyIn::Right), SettingsOut::Changed);
        assert_eq!(v.key(KeyIn::Right), SettingsOut::None, "stops at the last choice");
        v.key(KeyIn::Down);
        v.key(KeyIn::Left);
        assert_eq!(v.s.utc_offset_min, -30);
        v.key(KeyIn::Down);
        v.key(KeyIn::Left);
        assert_eq!(v.s.theme, THEMES.len() - 1, "themes wrap around");
        v.key(KeyIn::Down);
        assert_eq!(v.key(KeyIn::Enter), SettingsOut::Password);
        v.key(KeyIn::Down);
        assert_eq!(v.selected(), 3);
    }

    #[test]
    fn clicks_on_the_buttons() {
        let w = 600;
        let mut v = SettingsView::new(Settings::default(), 2, 1);
        let (x, y) = v.control_center(1, true, w);
        assert_eq!(v.click(x, y, w), SettingsOut::Changed);
        assert_eq!(v.s.utc_offset_min, 30);
        let (x, y) = v.control_center(0, false, w);
        v.click(x, y, w);
        assert_eq!(v.s.lock_minutes, 5);
        let (x, y) = v.control_center(3, true, w);
        assert_eq!(v.click(x, y, w), SettingsOut::Password);
        let r = v.row_rect(2, w);
        assert_eq!(v.click(r.x + 5, r.y + 5, w), SettingsOut::None, "the label only selects");
        assert_eq!(v.selected(), 2);
        v.about.push((String::from("CPUs"), String::from("4")));
        let mut c = Canvas::new(w as usize, 420, 0);
        v.render(&mut c);
    }
}
