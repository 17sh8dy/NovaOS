//! Calculator: a standard calculator with exact decimal arithmetic (0.1 + 0.2
//! is 0.3, not 0.30000000000000004), immediate execution like a pocket
//! calculator (`2 + 3 * 4 =` is 20), percent, square, square root, 1/x, sign
//! change, backspace, and a history of results that can be clicked to reuse.
//! Works with the mouse and the keyboard (digits, `+ - * / % .`, Enter or `=`,
//! Backspace, Esc = C, Delete = CE).

use crate::ui::{self, KeyIn};
use crate::{ACCENT, DANGER, DIM, INK, PANEL};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use nova_gfx::{Canvas, Rect, fit, mix, text_height, text_width};

// ---- Exact decimals -------------------------------------------------------------------

/// Decimal places kept.
const PLACES: u32 = 9;
const SCALE: i128 = 1_000_000_000;

/// A fixed-point decimal: `raw / 10^9`. Every operation is checked; overflow is an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Dec(i128);

/// Largest magnitude shown and computed (keeps every operation inside i128).
const LIMIT: i128 = 100_000_000_000_000_000 * SCALE; // 1e17

impl Dec {
    pub const ZERO: Dec = Dec(0);

    pub fn from_int(v: i64) -> Dec {
        Dec(v as i128 * SCALE)
    }

    fn checked(raw: Option<i128>) -> Option<Dec> {
        raw.filter(|r| r.abs() < LIMIT).map(Dec)
    }

    /// Parses `-12.5`, `.5`, `3.` (extra places are cut, not rounded).
    pub fn parse(s: &str) -> Option<Dec> {
        let (neg, s) = s.strip_prefix('-').map_or((false, s), |r| (true, r));
        let (int, frac) = s.split_once('.').unwrap_or((s, ""));
        if int.is_empty() && frac.is_empty() || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut raw: i128 = 0;
        for b in int.bytes() {
            raw = raw.checked_mul(10)?.checked_add((b - b'0') as i128)?;
        }
        raw = raw.checked_mul(SCALE)?;
        let mut place = SCALE / 10;
        for b in frac.bytes().take(PLACES as usize) {
            raw += (b - b'0') as i128 * place;
            place /= 10;
        }
        Self::checked(Some(if neg { -raw } else { raw }))
    }

    pub fn plus(self, o: Dec) -> Option<Dec> {
        Self::checked(self.0.checked_add(o.0))
    }

    pub fn minus(self, o: Dec) -> Option<Dec> {
        Self::checked(self.0.checked_sub(o.0))
    }

    /// Rounds half away from zero to 9 places.
    pub fn times(self, o: Dec) -> Option<Dec> {
        let p = self.0.checked_mul(o.0)?;
        Self::checked(Some(div_round(p, SCALE)))
    }

    pub fn over(self, o: Dec) -> Option<Dec> {
        if o.0 == 0 {
            return None;
        }
        let n = self.0.checked_mul(SCALE)?;
        Self::checked(Some(div_round(n, o.0)))
    }

    pub fn sqrt(self) -> Option<Dec> {
        if self.0 < 0 {
            return None;
        }
        // sqrt(raw / S) = sqrt(raw * S) / S
        Self::checked(Some(isqrt(self.0.checked_mul(SCALE)?)))
    }

    pub fn negated(self) -> Dec {
        Dec(-self.0)
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }
}

fn div_round(n: i128, d: i128) -> i128 {
    let q = n / d;
    let r = n % d;
    if 2 * r.abs() >= d.abs() { q + if (n < 0) != (d < 0) { -1 } else { 1 } } else { q }
}

