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
use crate::files::{self, VFS};
use crate::process::{Fd, OpenFile};
use crate::{interrupts, pipe, process, sched, shell};
use nova_fs::{FsError, NodeKind};
use core::arch::global_asm;

pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_READ: u64 = 2;
pub const SYS_GETPID: u64 = 3;
pub const SYS_YIELD: u64 = 4;
pub const SYS_SLEEP_MS: u64 = 5;
pub const SYS_UPTIME_MS: u64 = 6;
pub const SYS_SBRK: u64 = 7;
pub const SYS_OPEN: u64 = 8;
pub const SYS_CLOSE: u64 = 9;
pub const SYS_SEEK: u64 = 10;
pub const SYS_READDIR: u64 = 11;
pub const SYS_STAT: u64 = 12;
pub const SYS_MKDIR: u64 = 13;
pub const SYS_UNLINK: u64 = 14;
pub const SYS_GETARGS: u64 = 15;
pub const SYS_SPAWN: u64 = 16;
pub const SYS_WAIT: u64 = 17;
pub const SYS_PIPE: u64 = 18;
pub const SYS_CHDIR: u64 = 19;
pub const SYS_GETCWD: u64 = 20;
pub const SYS_READLINE: u64 = 21;
pub const SYS_CTL: u64 = 22;

/// `open` flags.
const O_WRITE: u64 = 1;
const O_CREATE: u64 = 2;
const O_TRUNC: u64 = 4;

const ENOENT: i64 = -2;
const EINTR: i64 = -4;
const EBADF: i64 = -9;
const ENOMEM: i64 = -12;
const ENOTDIR: i64 = -20;
const EINVAL: i64 = -22;
const EPIPE: i64 = -32;
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
    let (nr, a, b, c, d, e) = (f.rax, f.rdi, f.rsi, f.rdx, f.r10, f.r8);
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
        SYS_OPEN => sys_open(a, b, c),
        SYS_CLOSE => sys_close(a),
        SYS_SEEK => sys_seek(a, b as i64, c),
        SYS_READDIR => sys_readdir(a, b, c, d),
        SYS_STAT => sys_stat(a, b, c),
        SYS_MKDIR => with_path(a, b, |p| VFS.lock().mkdir(p).map(|_| 0)),
        SYS_UNLINK => with_path(a, b, |p| VFS.lock().remove(p).map(|_| 0)),
        SYS_GETARGS => sys_getargs(a, b),
        SYS_SPAWN => sys_spawn(a, b, c, d, e),
        SYS_WAIT => process::wait(a as usize),
        SYS_PIPE => sys_pipe(a),
        SYS_CHDIR => sys_chdir(a, b),
        SYS_GETCWD => sys_getcwd(a, b),
        SYS_READLINE => sys_readline(a, b, c, d),
        SYS_CTL => sys_ctl(a, b),
        _ => ENOSYS,
    };
    f.rax = ret as u64;
    nova_x86::cli();
}

/// Reads a path argument from user memory (at most 512 bytes) and makes it absolute.
fn user_path(proc: &process::Process, ptr: u64, len: u64) -> Result<alloc::string::String, i64> {
    if len == 0 || len > 512 {
        return Err(EINVAL);
    }
    let bytes = proc.copy_from_user(ptr, len as usize)?;
    let s = core::str::from_utf8(&bytes).map_err(|_| EINVAL)?;
    Ok(files::absolute(&proc.cwd(), s))
}

/// Runs `f` with the (absolute) path argument; maps file system errors to errno.
fn with_path(ptr: u64, len: u64, f: impl FnOnce(&str) -> Result<i64, FsError>) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    match user_path(&proc, ptr, len) {
        Ok(p) => f(&p).unwrap_or_else(files::errno),
        Err(e) => e,
    }
}

