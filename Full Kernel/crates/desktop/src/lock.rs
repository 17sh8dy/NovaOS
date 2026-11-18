//! The lock screen: the password prompt shown at boot (when a password is
//! set), after "Lock" (launcher, `lock` command, Ctrl+Alt+L), and the same
//! card for creating or changing the password.
//!
//! Pure logic like the rest of the desktop: it never sees a stored password.
//! When the user presses Enter it hands back an [`Outcome`] (`Check` this
//! guess, `Set` this new password) and the kernel answers with
//! [`Lock::checked`] / [`Lock::saved`]. Wrong guesses are rate limited: after
//! the third in a row each one costs a growing wait (5 s, 10 s, ... up to 60 s).

use alloc::string::String;
use nova_gfx::{Canvas, Rect, mix, rgb, text_height, text_width};

use crate::{ACCENT, DANGER, DIM, INK, Metrics};

/// What the card is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Locked: the right password opens the desktop. Cannot be cancelled.
    Unlock,
    /// No password yet: choose one (typed twice).
    Create,
    /// Change it: the current one first, then the new one twice.
    Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Current,
    New,
    Confirm,
    /// Saved (or failed to save); any key closes the card.
    Done,
}

/// A key the lock screen understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockKey {
    Char(char),
    Backspace,
    Enter,
    Escape,
}

/// What the kernel has to do after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// Is this the password? Answer with [`Lock::checked`].
    Check(String),
    /// Store this as the new password. Answer with [`Lock::saved`].
    Set(String),
    /// The card is finished (cancelled, or closed after saving): remove it.
    Close,
}

pub const MAX_LEN: usize = 64;
/// Wrong guesses allowed before waiting starts.
const FREE_TRIES: u32 = 3;

pub struct Lock {
    pub mode: Mode,
    step: Step,
    field: String,
    first: String,
    message: String,
    error: bool,
    failures: u32,
    /// No guesses before this second (uptime seconds).
    wait_until: u64,
    now: u64,
}

impl Lock {
    pub fn new(mode: Mode) -> Self {
        let step = match mode {
            Mode::Unlock | Mode::Change => Step::Current,
            Mode::Create => Step::New,
        };
        Self { mode, step, field: String::new(), first: String::new(), message: String::new(), error: false, failures: 0, wait_until: 0, now: 0 }
    }

    /// Characters in the field (what the dots show).
    pub fn typed(&self) -> usize {
        self.field.chars().count()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Seconds left before another guess is allowed.
    pub fn wait_left(&self) -> u64 {
        self.wait_until.saturating_sub(self.now)
    }

    fn say(&mut self, text: &str, error: bool) {
        self.message = String::from(text);
        self.error = error;
    }

    pub fn title(&self) -> &'static str {
        match self.mode {
            Mode::Unlock => "Nova is locked",
            Mode::Create => "Create a password",
            Mode::Change => "Change password",
        }
    }