fn isqrt(n: i128) -> i128 {
    if n < 2 {
        return n;
    }
    // Newton's method from a power-of-two guess above the root.
    let mut x: i128 = 1 << (128 - n.leading_zeros()).div_ceil(2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

impl core::fmt::Display for Dec {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let neg = self.0 < 0;
        let a = self.0.unsigned_abs();
        let (int, frac) = (a / SCALE as u128, a % SCALE as u128);
        let mut s = String::new();
        if neg {
            s.push('-');
        }
        // Thousands separators make big numbers readable.
        let digits = int.to_string();
        for (i, ch) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                s.push(',');
            }
            s.push(ch);
        }
        if frac != 0 {
            let f9 = alloc::format!("{:09}", frac);
            s.push('.');
            s.push_str(f9.trim_end_matches('0'));
        }
        f.write_str(&s)
    }
}

// ---- The calculator ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

impl Op {
    fn sign(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
        }
    }

    fn apply(self, a: Dec, b: Dec) -> Result<Dec, &'static str> {
        let r = match self {
            Op::Add => a.plus(b),
            Op::Sub => a.minus(b),
            Op::Mul => a.times(b),
            Op::Div if b.is_zero() => return Err("Cannot divide by zero"),
            Op::Div => a.over(b),
        };
        r.ok_or("Too big")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalcKey {
    Digit(u8),
    Dot,
    Op(Op),
    Equals,
    /// C: start over.
    Clear,
    /// CE: clear the number being typed.
    ClearEntry,
    Backspace,
    Negate,
    Percent,
    Square,
    Sqrt,
    Reciprocal,
}

/// The keypad, row by row: label and key.
const PAD: [[(&str, CalcKey); 4]; 6] = [
    [("%", CalcKey::Percent), ("CE", CalcKey::ClearEntry), ("C", CalcKey::Clear), ("<-", CalcKey::Backspace)],
    [("1/x", CalcKey::Reciprocal), ("x^2", CalcKey::Square), ("sqrt", CalcKey::Sqrt), ("/", CalcKey::Op(Op::Div))],
    [("7", CalcKey::Digit(7)), ("8", CalcKey::Digit(8)), ("9", CalcKey::Digit(9)), ("*", CalcKey::Op(Op::Mul))],
    [("4", CalcKey::Digit(4)), ("5", CalcKey::Digit(5)), ("6", CalcKey::Digit(6)), ("-", CalcKey::Op(Op::Sub))],
    [("1", CalcKey::Digit(1)), ("2", CalcKey::Digit(2)), ("3", CalcKey::Digit(3)), ("+", CalcKey::Op(Op::Add))],
    [("+/-", CalcKey::Negate), ("0", CalcKey::Digit(0)), (".", CalcKey::Dot), ("=", CalcKey::Equals)],
];

const MAX_DIGITS: usize = 16;
const HISTORY: usize = 12;

pub struct Calculator {
    /// The number shown when not typing.
    cur: Dec,
    /// What is being typed (shown as typed).
    typing: Option<String>,
    /// The left operand and operator waiting for the right one.
    pending: Option<(Dec, Op)>,
    /// The line above the number ("12 + 7 =").
    expr: String,
    error: Option<&'static str>,
    history: Vec<(String, Dec)>,
    scale: i32,
    u: i32,
}

impl Calculator {
    pub fn new(scale: i32, u: i32) -> Self {
        Self { cur: Dec::ZERO, typing: None, pending: None, expr: String::new(), error: None, history: Vec::new(), scale, u }
    }

    /// What the big display shows.
    pub fn display(&self) -> String {
        if let Some(e) = self.error {
            return String::from(e);
        }
        match &self.typing {
            Some(t) => {
                // Group the integer part like results are grouped.
                let (neg, body) = t.strip_prefix('-').map_or((false, t.as_str()), |b| (true, b));
                let (int, frac) = body.split_once('.').map_or((body, None), |(i, f)| (i, Some(f)));
                let int_val = Dec::parse(if int.is_empty() { "0" } else { int }).unwrap_or_default();
                let mut s = int_val.to_string();
                if neg {
                    s.insert(0, '-');
                }
                if let Some(f) = frac {
                    s.push('.');
                    s.push_str(f);
                }
                s
            }
            None => self.cur.to_string(),
        }
    }

