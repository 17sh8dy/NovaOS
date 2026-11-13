//! The built-in apps' views: what the Files and System windows show and how
//! they react to clicks and keys. They only draw into a canvas and *ask* for
//! data ([`Request`]); the kernel reads the file system and hands the answers
//! back, so all of this is testable on the host.

use crate::{ACCENT, DANGER, DIM, INK, PANEL};
use alloc::string::String;
use alloc::vec::Vec;
use nova_gfx::{Canvas, Rect, fit, mix, rgb, text_height, text_width};

const HEADER: nova_gfx::Color = rgb(0x12, 0x16, 0x34);
const SELECTED: nova_gfx::Color = rgb(0x1E, 0x3A, 0x4A);
const RULE: nova_gfx::Color = rgb(0x23, 0x29, 0x58);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

/// Keyboard navigation in a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// Enter: open the selection.
    Open,
    /// Backspace: the parent folder.
    Back,
}

/// Data the Files view wants the kernel to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// List this folder, then call [`FilesView::set_listing`].
    List(String),
    /// Read the start of this file, then call [`FilesView::set_preview`].
    Preview(String),
}

pub fn join(dir: &str, name: &str) -> String {
    if dir == "/" { alloc::format!("/{name}") } else { alloc::format!("{dir}/{name}") }
}

pub fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) | None => String::from("/"),
        Some(i) => String::from(&path[..i]),
    }
}

fn size_text(n: u64) -> String {
    if n < 10 * 1024 {
        alloc::format!("{n} B")
    } else if n < 10 << 20 {
        alloc::format!("{} KiB", n >> 10)
    } else {
        alloc::format!("{} MiB", n >> 20)
    }
}

/// The Files window: a folder listing on the left, a preview of the selected file on the right.
pub struct FilesView {
    pub path: String,
    entries: Vec<Entry>,
    selected: usize,
    scroll: usize,
    error: Option<String>,
    preview_name: String,
    preview: Vec<String>,
    scale: i32,
    u: i32,
}

/// Bytes of a file the preview reads.
pub const PREVIEW_BYTES: usize = 4096;

impl FilesView {
    pub fn new(scale: i32, u: i32) -> Self {
        Self { path: String::from("/"), entries: Vec::new(), selected: 0, scroll: 0, error: None, preview_name: String::new(), preview: Vec::new(), scale, u }
    }

    fn row_h(&self) -> i32 {
        text_height(self.scale) + 8 * self.u
    }

    fn header(&self, w: i32) -> Rect {
        Rect::new(0, 0, w, self.row_h() + 6 * self.u)
    }

    fn list_area(&self, w: i32, h: i32) -> Rect {
        let top = self.header(w).bottom();
        Rect::new(0, top, w * 3 / 5, h - top - self.row_h())
    }

    fn preview_area(&self, w: i32, h: i32) -> Rect {
        let l = self.list_area(w, h);
        Rect::new(l.right() + self.u, l.y, w - l.right() - self.u, l.h)
    }

