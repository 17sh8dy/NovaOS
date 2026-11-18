//! The desktop's main loop and its apps.
//!
//! The boot thread ends up here: it follows the mouse (moving the pointer,
//! clicking and dragging windows), hands keys typed into the Files window to
//! it, keeps the clock and the System window up to date, and sleeps (`hlt`)
//! between interrupts. Anything that touches the disk (listing a folder,
//! previewing a file) happens here, *outside* the GUI lock, and the result is
//! handed to the view afterwards.
//!
//! Everything the tests need to find on screen is logged: where windows open,
//! their title bars and buttons, menu entries, and what each click did.

use crate::files::VFS;
use crate::gui::{self, FILES, LOCK, PASSWORD, RESTART, SYSTEM, TERMINAL};
use crate::{auth, cpu, files, heap, interrupts, kprintln, pmm, process, ps2, rtc, sched, xhci};
use alloc::string::String;
use alloc::vec::Vec;
use nova_desktop::Event;
use nova_desktop::lock::{LockKey, Mode, Outcome};
use nova_desktop::apps::{Entry, Nav, PREVIEW_BYTES, Request, join};
use nova_fs::NodeKind;
use nova_gfx::Rect;
use nova_kbd::{Action, Input, Key};
use nova_ps2::Buttons;

fn name(id: u32) -> &'static str {
    match id {
        TERMINAL => "Terminal",
        FILES => "Files",
        SYSTEM => "System",
        LOCK => "Lock",
        PASSWORD => "Password",
        RESTART => "Restart",
        _ => "?",
    }
}

fn center(r: Rect) -> (i32, i32) {
    (r.x + r.w / 2, r.y + r.h / 2)
}

/// Logs where a window's parts are (for the tests, and for anyone reading the log).
fn log_geometry(id: u32, what: &str) {
    let geo = gui::with(|g| {
        let (title, close, min) = g.desk.chrome(id)?;
        let task = g.desk.task_buttons().into_iter().find(|(i, _)| *i == id).map(|(_, r)| center(r)).unwrap_or((0, 0));
        let r = g.desk.window(id)?.rect;
        Some((r, center(title), center(close), center(min), task))
    })
    .flatten();
    if let Some((r, t, c, m, task)) = geo {
        kprintln!(
            "wm: {} {what} at ({}, {}) {}x{}: title ({}, {}), close ({}, {}), minimize ({}, {}), taskbar ({}, {})",
            name(id),
            r.x,
            r.y,
            r.w,
            r.h,
            t.0,
            t.1,
            c.0,
            c.1,
            m.0,
            m.1,
            task.0,
            task.1
        );
    }
}

/// Reads a folder for the Files window.
fn list(path: &str) -> Result<Vec<Entry>, String> {
    let mut vfs = VFS.lock();
    let entries = vfs.readdir(path).map_err(|e| alloc::format!("{e}"))?;
    Ok(entries
        .into_iter()
        .map(|e| {
            let dir = e.kind == NodeKind::Dir;
            let size = if dir { 0 } else { vfs.stat(&join(path, &e.name)).map_or(0, |s| s.size) };
            Entry { name: e.name, dir, size }
        })
        .collect())
}

/// Reads the start of a file for the preview.
fn head(path: &str) -> Result<Vec<u8>, String> {
    let mut vfs = VFS.lock();
    let h = vfs.resolve(path).map_err(|e| alloc::format!("{e}"))?;
    let mut buf = alloc::vec![0u8; PREVIEW_BYTES];
    let n = vfs.read_at(&h, 0, &mut buf).map_err(|e| alloc::format!("{e}"))?;
    buf.truncate(n);
    Ok(buf)
}

/// Does what the Files view asked for and shows the answer.
fn fulfil(req: Option<Request>) {
    match req {
        Some(Request::List(path)) => {
            let listing = list(&path);
            match &listing {
                Ok(l) => kprintln!("files: {path} ({} entries)", l.len()),
                Err(e) => kprintln!("files: {path}: {e}"),
            }
            gui::with(|g| {
                g.files.set_listing(&path, listing);
                g.render_files();
            });
        }
        Some(Request::Preview(path)) => {
            let data = head(&path);
            if let Ok(d) = &data {
                kprintln!("files: preview {path} ({} bytes)", d.len());
            }
            let short = String::from(path.rsplit('/').next().unwrap_or(&path));
            gui::with(|g| {
                g.files.set_preview(&short, data);
                g.render_files();
            });
        }
        None => {
            gui::with(|g| g.render_files());
        }
    }
}

