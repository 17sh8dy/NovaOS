//! Counts to ten, sleeping between numbers. Run several at once to see the
//! scheduler interleave processes.

#![no_std]
#![no_main]

use nova_user::{entry, getpid, println, sleep_ms};

fn main() -> i64 {
    let pid = getpid();
    for i in 1..=10 {
        println!("[{pid}] counter {i}");
        sleep_ms(100);
    }
    0
}

entry!(main);
