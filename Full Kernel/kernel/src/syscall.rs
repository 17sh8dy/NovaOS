//! System calls: the user-mode `syscall` instruction lands in
//! `nova_syscall_entry`, which saves the user state in the same layout the
//! exception stubs use, runs [`nova_syscall_handler`], and returns to ring 3
//! with `iretq`.
//!
//! ABI (see `user/src/lib.rs`): number in `rax`; arguments in `rdi, rsi, rdx,
//! r10, r8, r9`; result in `rax`, negative = -errno. All user pointers are
//! checked by walking the process's page tables before the kernel touches
//! them (`Process::copy_from_user` / `copy_to_user`), so a bad pointer is an
//! error return, never a kernel fault.

use crate::exceptions::ExceptionFrame;
use crate::gui::{COLOR_ERROR, COLOR_TEXT};
use crate::{interrupts, process, sched, shell};
use core::arch::global_asm;

pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_READ: u64 = 2;
pub const SYS_GETPID: u64 = 3;
pub const SYS_YIELD: u64 = 4;
pub const SYS_SLEEP_MS: u64 = 5;
pub const SYS_UPTIME_MS: u64 = 6;
pub const SYS_SBRK: u64 = 7;

const EBADF: i64 = -9;
const EINVAL: i64 = -22;
const ENOSYS: i64 = -38;

global_asm!(
    r#"
    .section .text.syscall,"ax"
    .global nova_syscall_entry
    // SYSCALL: rcx = user rip, r11 = user rflags, rsp = still the user's, interrupts off.
    // gs:[8] = scratch for the user rsp, gs:[16] = this thread's kernel stack top
    // (see cpu::PerCpu).
    nova_syscall_entry:
        swapgs
        mov gs:[8], rsp
        mov rsp, gs:[16]
        // Build the same frame an interrupt would (ss, rsp, rflags, cs, rip, error, vector)...
        push {ss}
        push qword ptr gs:[8]
        push r11
        push {cs}
        push rcx
        push 0
        push 0x80
        // ...then the general registers, in the order of ExceptionFrame.
        push rax
        push rbx
        push rcx
        push rdx
        push rsi
        push rdi
        push rbp
        push r8
        push r9
        push r10
        push r11
        push r12
        push r13
        push r14
        push r15
        mov rdi, rsp
        cld
        call {handler}
        pop r15
        pop r14
        pop r13
        pop r12
        pop r11
        pop r10
        pop r9
        pop r8
        pop rbp
        pop rdi
        pop rsi
        pop rdx
        pop rcx
        pop rbx
        pop rax
        add rsp, 16
        swapgs
        iretq
    "#,
    ss = const nova_x86::gdt::USER_DS as u32,
    cs = const nova_x86::gdt::USER_CS as u32,
    handler = sym nova_syscall_handler,
);

unsafe extern "C" {
    fn nova_syscall_entry();
}

/// Address of the SYSCALL entry stub (for the LSTAR register).
pub fn entry_address() -> u64 {
    nova_syscall_entry as *const () as usize as u64
}

extern "sysv64" fn nova_syscall_handler(f: &mut ExceptionFrame) {
    // The SYSCALL instruction left interrupts off; the call itself may block or be preempted.
    nova_x86::sti();
    let (nr, a, b, c) = (f.rax, f.rdi, f.rsi, f.rdx);
    let ret: i64 = match nr {
        SYS_EXIT => process::exit_current(a as i64),
        SYS_WRITE => sys_write(a, b, c),
        SYS_READ => sys_read(a, b, c),
        SYS_GETPID => sched::current_process().map_or(0, |p| p.pid as i64),
        SYS_YIELD => {
            sched::yield_now();
            0
        }
        SYS_SLEEP_MS => {
            sched::sleep_ms(a.min(60_000));
            0
        }
        SYS_UPTIME_MS => (interrupts::ticks() * 1000 / interrupts::TICK_HZ as u64) as i64,
        SYS_SBRK => match sched::current_process().map(|p| p.sbrk(a)) {
            Some(Ok(old)) => old as i64,
            Some(Err(e)) => e,
            None => EINVAL,
        },
        _ => ENOSYS,
    };
    f.rax = ret as u64;
    nova_x86::cli();
}

fn sys_write(fd: u64, ptr: u64, len: u64) -> i64 {
    if fd != 1 && fd != 2 {
        return EBADF;
    }
    if len as usize > process::MAX_COPY {
        return EINVAL;
    }
    let Some(proc) = sched::current_process() else { return EINVAL };
    match proc.copy_from_user(ptr, len as usize) {
        Ok(bytes) => {
            shell::out(&alloc::string::String::from_utf8_lossy(&bytes), if fd == 2 { COLOR_ERROR } else { COLOR_TEXT });
            bytes.len() as i64
        }
        Err(e) => e,
    }
}

fn sys_read(fd: u64, ptr: u64, len: u64) -> i64 {
    if fd != 0 {
        return EBADF;
    }
    if len as usize > process::MAX_COPY {
        return EINVAL;
    }
    let Some(proc) = sched::current_process() else { return EINVAL };
    // Read a whole line from the keyboard (the shell's line editor does the work).
    let mut line = shell::read_line_for_user().into_bytes();
    line.push(b'\n');
    line.truncate(len as usize);
    match proc.copy_to_user(ptr, &line) {
        Ok(()) => line.len() as i64,
        Err(e) => e,
    }
}
