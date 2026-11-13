//! `grep [-v] TEXT`: prints the lines of its input that contain TEXT (with
//! `-v`: that don't). Reads standard input, so it is useful in a pipeline:
//! `ls /bin | grep e`. Exit code 0 if any line was printed, 1 if none.

#![no_std]
#![no_main]

use nova_user::{args, entry, println, read, write};

fn main() -> i64 {
    let mut argbuf = [0u8; 256];
    let a = args(&mut argbuf);
    let (invert, pattern) = match a.strip_prefix("-v ") {
        Some(p) => (true, p.trim()),
        None => (false, a),
    };
    if pattern.is_empty() {
        println!("usage: grep [-v] TEXT   (reads its input, e.g. ls /bin | grep e)");
        return 2;
    }
    let mut line = [0u8; 1024];
    let mut len = 0;
    let mut chunk = [0u8; 512];
    let mut printed = false;
    let emit = |l: &[u8], printed: &mut bool| {
        let text = core::str::from_utf8(l).unwrap_or("");
        if text.contains(pattern) != invert {
            // One write per line, newline included, so lines arrive whole.
            let mut out = [0u8; 1025];
            out[..l.len()].copy_from_slice(l);
            out[l.len()] = b'\n';
            write(1, &out[..=l.len()]);
            *printed = true;
        }
    };
    loop {
        let n = read(0, &mut chunk);
        if n <= 0 {
            break;
        }
        for &b in &chunk[..n as usize] {
            if b == b'\n' {
                emit(&line[..len], &mut printed);
                len = 0;
            } else if len < line.len() {
                line[len] = b;
                len += 1;
            }
        }
    }
    if len > 0 {
        emit(&line[..len], &mut printed);
    }
    if printed { 0 } else { 1 }
}

entry!(main);
