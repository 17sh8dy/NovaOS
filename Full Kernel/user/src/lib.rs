//! The Nova user-space runtime: system calls, `print!`, a panic handler and
//! the program entry point. Programs are `no_std` binaries that talk to the
//! kernel only through `syscall`.
//!
//! System call ABI: number in `rax`; arguments in `rdi, rsi, rdx, r10, r8, r9`;
//! result in `rax` (negative = error). `rcx` and `r11` are clobbered.

#![no_std]

use core::arch::asm;
use core::fmt;

pub mod nr {
    pub const EXIT: u64 = 0;
    pub const WRITE: u64 = 1;
    pub const READ: u64 = 2;
    pub const GETPID: u64 = 3;
    pub const YIELD: u64 = 4;
    pub const SLEEP_MS: u64 = 5;
    pub const UPTIME_MS: u64 = 6;
    pub const SBRK: u64 = 7;
}

/// Error numbers returned (negated) by the kernel.
pub mod errno {
    pub const EFAULT: i64 = 14;
    pub const EINVAL: i64 = 22;
    pub const ENOMEM: i64 = 12;
    pub const ENOSYS: i64 = 38;
}

/// Raw system call with up to three arguments.
///
/// # Safety
/// The kernel validates every pointer, but some calls (like `exit`) never return.
#[inline]
pub unsafe fn syscall3(nr: u64, a: u64, b: u64, c: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            in("rdi") a,
            in("rsi") b,
            in("rdx") c,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// Ends the program.
pub fn exit(code: i64) -> ! {
    // SAFETY: exit takes one integer.
    unsafe { syscall3(nr::EXIT, code as u64, 0, 0) };
    loop {
        core::hint::spin_loop();
    }
}

/// Writes bytes to the console (fd 1 or 2). Returns the count written or -errno.
pub fn write(fd: u64, data: &[u8]) -> i64 {
    // SAFETY: the kernel checks the pointer and length.
    unsafe { syscall3(nr::WRITE, fd, data.as_ptr() as u64, data.len() as u64) }
}

/// Reads one line from the keyboard (fd 0) into `buf`, including the newline.
/// Blocks until Enter is pressed. Returns the count read or -errno.
pub fn read(fd: u64, buf: &mut [u8]) -> i64 {
    // SAFETY: the kernel checks the pointer and length.
    unsafe { syscall3(nr::READ, fd, buf.as_mut_ptr() as u64, buf.len() as u64) }
}

pub fn getpid() -> u64 {
    // SAFETY: no arguments.
    unsafe { syscall3(nr::GETPID, 0, 0, 0) as u64 }
}

pub fn yield_now() {
    // SAFETY: no arguments.
    unsafe { syscall3(nr::YIELD, 0, 0, 0) };
}

pub fn sleep_ms(ms: u64) {
    // SAFETY: one integer argument.
    unsafe { syscall3(nr::SLEEP_MS, ms, 0, 0) };
}

/// Milliseconds since the kernel booted.
pub fn uptime_ms() -> u64 {
    // SAFETY: no arguments.
    unsafe { syscall3(nr::UPTIME_MS, 0, 0, 0) as u64 }
}

/// Grows the program's heap by `bytes` (zero-filled) and returns the old
/// break (the start of the new memory), or a negative errno.
pub fn sbrk(bytes: u64) -> i64 {
    // SAFETY: one integer argument.
    unsafe { syscall3(nr::SBRK, bytes, 0, 0) }
}

/// A formatter that collects a whole `print!` and sends it in one `write`,
/// so concurrent programs' lines don't interleave mid-line.
struct Buffered {
    buf: [u8; 256],
    len: usize,
}

impl Buffered {
    fn flush(&mut self) {
        if self.len > 0 {
            write(1, &self.buf[..self.len]);
            self.len = 0;
        }
    }
}

impl fmt::Write for Buffered {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            if self.len == self.buf.len() {
                self.flush();
            }
            self.buf[self.len] = b;
            self.len += 1;
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let mut b = Buffered { buf: [0; 256], len: 0 };
    let _ = fmt::write(&mut b, args);
    b.flush();
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::_print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! println {
    () => { $crate::_print(format_args!("\n")) };
    ($($arg:tt)*) => { $crate::_print(format_args!("{}\n", format_args!($($arg)*))) };
}

/// Defines the program entry point: `entry!(main)` where `fn main() -> i64`
/// returns the exit code.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".text._start")]
        pub extern "C" fn _start() -> ! {
            let code: i64 = $main();
            $crate::exit(code)
        }
    };
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    _print(format_args!("panic: {info}\n"));
    exit(101)
}