fn format_clock(t: &nova_x86::rtc::DateTime) -> String {
    alloc::format!("{:02}:{:02}", t.hour, t.minute)
}

/// "Tuesday 17 November 2026" (for the lock screen).
fn format_date(t: &nova_x86::rtc::DateTime) -> String {
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let (m, d) = (t.month.clamp(1, 12) as usize, t.day as usize);
    // Sakamoto's weekday formula (0 = Sunday).
    const T: [usize; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = t.year as usize - usize::from(m < 3);
    let wd = (y + y / 4 - y / 100 + y / 400 + T[m - 1] + d) % 7;
    alloc::format!("{} {} {} {}", DAYS[wd], d, MONTHS[m - 1], t.year)
}

// ---- Login and lock ----

/// Locks the screen. With no password set yet, asks for one instead.
pub fn lock_now() {
    if auth::is_set() {
        gui::with(|g| g.desk.show_lock(Mode::Unlock));
        kprintln!("login: screen locked");
    } else {
        gui::with(|g| g.desk.show_lock(Mode::Create));
        kprintln!("login: no password yet; the card asks for a new one");
    }
}

/// Opens the card that sets (or changes) the password.
pub fn password_card() {
    let mode = if auth::is_set() { Mode::Change } else { Mode::Create };
    gui::with(|g| g.desk.show_lock(mode));
    kprintln!("login: password card open ({})", if mode == Mode::Change { "change" } else { "create" });
}

/// Acts on what the lock screen asked for. Hashing happens outside the GUI lock.
fn lock_outcome(out: Outcome) {
    match out {
        Outcome::None => {}
        Outcome::Check(guess) => {
            let ok = auth::verify(&guess);
            drop(guess);
            let (next, failures, mode) = gui::with(|g| {
                let next = g.desk.lock_checked(ok);
                let f = g.desk.lock_state().map_or(0, |l| l.failures());
                (next, f, g.desk.lock_shown())
            })
            .unwrap_or((Outcome::None, 0, None));
            match (ok, &next) {
                (true, Outcome::Close) => kprintln!("login: unlocked"),
                (true, _) => kprintln!("login: current password accepted"),
                (false, _) => kprintln!("login: wrong password ({failures} in a row)"),
            }
            if mode.is_none() {
                gui::with(|g| g.refresh_cursor());
            }
        }
        Outcome::Set(new) => {
            let result = auth::set(&new);
            drop(new);
            gui::with(|g| g.desk.lock_saved(result.as_deref().map_err(|e| e.as_str())));
        }
        Outcome::Close => {
            kprintln!("login: card closed");
            gui::with(|g| g.refresh_cursor());
        }
    }
}

/// A key typed while the lock screen or the password card is up.
fn lock_input(input: Input) {
    let k = match input.action {
        Action::Char(c) if !input.ctrl && !input.alt => LockKey::Char(c),
        Action::Key(Key::Enter) => LockKey::Enter,
        Action::Key(Key::Backspace) => LockKey::Backspace,
        Action::Key(Key::Escape) => LockKey::Escape,
        _ => return,
    };
    let out = gui::with(|g| g.desk.lock_key(k)).unwrap_or(Outcome::None);
    lock_outcome(out);
}

fn restart() -> ! {
    kprintln!("power: restarting");
    files::shutdown();
    ps2::reset_cpu();
    crate::safety::reset_machine()
}

/// The System window's lines (gathered outside the GUI lock).
fn system_lines() -> Vec<(String, String)> {
    let mut l: Vec<(String, String)> = Vec::new();
    let mut add = |a: &str, b: String| l.push((String::from(a), b));
    add("", String::from("Nova OS - Full Kernel"));
    add("Version", String::from(env!("CARGO_PKG_VERSION")));
    let secs = interrupts::ticks() / interrupts::TICK_HZ as u64;
    add("Uptime", alloc::format!("{}h {:02}m {:02}s", secs / 3600, secs / 60 % 60, secs % 60));
    if let Some(t) = rtc::now() {
        add("Clock", alloc::format!("{}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second));
    }
    add("CPUs", alloc::format!("{} online", cpu::online_count()));
    if let Some(s) = pmm::stats() {
        add("Memory", alloc::format!("{} MiB free of {} MiB", s.free_frames * 4 / 1024, s.tracked_frames * 4 / 1024));
    }
    let (heap_total, heap_free) = heap::stats();
    add("Kernel heap", alloc::format!("{} KiB, {} KiB free", heap_total / 1024, heap_free / 1024));
    let st = sched::stats();
    add("Threads", alloc::format!("{} ({} context switches)", st.threads, st.switches));
    add("Processes", alloc::format!("{} running", process::list().len()));
    for (i, model, bytes, bus) in files::disks() {
        let root = if files::root_disk() == Some(i) { ", holds /" } else { "" };
        add(&alloc::format!("Disk {i}"), alloc::format!("{model}, {} MiB, {bus}{root}", bytes >> 20));
    }
    for line in xhci::describe().into_iter().filter(|s| s.starts_with("  port")) {
        add("USB", String::from(line.trim()));
    }
    let boot = gui::with(|g| g.boot_lines.clone()).unwrap_or_default();
    l.extend(boot);
    l
}

fn refresh_system() {
    if gui::with(|g| g.desk.state(SYSTEM) == Some(nova_desktop::State::Open)) != Some(true) {
        return;
    }
    let lines = system_lines();
    gui::with(|g| g.render_system(&lines));
}

fn open_app(id: u32) {
    gui::with(|g| g.show(id));
    log_geometry(id, "opened");
    match id {
        FILES => {
            let path = gui::with(|g| g.files.path.clone()).unwrap_or_else(|| String::from("/"));
            fulfil(Some(Request::List(path)));
        }
        SYSTEM => {
            refresh_system();
            kprintln!("system: window shows {} lines", system_lines().len());
        }
        _ => {}
    }
}

fn handle(e: Event) {
    match e {
        Event::Focused(id) => kprintln!("wm: focus {}", name(id)),
        Event::MenuOpened => {
            let items = gui::with(|g| {
                let labels: Vec<String> = g.desk.menu().iter().map(|(l, _)| l.clone()).collect();
                labels.into_iter().enumerate().map(|(i, l)| (l, center(g.desk.menu_item(i)))).collect::<Vec<_>>()
            })
            .unwrap_or_default();
            let s: Vec<String> = items.iter().map(|(n, (x, y))| alloc::format!("{n} at ({x}, {y})")).collect();
            kprintln!("desktop: menu opened: {}", s.join(", "));
        }
        Event::MenuClosed => kprintln!("desktop: menu closed"),
        Event::MenuChose(LOCK) => lock_now(),
        Event::MenuChose(PASSWORD) => password_card(),
        Event::MenuChose(RESTART) => restart(),
        Event::MenuChose(id) => open_app(id),
        Event::Closed(id) | Event::Minimized(id) => {
            let what = if matches!(e, Event::Closed(_)) { "closed" } else { "minimized" };
            let next = gui::with(|g| g.desk.focused()).flatten();
            kprintln!("wm: {} {what}; focus {}", name(id), next.map_or("none", name));
        }
        Event::Restored(id) => kprintln!("wm: {} restored", name(id)),
        Event::DragStarted(_) => {}
        Event::Moved(id, ..) => log_geometry(id, "moved"),
        Event::Click(FILES, x, y) => {
            let req = gui::with(|g| {
                let (w, h) = g.content_size(FILES);
                g.files.click(x, y, w, h)
            })
            .flatten();
            fulfil(req);
        }
        Event::Click(..) => {}
    }
}

/// Keys typed while a window other than the Terminal has focus.
fn ui_key(input: Input) {
    // Desktop shortcuts, whatever has the focus.
    if input.ctrl && input.alt && matches!(input.action, Action::Char('l' | 'L')) {
        if gui::with(|g| g.desk.lock_shown()).flatten().is_none() {
            lock_now();
        }
        return;
    }
    if gui::with(|g| g.desk.lock_shown()).flatten().is_some() {
        lock_input(input);
        return;
    }
    if input.alt && matches!(input.action, Action::Key(Key::Tab)) {
        if let Some(e) = gui::with(|g| {
            let e = g.desk.cycle();
            g.refresh_cursor();
            e
        })
        .flatten()
        {
            let id = match e {
                Event::Focused(id) | Event::Restored(id) => id,
                _ => 0,
            };
            kprintln!("wm: alt+tab to {}", name(id));
        }
        return;
    }
    let focused = gui::with(|g| g.desk.focused()).flatten();
    if focused != Some(FILES) {
        return;
    }
    let nav = match input.action {
        Action::Key(Key::Up) => Nav::Up,
        Action::Key(Key::Down) => Nav::Down,
        Action::Key(Key::Home) => Nav::Home,
        Action::Key(Key::End) => Nav::End,
        Action::Key(Key::PageUp) => Nav::PageUp,
        Action::Key(Key::PageDown) => Nav::PageDown,
        Action::Key(Key::Enter) => Nav::Open,
        Action::Key(Key::Backspace) => Nav::Back,
        _ => return,
    };
    let req = gui::with(|g| {
        let (_, h) = g.content_size(FILES);
        g.files.key(nav, h)
    })
    .flatten();
    fulfil(req);
}

/// The scroll wheel over the Files list moves the selection.
fn wheel(x: i32, y: i32, amount: i8) {
    let over_files = gui::with(|g| matches!(g.desk.hit(x, y), nova_desktop::Hit::Content(FILES, ..))) == Some(true);
    if !over_files {
        return;
    }
    let nav = if amount > 0 { Nav::Down } else { Nav::Up };
    for _ in 0..amount.unsigned_abs().min(10) {
        let req = gui::with(|g| {
            let (_, h) = g.content_size(FILES);
            g.files.key(nav, h)
        })
        .flatten();
        fulfil(req);
    }
}

/// The boot thread's last job: run the desktop. With interrupts the CPU
/// sleeps (`hlt`) between events; without them it polls.
pub fn run(irq_mode: bool) -> ! {
    let mut moved_once = false;
    let mut last_secs = u64::MAX;
    let mut last_blink = 0;
    let mut wakeups: u64 = 0;
    let mut idle_reported = false;
    let mut clock_logged = false;

    log_geometry(TERMINAL, "open");
    if auth::is_set() {
        gui::with(|g| g.desk.show_lock(Mode::Unlock));
        kprintln!("login: locked at boot; the password opens the desktop");
    }
    if let Some(l) = gui::with(|g| center(g.desk.launcher())) {
        kprintln!("desktop: ready; launcher at ({}, {})", l.0, l.1);
    }

    crate::safety::heartbeat();
    crate::faults::desktop_hang_if_requested();
    loop {
        crate::safety::heartbeat();
        if !irq_mode {
            ps2::pump();
        }
        while let Some(ev) = ps2::pop() {
            let Some((change, x, y, events)) = gui::mouse(ev) else { continue };
            if change.is_empty() {
                continue;
            }
            if change.moved && !moved_once {
                moved_once = true;
                kprintln!("mouse: first movement, pointer at ({x}, {y})");
            }
            for (bit, label) in [(Buttons::LEFT, "left"), (Buttons::RIGHT, "right"), (Buttons::MIDDLE, "middle")] {
                if change.pressed.0 & bit != 0 {
                    kprintln!("mouse: {label} button down at ({x}, {y})");
                }
                if change.released.0 & bit != 0 {
                    kprintln!("mouse: {label} button up at ({x}, {y})");
                }
            }
            if change.wheel != 0 {
                kprintln!("mouse: wheel {}", change.wheel);
                wheel(x, y, change.wheel);
            }
            for e in events {
                handle(e);
            }
        }
        while let Some(k) = ps2::pop_ui_key() {
            ui_key(k);
        }

        let ticks = interrupts::ticks();
        let secs = ticks / interrupts::TICK_HZ as u64;
        if secs != last_secs {
            last_secs = secs;
            match rtc::now() {
                Some(t) => {
                    if !clock_logged {
                        clock_logged = true;
                        kprintln!("clock: {}-{:02}-{:02} {:02}:{:02}:{:02} (CMOS RTC)", t.year, t.month, t.day, t.hour, t.minute, t.second);
                    }
                    gui::set_clock(&format_clock(&t));
                    let date = format_date(&t);
                    gui::with(|g| g.desk.set_date(&date));
                }
                None => gui::set_clock(&alloc::format!("up {secs}s")),
            }
            refresh_system();
            gui::with(|g| g.desk.lock_tick(secs));
        }
        // The text cursor blinks twice a second.
        if ticks / (interrupts::TICK_HZ as u64 / 2) != last_blink {
            last_blink = ticks / (interrupts::TICK_HZ as u64 / 2);
            gui::blink();
        }
        // Proof that the CPU really sleeps: a busy loop would have spun
        // millions of times by now, an idle one wakes about once per tick.
        if irq_mode && secs >= 3 && !idle_reported {
            idle_reported = true;
            kprintln!(
                "idle: up {secs} s, {wakeups} wakeups from hlt, {} stray + {} spurious interrupts",
                interrupts::stray_count(),
                interrupts::spurious_count()
            );
        }
        if irq_mode {
            wakeups += 1;
            // Sleep until the next interrupt, unless something arrived since we looked.
            nova_x86::cli();
            if ps2::has_events() || ps2::has_ui_keys() {
                nova_x86::sti();
            } else {
                nova_x86::sti_hlt();
            }
        } else {
            core::hint::spin_loop();
        }
    }
}
