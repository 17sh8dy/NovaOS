//! Notes: a plain-text editor. Typing, Enter (keeps the line's indent),
//! Backspace/Delete across lines, arrows, Home/End, Page Up/Down, Tab (four
//! spaces), click to place the cursor, line numbers, horizontal and vertical
//! scrolling, undo/redo (Ctrl+Z / Ctrl+Y), New / Open / Save / Save as (Ctrl+N,
//! Ctrl+O, Ctrl+S) with a path prompt, and a guard against losing unsaved text.
//!
//! The view never touches the disk: it asks with a [`NotesReq`] and the kernel
//! answers with [`Notes::loaded`] / [`Notes::saved`].

use crate::ui::{self, KeyIn};
use crate::{ACCENT, DANGER, DIM, INK, PANEL};
use alloc::string::String;
use alloc::vec::Vec;
use nova_gfx::{Canvas, Rect, fit, mix, rgb, text_height, text_width};

/// What Notes needs the kernel to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesReq {
    Save(String, Vec<u8>),
    Open(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptKind {
    SaveAs,
    Open,
}

struct Prompt {
    kind: PromptKind,
    text: String,
}

#[derive(Clone)]
struct Snap {
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
}

/// Where new files go when given a bare name.
pub const DOCUMENTS: &str = "/documents";
/// Largest file Notes opens.
pub const MAX_BYTES: usize = 256 * 1024;
const UNDO_DEPTH: usize = 200;
const TOOLBAR: [&str; 4] = ["New", "Open", "Save", "Save as"];
const CURRENT_LINE: nova_gfx::Color = rgb(0x10, 0x14, 0x30);

pub struct Notes {
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
    top: usize,
    left: usize,
    path: Option<String>,
    modified: bool,
    prompt: Option<Prompt>,
    status: String,
    status_error: bool,
    /// The next New/Open discards unsaved text (the user was warned once).
    discard_ok: bool,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// The last edit was typing a character (typing is undone a word-run at a time).
    typing: bool,
    scale: i32,
    u: i32,
}

/// A path for what the user typed: bare names go to /documents.
pub fn resolve(input: &str) -> String {
    let t = input.trim();
    if t.starts_with('/') { String::from(t) } else { alloc::format!("{DOCUMENTS}/{t}") }
}

impl Notes {
    pub fn new(scale: i32, u: i32) -> Self {
        Self {
            lines: alloc::vec![Vec::new()],
            row: 0,
            col: 0,
            top: 0,
            left: 0,
            path: None,
            modified: false,
            prompt: None,
            status: String::new(),
            status_error: false,
            discard_ok: false,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: false,
            scale,
            u,
        }
    }

    // ---- State ----

    pub fn text(&self) -> String {
        let mut s = String::new();
        for (i, l) in self.lines.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            s.extend(l.iter());
        }
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn prompt_text(&self) -> Option<&str> {
        self.prompt.as_ref().map(|p| p.text.as_str())
    }

    /// The window title: "name* - Notes".
    pub fn title(&self) -> String {
        let name = self.path.as_deref().map_or("Untitled", |p| p.rsplit('/').next().unwrap_or(p));
        alloc::format!("{name}{} - Notes", if self.modified { "*" } else { "" })
    }

    fn say(&mut self, text: &str, error: bool) {
        self.status = String::from(text);
        self.status_error = error;
    }

    fn snapshot(&mut self) {
        self.undo.push(Snap { lines: self.lines.clone(), row: self.row, col: self.col });
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn edited(&mut self) {
        self.modified = true;
        self.discard_ok = false;
    }

    fn set_text(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\t', "    ");
        let mut lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().filter(|c| !c.is_control()).collect()).collect();
        if lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        self.lines = lines;
        self.row = 0;
        self.col = 0;
        self.top = 0;
        self.left = 0;
        self.undo.clear();
        self.redo.clear();
        self.typing = false;
    }

    // ---- Disk answers ----

    /// The kernel read `path` (or could not).
    pub fn loaded(&mut self, path: &str, data: Result<Vec<u8>, String>) {
        match data {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                self.set_text(&text);
                self.path = Some(String::from(path));
                self.modified = false;
                self.discard_ok = false;
                self.say(&alloc::format!("Opened {path}"), false);
            }
            Err(e) => self.say(&alloc::format!("Could not open {path}: {e}"), true),
        }
    }

    /// The kernel wrote `path` (or could not).
    pub fn saved(&mut self, path: &str, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.path = Some(String::from(path));
                self.modified = false;
                self.say(&alloc::format!("Saved {path}"), false);
            }
            Err(e) => self.say(&alloc::format!("Could not save {path}: {e}"), true),
        }
    }

    // ---- Commands ----

    fn save(&mut self) -> Option<NotesReq> {
        match self.path.clone() {
            Some(p) => Some(NotesReq::Save(p, self.text().into_bytes())),
            None => {
                self.save_as();
                None
            }
        }
    }

    fn save_as(&mut self) {
        let text = self.path.clone().unwrap_or_else(|| alloc::format!("{DOCUMENTS}/untitled.txt"));
        self.prompt = Some(Prompt { kind: PromptKind::SaveAs, text });
    }

    /// Unsaved text is only thrown away on the second try.
    fn may_discard(&mut self) -> bool {
        if !self.modified || self.discard_ok {
            return true;
        }
        self.discard_ok = true;
        self.say("Unsaved changes! Do it again to discard them (or Ctrl+S to save)", true);
        false
    }

    fn new_doc(&mut self) {
        if self.may_discard() {
            self.set_text("");
            self.path = None;
            self.modified = false;
            self.discard_ok = false;
            self.say("New document", false);
        }
    }

    fn open_prompt(&mut self) {
        if self.may_discard() {
            self.prompt = Some(Prompt { kind: PromptKind::Open, text: alloc::format!("{DOCUMENTS}/") });
        }
    }

    /// Opens a file chosen elsewhere (Files' "Edit in Notes"). Unsaved text is kept unless discarded.
    pub fn request_open(&mut self, path: &str) -> Option<NotesReq> {
        if !self.may_discard() {
            return None;
        }
        Some(NotesReq::Open(String::from(path)))
    }

    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(Snap { lines: core::mem::replace(&mut self.lines, s.lines), row: self.row, col: self.col });
            self.row = s.row;
            self.col = s.col;
            self.edited();
            self.typing = false;
        } else {
            self.say("Nothing to undo", false);
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(Snap { lines: core::mem::replace(&mut self.lines, s.lines), row: self.row, col: self.col });
            self.row = s.row;
            self.col = s.col;
            self.edited();
            self.typing = false;
        }
    }

    // ---- Editing ----

    fn insert(&mut self, c: char) {
        if !self.typing || c == ' ' {
            self.snapshot();
        }
        self.typing = true;
        self.lines[self.row].insert(self.col, c);
        self.col += 1;
        self.edited();
    }

    fn newline(&mut self) {
        self.snapshot();
        self.typing = false;
        let indent: Vec<char> = self.lines[self.row].iter().take_while(|c| **c == ' ').copied().collect();
        let indent = &indent[..indent.len().min(self.col)];
        let rest = self.lines[self.row].split_off(self.col);
        let mut next = indent.to_vec();
        self.col = next.len();
        next.extend(rest);
        self.row += 1;
        self.lines.insert(self.row, next);
        self.edited();
    }

    fn backspace(&mut self) {
        if self.col == 0 && self.row == 0 {
            return;
        }
        self.snapshot();
        self.typing = false;
        if self.col > 0 {
            self.col -= 1;
            self.lines[self.row].remove(self.col);
        } else {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].len();
            self.lines[self.row].extend(line);
        }
        self.edited();
    }

    fn delete(&mut self) {
        let len = self.lines[self.row].len();
        if self.col == len && self.row + 1 == self.lines.len() {
            return;
        }
        self.snapshot();
        self.typing = false;
        if self.col < len {
            self.lines[self.row].remove(self.col);
        } else {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].extend(next);
        }
        self.edited();
    }

    fn clamp_col(&mut self) {
        self.col = self.col.min(self.lines[self.row].len());
    }

    /// A key, in a window whose content is `w` x `h`.
    pub fn key(&mut self, k: KeyIn, w: i32, h: i32) -> Option<NotesReq> {
        if let Some(p) = &mut self.prompt {
            match k {
                KeyIn::Char(c) if !c.is_control() && p.text.chars().count() < 120 => p.text.push(c),
                KeyIn::Backspace => {
                    p.text.pop();
                }
                KeyIn::Escape => {
                    self.prompt = None;
                    self.say("Cancelled", false);
                }
                KeyIn::Enter => {
                    let p = self.prompt.take()?;
                    let path = resolve(&p.text);
                    if path.ends_with('/') || p.text.trim().is_empty() {
                        self.say("Type a file name", true);
                        self.prompt = Some(p);
                        return None;
                    }
                    return Some(match p.kind {
                        PromptKind::SaveAs => NotesReq::Save(path, self.text().into_bytes()),
                        PromptKind::Open => NotesReq::Open(path),
                    });
                }
                _ => {}
            }
            return None;
        }
        let page = self.visible_rows(h).max(1);
        let mut req = None;
        match k {
            KeyIn::Char(c) if !c.is_control() => self.insert(c),
            KeyIn::Tab => {
                for _ in 0..4 {
                    self.insert(' ');
                }
            }
            KeyIn::Enter => self.newline(),
            KeyIn::Backspace => self.backspace(),
            KeyIn::Delete => self.delete(),
            KeyIn::Left => {
                self.typing = false;
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.lines[self.row].len();
                }
            }
            KeyIn::Right => {
                self.typing = false;
                if self.col < self.lines[self.row].len() {
                    self.col += 1;
                } else if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = 0;
                }
            }
            KeyIn::Up | KeyIn::PageUp => {
                self.typing = false;
                self.row = self.row.saturating_sub(if k == KeyIn::Up { 1 } else { page });
                self.clamp_col();
            }
            KeyIn::Down | KeyIn::PageDown => {
                self.typing = false;
                self.row = (self.row + if k == KeyIn::Down { 1 } else { page }).min(self.lines.len() - 1);
                self.clamp_col();
            }
            KeyIn::Home => self.col = 0,
            KeyIn::End => self.col = self.lines[self.row].len(),
            KeyIn::Ctrl('s') => req = self.save(),
            KeyIn::Ctrl('o') => self.open_prompt(),
            KeyIn::Ctrl('n') => self.new_doc(),
            KeyIn::Ctrl('z') => self.undo(),
            KeyIn::Ctrl('y') => self.redo(),
            _ => {}
        }
        self.keep_visible(w, h);
        req
    }

    // ---- Layout ----

    fn cell(&self) -> (i32, i32) {
        ((nova_gfx::font::WIDTH as i32 + 1) * self.scale, text_height(self.scale) + 4 * self.u)
    }

    fn bar_h(&self) -> i32 {
        text_height(self.scale) + 14 * self.u
    }

    fn gutter_w(&self) -> i32 {
        let digits = alloc::format!("{}", self.lines.len().max(99)).len() as i32;
        digits * self.cell().0 + 16 * self.u
    }

    fn text_area(&self, w: i32, h: i32) -> Rect {
        let g = self.gutter_w();
        Rect::new(g, self.bar_h(), w - g, h - 2 * self.bar_h())
    }

    fn visible_rows(&self, h: i32) -> usize {
        (self.text_area(0, h).h / self.cell().1).max(1) as usize
    }

    fn visible_cols(&self, w: i32) -> usize {
        ((self.text_area(w, 0).w - 8 * self.u) / self.cell().0).max(1) as usize
    }

    fn keep_visible(&mut self, w: i32, h: i32) {
        let (rows, cols) = (self.visible_rows(h), self.visible_cols(w));
        if self.row < self.top {
            self.top = self.row;
        } else if self.row >= self.top + rows {
            self.top = self.row + 1 - rows;
        }
        if self.col < self.left {
            self.left = self.col;
        } else if self.col >= self.left + cols {
            self.left = self.col + 1 - cols;
        }
    }

    fn tool_rect(&self, i: usize) -> Rect {
        let u = self.u;
        let mut x = 6 * u;
        for label in &TOOLBAR[..i] {
            x += text_width(label, self.scale) + 20 * u + 4 * u;
        }
        Rect::new(x, 4 * u, text_width(TOOLBAR[i], self.scale) + 20 * u, self.bar_h() - 8 * u)
    }

    /// Centre of a toolbar button (for tests and logs).
    pub fn tool_center(&self, label: &str) -> Option<(i32, i32)> {
        let i = TOOLBAR.iter().position(|l| *l == label)?;
        let r = self.tool_rect(i);
        Some((r.x + r.w / 2, r.y + r.h / 2))
    }

    /// A click: toolbar buttons, or placing the cursor in the text.
    pub fn click(&mut self, x: i32, y: i32, w: i32, h: i32) -> Option<NotesReq> {
        for i in 0..TOOLBAR.len() {
            if self.tool_rect(i).contains(x, y) {
                self.prompt = None;
                return match i {
                    0 => {
                        self.new_doc();
                        None
                    }
                    1 => {
                        self.open_prompt();
                        None
                    }
                    2 => self.save(),
                    _ => {
                        self.save_as();
                        None
                    }
                };
            }
        }
        let t = self.text_area(w, h);
        if t.contains(x, y) {
            let (cw, lh) = self.cell();
            let row = self.top + ((y - t.y) / lh) as usize;
            self.row = row.min(self.lines.len() - 1);
            self.col = (self.left + ((x - t.x - 4 * self.u + cw / 2).max(0) / cw) as usize).min(self.lines[self.row].len());
            self.typing = false;
        }
        None
    }

    // ---- Drawing ----

    pub fn render(&self, c: &mut Canvas) {
        let (w, h) = (c.width() as i32, c.height() as i32);
        let (u, s) = (self.u, self.scale);
        let all = c.bounds();
        c.fill(all, PANEL);
        let bh = self.bar_h();
        let bar = Rect::new(0, 0, w, bh);
        c.fill(bar, ui::FACE_SOFT);
        for (i, label) in TOOLBAR.iter().enumerate() {
            ui::button_on(c, self.tool_rect(i), label, s, u);
        }
        let name = self.title();
        let after = self.tool_rect(TOOLBAR.len() - 1).right() + 14 * u;
        let name = fit(&name, s, w - after - 10 * u);
        c.text(w - text_width(&name, s) - 10 * u, (bh - text_height(s)) / 2 + u, s, if self.modified { ACCENT } else { DIM }, &name, bar);

        let t = self.text_area(w, h);
        let (cw, lh) = self.cell();
        let g = Rect::new(0, t.y, t.x, t.h);
        c.fill(g, mix(PANEL, 0, 70));
        for (k, i) in (self.top..self.lines.len()).take(self.visible_rows(h)).enumerate() {
            let y = t.y + k as i32 * lh;
            if i == self.row {
                c.fill(Rect::new(t.x, y, t.w, lh), CURRENT_LINE);
            }
            let num = alloc::format!("{}", i + 1);
            c.text(t.x - text_width(&num, s) - 8 * u, y + 2 * u, s, if i == self.row { INK } else { DIM }, &num, g);
            let line = &self.lines[i];
            let shown: String = line.iter().skip(self.left).take(self.visible_cols(w) + 1).collect();
            c.text(t.x + 4 * u, y + 2 * u, s, INK, &shown, t);
            if i == self.row && self.prompt.is_none() {
                let cx = t.x + 4 * u + (self.col - self.left.min(self.col)) as i32 * cw;
                c.fill_clipped(Rect::new(cx - u, y, 2 * u, lh), ACCENT, t);
            }
        }

        let foot = Rect::new(0, h - bh, w, bh);
        c.fill(foot, ui::FACE_SOFT);
        let ty = foot.y + (bh - text_height(s)) / 2 + u;
        match &self.prompt {
            Some(p) => {
                let label = if p.kind == PromptKind::SaveAs { "Save as: " } else { "Open: " };
                c.text(10 * u, ty, s, ACCENT, label, foot);
                let lx = 10 * u + text_width(label, s);
                let field = fit(&p.text, s, w - lx - 30 * u);
                c.text(lx, ty, s, INK, &field, foot);
                c.fill_clipped(Rect::new(lx + text_width(&field, s) + u, foot.y + 5 * u, 2 * u, bh - 10 * u), ACCENT, foot);
            }
            None => {
                let pos = alloc::format!("Ln {}, Col {}   {} lines", self.row + 1, self.col + 1, self.lines.len());
                c.text(10 * u, ty, s, DIM, &pos, foot);
                let msg = if self.status.is_empty() { "Ctrl+S save  Ctrl+O open  Ctrl+Z undo" } else { &self.status };
                let x = 10 * u + text_width(&pos, s) + 24 * u;
                let msg = fit(msg, s, w - x - 10 * u);
                c.text(x, ty, s, if self.status_error { DANGER } else { DIM }, &msg, foot);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: i32 = 600;
    const H: i32 = 300;

    fn typed(n: &mut Notes, s: &str) {
        for ch in s.chars() {
            let k = if ch == '\n' { KeyIn::Enter } else { KeyIn::Char(ch) };
            n.key(k, W, H);
        }
    }

    #[test]
    fn typing_lines_and_indent() {
        let mut n = Notes::new(2, 1);
        typed(&mut n, "fn main() {\n    hello\nbye");
        assert_eq!(n.text(), "fn main() {\n    hello\n    bye\n", "Enter keeps the indent");
        assert_eq!(n.cursor(), (2, 7));
        assert!(n.is_modified());
        assert_eq!(n.title(), "Untitled* - Notes");
        for _ in 0..3 {
            n.key(KeyIn::Left, W, H);
        }
        for _ in 0..4 {
            n.key(KeyIn::Backspace, W, H);
        }
        n.key(KeyIn::Backspace, W, H); // joins with the line above
        assert_eq!(n.text(), "fn main() {\n    hellobye\n");
        n.key(KeyIn::Delete, W, H);
        assert_eq!(n.text(), "fn main() {\n    helloye\n");
        n.key(KeyIn::Up, W, H);
        n.key(KeyIn::End, W, H);
        n.key(KeyIn::Delete, W, H); // joins the next line
        assert_eq!(n.text(), "fn main() {    helloye\n");
        n.key(KeyIn::Tab, W, H);
        assert_eq!(n.cursor(), (0, 15));
    }

    #[test]
    fn cursor_movement_wraps_between_lines() {
        let mut n = Notes::new(2, 1);
        typed(&mut n, "ab\ncdef");
        n.key(KeyIn::Up, W, H);
        assert_eq!(n.cursor(), (0, 2), "column clamped to the shorter line");
        n.key(KeyIn::Right, W, H);
        assert_eq!(n.cursor(), (1, 0));
        n.key(KeyIn::Left, W, H);
        assert_eq!(n.cursor(), (0, 2));
        n.key(KeyIn::PageDown, W, H);
        assert_eq!(n.cursor().0, 1);
    }

    #[test]
    fn undo_and_redo() {
        let mut n = Notes::new(2, 1);
        typed(&mut n, "hello world");
        n.key(KeyIn::Backspace, W, H);
        assert_eq!(n.text(), "hello worl\n");
        n.key(KeyIn::Ctrl('z'), W, H);
        assert_eq!(n.text(), "hello world\n");
        n.key(KeyIn::Ctrl('z'), W, H);
        assert_eq!(n.text(), "hello\n", "typing is undone a word at a time");
        n.key(KeyIn::Ctrl('z'), W, H);
        assert_eq!(n.text(), "");
        n.key(KeyIn::Ctrl('y'), W, H);
        n.key(KeyIn::Ctrl('y'), W, H);
        assert_eq!(n.text(), "hello world\n");
        typed(&mut n, "!");
        n.key(KeyIn::Ctrl('y'), W, H);
        assert_eq!(n.text(), "hello world!\n", "a new edit clears redo");
    }

    #[test]
    fn save_asks_for_a_name_then_saves_in_place() {
        let mut n = Notes::new(2, 1);
        typed(&mut n, "list");
        assert_eq!(n.key(KeyIn::Ctrl('s'), W, H), None);
        assert_eq!(n.prompt_text(), Some("/documents/untitled.txt"));
        for _ in 0.."untitled.txt".len() {
            n.key(KeyIn::Backspace, W, H);
        }
        typed(&mut n, "todo.txt");
        let req = n.key(KeyIn::Enter, W, H);
        assert_eq!(req, Some(NotesReq::Save(String::from("/documents/todo.txt"), b"list\n".to_vec())));
        n.saved("/documents/todo.txt", Ok(()));
        assert!(!n.is_modified());
        assert_eq!(n.title(), "todo.txt - Notes");
        typed(&mut n, "!");
        assert!(matches!(n.key(KeyIn::Ctrl('s'), W, H), Some(NotesReq::Save(p, _)) if p == "/documents/todo.txt"));
        n.saved("/documents/todo.txt", Err(String::from("disk full")));
        assert!(n.is_modified());
        assert!(n.status().contains("disk full"));
        assert_eq!(resolve("a.txt"), "/documents/a.txt");
        assert_eq!(resolve(" /x/y.txt "), "/x/y.txt");
    }

    #[test]
    fn unsaved_text_is_not_lost_by_accident() {
        let mut n = Notes::new(2, 1);
        typed(&mut n, "precious");
        n.key(KeyIn::Ctrl('n'), W, H);
        assert_eq!(n.text(), "precious\n", "first New only warns");
        assert!(n.status().contains("Unsaved"));
        n.key(KeyIn::Ctrl('n'), W, H);
        assert_eq!(n.text(), "");
        typed(&mut n, "again");
        assert_eq!(n.request_open("/a.txt"), None);
        assert_eq!(n.request_open("/a.txt"), Some(NotesReq::Open(String::from("/a.txt"))));
    }

    #[test]
    fn open_loads_text() {
        let mut n = Notes::new(2, 1);
        n.key(KeyIn::Ctrl('o'), W, H);
        typed(&mut n, "readme.txt");
        assert_eq!(n.key(KeyIn::Enter, W, H), Some(NotesReq::Open(String::from("/documents/readme.txt"))));
        n.loaded("/documents/readme.txt", Ok(b"a\r\n\tb\n".to_vec()));
        assert_eq!(n.text(), "a\n    b\n");
        assert!(!n.is_modified());
        n.key(KeyIn::Ctrl('z'), W, H);
        assert_eq!(n.text(), "a\n    b\n", "nothing to undo after opening");
        n.loaded("/nope", Err(String::from("not found")));
        assert!(n.status().contains("not found"));
        assert_eq!(n.text(), "a\n    b\n");
        // Escape cancels a prompt.
        n.key(KeyIn::Ctrl('o'), W, H);
        n.key(KeyIn::Escape, W, H);
        assert_eq!(n.prompt_text(), None);
    }

    #[test]
    fn clicks_place_the_cursor_and_long_text_scrolls() {
        let mut n = Notes::new(2, 1);
        for i in 0..100 {
            typed(&mut n, &alloc::format!("line {i}\n"));
        }
        assert!(n.top > 0, "scrolled to keep the cursor visible");
        let (cw, lh) = n.cell();
        let t = n.text_area(W, H);
        n.key(KeyIn::Ctrl('z'), W, H); // harmless
        let before_top = n.top;
        n.click(t.x + 4 + 3 * cw, t.y + lh / 2, W, H);
        assert_eq!(n.cursor(), (before_top, 3));
        let long: String = core::iter::repeat_n('x', 300).collect();
        typed(&mut n, &long);
        assert!(n.left > 0, "scrolled sideways");
        let (x, y) = n.tool_center("Save as").unwrap();
        n.click(x, y, W, H);
        assert!(n.prompt_text().is_some());
        let mut c = Canvas::new(W as usize, H as usize, 0);
        n.render(&mut c);
    }
}
