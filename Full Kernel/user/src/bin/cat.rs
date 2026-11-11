//! `cat FILE`: prints a file (open / read / close system calls).

#![no_std]
#![no_main]

use nova_user::{args, close, entry, open, println, read, write};

fn main() -> i64 {
    let mut argbuf = [0u8; 256];
    let path = args(&mut argbuf);
    if path.is_empty() {
        println!("usage: cat FILE");
        return 2;
    }
    let fd = open(path, 0);
    if fd < 0 {
        println!("cat: {path}: error {}", -fd);
        return 1;
    }
    let mut buf = [0u8; 512];
    loop {
        let n = read(fd as u64, &mut buf);
        if n < 0 {
            println!("cat: read error {}", -n);
            return 1;
        }
        if n == 0 {
            break;
        }
        write(1, &buf[..n as usize]);
    }
    close(fd as u64);
    0
}

entry!(main);
