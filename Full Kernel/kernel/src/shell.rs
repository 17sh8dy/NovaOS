//! The Nova shell: a kernel thread that reads typed lines from the keyboard,
//! runs built-in commands and prints on the console.
//!
//! It sleeps (`sched::park`) while there is nothing to read; the keyboard
//! interrupt wakes it. Every console line is also mirrored to the serial log
//! as `con: ...`, which is how the automated tests see what the screen shows.

use crate::gui::{self, COLOR_ACCENT, COLOR_DIM, COLOR_ERROR, COLOR_TEXT};
use crate::{cpu, heap, interrupts, pmm, ps2, sched};
use alloc::string::String;
use nova_console::{Edit, EditKey, LineEditor};
use nova_kbd::{Action, Input, Key};

const PROMPT: &str = "nova> ";

/// Prints text on the console and mirrors it to the serial log.
fn out(text: &str, color: u8) {
    gui::console_write(text, color);
    for line in text.lines() {
        crate::kprintln!("con: {line}");
    }
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

/// Reads one line, editing in place. Returns None on Ctrl+C.
fn read_line(editor: &mut LineEditor) -> Option<String> {
    gui::console_write(PROMPT, COLOR_ACCENT);
    let mut start = gui::with_console(|g| g.cursor()).unwrap_or((0, 0));
    let cols = gui::console_size().map_or(80, |s| s.0);
    let max = cols.saturating_sub(start.0 + 1);
    loop {
        while let Some(input) = ps2::pop_key() {
            let Some(key) = edit_key(input) else {
                if input.ctrl && matches!(input.action, Action::Char('l')) {
                    gui::with_console(|g| g.clear());
                    gui::console_write(PROMPT, COLOR_ACCENT);
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
                    crate::kprintln!("con: {PROMPT}{line}");
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
        "version" => outln(&alloc::format!("Nova kernel {} (x86_64, {} CPUs)", env!("CARGO_PKG_VERSION"), cpu::online_count())),
        "panic" => panic!("the 'panic' shell command was run"),
        "reboot" => {
            outln("restarting...");
            ps2::reset_cpu();
            errln("the keyboard controller did not reset the machine");
        }
        "halt" => {
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
        if let Some(line) = read_line(&mut editor) {
            execute(&line);
        }
    }
}
