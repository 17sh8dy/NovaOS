//! A text console model: a grid of character cells with a cursor, scrolling
//! and per-row dirty tracking (so a renderer redraws only what changed), plus
//! a line editor with history. No drawing and no keyboard hardware here, so
//! it is unit-tested on the host.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

use alloc::string::String;
use alloc::vec::Vec;

/// A colour index; the renderer maps these to real colours.
pub type Color = u8;
pub const COLOR_DEFAULT: Color = 0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub color: Color,
}

const BLANK: Cell = Cell { ch: ' ', color: COLOR_DEFAULT };

/// A `cols` x `rows` grid of cells with a text cursor.
pub struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    dirty: Vec<bool>,
    cx: usize,
    cy: usize,
    color: Color,
    /// Rows that have scrolled off the top since the last call to [`Grid::take_scrolled`].
    scrolled: usize,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Self { cols, rows, cells: alloc::vec![BLANK; cols * rows], dirty: alloc::vec![true; rows], cx: 0, cy: 0, color: COLOR_DEFAULT, scrolled: 0 }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cursor(&self) -> (usize, usize) {
        (self.cx, self.cy)
    }

    pub fn cell(&self, col: usize, row: usize) -> Cell {
        self.cells[row * self.cols + col]
    }

    /// The colour new text is written in.
    pub fn set_color(&mut self, color: Color) {
        self.color = color;
    }

    fn mark(&mut self, row: usize) {
        self.dirty[row] = true;
    }

    /// Moves the cursor (clamped to the grid).
    pub fn set_cursor(&mut self, col: usize, row: usize) {
        self.mark(self.cy);
        self.cx = col.min(self.cols - 1);
        self.cy = row.min(self.rows - 1);
        self.mark(self.cy);
    }

    /// Scrolls the whole grid up one row (the top row is lost).
    pub fn scroll_up(&mut self) {
        self.cells.copy_within(self.cols.., 0);
        let last = (self.rows - 1) * self.cols;
        self.cells[last..].fill(BLANK);
        self.dirty.fill(true);
        self.scrolled += 1;
    }

    pub fn take_scrolled(&mut self) -> usize {
        core::mem::take(&mut self.scrolled)
    }

    fn newline(&mut self) {
        self.mark(self.cy);
        self.cx = 0;
        if self.cy + 1 == self.rows {
            self.scroll_up();
        } else {
            self.cy += 1;
        }
        self.mark(self.cy);
    }

    /// Writes one character at the cursor. Handles `\n`, `\r`, `\t`,
    /// backspace (moves left, erasing) and wrapping/scrolling.
    pub fn put(&mut self, ch: char) {
        match ch {
            '\n' => self.newline(),
            '\r' => {
                self.mark(self.cy);
                self.cx = 0;
            }
            '\t' => {
                let next = (self.cx / 8 + 1) * 8;
                while self.cx < next.min(self.cols) {
                    self.put(' ');
                }
            }
            '\u{8}' => {
                if self.cx > 0 {
                    self.cx -= 1;
                    self.cells[self.cy * self.cols + self.cx] = BLANK;
                    self.mark(self.cy);
                }
            }
            c if c.is_control() => {}
            c => {
                if self.cx == self.cols {
                    self.newline();
                }
                self.cells[self.cy * self.cols + self.cx] = Cell { ch: c, color: self.color };
                self.cx += 1;
                self.mark(self.cy);
                // Wrapping happens lazily, on the next character, like most terminals.
                if self.cx == self.cols {
                    self.cx = self.cols; // parked past the last column
                }
            }
        }
    }

    pub fn write_str(&mut self, s: &str) {
        for c in s.chars() {
            self.put(c);
        }
    }

    /// Blanks from the cursor to the end of its row.
    pub fn clear_to_end_of_row(&mut self) {
        let start = self.cy * self.cols + self.cx.min(self.cols);
        let end = (self.cy + 1) * self.cols;
        self.cells[start..end].fill(BLANK);
        self.mark(self.cy);
    }

    /// Clears everything and homes the cursor.
    pub fn clear(&mut self) {
        self.cells.fill(BLANK);
        self.dirty.fill(true);
        self.cx = 0;
        self.cy = 0;
    }

    /// True if `row` changed since it was last taken.
    pub fn is_dirty(&self, row: usize) -> bool {
        self.dirty[row]
    }

    /// Marks a row clean (call after redrawing it).
    pub fn clean(&mut self, row: usize) {
        self.dirty[row] = false;
    }

    /// Marks one row for redrawing (e.g. when only the cursor's look changed).
    pub fn mark_dirty(&mut self, row: usize) {
        self.dirty[row.min(self.rows - 1)] = true;
    }

    /// Forces a full redraw.
    pub fn mark_all_dirty(&mut self) {
        self.dirty.fill(true);
    }
}

