//! Writes a 5 MiB file (past the point where NovaFS switches to double-indirect
//! blocks), reads it back and verifies every byte, then deletes it.

#![no_std]
#![no_main]

use nova_user::{O_CREATE, O_TRUNC, O_WRITE, close, entry, open, println, read, unlink, write};

const CHUNK: usize = 4096;
const CHUNKS: usize = 1280; // 5 MiB

/// A cheap pattern that differs per chunk and per byte.
fn fill(chunk: usize, buf: &mut [u8; CHUNK]) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = (chunk.wrapping_mul(131) as u8).wrapping_add((i as u8).wrapping_mul(7)) ^ (i >> 8) as u8;
    }
}

fn main() -> i64 {
    let path = "/fsbig.dat";
    let _ = unlink(path);
    let fd = open(path, O_WRITE | O_CREATE | O_TRUNC);
    if fd < 0 {
        println!("fsbig: cannot create: {fd}");
        return 1;
    }
    let mut buf = [0u8; CHUNK];
    for c in 0..CHUNKS {
        fill(c, &mut buf);
        let n = write(fd as u64, &buf);
        if n != CHUNK as i64 {
            println!("fsbig: write of chunk {c} returned {n}");
            return 1;
        }
    }
    close(fd as u64);

    let fd = open(path, 0);
    if fd < 0 {
        println!("fsbig: cannot reopen: {fd}");
        return 1;
    }
    let mut expect = [0u8; CHUNK];
    for c in 0..CHUNKS {
        let n = read(fd as u64, &mut buf);
        fill(c, &mut expect);
        if n != CHUNK as i64 || buf != expect {
            println!("fsbig: chunk {c} came back wrong (read returned {n})");
            return 1;
        }
    }
    if read(fd as u64, &mut buf) != 0 {
        println!("fsbig: the file is longer than expected");
        return 1;
    }
    close(fd as u64);
    if unlink(path) != 0 {
        println!("fsbig: unlink failed");
        return 1;
    }
    println!("fsbig ok: {} MiB written, verified byte for byte, deleted", CHUNKS * CHUNK >> 20);
    0
}

entry!(main);
