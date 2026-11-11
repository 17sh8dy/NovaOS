//! `ls [DIR]`: lists a directory (readdir system call).

#![no_std]
#![no_main]

use nova_user::{args, entry, println, readdir};

fn main() -> i64 {
    let mut argbuf = [0u8; 256];
    let given = args(&mut argbuf);
    let path = if given.is_empty() { "." } else { given };
    let mut buf = [0u8; 4096];
    let n = readdir(path, &mut buf);
    if n < 0 {
        println!("ls: {path}: error {}", -n);
        return 1;
    }
    for line in core::str::from_utf8(&buf[..n as usize]).unwrap_or("").lines() {
        match line.split_once(' ') {
            Some(("d", name)) => println!("  <dir>  {name}/"),
            Some(("f", rest)) => match rest.rsplit_once(' ') {
                Some((name, size)) => println!("  {size:>6}  {name}"),
                None => println!("  {rest}"),
            },
            _ => {}
        }
    }
    0
}

entry!(main);