/// A key the line editor understands (the kernel maps real key input onto this).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKey {
    Char(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    Enter,
    /// Ctrl+U: delete everything before the cursor.
    KillLine,
    /// Ctrl+W: delete the word before the cursor.
    KillWord,
    /// Ctrl+C: abandon the line.
    Cancel,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Edit {
    /// The line changed (or the cursor moved): redraw it.
    Changed,
    /// Enter was pressed: here is the finished line.
    Submitted(String),
    /// Ctrl+C.
    Cancelled,
    /// Nothing happened.
    Nothing,
}

const HISTORY: usize = 32;

/// Edits one line of input, with history on Up/Down.
#[derive(Default)]
pub struct LineEditor {
    /// The line as typed, as chars (so the cursor is a char index).
    line: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    /// Which history entry is shown (`None` = the line being typed).
    browsing: Option<usize>,
    /// What was being typed before Up was pressed.
    draft: Vec<char>,
}

impl LineEditor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> String {
        self.line.iter().collect()
    }

    /// Cursor position as a char index into the text.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_line(&mut self, text: &str) {
        self.line = text.chars().collect();
        self.cursor = self.line.len();
    }

    pub fn handle(&mut self, key: EditKey) -> Edit {
        match key {
            EditKey::Char(c) if !c.is_control() => {
                self.browsing = None;
                self.line.insert(self.cursor, c);
                self.cursor += 1;
                Edit::Changed
            }
            EditKey::Char(_) => Edit::Nothing,
            EditKey::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.line.remove(self.cursor);
                Edit::Changed
            }
            EditKey::Delete if self.cursor < self.line.len() => {
                self.line.remove(self.cursor);
                Edit::Changed
            }
            EditKey::Left if self.cursor > 0 => {
                self.cursor -= 1;
                Edit::Changed
            }
            EditKey::Right if self.cursor < self.line.len() => {
                self.cursor += 1;
                Edit::Changed
            }
            EditKey::Home if self.cursor > 0 => {
                self.cursor = 0;
                Edit::Changed
            }
            EditKey::End if self.cursor < self.line.len() => {
                self.cursor = self.line.len();
                Edit::Changed
            }
            EditKey::KillLine if self.cursor > 0 => {
                self.line.drain(..self.cursor);
                self.cursor = 0;
                Edit::Changed
            }
            EditKey::KillWord if self.cursor > 0 => {
                let mut start = self.cursor;
                while start > 0 && self.line[start - 1] == ' ' {
                    start -= 1;
                }
                while start > 0 && self.line[start - 1] != ' ' {
                    start -= 1;
                }
                self.line.drain(start..self.cursor);
                self.cursor = start;
                Edit::Changed
            }
            EditKey::Up => self.browse(true),
            EditKey::Down => self.browse(false),
            EditKey::Cancel => {
                self.line.clear();
                self.cursor = 0;
                self.browsing = None;
                Edit::Cancelled
            }
            EditKey::Enter => {
                let text = self.text();
                self.line.clear();
                self.cursor = 0;
                self.browsing = None;
                if !text.trim().is_empty() && self.history.last() != Some(&text) {
                    if self.history.len() == HISTORY {
                        self.history.remove(0);
                    }
                    self.history.push(text.clone());
                }
                Edit::Submitted(text)
            }
            _ => Edit::Nothing,
        }
    }

    fn browse(&mut self, older: bool) -> Edit {
        if self.history.is_empty() {
            return Edit::Nothing;
        }
        let next = match (self.browsing, older) {
            (None, true) => {
                self.draft = self.line.clone();
                Some(self.history.len() - 1)
            }
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), false) => None,
            (None, false) => return Edit::Nothing,
        };
        self.browsing = next;
        match next {
            Some(i) => {
                let t = self.history[i].clone();
                self.set_line(&t);
            }
            None => {
                self.line = core::mem::take(&mut self.draft);
                self.cursor = self.line.len();
            }
        }
        Edit::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::ToString;

    fn row_text(g: &Grid, row: usize) -> String {
        (0..g.cols()).map(|c| g.cell(c, row).ch).collect::<String>().trim_end().to_string()
    }

    #[test]
    fn text_wraps_at_the_right_edge() {
        let mut g = Grid::new(5, 3);
        g.write_str("abcdefgh");
        assert_eq!(row_text(&g, 0), "abcde");
        assert_eq!(row_text(&g, 1), "fgh");
        assert_eq!(g.cursor(), (3, 1));
    }

    #[test]
    fn writing_exactly_a_full_row_does_not_leave_a_blank_row() {
        let mut g = Grid::new(5, 3);
        g.write_str("abcde\nxy");
        assert_eq!(row_text(&g, 0), "abcde");
        assert_eq!(row_text(&g, 1), "xy", "the newline after a full row starts the next row, not a blank one");
        assert_eq!(row_text(&g, 2), "");
    }

    #[test]
    fn scrolls_when_the_bottom_is_passed() {
        let mut g = Grid::new(6, 2);
        g.write_str("one\ntwo\nthree");
        assert_eq!(row_text(&g, 0), "two");
        assert_eq!(row_text(&g, 1), "three");
        assert_eq!(g.take_scrolled(), 1);
        assert_eq!(g.take_scrolled(), 0);
        assert_eq!(g.cursor().1, 1);
    }

    #[test]
    fn backspace_tab_carriage_return() {
        let mut g = Grid::new(20, 2);
        g.write_str("abc\u{8}d");
        assert_eq!(row_text(&g, 0), "abd");
        g.write_str("\r12");
        assert_eq!(row_text(&g, 0), "12d");
        g.write_str("\n\tx");
        assert_eq!(g.cell(8, 1).ch, 'x');
        // Backspace never goes left of column 0.
        g.set_cursor(0, 0);
        g.put('\u{8}');
        assert_eq!(g.cursor(), (0, 0));
    }

    #[test]
    fn colours_apply_to_new_text_only() {
        let mut g = Grid::new(10, 1);
        g.write_str("a");
        g.set_color(3);
        g.write_str("b");
        assert_eq!((g.cell(0, 0).color, g.cell(1, 0).color), (0, 3));
    }

    #[test]
    fn dirty_rows_track_changes_and_scrolling_dirties_everything() {
        let mut g = Grid::new(10, 3);
        for r in 0..3 {
            g.clean(r);
        }
        g.set_cursor(0, 1); // moving the cursor dirties the row it left and the row it enters
        assert!(g.is_dirty(0) && g.is_dirty(1) && !g.is_dirty(2));
        for r in 0..3 {
            g.clean(r);
        }
        g.put('x');
        assert!(!g.is_dirty(0) && g.is_dirty(1) && !g.is_dirty(2));
        for r in 0..3 {
            g.clean(r);
        }
        g.scroll_up();
        assert!(g.is_dirty(0) && g.is_dirty(1) && g.is_dirty(2));
    }

    #[test]
    fn clear_to_end_of_row_and_clear() {
        let mut g = Grid::new(6, 2);
        g.write_str("hello");
        g.set_cursor(2, 0);
        g.clear_to_end_of_row();
        assert_eq!(row_text(&g, 0), "he");
        g.clear();
        assert_eq!(row_text(&g, 0), "");
        assert_eq!(g.cursor(), (0, 0));
    }

    #[test]
    fn control_characters_are_ignored_not_drawn() {
        let mut g = Grid::new(5, 1);
        g.write_str("a\u{7}\u{1b}b");
        assert_eq!(row_text(&g, 0), "ab");
    }

    fn type_str(e: &mut LineEditor, s: &str) {
        for c in s.chars() {
            e.handle(EditKey::Char(c));
        }
    }

    #[test]
    fn edits_in_the_middle_of_a_line() {
        let mut e = LineEditor::new();
        type_str(&mut e, "helo");
        e.handle(EditKey::Left);
        e.handle(EditKey::Char('l'));
        assert_eq!(e.text(), "hello");
        assert_eq!(e.cursor(), 4);
        e.handle(EditKey::Home);
        e.handle(EditKey::Delete);
        assert_eq!(e.text(), "ello");
        e.handle(EditKey::End);
        e.handle(EditKey::Backspace);
        assert_eq!(e.text(), "ell");
        assert_eq!(e.handle(EditKey::Backspace), Edit::Changed);
        e.handle(EditKey::Home);
        assert_eq!(e.handle(EditKey::Backspace), Edit::Nothing, "nothing before the cursor");
        assert_eq!(e.handle(EditKey::Left), Edit::Nothing);
    }

    #[test]
    fn kill_line_and_kill_word() {
        let mut e = LineEditor::new();
        type_str(&mut e, "echo hello  world");
        e.handle(EditKey::KillWord);
        assert_eq!(e.text(), "echo hello  ");
        e.handle(EditKey::KillWord);
        assert_eq!(e.text(), "echo ");
        e.handle(EditKey::Left);
        e.handle(EditKey::KillLine);
        assert_eq!(e.text(), " ");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn enter_submits_and_clears() {
        let mut e = LineEditor::new();
        type_str(&mut e, "ls");
        assert_eq!(e.handle(EditKey::Enter), Edit::Submitted("ls".into()));
        assert_eq!(e.text(), "");
        assert_eq!(e.handle(EditKey::Enter), Edit::Submitted("".into()), "empty lines submit too");
    }

    #[test]
    fn cancel_discards_the_line() {
        let mut e = LineEditor::new();
        type_str(&mut e, "oops");
        assert_eq!(e.handle(EditKey::Cancel), Edit::Cancelled);
        assert_eq!(e.text(), "");
    }

    #[test]
    fn history_browsing_keeps_the_draft() {
        let mut e = LineEditor::new();
        for line in ["one", "two", "two", "three"] {
            type_str(&mut e, line);
            e.handle(EditKey::Enter);
        }
        type_str(&mut e, "dr");
        e.handle(EditKey::Up);
        assert_eq!(e.text(), "three");
        e.handle(EditKey::Up);
        assert_eq!(e.text(), "two", "the duplicate 'two' was stored once");
        e.handle(EditKey::Up);
        assert_eq!(e.text(), "one");
        e.handle(EditKey::Up);
        assert_eq!(e.text(), "one", "stops at the oldest");
        e.handle(EditKey::Down);
        e.handle(EditKey::Down);
        e.handle(EditKey::Down);
        assert_eq!(e.text(), "dr", "coming back down restores what was being typed");
        assert_eq!(e.handle(EditKey::Down), Edit::Nothing);
    }

    #[test]
    fn history_is_bounded() {
        let mut e = LineEditor::new();
        for i in 0..(HISTORY + 10) {
            type_str(&mut e, &std::format!("cmd{i}"));
            e.handle(EditKey::Enter);
        }
        for _ in 0..(HISTORY + 5) {
            e.handle(EditKey::Up);
        }
        assert_eq!(e.text(), "cmd10", "the oldest entries were dropped");
    }

    #[test]
    fn non_printable_chars_are_not_inserted() {
        let mut e = LineEditor::new();
        assert_eq!(e.handle(EditKey::Char('\u{7}')), Edit::Nothing);
        assert_eq!(e.text(), "");
    }
}