    pub fn expression(&self) -> &str {
        &self.expr
    }

    pub fn history(&self) -> &[(String, Dec)] {
        &self.history
    }

    /// The value on display.
    pub fn value(&self) -> Dec {
        self.typing.as_deref().and_then(Dec::parse).unwrap_or(self.cur)
    }

    fn set_result(&mut self, r: Result<Dec, &'static str>) {
        self.typing = None;
        match r {
            Ok(v) => self.cur = v,
            Err(e) => {
                self.error = Some(e);
                self.pending = None;
                self.cur = Dec::ZERO;
            }
        }
    }

    /// The keyboard: digits, operators, Enter/=, Backspace, Esc, Delete, `%`, `.`/`,`.
    pub fn key_for(k: KeyIn) -> Option<CalcKey> {
        Some(match k {
            KeyIn::Char(c @ '0'..='9') => CalcKey::Digit(c as u8 - b'0'),
            KeyIn::Char('.' | ',') => CalcKey::Dot,
            KeyIn::Char('+') => CalcKey::Op(Op::Add),
            KeyIn::Char('-') => CalcKey::Op(Op::Sub),
            KeyIn::Char('*' | 'x' | 'X') => CalcKey::Op(Op::Mul),
            KeyIn::Char('/') => CalcKey::Op(Op::Div),
            KeyIn::Char('%') => CalcKey::Percent,
            KeyIn::Char('=') | KeyIn::Enter => CalcKey::Equals,
            KeyIn::Char('r') => CalcKey::Reciprocal,
            KeyIn::Char('q') => CalcKey::Square,
            KeyIn::Char('s') => CalcKey::Sqrt,
            KeyIn::Backspace => CalcKey::Backspace,
            KeyIn::Escape => CalcKey::Clear,
            KeyIn::Delete => CalcKey::ClearEntry,
            _ => return None,
        })
    }

    pub fn press(&mut self, k: CalcKey) {
        if self.error.is_some() {
            self.error = None;
            self.expr.clear();
            if !matches!(k, CalcKey::Digit(_) | CalcKey::Dot) {
                return;
            }
        }
        match k {
            CalcKey::Digit(d) => {
                let t = self.typing.get_or_insert_with(String::new);
                let digits = t.chars().filter(|c| c.is_ascii_digit()).count();
                if digits >= MAX_DIGITS {
                    return;
                }
                if t == "0" || t == "-0" {
                    t.pop();
                }
                t.push((b'0' + d) as char);
                if self.pending.is_none() && self.expr.ends_with('=') {
                    self.expr.clear();
                }
            }
            CalcKey::Dot => {
                let t = self.typing.get_or_insert_with(|| String::from("0"));
                if !t.contains('.') {
                    if t.is_empty() || t == "-" {
                        t.push('0');
                    }
                    t.push('.');
                }
            }
            CalcKey::Backspace => {
                if let Some(t) = &mut self.typing {
                    t.pop();
                    if t.is_empty() || t == "-" {
                        self.typing = None;
                        self.cur = Dec::ZERO;
                    }
                }
            }
            CalcKey::ClearEntry => {
                self.typing = None;
                self.cur = Dec::ZERO;
            }
            CalcKey::Clear => {
                self.typing = None;
                self.cur = Dec::ZERO;
                self.pending = None;
                self.expr.clear();
            }
            CalcKey::Negate => match &mut self.typing {
                Some(t) => {
                    if t.starts_with('-') {
                        t.remove(0);
                    } else {
                        t.insert(0, '-');
                    }
                }
                None => self.cur = self.cur.negated(),
            },
            CalcKey::Op(op) => {
                let v = self.value();
                let left = match (self.pending, self.typing.is_some()) {
                    // A second operator in a row just replaces the first.
                    (Some((a, _)), false) => Ok(a),
                    (Some((a, p)), true) => p.apply(a, v),
                    (None, _) => Ok(v),
                };
                match left {
                    Ok(a) => {
                        self.set_result(Ok(a));
                        self.pending = Some((a, op));
                        self.expr = alloc::format!("{a} {}", op.sign());
                    }
                    Err(e) => self.set_result(Err(e)),
                }
            }
            CalcKey::Equals => {
                let Some((a, op)) = self.pending else { return };
                let b = self.value();
                let r = op.apply(a, b);
                self.expr = alloc::format!("{a} {} {b} =", op.sign());
                self.pending = None;
                if let Ok(v) = r {
                    self.history.insert(0, (alloc::format!("{a} {} {b} =", op.sign()), v));
                    self.history.truncate(HISTORY);
                }
                self.set_result(r);
            }
            CalcKey::Percent => {
                let v = self.value();
                let hundred = Dec::from_int(100);
                // a + b% means b percent *of a*; a * b% means b / 100.
                let r = match self.pending {
                    Some((a, Op::Add | Op::Sub)) => a.times(v).and_then(|x| x.over(hundred)),
                    _ => v.over(hundred),
                };
                self.set_result(r.ok_or("Too big"));
            }
            CalcKey::Square => {
                let v = self.value();
                self.expr = alloc::format!("sqr({v})");
                self.set_result(v.times(v).ok_or("Too big"));
            }
            CalcKey::Sqrt => {
                let v = self.value();
                self.expr = alloc::format!("sqrt({v})");
                self.set_result(if v < Dec::ZERO { Err("Invalid input") } else { v.sqrt().ok_or("Too big") });
            }
            CalcKey::Reciprocal => {
                let v = self.value();
                self.expr = alloc::format!("1/({v})");
                self.set_result(if v.is_zero() { Err("Cannot divide by zero") } else { Dec::from_int(1).over(v).ok_or("Too big") });
            }
        }
    }