fn sys_open(ptr: u64, len: u64, flags: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    let path = match user_path(&proc, ptr, len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let mut vfs = VFS.lock();
    let handle = match vfs.resolve(&path) {
        Ok(h) => h,
        Err(FsError::NotFound) if flags & O_CREATE != 0 => match vfs.create(&path, NodeKind::File) {
            Ok(h) => h,
            Err(e) => return files::errno(e),
        },
        Err(e) => return files::errno(e),
    };
    if handle.kind == NodeKind::Dir {
        return files::errno(FsError::IsDir);
    }
    if flags & O_TRUNC != 0
        && flags & O_WRITE != 0
        && let Err(e) = vfs.truncate(&handle, 0)
    {
        return files::errno(e);
    }
    drop(vfs);
    match proc.add_file(OpenFile { handle, offset: 0, writable: flags & O_WRITE != 0 }) {
        Some(fd) => fd as i64,
        None => -24, // EMFILE
    }
}

fn sys_close(fd: u64) -> i64 {
    match sched::current_process() {
        Some(p) if p.close_fd(fd) => 0,
        _ => EBADF,
    }
}

fn sys_seek(fd: u64, offset: i64, whence: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    let Some(file) = proc.file(fd) else { return EBADF };
    let size = match VFS.lock().stat_handle(&file.handle) {
        Ok(s) => s.size as i64,
        Err(e) => return files::errno(e),
    };
    let base = match whence {
        0 => 0,
        1 => file.offset as i64,
        2 => size,
        _ => return EINVAL,
    };
    let Some(new) = base.checked_add(offset).filter(|n| *n >= 0) else { return EINVAL };
    proc.update_file(fd, |f| f.offset = new as u64);
    new
}

/// Lists a directory into the buffer as text lines: `d NAME` or `f NAME SIZE`.
fn sys_readdir(path_ptr: u64, path_len: u64, buf: u64, buf_len: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    let path = match user_path(&proc, path_ptr, path_len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if buf_len as usize > process::MAX_COPY {
        return EINVAL;
    }
    let mut text = alloc::string::String::new();
    {
        let mut vfs = VFS.lock();
        let entries = match vfs.readdir(&path) {
            Ok(e) => e,
            Err(e) => return files::errno(e),
        };
        for e in entries {
            if e.kind == NodeKind::Dir {
                text.push_str(&alloc::format!("d {}\n", e.name));
            } else {
                let size = vfs.stat(&alloc::format!("{}/{}", path.trim_end_matches('/'), e.name)).map_or(0, |s| s.size);
                text.push_str(&alloc::format!("f {} {size}\n", e.name));
            }
        }
    }
    let mut bytes = text.into_bytes();
    bytes.truncate(buf_len as usize);
    match proc.copy_to_user(buf, &bytes) {
        Ok(()) => bytes.len() as i64,
        Err(e) => e,
    }
}

/// Writes 16 bytes at `out`: the size (u64) and the kind (0 = file, 1 = directory).
fn sys_stat(path_ptr: u64, path_len: u64, out: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    let path = match user_path(&proc, path_ptr, path_len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let st = match VFS.lock().stat(&path) {
        Ok(s) => s,
        Err(e) => return files::errno(e),
    };
    let mut rec = [0u8; 16];
    rec[..8].copy_from_slice(&st.size.to_le_bytes());
    rec[8..].copy_from_slice(&(if st.kind == NodeKind::Dir { 1u64 } else { 0 }).to_le_bytes());
    match proc.copy_to_user(out, &rec) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

fn sys_getargs(ptr: u64, len: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    let mut bytes = proc.args.clone().into_bytes();
    bytes.truncate((len as usize).min(process::MAX_COPY));
    match proc.copy_to_user(ptr, &bytes) {
        Ok(()) => bytes.len() as i64,
        Err(e) => e,
    }
}

/// `read` / `write` on an open file.
fn file_io(fd: u64, ptr: u64, len: u64, write: bool) -> i64 {
    if len as usize > process::MAX_COPY {
        return EINVAL;
    }
    let Some(proc) = sched::current_process() else { return EINVAL };
    let Some(file) = proc.file(fd) else { return EBADF };
    if write {
        if !file.writable {
            return EBADF;
        }
        let data = match proc.copy_from_user(ptr, len as usize) {
            Ok(d) => d,
            Err(e) => return e,
        };
        match VFS.lock().write_at(&file.handle, file.offset, &data) {
            Ok(n) => {
                proc.update_file(fd, |f| f.offset += n as u64);
                n as i64
            }
            Err(e) => files::errno(e),
        }
    } else {
        let mut buf = alloc::vec![0u8; len as usize];
        match VFS.lock().read_at(&file.handle, file.offset, &mut buf) {
            Ok(n) => {
                if let Err(e) = proc.copy_to_user(ptr, &buf[..n]) {
                    return e;
                }
                proc.update_file(fd, |f| f.offset += n as u64);
                n as i64
            }
            Err(e) => files::errno(e),
        }
    }
}

fn sys_write(fd: u64, ptr: u64, len: u64) -> i64 {
    if len as usize > process::MAX_COPY {
        return EINVAL;
    }
    let Some(proc) = sched::current_process() else { return EINVAL };
    match proc.fd(fd) {
        Some(Fd::File(_)) => file_io(fd, ptr, len, true),
        Some(Fd::Console) => match proc.copy_from_user(ptr, len as usize) {
            Ok(bytes) => {
                shell::out(&alloc::string::String::from_utf8_lossy(&bytes), if fd == 2 { COLOR_ERROR } else { COLOR_TEXT });
                bytes.len() as i64
            }
            Err(e) => e,
        },
        Some(Fd::PipeOut(w)) => match proc.copy_from_user(ptr, len as usize) {
            Ok(bytes) => w.write(&bytes).map_or(EPIPE, |n| n as i64),
            Err(e) => e,
        },
        Some(Fd::PipeIn(_)) | None => EBADF,
    }
}

fn sys_read(fd: u64, ptr: u64, len: u64) -> i64 {
    if len as usize > process::MAX_COPY {
        return EINVAL;
    }
    let Some(proc) = sched::current_process() else { return EINVAL };
    let data = match proc.fd(fd) {
        Some(Fd::File(_)) => return file_io(fd, ptr, len, false),
        // A whole line from the keyboard (the Terminal's line editor does the
        // work); Ctrl+C is end-of-file.
        Some(Fd::Console) => match shell::read_line_for_user() {
            Some(line) => {
                let mut b = line.into_bytes();
                b.push(b'\n');
                b
            }
            None => alloc::vec::Vec::new(),
        },
        Some(Fd::PipeIn(r)) => r.read(len as usize),
        Some(Fd::PipeOut(_)) | None => return EBADF,
    };
    let mut data = data;
    data.truncate(len as usize);
    match proc.copy_to_user(ptr, &data) {
        Ok(()) => data.len() as i64,
        Err(e) => e,
    }
}

/// Reads a short string argument from user memory.
fn user_str(proc: &process::Process, ptr: u64, len: u64, max: u64) -> Result<alloc::string::String, i64> {
    if len > max {
        return Err(EINVAL);
    }
    let bytes = proc.copy_from_user(ptr, len as usize)?;
    core::str::from_utf8(&bytes).map(alloc::string::String::from).map_err(|_| EINVAL)
}

/// `spawn(command, stdin fd, stdout fd, flags)`: starts "PROGRAM ARGS..." as a
/// child. A descriptor of u64::MAX means "the console". Flag 1 = background
/// (not waited for; the kernel reports how it ends). Returns the pid.
fn sys_spawn(ptr: u64, len: u64, stdin: u64, stdout: u64, flags: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    let line = match user_str(&proc, ptr, len, 1024) {
        Ok(l) => l,
        Err(e) => return e,
    };
    let line = line.trim();
    let (spec, args) = line.split_once(' ').map_or((line, ""), |(a, b)| (a, b.trim()));
    if spec.is_empty() {
        return EINVAL;
    }
    let pick = |fd: u64| -> Result<Option<Fd>, i64> {
        if fd == u64::MAX {
            return Ok(None);
        }
        proc.fd(fd).map(Some).ok_or(EBADF)
    };
    let (stdin, stdout) = match (pick(stdin), pick(stdout)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    if !process::exists(spec) {
        return ENOENT;
    }
    match process::spawn_child(spec, args, stdin, stdout, flags & 1 != 0) {
        Ok(pid) => pid as i64,
        Err(e) => {
            shell::announce(&e, true);
            ENOMEM
        }
    }
}

/// `pipe(out)`: writes the read and write descriptors (two u64s) at `out`.
fn sys_pipe(out: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    let (r, w) = pipe::new();
    let Some(rfd) = proc.add_fd(Fd::PipeIn(r)) else { return -24 };
    let Some(wfd) = proc.add_fd(Fd::PipeOut(w)) else {
        proc.close_fd(rfd);
        return -24;
    };
    let mut rec = [0u8; 16];
    rec[..8].copy_from_slice(&rfd.to_le_bytes());
    rec[8..].copy_from_slice(&wfd.to_le_bytes());
    match proc.copy_to_user(out, &rec) {
        Ok(()) => 0,
        Err(e) => {
            proc.close_fd(rfd);
            proc.close_fd(wfd);
            e
        }
    }
}

fn sys_chdir(ptr: u64, len: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    let path = match user_path(&proc, ptr, len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match VFS.lock().stat(&path) {
        Ok(st) if st.kind == NodeKind::Dir => {
            let norm = nova_fs::path::components(&path).map(|c| nova_fs::path::join(&c)).unwrap_or(path);
            proc.set_cwd(&norm);
            0
        }
        Ok(_) => ENOTDIR,
        Err(e) => files::errno(e),
    }
}

fn sys_getcwd(ptr: u64, len: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    let mut bytes = proc.cwd().into_bytes();
    bytes.truncate((len as usize).min(process::MAX_COPY));
    match proc.copy_to_user(ptr, &bytes) {
        Ok(()) => bytes.len() as i64,
        Err(e) => e,
    }
}

/// `readline(prompt, buf)`: the Terminal's line editor, with history (what
/// the shell reads commands with). Returns the length, or -EINTR on Ctrl+C.
fn sys_readline(prompt_ptr: u64, prompt_len: u64, buf: u64, len: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    if !matches!(proc.fd(0), Some(Fd::Console)) {
        return EBADF;
    }
    let prompt = match user_str(&proc, prompt_ptr, prompt_len, 64) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let Some(line) = shell::read_command_line(&prompt) else { return EINTR };
    let mut bytes = line.into_bytes();
    bytes.truncate((len as usize).min(process::MAX_COPY));
    match proc.copy_to_user(buf, &bytes) {
        Ok(()) => bytes.len() as i64,
        Err(e) => e,
    }
}

/// `ctl(command)`: runs one of the kernel's system commands (disks, format,
/// mem, ps, ...) for the shell; it prints on the console.
fn sys_ctl(ptr: u64, len: u64) -> i64 {
    let Some(proc) = sched::current_process() else { return EINVAL };
    match user_str(&proc, ptr, len, 1024) {
        Ok(line) => {
            shell::execute(line.trim());
            0
        }
        Err(e) => e,
    }
}
