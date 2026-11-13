//! `nsh`, the Nova shell: an ordinary user program (ring 3, system calls only).
//!
//! It reads command lines from the Terminal (`readline`: the kernel's line
//! editor with history), and:
//! * runs its built-ins: `cd`, `pwd`, `echo`, `run`, `spawn`, `help`, `exit`;
//! * runs programs (`ls /bin`, `/bin/hello`), waiting for them (`spawn` + `wait`);
//! * connects programs with pipes: `ls /bin | grep e | wc`;
//! * runs a command in the background with a trailing `&`;
//! * hands anything else to the kernel's system commands (`ctl`): `disks`,
//!   `format`, `mem`, `ps`, `install`, ...
//!
//! No heap: everything lives in fixed buffers on the stack.

#![no_std]
#![no_main]

use nova_user::{CONSOLE, chdir, close, ctl, entry, errno, getcwd, pipe, print, println, readline, spawn, wait, write};

const PROMPT: &str = "nova> ";
const MAX_STAGES: usize = 8;

fn error_text(e: i64) -> &'static str {
    match -e {
        errno::ENOENT => "no such file or directory",
        errno::ENOTDIR => "not a directory",
        errno::EINVAL => "invalid argument",
        errno::ENOMEM => "out of memory",
        errno::EPIPE => "broken pipe",
        _ => "error",
    }
}

fn eprintln(text: &str) {
    write(2, text.as_bytes());
    write(2, b"\n");
}

/// The program name in a command ("/bin/ls -x" -> "ls").
fn program_name(command: &str) -> &str {
    let word = command.split_whitespace().next().unwrap_or("");
    word.rsplit('/').next().unwrap_or(word)
}

fn help() {
    println!("nsh, the Nova shell (a user program):");
    println!("  cd [DIR], pwd, echo TEXT, run PROGRAM [ARGS], spawn P1 P2 ..., exit");
    println!("  PROGRAM [ARGS]    run a program and wait for it (ls, cat, grep, wc, ...)");
    println!("  A | B | C         connect programs with pipes: ls /bin | grep e | wc");
    println!("  COMMAND &         run it in the background");
    println!("System commands (the kernel runs these for the shell):");
    ctl("help");
}

/// Runs one program in the foreground. `verbose`: report how it ended (the `run` built-in).
fn run_program(command: &str, verbose: bool) {
    let name = program_name(command);
    let pid = spawn(command, CONSOLE, CONSOLE, false);
    if pid == -errno::ENOENT {
        if verbose {
            eprintln("no such program (try 'progs', or give a path)");
        } else {
            // Not a program: maybe a system command (this also reports unknown commands).
            ctl(command);
        }
        return;
    }
    if pid < 0 {
        println!("nsh: {name}: {}", error_text(pid));
        return;
    }
    let code = wait(pid as u64);
    if verbose {
        println!("[process {pid} ({name}) exited with code {code}]");
    } else if code > 0 && code < 128 {
        // (128 and up: the kernel has already said why it was killed.)
        println!("[{name} exited with code {code}]");
    }
}

fn background(command: &str) {
    let name = program_name(command);
    let pid = spawn(command, CONSOLE, CONSOLE, true);
    if pid < 0 {
        println!("nsh: {name}: {}", if pid == -errno::ENOENT { "no such program" } else { error_text(pid) });
    } else {
        println!("[process {pid} ({name}) started in the background]");
    }
}

/// `a | b | c`: every stage's output feeds the next one's input.
fn pipeline(line: &str, bg: bool) {
    let mut stages = [""; MAX_STAGES];
    let mut n = 0;
    for part in line.split('|') {
        if n == MAX_STAGES {
            println!("nsh: at most {MAX_STAGES} programs in a pipeline");
            return;
        }
        let part = part.trim();
        if part.is_empty() {
            println!("nsh: empty command in the pipeline");
            return;
        }
        stages[n] = part;
        n += 1;
    }
    let mut pids = [0i64; MAX_STAGES];
    let mut prev_read = CONSOLE;
    for i in 0..n {
        let (stdout, next_read) = if i + 1 < n {
            match pipe() {
                Ok((r, w)) => (w, r),
                Err(e) => {
                    println!("nsh: pipe: {}", error_text(e));
                    break;
                }
            }
        } else {
            (CONSOLE, CONSOLE)
        };
        pids[i] = spawn(stages[i], prev_read, stdout, bg);
        if pids[i] < 0 {
            println!("nsh: {}: {}", program_name(stages[i]), if pids[i] == -errno::ENOENT { "no such program" } else { error_text(pids[i]) });
        }
        // Our copies of the pipe ends must go, or the readers never see the end.
        if prev_read != CONSOLE {
            close(prev_read);
        }
        if stdout != CONSOLE {
            close(stdout);
        }
        prev_read = next_read;
    }
    if prev_read != CONSOLE {
        close(prev_read);
    }
    if bg {
        println!("[pipeline of {n} programs started in the background]");
        return;
    }
    let mut last = 0;
    for &pid in pids.iter().take(n) {
        if pid > 0 {
            last = wait(pid as u64);
        }
    }
    if last > 0 && last < 128 {
        println!("[{} exited with code {last}]", program_name(stages[n - 1]));
    }
}

fn execute(line: &str) {
    let (line, bg) = match line.strip_suffix('&') {
        Some(rest) => (rest.trim(), true),
        None => (line, false),
    };
    if line.is_empty() {
        return;
    }
    if line.contains('|') {
        return pipeline(line, bg);
    }
    let (word, rest) = line.split_once(' ').map_or((line, ""), |(a, b)| (a, b.trim()));
    match word {
        "cd" => {
            let dir = if rest.is_empty() { "/" } else { rest };
            let r = chdir(dir);
            if r < 0 {
                println!("cd: {dir}: {}", error_text(r));
            }
        }
        "pwd" => {
            let mut buf = [0u8; 256];
            println!("{}", getcwd(&mut buf));
        }
        "echo" => println!("{rest}"),
        "help" | "?" => help(),
        "run" if rest.is_empty() => println!("usage: run PROGRAM [ARGS]"),
        "run" => run_program(rest, true),
        "spawn" => {
            for name in rest.split_whitespace() {
                background(name);
            }
        }
        _ if bg => background(line),
        _ => run_program(line, false),
    }
}

fn main() -> i64 {
    print!("Nova shell (nsh, a user program). Type 'help'.\n");
    let mut buf = [0u8; 512];
    loop {
        let n = readline(PROMPT, &mut buf);
        if n < 0 {
            continue; // Ctrl+C: the line was abandoned
        }
        let line = core::str::from_utf8(&buf[..n as usize]).unwrap_or("").trim();
        if line == "exit" {
            return 0;
        }
        if let Some(code) = line.strip_prefix("exit ") {
            return code.trim().parse().unwrap_or(0);
        }
        execute(line);
    }
}

entry!(main);