    // ---- Layout and drawing ----

    fn history_w(&self, w: i32) -> i32 {
        if w >= 520 * self.u { (w * 38 / 100).min(320 * self.u) } else { 0 }
    }

    fn display_rect(&self, w: i32) -> Rect {
        let u = self.u;
        Rect::new(10 * u, 10 * u, w - self.history_w(w) - 20 * u, 18 * u + text_height(self.scale) + text_height(self.big()))
    }

    fn big(&self) -> i32 {
        2 * self.scale
    }

    /// Screen rectangle of keypad button (row, col) in a `w` x `h` window.
    fn key_rect(&self, row: usize, col: usize, w: i32, h: i32) -> Rect {
        let u = self.u;
        let d = self.display_rect(w);
        let top = d.bottom() + 8 * u;
        let (gap, left, width) = (4 * u, d.x, d.w);
        let bw = (width - 3 * gap) / 4;
        let bh = ((h - top - 10 * u - 5 * gap) / 6).max(10 * u);
        Rect::new(left + col as i32 * (bw + gap), top + row as i32 * (bh + gap), bw, bh)
    }

    /// The centre of the button with this label (for tests and logs).
    pub fn button_center(&self, label: &str, w: i32, h: i32) -> Option<(i32, i32)> {
        for (r, row) in PAD.iter().enumerate() {
            for (c, (l, _)) in row.iter().enumerate() {
                if *l == label {
                    let k = self.key_rect(r, c, w, h);
                    return Some((k.x + k.w / 2, k.y + k.h / 2));
                }
            }
        }
        None
    }

    fn history_row(&self, i: usize, w: i32) -> Rect {
        let u = self.u;
        let hw = self.history_w(w);
        let lh = 2 * text_height(self.scale) + 12 * u;
        Rect::new(w - hw, 30 * u + i as i32 * lh, hw - 10 * u, lh)
    }

    /// A click at (x, y) in a `w` x `h` window. Returns true if anything changed.
    pub fn click(&mut self, x: i32, y: i32, w: i32, h: i32) -> bool {
        for (r, row) in PAD.iter().enumerate() {
            for (c, (_, k)) in row.iter().enumerate() {
                if self.key_rect(r, c, w, h).contains(x, y) {
                    self.press(*k);
                    return true;
                }
            }
        }
        if self.history_w(w) > 0 {
            for i in 0..self.history.len() {
                if self.history_row(i, w).contains(x, y) {
                    self.error = None;
                    self.typing = None;
                    self.cur = self.history[i].1;
                    return true;
                }
            }
        }
        false
    }

