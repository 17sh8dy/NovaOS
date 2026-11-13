//! The Nova shell: a kernel thread that reads typed lines from the keyboard,
//! runs built-in commands and prints on the console.
//!
//! It sleeps (`sched::park`) while there is nothing to read; the keyboard
//! interrupt wakes it. Every console line is also mirrored to the serial log
//! as `con: ...`, which is how the automated tests see what the screen shows.

use crate::gui::{self, COLOR_ACCENT, COLOR_DIM, COLOR_ERROR, COLOR_TEXT};
use crate::files::{self, VFS};
use crate::log::SpinLock;
use crate::{cpu, heap, interrupts, pmm, process, programs, ps2, sched};
use alloc::string::String;
use nova_console::{Edit, EditKey, LineEditor};
use nova_fs::NodeKind;
use nova_kbd::{Action, Input, Key};

const PROMPT: &str = "nova> ";

/// The working directory relative paths are resolved against.
static CWD: SpinLock<String> = SpinLock::new(String::new());

/// The shell's current directory (what a new process inherits).
pub fn cwd() -> String {
    let c = CWD.with(|c| c.clone());
    if c.is_empty() { String::from("/") } else { c }
}

fn abs(arg: &str) -> String {
    files::absolute(&cwd(), arg)
}

fn fs_err(what: &str, e: nova_fs::FsError) {
    errln(&alloc::format!("{what}: {e}"));
}

const MAX_CAT: usize = 64 * 1024;

/// Prints text on the console and mirrors it to the serial log.
pub fn out(text: &str, color: u8) {
    gui::console_write(text, color);
    for line in text.lines() {
        crate::kprintln!("con: {line}");
    }
}

/// A status line from the kernel (a process starting, ending, or being killed).
pub fn announce(text: &str, error: bool) {
    out(text, if error { COLOR_ERROR } else { COLOR_DIM });
    gui::console_write("\n", COLOR_TEXT);
}

fn outln(text: &str) {
    out(text, COLOR_TEXT);
    gui::console_write("\n", COLOR_TEXT);
}

fn errln(text: &str) {
    out(text, COLOR_ERROR);
    gui::console_write("\n", COLOR_TEXT);
}

/// Maps typed input onto the line editor's keys. `None` = not for the editor.
fn edit_key(input: Input) -> Option<EditKey> {
    Some(match input.action {
        Action::Char(c) if input.ctrl => match c {
            'c' => EditKey::Cancel,
            'u' => EditKey::KillLine,
            'w' => EditKey::KillWord,
            'a' => EditKey::Home,
            'e' => EditKey::End,
            _ => return None,
        },
        Action::Char(c) => EditKey::Char(c),
        Action::Key(Key::Enter) => EditKey::Enter,
        Action::Key(Key::Backspace) => EditKey::Backspace,
        Action::Key(Key::Delete) => EditKey::Delete,
        Action::Key(Key::Left) => EditKey::Left,
        Action::Key(Key::Right) => EditKey::Right,
        Action::Key(Key::Home) => EditKey::Home,
        Action::Key(Key::End) => EditKey::End,
        Action::Key(Key::Up) => EditKey::Up,
        Action::Key(Key::Down) => EditKey::Down,
        Action::Key(_) => return None,
    })
}

/// Repaints the line being edited: from the prompt's end to the end of the row.
fn redraw_line(start: (usize, usize), text: &str, cursor: usize, max: usize) {
    let text: String = text.chars().take(max).collect();
    gui::with_console(|g| {
        g.set_cursor(start.0, start.1);
        g.write_str(&text);
        g.clear_to_end_of_row();
        g.set_cursor(start.0 + cursor.min(max), start.1);
    });
}

/// Reads one line for a user program (the `read` system call): the same
/// line editor, no prompt (the program prints its own). The calling thread
/// becomes the one the keyboard wakes.
pub fn read_line_for_user() -> String {
    let mut editor = LineEditor::new();
    let line = read_line(&mut editor, None).unwrap_or_default();
    crate::kprintln!("con: (input) {line}");
    line
}

