//! `wc`: counts the lines, words and bytes of its input (`cat FILE | wc`).

#![no_std]
#![no_main]

use nova_user::{entry, println, read};

fn main() -> i64 {
    let (mut lines, mut words, mut bytes) = (0u64, 0u64, 0u64);
    let mut in_word = false;
    let mut chunk = [0u8; 512];
    loop {
        let n = read(0, &mut chunk);
        if n <= 0 {
            break;
        }
        for &b in &chunk[..n as usize] {
            bytes += 1;
            if b == b'\n' {
                lines += 1;
            }
            if b.is_ascii_whitespace() {
                in_word = false;
            } else if !in_word {
                in_word = true;
                words += 1;
            }
        }
    }
    println!("{lines} lines, {words} words, {bytes} bytes");
    0
}

entry!(main);