    pub fn prompt(&self) -> &'static str {
        match (self.mode, self.step) {
            (Mode::Unlock, _) => "Password",
            (_, Step::Current) => "Current password",
            (_, Step::New) => "New password",
            (_, Step::Confirm) => "Type it again",
            (_, Step::Done) => "",
        }
    }

    pub fn hint(&self) -> &'static str {
        match (self.mode, self.step) {
            (_, Step::Done) => "Press any key",
            (Mode::Unlock, _) => "Enter: unlock",
            _ => "Enter: next    Esc: cancel",
        }
    }

    /// Time passed (uptime seconds). Returns true if what the card shows changed.
    pub fn tick(&mut self, now: u64) -> bool {
        let before = self.wait_left();
        self.now = now;
        let left = self.wait_left();
        if left == before {
            return false;
        }
        if left > 0 {
            self.message = alloc::format!("Too many tries. Wait {left} s");
            self.error = true;
        } else {
            self.say("Try again", false);
        }
        true
    }

    pub fn key(&mut self, k: LockKey) -> Outcome {
        if self.step == Step::Done {
            return Outcome::Close;
        }
        match k {
            LockKey::Char(c) => {
                if !c.is_control() && self.field.chars().count() < MAX_LEN {
                    self.field.push(c);
                }
                Outcome::None
            }
            LockKey::Backspace => {
                self.field.pop();
                Outcome::None
            }
            LockKey::Escape => {
                if self.mode != Mode::Unlock {
                    return Outcome::Close;
                }
                self.field.clear();
                Outcome::None
            }
            LockKey::Enter => self.enter(),
        }
    }

    fn enter(&mut self) -> Outcome {
        let typed = core::mem::take(&mut self.field);
        match self.step {
            Step::Current => {
                if self.wait_left() > 0 {
                    let left = self.wait_left();
                    self.message = alloc::format!("Too many tries. Wait {left} s");
                    self.error = true;
                    return Outcome::None;
                }
                Outcome::Check(typed)
            }
            Step::New => {
                if typed.is_empty() {
                    self.say("Type a password first", true);
                    return Outcome::None;
                }
                self.first = typed;
                self.step = Step::Confirm;
                self.say("", false);
                Outcome::None
            }
            Step::Confirm => {
                if typed != self.first {
                    self.first.clear();
                    self.step = Step::New;
                    self.say("They did not match. Start again", true);
                    return Outcome::None;
                }
                self.first.clear();
                Outcome::Set(typed)
            }
            Step::Done => Outcome::Close,
        }
    }

    /// The kernel's answer to [`Outcome::Check`]. Returns `Outcome::Close` when
    /// the screen is unlocked.
    pub fn checked(&mut self, ok: bool) -> Outcome {
        if ok {
            self.failures = 0;
            return match self.mode {
                Mode::Unlock => Outcome::Close,
                _ => {
                    self.step = Step::New;
                    self.say("", false);
                    Outcome::None
                }
            };
        }
        self.failures += 1;
        if self.failures >= FREE_TRIES {
            let wait = (5 * (self.failures - FREE_TRIES + 1) as u64).min(60);
            self.wait_until = self.now + wait;
            self.message = alloc::format!("Wrong password. Wait {wait} s");
        } else {
            self.message = String::from("Wrong password");
        }
        self.error = true;
        Outcome::None
    }

    /// The kernel stored (or failed to store) the new password.
    pub fn saved(&mut self, result: Result<&str, &str>) {
        self.step = Step::Done;
        match result {
            Ok(m) => self.say(m, false),
            Err(m) => self.say(m, true),
        }
    }

    // ---- Drawing ----

    /// The card in the middle of a `screen`.
    pub fn card(m: &Metrics, screen: Rect) -> Rect {
        let u = m.u;
        let (w, h) = (380 * u, 230 * u);
        Rect::new(screen.x + (screen.w - w) / 2, screen.y + (screen.h - h) / 2 + 30 * u, w, h)
    }

    /// The big clock above the card.
    pub fn clock_rect(m: &Metrics, screen: Rect) -> Rect {
        let c = Self::card(m, screen);
        let h = text_height(8 * m.u) + text_height(2 * m.u) + 16 * m.u;
        Rect::new(screen.x, c.y - h - 24 * m.u, screen.w, h)
    }

    /// The password field.
    pub fn field_rect(m: &Metrics, screen: Rect) -> Rect {
        let c = Self::card(m, screen);
        let u = m.u;
        Rect::new(c.x + 30 * u, c.y + 122 * u, c.w - 60 * u, text_height(2 * u) + 20 * u)
    }

    /// Draws the card (and, when locked, the clock) into `back`, clipped to `clip`.
    pub fn draw(&self, back: &mut Canvas, m: &Metrics, screen: Rect, clip: Rect, clock: &str, date: &str) {
        let u = m.u;
        let ts = m.text;
        const CARD: u32 = rgb(0x10, 0x13, 0x2C);
        const EDGE: u32 = rgb(0x4A, 0x6A, 0xB0);
        const WELL: u32 = rgb(0x07, 0x09, 0x1A);

        if self.mode == Mode::Unlock {
            let cr = Self::clock_rect(m, screen);
            if cr.intersects(&clip) {
                let big = 8 * u;
                back.text(cr.x + (cr.w - text_width(clock, big)) / 2, cr.y, big, INK, clock, clip);
                let dy = cr.y + text_height(big) + 14 * u;
                back.text(cr.x + (cr.w - text_width(date, ts)) / 2, dy, ts, DIM, date, clip);
            }
        }

        let c = Self::card(m, screen);
        if !c.intersects(&clip) {
            return;
        }
        back.fill_clipped(c, EDGE, clip);
        back.fill_clipped(c.inset(u), CARD, clip);
        back.fill_clipped(Rect::new(c.x + u, c.y + u, c.w - 2 * u, 3 * u), ACCENT, clip);

        // The emblem: a star in a dark disc-ish square.
        let r = 18 * u;
        let (sx, sy) = (c.x + c.w / 2, c.y + 20 * u + r);
        let mut star = Canvas::new((2 * r + 1) as usize, (2 * r + 1) as usize, CARD);
        star.star(r, r, r, ACCENT);
        back.blit(&star, star.bounds(), sx - r, sy - r, clip.intersect(&c));

        let title = self.title();
        let ty = sy + r + 10 * u;
        back.text(c.x + (c.w - text_width(title, ts)) / 2, ty, ts, INK, title, clip);

        let f = Self::field_rect(m, screen);
        let prompt = self.prompt();
        back.text(f.x, f.y - text_height(ts) - 6 * u, ts, DIM, prompt, clip);
        if self.step != Step::Done {
            let focus = if self.error { DANGER } else { EDGE };
            back.fill_clipped(f, focus, clip);
            back.fill_clipped(f.inset(u), WELL, clip);
            // One dot per character (never the characters), then the caret.
            let d = 6 * u;
            let gap = 4 * u;
            let room = ((f.w - 20 * u) / (d + gap)).max(1) as usize;
            let n = self.typed().min(room);
            let y = f.y + (f.h - d) / 2;
            for i in 0..n {
                back.fill_clipped(Rect::new(f.x + 10 * u + i as i32 * (d + gap), y, d, d), INK, clip.intersect(&f));
            }
            let cx = f.x + 10 * u + n as i32 * (d + gap);
            back.fill_clipped(Rect::new(cx, f.y + 5 * u, 2 * u, f.h - 10 * u), ACCENT, clip.intersect(&f));
        }

        let my = f.bottom() + 10 * u;
        let msg = nova_gfx::fit(&self.message, ts, c.w - 40 * u);
        let color = if self.error { DANGER } else { ACCENT };
        back.text(c.x + (c.w - text_width(&msg, ts)) / 2, my, ts, color, &msg, clip);
        let hint = self.hint();
        back.text(c.x + (c.w - text_width(hint, ts)) / 2, c.bottom() - text_height(ts) - 12 * u, ts, mix(CARD, DIM, 200), hint, clip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_str(l: &mut Lock, s: &str) {
        for c in s.chars() {
            assert_eq!(l.key(LockKey::Char(c)), Outcome::None);
        }
    }

    #[test]
    fn unlock_asks_the_kernel_and_never_cancels() {
        let mut l = Lock::new(Mode::Unlock);
        type_str(&mut l, "hunter2");
        assert_eq!(l.typed(), 7);
        assert_eq!(l.key(LockKey::Backspace), Outcome::None);
        assert_eq!(l.typed(), 6);
        assert_eq!(l.key(LockKey::Escape), Outcome::None); // clears, does not close
        assert_eq!(l.typed(), 0);
        type_str(&mut l, "pw");
        assert_eq!(l.key(LockKey::Enter), Outcome::Check(String::from("pw")));
        assert_eq!(l.typed(), 0, "the field is cleared after Enter");
        assert_eq!(l.checked(false), Outcome::None);
        assert_eq!(l.message(), "Wrong password");
        type_str(&mut l, "pw2");
        assert_eq!(l.key(LockKey::Enter), Outcome::Check(String::from("pw2")));
        assert_eq!(l.checked(true), Outcome::Close);
    }

    #[test]
    fn wrong_guesses_are_rate_limited() {
        let mut l = Lock::new(Mode::Unlock);
        l.tick(100);
        for _ in 0..2 {
            assert!(matches!(l.key(LockKey::Enter), Outcome::Check(_)));
            l.checked(false);
            assert_eq!(l.wait_left(), 0);
        }
        assert!(matches!(l.key(LockKey::Enter), Outcome::Check(_)));
        l.checked(false);
        assert_eq!(l.wait_left(), 5);
        // While waiting, Enter does not even ask.
        type_str(&mut l, "x");
        assert_eq!(l.key(LockKey::Enter), Outcome::None);
        assert!(l.message().contains("Wait 5 s"));
        assert!(l.tick(102));
        assert!(l.message().contains("Wait 3 s"));
        assert!(!l.tick(102));
        assert!(l.tick(105));
        assert_eq!(l.wait_left(), 0);
        assert!(matches!(l.key(LockKey::Enter), Outcome::Check(_)));
        l.checked(false);
        assert_eq!(l.wait_left(), 10, "each further miss waits longer");
        for _ in 0..20 {
            l.tick(l.now + 61);
            l.key(LockKey::Enter);
            l.checked(false);
        }
        assert_eq!(l.wait_left(), 60, "capped");
        l.tick(l.now + 60);
        l.key(LockKey::Enter);
        assert_eq!(l.checked(true), Outcome::Close);
        assert_eq!(l.failures(), 0);
    }

    #[test]
    fn create_needs_two_matching_entries() {
        let mut l = Lock::new(Mode::Create);
        assert_eq!(l.prompt(), "New password");
        assert_eq!(l.key(LockKey::Enter), Outcome::None);
        assert!(l.message().contains("Type a password"));
        type_str(&mut l, "abc");
        assert_eq!(l.key(LockKey::Enter), Outcome::None);
        assert_eq!(l.prompt(), "Type it again");
        type_str(&mut l, "abd");
        assert_eq!(l.key(LockKey::Enter), Outcome::None);
        assert_eq!(l.prompt(), "New password");
        assert!(l.message().contains("did not match"));
        type_str(&mut l, "abc");
        l.key(LockKey::Enter);
        type_str(&mut l, "abc");
        assert_eq!(l.key(LockKey::Enter), Outcome::Set(String::from("abc")));
        l.saved(Ok("Password saved"));
        assert_eq!(l.message(), "Password saved");
        assert_eq!(l.key(LockKey::Char('q')), Outcome::Close);
    }

    #[test]
    fn change_checks_the_current_one_first_and_can_be_cancelled() {
        let mut l = Lock::new(Mode::Change);
        assert_eq!(l.prompt(), "Current password");
        type_str(&mut l, "old");
        assert_eq!(l.key(LockKey::Enter), Outcome::Check(String::from("old")));
        l.checked(false);
        assert_eq!(l.prompt(), "Current password");
        type_str(&mut l, "old");
        l.key(LockKey::Enter);
        l.checked(true);
        assert_eq!(l.prompt(), "New password");
        assert_eq!(l.key(LockKey::Escape), Outcome::Close);
    }

    #[test]
    fn the_field_is_bounded_and_ignores_control_characters() {
        let mut l = Lock::new(Mode::Unlock);
        for _ in 0..MAX_LEN + 10 {
            l.key(LockKey::Char('a'));
        }
        assert_eq!(l.typed(), MAX_LEN);
        let mut l = Lock::new(Mode::Unlock);
        l.key(LockKey::Char('\u{7}'));
        l.key(LockKey::Char('\t'));
        assert_eq!(l.typed(), 0);
    }
}