/// Reads one line, editing in place. Returns None on Ctrl+C.
fn read_line(editor: &mut LineEditor, prompt: Option<&str>) -> Option<String> {
    ps2::set_key_waiter(sched::current_id());
    if let Some(p) = prompt {
        gui::console_write(p, COLOR_ACCENT);
    }
    let mut start = gui::with_console(|g| g.cursor()).unwrap_or((0, 0));
    let cols = gui::console_size().map_or(80, |s| s.0);
    let max = cols.saturating_sub(start.0 + 1);
    loop {
        while let Some(input) = ps2::pop_key() {
            let Some(key) = edit_key(input) else {
                if input.ctrl && matches!(input.action, Action::Char('l')) {
                    gui::with_console(|g| g.clear());
                    if let Some(p) = prompt {
                        gui::console_write(p, COLOR_ACCENT);
                    }
                    start = gui::with_console(|g| g.cursor()).unwrap_or((0, 0));
                    redraw_line(start, &editor.text(), editor.cursor(), max);
                }
                continue;
            };
            // Typing past the end of the row is ignored (single-row editing).
            if matches!(key, EditKey::Char(_)) && editor.text().chars().count() >= max {
                continue;
            }
            match editor.handle(key) {
                Edit::Changed => redraw_line(start, &editor.text(), editor.cursor(), max),
                Edit::Submitted(line) => {
                    // Leave the finished line on screen (the editor is already empty again).
                    redraw_line(start, &line, line.chars().count(), max);
                    gui::console_write("\n", COLOR_TEXT);
                    if let Some(p) = prompt {
                        crate::kprintln!("con: {p}{line}");
                    }
                    return Some(line);
                }
                Edit::Cancelled => {
                    out("^C\n", COLOR_DIM);
                    return None;
                }
                Edit::Nothing => {}
            }
        }
        sched::park();
    }
}

fn mib(bytes: u64) -> u64 {
    bytes >> 20
}

const HELP: &str = "\
help            this list
clear           clear the screen
echo TEXT       print TEXT
uptime          time since boot
mem             memory: physical frames and the kernel heap
cpus            the CPUs
threads         scheduler statistics
sleep MS        sleep (tests blocking and the scheduler)
progs           list the built-in user programs
run NAME [ARGS] run a program (built-in, /bin/NAME, or a path) and wait
spawn NAME...   start user programs in the background
ps              list running user processes
Files:
pwd, cd [DIR], ls [DIR], cat FILE, write FILE TEXT, mkdir DIR, rm PATH,
stat PATH, cp SRC DST    work on the file system (paths may be relative)
disks           the disks found (SATA, NVMe); mounts / df: mounted file systems
format DISK erase   create a NovaFS on a disk (DESTROYS the disk)
install         copy the built-in programs to /bin on the disk
fscheck         check the disk's file system; sync: write everything out
usb             USB keyboards and mice attached
version         kernel version
panic           deliberately crash the kernel (shows the panic screen)
reboot          restart the machine
halt            stop everything
Keys: Left/Right/Home/End/Delete edit, Up/Down browse history,
      Ctrl+U kill line, Ctrl+W kill word, Ctrl+C cancel, Ctrl+L clear.";