    pub fn render(&self, c: &mut Canvas) {
        let (w, h) = (c.width() as i32, c.height() as i32);
        let (u, s) = (self.u, self.scale);
        let all = c.bounds();
        c.fill(all, PANEL);

        let d = self.display_rect(w);
        c.fill(d, ui::FACE_SOFT);
        c.text(d.right() - text_width(&fit(&self.expr, s, d.w - 16 * u), s) - 8 * u, d.y + 8 * u, s, DIM, &fit(&self.expr, s, d.w - 16 * u), d);
        let shown = self.display();
        let color = if self.error.is_some() { DANGER } else { INK };
        let mut big = self.big();
        while big > s && text_width(&shown, big) > d.w - 16 * u {
            big -= u;
        }
        let shown = fit(&shown, big, d.w - 16 * u);
        c.text(d.right() - text_width(&shown, big) - 8 * u, d.bottom() - text_height(big) - 6 * u, big, color, &shown, d);

        for (r, row) in PAD.iter().enumerate() {
            for (col, (label, k)) in row.iter().enumerate() {
                let rect = self.key_rect(r, col, w, h);
                match k {
                    CalcKey::Equals => ui::button(c, rect, label, s, u, ui::FACE_STRONG, INK),
                    CalcKey::Digit(_) | CalcKey::Dot | CalcKey::Negate => ui::button(c, rect, label, s, u, mix(ui::FACE, INK, 14), INK),
                    CalcKey::Op(_) => ui::button(c, rect, label, s, u, ui::FACE, ACCENT),
                    _ => ui::button_on(c, rect, label, s, u),
                }
            }
        }

        let hw = self.history_w(w);
        if hw > 0 {
            let panel = Rect::new(w - hw - 4 * u, 0, hw + 4 * u, h);
            c.fill(panel, mix(PANEL, 0, 60));
            c.text(panel.x + 10 * u, 10 * u, s, ACCENT, "History", panel);
            if self.history.is_empty() {
                c.text(panel.x + 10 * u, 34 * u, s, DIM, &fit("No results yet", s, hw - 20 * u), panel);
            }
            for (i, (e, v)) in self.history.iter().enumerate() {
                let r = self.history_row(i, w);
                if r.bottom() > h {
                    break;
                }
                let e = fit(e, s, r.w - 10 * u);
                let v = fit(&v.to_string(), s, r.w - 10 * u);
                c.text(r.right() - text_width(&e, s), r.y + 4 * u, s, DIM, &e, r);
                c.text(r.right() - text_width(&v, s), r.y + 8 * u + text_height(s), s, INK, &v, r);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }

    fn run(keys: &str) -> Calculator {
        let mut c = Calculator::new(2, 1);
        for ch in keys.chars() {
            let k = match ch {
                'C' => CalcKey::Clear,
                'E' => CalcKey::ClearEntry,
                'B' => CalcKey::Backspace,
                'N' => CalcKey::Negate,
                'S' => CalcKey::Sqrt,
                'Q' => CalcKey::Square,
                'R' => CalcKey::Reciprocal,
                _ => Calculator::key_for(KeyIn::Char(ch)).unwrap(),
            };
            c.press(k);
        }
        c
    }

    #[test]
    fn decimals_are_exact() {
        assert_eq!(d("0.1").plus(d("0.2")).unwrap(), d("0.3"));
        assert_eq!(d("1").over(d("3")).unwrap().to_string(), "0.333333333");
        assert_eq!(d("2").over(d("3")).unwrap().to_string(), "0.666666667", "rounded");
        assert_eq!(d("-7.25").times(d("4")).unwrap().to_string(), "-29");
        assert_eq!(d("2").sqrt().unwrap().to_string(), "1.414213562");
        assert_eq!(d("144").sqrt().unwrap(), d("12"));
        assert_eq!(d("1234567.5").to_string(), "1,234,567.5");
        assert_eq!(d(".5"), d("0.5"));
        assert!(Dec::parse("1.2.3").is_none() && Dec::parse("").is_none() && Dec::parse("x").is_none());
        assert!(d("99999999999999999").times(d("10")).is_none(), "overflow is caught");
        assert!(d("1").over(Dec::ZERO).is_none());
    }

    #[test]
    fn immediate_execution_and_equals() {
        assert_eq!(run("2+3*4=").display(), "20");
        assert_eq!(run("0.1+0.2=").display(), "0.3");
        assert_eq!(run("10/4=").display(), "2.5");
        let c = run("12+7=");
        assert_eq!(c.expression(), "12 + 7 =");
        assert_eq!(c.history()[0], (String::from("12 + 7 ="), d("19")));
        // A new number after = starts over; an operator continues from the result.
        assert_eq!(run("12+7=5").display(), "5");
        assert_eq!(run("12+7=*2=").display(), "38");
        // A second operator replaces the first.
        assert_eq!(run("8+-3=").display(), "5");
    }

    #[test]
    fn editing_keys() {
        assert_eq!(run("123B").display(), "12");
        assert_eq!(run("1B").display(), "0");
        assert_eq!(run("5+3E2=").display(), "7");
        assert_eq!(run("5+3C").display(), "0");
        assert_eq!(run("5N").display(), "-5");
        assert_eq!(run("5N+2=").display(), "-3");
        assert_eq!(run("..5").display(), "0.5");
        assert_eq!(run("1234567").display(), "1,234,567");
        assert_eq!(run("00012").display(), "12");
        assert_eq!(run("12345678901234567890").display(), "1,234,567,890,123,456", "16 digits at most");
    }

    #[test]
    fn functions_and_errors() {
        assert_eq!(run("200+10%=").display(), "220", "a + b% is b percent of a");
        assert_eq!(run("50*10%=").display(), "5");
        assert_eq!(run("9S").display(), "3");
        assert_eq!(run("12Q").display(), "144");
        assert_eq!(run("4R").display(), "0.25");
        let c = run("5/0=");
        assert_eq!(c.display(), "Cannot divide by zero");
        assert!(c.history().is_empty());
        assert_eq!(run("5/0=3+1=").display(), "4", "a digit after an error starts over");
        assert_eq!(run("9NS").display(), "Invalid input");
        assert_eq!(run("0R").display(), "Cannot divide by zero");
    }

    #[test]
    fn mouse_and_layout() {
        let (w, h) = (300, 400);
        let mut c = Calculator::new(2, 1);
        for label in ["7", "+", "8", "="] {
            let (x, y) = c.button_center(label, w, h).unwrap();
            assert!(c.click(x, y, w, h), "{label}");
        }
        assert_eq!(c.display(), "15");
        // Buttons do not overlap and stay inside the window.
        let mut seen: Vec<Rect> = Vec::new();
        for r in 0..6 {
            for col in 0..4 {
                let k = c.key_rect(r, col, w, h);
                assert!(k.x >= 0 && k.y >= 0 && k.right() <= w && k.bottom() <= h, "{k:?}");
                assert!(seen.iter().all(|o| !o.intersects(&k)));
                seen.push(k);
            }
        }
        // Wide windows get a history; clicking an entry reuses its result.
        let (w, h) = (700, 400);
        let mut c = run("6*7=");
        c.press(CalcKey::Clear);
        let r = c.history_row(0, w);
        assert!(c.click(r.x + 5, r.y + 5, w, h));
        assert_eq!(c.display(), "42");
        let mut canvas = Canvas::new(w as usize, h as usize, 0);
        c.render(&mut canvas);
        let mut small = Canvas::new(200, 260, 0);
        c.render(&mut small);
    }
}
