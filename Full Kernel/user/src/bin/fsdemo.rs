//! Exercises the file system from user space: create, write, seek, read back,
//! stat, directories, listing and deleting. Prints "fsdemo ok" if all of it works.

#![no_std]
#![no_main]

use nova_user::{O_CREATE, O_TRUNC, O_WRITE, SEEK_END, SEEK_SET, close, entry, mkdir, open, println, read, readdir, seek, stat, unlink, write};

fn check(what: &str, ok: bool) -> bool {
    if !ok {
        println!("fsdemo: FAILED: {what}");
    }
    ok
}

/// Formats `line N` into `out`; returns the length.
fn format_line(i: u32, out: &mut [u8; 24]) -> usize {
    let mut digits = [0u8; 10];
    let (mut d, mut v) = (0, i);
    loop {
        digits[d] = b'0' + (v % 10) as u8;
        d += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out[..5].copy_from_slice(b"line ");
    let mut n = 5;
    while d > 0 {
        d -= 1;
        out[n] = digits[d];
        n += 1;
    }
    out[n] = b'\n';
    n + 1
}

fn main() -> i64 {
    let dir = "/fsdemo.d";
    let file = "/fsdemo.d/numbers.txt";
    let _ = unlink(file);
    let _ = unlink(dir);
    if !check("mkdir", mkdir(dir) == 0) || !check("mkdir twice is refused", mkdir(dir) == -17) {
        return 1;
    }

    // Write 400 lines (more than one 4 KiB block of text in total).
    let fd = open(file, O_WRITE | O_CREATE | O_TRUNC);
    if !check("create", fd >= 3) {
        return 1;
    }
    let mut total = 0u64;
    for i in 0..400u32 {
        let mut line = [0u8; 24];
        let n = format_line(i, &mut line);
        if !check("write", write(fd as u64, &line[..n]) == n as i64) {
            return 1;
        }
        total += n as u64;
    }
    close(fd as u64);

    match stat(file) {
        Ok((size, is_dir)) => {
            if !check("size", size == total && !is_dir) {
                return 1;
            }
        }
        Err(e) => {
            println!("fsdemo: stat failed: {e}");
            return 1;
        }
    }

    // Read it back, from the middle, using seek.
    let fd = open(file, 0);
    if !check("reopen", fd >= 3) {
        return 1;
    }
    let end = seek(fd as u64, 0, SEEK_END);
    if !check("seek to end", end == total as i64) || !check("seek to 5", seek(fd as u64, 5, SEEK_SET) == 5) {
        return 1;
    }
    let mut buf = [0u8; 8];
    let n = read(fd as u64, &mut buf);
    if !check("read after seek", n == 8 && &buf[..2] == b"0\n") {
        return 1;
    }
    seek(fd as u64, 0, SEEK_END);
    if !check("read at the end gives 0", read(fd as u64, &mut buf) == 0) {
        return 1;
    }
    close(fd as u64);
    let refused = {
        let fd = open(file, 0);
        let r = write(fd as u64, b"x");
        close(fd as u64);
        r < 0
    };
    if !check("write to a read-only descriptor is refused", refused) {
        return 1;
    }

    // Listing and cleaning up.
    let mut list = [0u8; 256];
    let l = readdir(dir, &mut list);
    if !check("readdir", l > 0 && core::str::from_utf8(&list[..l as usize]).unwrap_or("").starts_with("f numbers.txt ")) {
        return 1;
    }
    if !check("rmdir of a non-empty directory is refused", unlink(dir) == -39)
        || !check("unlink file", unlink(file) == 0)
        || !check("unlink dir", unlink(dir) == 0)
        || !check("it is gone", stat(file).is_err())
    {
        return 1;
    }
    println!("fsdemo ok: wrote, read, seeked, listed and deleted {total} bytes");
    0
}

entry!(main);