fn execute(line: &str) {
    let mut parts = line.trim().splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    match cmd {
        "" => {}
        "help" | "?" => outln(HELP),
        "clear" | "cls" => {
            gui::with_console(|g| g.clear());
        }
        "echo" => outln(rest),
        "uptime" => {
            let ticks = interrupts::ticks();
            let hz = interrupts::TICK_HZ as u64;
            outln(&alloc::format!("up {}.{:02} s ({} ticks)", ticks / hz, ticks % hz, ticks));
        }
        "mem" => match pmm::stats() {
            Some(s) => {
                let (heap_total, heap_free) = heap::stats();
                outln(&alloc::format!(
                    "physical: {} MiB free of {} MiB tracked ({} frames free)",
                    mib(s.free_bytes()),
                    mib(s.tracked_frames as u64 * 4096),
                    s.free_frames
                ));
                outln(&alloc::format!("heap:     {} KiB mapped, {} KiB free", heap_total / 1024, heap_free / 1024));
            }
            None => errln("memory manager not ready"),
        },
        "cpus" => {
            outln(&alloc::format!("{} CPU(s) online", cpu::online_count()));
            for id in 0..cpu::MAX_CPUS {
                if let Some(c) = cpu::cpu(id).filter(|c| c.online.load(core::sync::atomic::Ordering::SeqCst)) {
                    outln(&alloc::format!("  cpu {id}: APIC id {}", c.apic_id.load(core::sync::atomic::Ordering::SeqCst)));
                }
            }
            outln(&alloc::format!("this shell runs on cpu {}", cpu::this_cpu().id));
        }
        "threads" => {
            let s = sched::stats();
            outln(&alloc::format!(
                "{} threads, {} context switches, {} stacks ({} pooled)",
                s.threads,
                s.switches,
                s.stacks_created,
                s.stacks_pooled
            ));
        }
        "sleep" => match rest.parse::<u64>() {
            Ok(ms) if ms <= 60_000 => {
                sched::sleep_ms(ms);
                outln(&alloc::format!("slept {ms} ms"));
            }
            _ => errln("usage: sleep MILLISECONDS (at most 60000)"),
        },
        "progs" => {
            for (name, size) in programs::list() {
                outln(&alloc::format!("  {name:<10} {size} bytes"));
            }
        }
        "run" => {
            let (spec, args) = rest.split_once(' ').map_or((rest, ""), |(a, b)| (a, b.trim()));
            match process::spawn(spec, args) {
                Ok((_, handle)) => handle.join_sleeping(),
                Err(e) => errln(&e),
            }
        }
        // `spawn a b c` starts several programs at once, in the background.
        "spawn" => {
            for name in rest.split_whitespace() {
                match process::spawn(name, "") {
                    Ok((pid, _)) => outln(&alloc::format!("[process {pid} ({name}) started in the background]")),
                    Err(e) => errln(&e),
                }
            }
        }
        "ps" => {
            let list = process::list();
            if list.is_empty() {
                outln("no user processes running");
            }
            for (pid, name) in list {
                outln(&alloc::format!("  pid {pid:<4} {name}"));
            }
        }
        "pwd" => outln(&cwd()),
        "cd" => {
            let target = abs(if rest.is_empty() { "/" } else { rest });
            match VFS.lock().stat(&target) {
                Ok(s) if s.kind == NodeKind::Dir => {
                    let norm = nova_fs::path::components(&target).map(|c| nova_fs::path::join(&c)).unwrap_or(target);
                    CWD.with(|c| *c = norm);
                }
                Ok(_) => fs_err(rest, nova_fs::FsError::NotDir),
                Err(e) => fs_err(rest, e),
            }
        }
        "ls" => {
            let path = abs(if rest.is_empty() { "." } else { rest });
            let mut vfs = VFS.lock();
            match vfs.readdir(&path) {
                Ok(entries) => {
                    for e in entries {
                        if e.kind == NodeKind::Dir {
                            outln(&alloc::format!("  {:>9}  {}/", "<dir>", e.name));
                        } else {
                            let size = vfs.stat(&alloc::format!("{}/{}", path.trim_end_matches('/'), e.name)).map_or(0, |s| s.size);
                            outln(&alloc::format!("  {size:>9}  {}", e.name));
                        }
                    }
                }
                Err(nova_fs::FsError::NotDir) => outln(&path),
                Err(e) => fs_err(rest, e),
            }
        }
        "cat" => match VFS.lock().read_file(&abs(rest)) {
            Ok(data) => {
                let shown = &data[..data.len().min(MAX_CAT)];
                outln(String::from_utf8_lossy(shown).trim_end_matches('\n'));
                if data.len() > MAX_CAT {
                    outln("... (truncated)");
                }
            }
            Err(e) => fs_err(rest, e),
        },
        "write" => match rest.split_once(' ') {
            Some((path, text)) => {
                let mut data = text.as_bytes().to_vec();
                data.push(b'\n');
                match VFS.lock().write_file(&abs(path), &data) {
                    Ok(()) => outln(&alloc::format!("wrote {} bytes to {path}", data.len())),
                    Err(e) => fs_err(path, e),
                }
            }
            None => errln("usage: write FILE TEXT"),
        },
        "mkdir" => match VFS.lock().mkdir(&abs(rest)) {
            Ok(()) => {}
            Err(e) => fs_err(rest, e),
        },
        "rm" => match VFS.lock().remove(&abs(rest)) {
            Ok(()) => {}
            Err(e) => fs_err(rest, e),
        },
        "stat" => match VFS.lock().stat(&abs(rest)) {
            Ok(s) => outln(&alloc::format!("{}: {} bytes ({})", rest, s.size, if s.kind == NodeKind::Dir { "directory" } else { "file" })),
            Err(e) => fs_err(rest, e),
        },
        "cp" => match rest.split_once(' ') {
            Some((src, dst)) => {
                let mut vfs = VFS.lock();
                match vfs.read_file(&abs(src)).and_then(|d| vfs.write_file(&abs(dst.trim()), &d).map(|()| d.len())) {
                    Ok(n) => outln(&alloc::format!("copied {n} bytes")),
                    Err(e) => fs_err(src, e),
                }
            }
            None => errln("usage: cp SRC DST"),
        },
        "mounts" | "df" => {
            for m in VFS.lock().mounts() {
                match m.stats {
                    Some(s) => outln(&alloc::format!(
                        "  {:<8} {:<7} {} KiB free of {} KiB",
                        m.path,
                        m.fs_name,
                        s.free_bytes / 1024,
                        s.total_bytes / 1024
                    )),
                    None => outln(&alloc::format!("  {:<8} {}", m.path, m.fs_name)),
                }
            }
        }
        "disks" => {
            let list = files::disks();
            if list.is_empty() {
                outln("no disks found");
            }
            for (i, model, bytes, bus) in list {
                let root = if files::root_disk() == Some(i) { "  (holds /)" } else { "" };
                outln(&alloc::format!("  disk {i}: {model}, {} MiB, {bus}{root}", bytes >> 20));
            }
        }
        "format" => {
            let mut it = rest.split_whitespace();
            match (it.next().and_then(|d| d.parse::<usize>().ok()), it.next()) {
                (Some(disk), Some("erase")) => {
                    if !process::list().is_empty() {
                        errln("stop the running processes first");
                    } else {
                        match files::format_disk(disk) {
                            Ok(()) => outln(&alloc::format!("formatted disk {disk} with NovaFS; it is now mounted at /")),
                            Err(e) => fs_err("format", e),
                        }
                    }
                }
                _ => errln("usage: format DISK erase   (see 'disks'; this DESTROYS everything on that disk)"),
            }
        }
        "install" => {
            let mut vfs = VFS.lock();
            let _ = vfs.mkdir("/bin");
            let mut n = 0;
            for (name, _) in programs::list() {
                let image = programs::find(name).unwrap_or(&[]);
                match vfs.write_file(&alloc::format!("/bin/{name}"), image) {
                    Ok(()) => n += 1,
                    Err(e) => fs_err(name, e),
                }
            }
            outln(&alloc::format!("installed {n} programs in /bin"));
        }
        "sync" => match files::sync() {
            Ok(()) => outln("synced"),
            Err(e) => fs_err("sync", e),
        },
        "fscheck" => match files::root_disk() {
            None => errln("the root file system is not on a disk"),
            Some(disk) => {
                let _ = files::sync();
                match files::fsck_disk(disk) {
                    Ok(problems) if problems.is_empty() => outln("file system is clean"),
                    Ok(problems) => {
                        errln(&alloc::format!("{} problem(s) found:", problems.len()));
                        for p in problems.iter().take(20) {
                            errln(&alloc::format!("  {p}"));
                        }
                    }
                    Err(e) => fs_err("fscheck", e),
                }
            }
        },
        "version" => outln(&alloc::format!("Nova kernel {} (x86_64, {} CPUs)", env!("CARGO_PKG_VERSION"), cpu::online_count())),
        "panic" => panic!("the 'panic' shell command was run"),
        "usb" => {
            let lines = crate::xhci::describe();
            if lines.is_empty() {
                outln("no USB controller found");
            }
            for l in lines {
                outln(&l);
            }
        }
        "reboot" => {
            files::shutdown();
            outln("restarting...");
            ps2::reset_cpu();
            errln("the keyboard controller did not reset the machine");
        }
        "halt" => {
            files::shutdown();
            outln("halted.");
            crate::smp::halt_other_cpus();
            nova_x86::halt_forever();
        }
        other => errln(&alloc::format!("unknown command '{other}' (try 'help')")),
    }
}

/// The shell thread's body.
pub fn run() {
    ps2::set_key_waiter(sched::current_id());
    gui::with_console(|g| g.clear());
    out("Nova shell. Type 'help'.\n", COLOR_DIM);
    crate::kprintln!("shell: ready");
    let mut editor = LineEditor::new();
    loop {
        if let Some(line) = read_line(&mut editor, Some(PROMPT)) {
            execute(&line);
        }
    }
}
