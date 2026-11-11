//! Reads a line from the keyboard and greets you by name (the read system call).

#![no_std]
#![no_main]

use nova_user::{entry, print, println, read};

fn main() -> i64 {
    print!("What is your name? ");
    let mut buf = [0u8; 64];
    let n = read(0, &mut buf);
    if n < 0 {
        println!("read failed: {n}");
        return 1;
    }
    let line = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?").trim();
    if line.is_empty() {
        println!("Hello, stranger!");
    } else {
        println!("Hello, {line}!");
    }
    0
}

entry!(main);