    fn visible_rows(&self, h: i32) -> usize {
        (self.list_area(1, h).h / self.row_h()).max(1) as usize
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    /// The folder's contents (or why it could not be read). Folders first, then files, by name;
    /// `..` on top unless this is `/`.
    pub fn set_listing(&mut self, path: &str, listing: Result<Vec<Entry>, String>) {
        self.path = String::from(path);
        self.selected = 0;
        self.scroll = 0;
        self.preview.clear();
        self.preview_name.clear();
        self.entries.clear();
        if path != "/" {
            self.entries.push(Entry { name: String::from(".."), dir: true, size: 0 });
        }
        match listing {
            Ok(mut list) => {
                list.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
                self.entries.extend(list);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// The first bytes of the selected file (or why they could not be read).
    pub fn set_preview(&mut self, name: &str, data: Result<Vec<u8>, String>) {
        self.preview_name = String::from(name);
        self.preview = match data {
            Ok(bytes) if bytes.contains(&0) => alloc::vec![String::from("(binary file)")],
            Ok(bytes) => String::from_utf8_lossy(&bytes[..bytes.len().min(PREVIEW_BYTES)])
                .lines()
                .take(60)
                .map(|l| l.chars().map(|c| if c == '\t' { ' ' } else if c.is_control() { '?' } else { c }).collect())
                .collect(),
            Err(e) => alloc::vec![alloc::format!("cannot read: {e}")],
        };
        if self.preview.is_empty() {
            self.preview.push(String::from("(empty file)"));
        }
    }

    fn select(&mut self, i: usize, h: i32) -> Option<Request> {
        if self.entries.is_empty() {
            return None;
        }
        let i = i.min(self.entries.len() - 1);
        self.selected = i;
        let rows = self.visible_rows(h);
        if i < self.scroll {
            self.scroll = i;
        } else if i >= self.scroll + rows {
            self.scroll = i + 1 - rows;
        }
        let e = &self.entries[i];
        if e.dir {
            self.preview.clear();
            self.preview_name.clear();
            None
        } else {
            Some(Request::Preview(join(&self.path, &e.name)))
        }
    }

    fn open(&mut self) -> Option<Request> {
        let e = self.entries.get(self.selected)?;
        if e.name == ".." {
            Some(Request::List(parent(&self.path)))
        } else if e.dir {
            Some(Request::List(join(&self.path, &e.name)))
        } else {
            Some(Request::Preview(join(&self.path, &e.name)))
        }
    }

    /// A click at (x, y) in a `w` x `h` content area: selects a row; clicking
    /// the selected row again opens it.
    pub fn click(&mut self, x: i32, y: i32, w: i32, h: i32) -> Option<Request> {
        let l = self.list_area(w, h);
        if !l.contains(x, y) {
            return None;
        }
        let i = self.scroll + ((y - l.y) / self.row_h()) as usize;
        if i >= self.entries.len() {
            return None;
        }
        if i == self.selected {
            self.open()
        } else {
            self.select(i, h)
        }
    }

    pub fn key(&mut self, nav: Nav, h: i32) -> Option<Request> {
        let page = self.visible_rows(h);
        let last = self.entries.len().saturating_sub(1);
        match nav {
            Nav::Up => self.select(self.selected.saturating_sub(1), h),
            Nav::Down => self.select((self.selected + 1).min(last), h),
            Nav::Home => self.select(0, h),
            Nav::End => self.select(last, h),
            Nav::PageUp => self.select(self.selected.saturating_sub(page), h),
            Nav::PageDown => self.select((self.selected + page).min(last), h),
            Nav::Open => self.open(),
            Nav::Back if self.path != "/" => Some(Request::List(parent(&self.path))),
            Nav::Back => None,
        }
    }

    /// Screen position (content coordinates) of the middle of row `i`, if visible.
    pub fn row_center(&self, i: usize, w: i32, h: i32) -> Option<(i32, i32)> {
        let l = self.list_area(w, h);
        let r = i.checked_sub(self.scroll)?;
        (r < self.visible_rows(h)).then(|| (l.x + l.w / 2, l.y + r as i32 * self.row_h() + self.row_h() / 2))
    }

    pub fn render(&self, c: &mut Canvas) {
        let (w, h) = (c.width() as i32, c.height() as i32);
        let (u, s) = (self.u, self.scale);
        let all = c.bounds();
        c.fill(all, PANEL);
        let hdr = self.header(w);
        c.fill(hdr, HEADER);
        let ty = |r: Rect| r.y + (r.h - text_height(s)) / 2 + u;
        c.text(10 * u, ty(hdr), s, ACCENT, &fit(&self.path, s, w - 20 * u), hdr);

        let l = self.list_area(w, h);
        let rh = self.row_h();
        if let Some(e) = &self.error {
            c.text(l.x + 10 * u, l.y + 8 * u, s, DANGER, &fit(e, s, l.w - 20 * u), l);
        }
        for (row, (i, e)) in self.entries.iter().enumerate().skip(self.scroll).take(self.visible_rows(h)).map(|(i, e)| (i - self.scroll, (i, e))) {
            let r = Rect::new(l.x, l.y + row as i32 * rh, l.w, rh);
            if i == self.selected {
                c.fill(r, SELECTED);
                c.fill(Rect::new(r.x, r.y, 3 * u, r.h), ACCENT);
            }
            let (label, color) = if e.dir { (alloc::format!("{}/", e.name), ACCENT) } else { (e.name.clone(), INK) };
            let size = if e.dir { String::new() } else { size_text(e.size) };
            let sw = text_width(&size, s);
            c.text(r.x + 12 * u, ty(r), s, color, &fit(&label, s, r.w - sw - 30 * u), r);
            c.text(r.right() - sw - 10 * u, ty(r), s, DIM, &size, r);
        }
        if self.entries.len() > self.visible_rows(h) {
            // A scroll indicator.
            let total = self.entries.len() as i32;
            let bar_h = (l.h * self.visible_rows(h) as i32 / total).max(6 * u);
            let y = l.y + (l.h - bar_h) * self.scroll as i32 / (total - self.visible_rows(h) as i32).max(1);
            c.fill(Rect::new(l.right() - 3 * u, y, 2 * u, bar_h), DIM);
        }

        let p = self.preview_area(w, h);
        c.fill(Rect::new(p.x - u, p.y, u, p.h), RULE);
        c.fill(p, mix(PANEL, 0, 60));
        if self.preview_name.is_empty() {
            c.text(p.x + 10 * u, p.y + 8 * u, s, DIM, &fit("Select a file to preview it", s, p.w - 20 * u), p);
        } else {
            c.text(p.x + 10 * u, p.y + 8 * u, s, ACCENT, &fit(&self.preview_name, s, p.w - 20 * u), p);
            let lh = text_height(s) + 2 * u;
            // Long lines wrap at the pane's width.
            let per_line = ((p.w - 20 * u) / ((nova_gfx::font::WIDTH as i32 + 1) * s)).max(1) as usize;
            let rows = self.preview.iter().flat_map(|line| {
                let chars: Vec<char> = line.chars().collect();
                let n = chars.len().div_ceil(per_line).max(1);
                (0..n).map(move |i| chars[(i * per_line).min(chars.len())..((i + 1) * per_line).min(chars.len())].iter().collect::<String>())
            });
            for (k, row) in rows.enumerate() {
                let y = p.y + 8 * u + (k as i32 + 1) * lh + 4 * u;
                if y + lh > p.bottom() {
                    break;
                }
                c.text(p.x + 10 * u, y, s, INK, &row, p);
            }
        }

        let foot = Rect::new(0, h - rh, w, rh);
        c.fill(foot, HEADER);
        let items = self.entries.iter().filter(|e| e.name != "..").count();
        let help = alloc::format!("{items} item(s)   Enter open   Backspace up");
        c.text(10 * u, ty(foot), s, DIM, &fit(&help, s, w - 20 * u), foot);
    }
}

/// A simple "label: value" page (the System window).
pub fn render_info(c: &mut Canvas, scale: i32, u: i32, lines: &[(String, String)]) {
    let all = c.bounds();
    c.fill(all, PANEL);
    let lh = text_height(scale) + 6 * u;
    let label_w = lines.iter().map(|(l, _)| text_width(l, scale)).max().unwrap_or(0) + 24 * u;
    for (i, (label, value)) in lines.iter().enumerate() {
        let y = 12 * u + i as i32 * lh;
        if label.is_empty() {
            c.text(14 * u, y, scale, ACCENT, value, all);
            continue;
        }
        c.text(14 * u, y, scale, DIM, label, all);
        c.text(14 * u + label_w, y, scale, INK, &fit(value, scale, all.w - label_w - 28 * u), all);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    fn e(name: &str, dir: bool, size: u64) -> Entry {
        Entry { name: String::from(name), dir, size }
    }

    const W: i32 = 600;
    const H: i32 = 400;

    fn view() -> FilesView {
        let mut v = FilesView::new(2, 1);
        v.set_listing("/docs", Ok(vec![e("b.txt", false, 5), e("sub", true, 0), e("a.txt", false, 20000)]));
        v
    }

    #[test]
    fn paths() {
        assert_eq!(join("/", "bin"), "/bin");
        assert_eq!(join("/bin", "ls"), "/bin/ls");
        assert_eq!(parent("/bin/ls"), "/bin");
        assert_eq!(parent("/bin"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(size_text(5), "5 B");
        assert_eq!(size_text(20000), "19 KiB");
        assert_eq!(size_text(50 << 20), "50 MiB");
    }

    #[test]
    fn listing_is_sorted_with_parent_first() {
        let v = view();
        let names: Vec<_> = v.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["..", "sub", "a.txt", "b.txt"]);
        let mut root = FilesView::new(2, 1);
        root.set_listing("/", Ok(vec![e("bin", true, 0)]));
        assert_eq!(root.entries().len(), 1, "no .. at the root");
        root.set_listing("/x", Err(String::from("no such file or directory")));
        assert_eq!(root.entries().len(), 1);
    }

    #[test]
    fn keyboard_navigation() {
        let mut v = view();
        assert_eq!(v.key(Nav::Down, H), None, "sub is a folder: nothing to preview");
        assert_eq!(v.key(Nav::Down, H), Some(Request::Preview(String::from("/docs/a.txt"))));
        assert_eq!(v.key(Nav::End, H), Some(Request::Preview(String::from("/docs/b.txt"))));
        assert_eq!(v.key(Nav::Down, H), Some(Request::Preview(String::from("/docs/b.txt"))), "stays on the last");
        v.key(Nav::Home, H);
        assert_eq!(v.key(Nav::Open, H), Some(Request::List(String::from("/"))), ".. goes up");
        v.key(Nav::Down, H);
        assert_eq!(v.key(Nav::Open, H), Some(Request::List(String::from("/docs/sub"))));
        assert_eq!(v.key(Nav::Back, H), Some(Request::List(String::from("/"))));
    }

    #[test]
    fn clicking_selects_then_opens() {
        let mut v = view();
        let (x, y) = v.row_center(1, W, H).unwrap();
        assert_eq!(v.click(x, y, W, H), None);
        assert_eq!(v.selected().unwrap().name, "sub");
        assert_eq!(v.click(x, y, W, H), Some(Request::List(String::from("/docs/sub"))));
        let (x, y) = v.row_center(3, W, H).unwrap();
        assert_eq!(v.click(x, y, W, H), Some(Request::Preview(String::from("/docs/b.txt"))));
        assert_eq!(v.click(5, 2, W, H), None, "header");
        let (x, _) = v.row_center(0, W, H).unwrap();
        assert_eq!(v.click(x, H - 60, W, H), None, "below the last row");
    }

    #[test]
    fn long_lists_scroll() {
        let mut v = FilesView::new(2, 1);
        v.set_listing("/", Ok((0..100).map(|i| e(&std::format!("f{i:03}"), false, 1)).collect()));
        let rows = v.visible_rows(H);
        assert!(rows < 100);
        assert!(v.row_center(rows, W, H).is_none());
        v.key(Nav::End, H);
        assert_eq!(v.selected().unwrap().name, "f099");
        assert!(v.row_center(99, W, H).is_some(), "the selection is scrolled into view");
        assert!(v.row_center(0, W, H).is_none());
        v.key(Nav::PageUp, H);
        assert_eq!(v.selected().unwrap().name, std::format!("f{:03}", 99 - rows));
    }

    #[test]
    fn previews() {
        let mut v = view();
        v.set_preview("a.txt", Ok(b"line one\n\tline two\x07\n".to_vec()));
        assert_eq!(v.preview, vec!["line one", " line two?"]);
        v.set_preview("x", Ok(vec![0x7F, 0x45, 0x4C, 0x46, 0]));
        assert_eq!(v.preview, vec!["(binary file)"]);
        v.set_preview("e", Ok(Vec::new()));
        assert_eq!(v.preview, vec!["(empty file)"]);
        v.set_preview("e", Err(String::from("I/O error")));
        assert_eq!(v.preview, vec!["cannot read: I/O error"]);
    }

    #[test]
    fn render_draws_the_selection() {
        let mut v = view();
        v.key(Nav::Down, H);
        let mut c = Canvas::new(W as usize, H as usize, 0);
        v.render(&mut c);
        let (_, y) = v.row_center(1, W, H).unwrap();
        assert_eq!(c.get(1, y), Some(ACCENT), "selection marker on the left edge");
        assert_eq!(c.get(W - 2, H - 2), Some(HEADER), "footer");
        let mut c2 = Canvas::new(300, 200, 0);
        render_info(&mut c2, 2, 1, &[(String::new(), String::from("Nova")), (String::from("CPUs"), String::from("4"))]);
        assert!(c2.pixels().contains(&INK));
    }
}
