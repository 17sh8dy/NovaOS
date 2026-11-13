//! User processes: an address space of their own, an ELF program loaded
//! into it, and one kernel thread that enters ring 3.
//!
//! Address space layout (lower half, private to the process):
//!
//! | Range | What |
//! |---|---|
//! | `0x40_0000 ..` | the program, each page mapped per its ELF flags (W^X enforced) |
//! | `0x1000_0000 ..` | the heap, grown by `sbrk` |
//! | below `0x7FFF_FFFF_F000` | a 64 KiB stack (the page below it stays unmapped) |
//!
//! The upper half (the kernel) is shared by every address space, mapped
//! supervisor-only, so a process can never touch it.
//!
//! When a process ends (or faults) its frames and page tables are returned to
//! the frame allocator.

use crate::log::SpinLock;
use crate::{pipe, pmm, sched, shell, vm};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use nova_elf::{Elf, PF_W, PF_X};
use nova_fs::vfs::Handle;
use nova_paging::dynamic::{AddressSpace, TableMemory};
use nova_paging::{NO_EXECUTE, PAGE_SIZE, USER, WRITABLE};
use nova_x86::exception as exc;
use nova_x86::gdt::{USER_CS, USER_DS};

/// One past the highest user address (the top of the canonical lower half).
const USER_LIMIT: u64 = 0x0000_8000_0000_0000;
const STACK_TOP: u64 = 0x0000_7FFF_FFFF_F000;
const STACK_PAGES: u64 = 16;
const HEAP_START: u64 = 0x1000_0000;
const HEAP_MAX: u64 = 64 << 20;
/// The biggest single write/read a system call will copy.
pub const MAX_COPY: usize = 64 * 1024;

/// Where a process's page tables and data frames come from, and a record of
/// them all so they can be given back.
struct ProcMemory {
    direct: u64,
    tables: Vec<u64>,
    frames: Vec<u64>,
}

impl ProcMemory {
    /// A zeroed data frame owned by this process.
    fn alloc_frame(&mut self) -> Option<u64> {
        let f = pmm::alloc_frame()?;
        // SAFETY: a frame we own, in the direct map.
        unsafe { core::ptr::write_bytes((self.direct + f) as *mut u8, 0, 4096) };
        self.frames.push(f);
        Some(f)
    }

    /// Gives every frame and table back to the frame allocator.
    fn release(&mut self) {
        for f in self.frames.drain(..).chain(self.tables.drain(..)) {
            let _ = pmm::free_frame(f);
        }
    }
}

impl TableMemory for ProcMemory {
    fn alloc_table(&mut self) -> Option<u64> {
        let f = pmm::alloc_frame()?;
        // SAFETY: a frame we own, in the direct map.
        unsafe { core::ptr::write_bytes((self.direct + f) as *mut u8, 0, 4096) };
        self.tables.push(f);
        Some(f)
    }

    fn table(&mut self, phys: u64) -> &mut [u64; 512] {
        // SAFETY: table frames are RAM inside the direct map, edited only through the locked space.
        unsafe { &mut *((self.direct + phys) as *mut [u64; 512]) }
    }
}

/// An open file: where it is and how far we have read or written.
#[derive(Clone, Copy)]
pub struct OpenFile {
    pub handle: Handle,
    pub offset: u64,
    pub writable: bool,
}

/// What a file descriptor refers to.
#[derive(Clone)]
pub enum Fd {
    /// The Terminal: reads are typed lines, writes appear on screen.
    Console,
    File(OpenFile),
    /// An end of a pipe; the end closes when its last descriptor does.
    PipeIn(Arc<pipe::Reader>),
    PipeOut(Arc<pipe::Writer>),
}

/// Descriptors 0, 1 and 2 (stdin, stdout, stderr) start as the console.
const MAX_FDS: usize = 64;

pub struct Process {
    pub pid: usize,
    pub name: String,
    /// The command-line arguments (one string; the program splits it).
    pub args: String,
    /// The working directory relative paths are resolved against.
    cwd: SpinLock<String>,
    /// File descriptors, indexed by number.
    fds: SpinLock<Vec<Option<Fd>>>,
    /// Whether its parent will `wait` for it (then the parent reports how it
    /// ended, not the kernel).
    waited: AtomicBool,
    root: u64,
    space: SpinLock<Option<AddressSpace<ProcMemory>>>,
    /// The current end of the heap (the next `sbrk` starts here).
    brk: AtomicU64,
    exit_code: AtomicI64,
}

impl Process {
    pub fn root(&self) -> u64 {
        self.root
    }

    pub fn cwd(&self) -> String {
        self.cwd.with(|c| c.clone())
    }

    pub fn set_cwd(&self, dir: &str) {
        self.cwd.with(|c| *c = String::from(dir));
    }

    /// Puts `fd` in the lowest free descriptor and returns its number, or None if the table is full.
    pub fn add_fd(&self, fd: Fd) -> Option<u64> {
        self.fds.with(|t| {
            let slot = match t.iter().position(|f| f.is_none()) {
                Some(i) => i,
                None if t.len() < MAX_FDS => {
                    t.push(None);
                    t.len() - 1
                }
                None => return None,
            };
            t[slot] = Some(fd);
            Some(slot as u64)
        })
    }

    pub fn add_file(&self, file: OpenFile) -> Option<u64> {
        self.add_fd(Fd::File(file))
    }

    /// What descriptor `fd` refers to.
    pub fn fd(&self, fd: u64) -> Option<Fd> {
        self.fds.with(|t| t.get(fd as usize).cloned().flatten())
    }

    /// A copy of the open file `fd`, if it is a file.
    pub fn file(&self, fd: u64) -> Option<OpenFile> {
        match self.fd(fd)? {
            Fd::File(f) => Some(f),
            _ => None,
        }
    }

    /// Replaces the saved state of open file `fd` (e.g. after a read moved the offset).
    pub fn update_file(&self, fd: u64, f: impl FnOnce(&mut OpenFile)) {
        self.fds.with(|t| {
            if let Some(Some(Fd::File(file))) = t.get_mut(fd as usize) {
                f(file);
            }
        });
    }

    pub fn close_fd(&self, fd: u64) -> bool {
        let old = self.fds.with(|t| t.get_mut(fd as usize).and_then(|f| f.take()));
        // Dropped outside the table's lock: closing a pipe end may wake someone.
        old.is_some()
    }

    /// Closes everything (the process is ending).
    fn close_all(&self) {
        let all = self.fds.with(core::mem::take);
        drop(all);
    }

    /// Checks that `[addr, addr+len)` is user memory that is mapped with the
    /// needed permission, then calls `f(direct-map pointer, chunk length)` for
    /// each page-sized piece. Fails (-EFAULT) on anything else: kernel
    /// addresses, unmapped pages, wrap-around.
    fn walk_user(&self, addr: u64, len: usize, write: bool, mut f: impl FnMut(*mut u8, usize)) -> Result<(), i64> {
        const EFAULT: i64 = -14;
        let end = addr.checked_add(len as u64).ok_or(EFAULT)?;
        if len == 0 {
            return Ok(());
        }
        if addr < PAGE_SIZE || end > USER_LIMIT {
            return Err(EFAULT);
        }
        let direct = crate::bootdata::info().physical_memory_offset;
        self.space.with(|space| {
            let space = space.as_mut().ok_or(EFAULT)?;
            // Check everything first, so a bad range copies nothing.
            let mut page = addr & !(PAGE_SIZE - 1);
            while page < end {
                let (_, flags) = space.translate(page).ok_or(EFAULT)?;
                if flags & USER == 0 || (write && flags & WRITABLE == 0) {
                    return Err(EFAULT);
                }
                page += PAGE_SIZE;
            }
            let mut cur = addr;
            while cur < end {
                let (phys, _) = space.translate(cur).ok_or(EFAULT)?;
                let chunk = ((PAGE_SIZE - (cur & (PAGE_SIZE - 1))).min(end - cur)) as usize;
                f((direct + phys) as *mut u8, chunk);
                cur += chunk as u64;
            }
            Ok(())
        })
    }

    pub fn copy_from_user(&self, addr: u64, len: usize) -> Result<Vec<u8>, i64> {
        let mut out = Vec::with_capacity(len);
        // SAFETY: walk_user hands out pointers to mapped user frames of the given length.
        self.walk_user(addr, len, false, |p, n| out.extend_from_slice(unsafe { core::slice::from_raw_parts(p, n) }))?;
        Ok(out)
    }

    pub fn copy_to_user(&self, addr: u64, data: &[u8]) -> Result<(), i64> {
        let mut off = 0;
        // SAFETY: as above; the pointers are writable user frames (checked).
        self.walk_user(addr, data.len(), true, |p, n| {
            unsafe { core::ptr::copy_nonoverlapping(data[off..].as_ptr(), p, n) };
            off += n;
        })
    }

    /// Grows the heap by `bytes` (rounded up to pages) with zeroed memory and
    /// returns where the new memory starts, or -ENOMEM / -EINVAL.
    pub fn sbrk(&self, bytes: u64) -> Result<u64, i64> {
        const ENOMEM: i64 = -12;
        let bytes = bytes.checked_add(PAGE_SIZE - 1).ok_or(ENOMEM)? & !(PAGE_SIZE - 1);
        let old = self.brk.load(Ordering::SeqCst);
        if old.checked_add(bytes).is_none_or(|new| new > HEAP_START + HEAP_MAX) {
            return Err(ENOMEM);
        }
        let nx = if crate::bootdata::info().nx_enabled != 0 { NO_EXECUTE } else { 0 };
        self.space.with(|space| {
            let space = space.as_mut().ok_or(ENOMEM)?;
            for off in (0..bytes).step_by(PAGE_SIZE as usize) {
                let frame = space.memory().alloc_frame().ok_or(ENOMEM)?;
                space.map_4k(old + off, frame, USER | WRITABLE | nx).map_err(|_| ENOMEM)?;
            }
            Ok::<(), i64>(())
        })?;
        self.brk.store(old + bytes, Ordering::SeqCst);
        Ok(old)
    }

    /// Tears the address space down (frames and tables back to the allocator).
    fn destroy_space(&self) {
        if let Some(mut space) = self.space.with(|s| s.take()) {
            space.memory().release();
        }
    }
}

static PROCS: SpinLock<Vec<Arc<Process>>> = SpinLock::new(Vec::new());
static NEXT_PID: AtomicUsize = AtomicUsize::new(1);

/// (pid, name) of every live process.
pub fn list() -> Vec<(usize, String)> {
    PROCS.with(|p| p.iter().map(|p| (p.pid, p.name.clone())).collect())
}

/// Builds a process from an ELF image: new address space (sharing the
/// kernel's upper half), program pages, heap bookkeeping and stack.
fn create(name: &str, image: &[u8], args: &str, cwd: &str) -> Result<(Arc<Process>, u64, u64), String> {
    let elf = Elf::parse(image).map_err(|e| alloc::format!("not a valid program: {e}"))?;
    // Policy: user code lives in the lower half, above page 0, below the stack,
    // and no page may be both writable and executable.
    let (lo, hi) = elf.load_span();
    if lo < 0x1000 || hi > HEAP_START {
        return Err(alloc::format!("program wants addresses {lo:#x}..{hi:#x}; allowed: 0x1000..{HEAP_START:#x}"));
    }
    for seg in elf.segments() {
        if seg.writable() && seg.executable() {
            return Err(String::from("program has a writable+executable segment (W^X)"));
        }
    }

    let direct = crate::bootdata::info().physical_memory_offset;
    let nx = if crate::bootdata::info().nx_enabled != 0 { NO_EXECUTE } else { 0 };
    let mem = ProcMemory { direct, tables: Vec::new(), frames: Vec::new() };
    let mut space = AddressSpace::new(mem).map_err(|_| String::from("out of memory (root table)"))?;
    let root = space.root();

    // Share the kernel's upper half.
    // SAFETY: the kernel root is a valid PML4 in the direct map; `root` is our fresh table.
    unsafe {
        let kernel = (direct + vm::root()) as *const u64;
        let mine = space.memory().table(root);
        for (i, slot) in mine.iter_mut().enumerate().skip(256) {
            *slot = kernel.add(i).read();
        }
    }

    let fail = |space: &mut AddressSpace<ProcMemory>, msg: &str| -> String {
        space.memory().release();
        String::from(msg)
    };

    // The program's pages.
    let mut page = lo;
    while page < hi {
        let pf = elf.page_flags(page);
        let flags = USER | if pf & PF_W != 0 { WRITABLE } else { 0 } | if pf & PF_X != 0 { 0 } else { nx };
        let Some(frame) = space.memory().alloc_frame() else { return Err(fail(&mut space, "out of memory (program pages)")) };
        if space.map_4k(page, frame, flags).is_err() {
            return Err(fail(&mut space, "could not map a program page"));
        }
        // Copy the file bytes that land on this page (the rest stays zero: bss).
        for seg in elf.segments() {
            let (seg_lo, seg_hi) = (seg.vaddr, seg.vaddr + seg.filesz);
            let (from, to) = (seg_lo.max(page), seg_hi.min(page + PAGE_SIZE));
            if from < to {
                let data = elf.segment_data(&seg);
                let src = &data[(from - seg_lo) as usize..(to - seg_lo) as usize];
                // SAFETY: `frame` is ours and in the direct map; the range lies within the page.
                unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), (direct + frame + (from - page)) as *mut u8, src.len()) };
            }
        }
        page += PAGE_SIZE;
    }

    // The stack: STACK_PAGES pages below STACK_TOP (the page below them stays unmapped).
    for i in 1..=STACK_PAGES {
        let Some(frame) = space.memory().alloc_frame() else { return Err(fail(&mut space, "out of memory (stack)")) };
        if space.map_4k(STACK_TOP - i * PAGE_SIZE, frame, USER | WRITABLE | nx).is_err() {
            return Err(fail(&mut space, "could not map the stack"));
        }
    }

    let pid = NEXT_PID.fetch_add(1, Ordering::SeqCst);
    let proc = Arc::new(Process {
        pid,
        name: String::from(name),
        args: String::from(args),
        cwd: SpinLock::new(String::from(cwd)),
        fds: SpinLock::new(alloc::vec![Some(Fd::Console), Some(Fd::Console), Some(Fd::Console)]),
        waited: AtomicBool::new(false),
        root,
        space: SpinLock::new(Some(space)),
        brk: AtomicU64::new(HEAP_START),
        exit_code: AtomicI64::new(0),
    });
    // The initial stack pointer looks like just after a `call`: 8 mod 16.
    Ok((proc, elf.entry, STACK_TOP - 8))
}

/// Finds the program image for `spec`: a path (anything with a `/`), else a
/// built-in program, else `/bin/NAME` on the file system.
fn load_image(spec: &str, cwd: &str) -> Result<(String, Vec<u8>), String> {
    if spec.contains('/') {
        let path = crate::files::absolute(cwd, spec);
        let data = crate::files::VFS.lock().read_file(&path).map_err(|e| alloc::format!("{path}: {e}"))?;
        let name = path.rsplit('/').next().unwrap_or(&path);
        return Ok((String::from(name), data));
    }
    if let Some(image) = crate::programs::find(spec) {
        return Ok((String::from(spec), image.to_vec()));
    }
    let path = alloc::format!("/bin/{spec}");
    match crate::files::VFS.lock().read_file(&path) {
        Ok(data) => Ok((String::from(spec), data)),
        Err(_) => Err(alloc::format!("no such program '{spec}' (try 'progs', or give a path)")),
    }
}

/// Is there a program called `spec` (see [`load_image`])?
pub fn exists(spec: &str) -> bool {
    load_image(spec, &shell::cwd()).is_ok()
}

/// Starts a program (see [`load_image`]) as a new process with `args`.
/// Returns its pid and a handle to wait for it.
pub fn spawn(spec: &str, args: &str) -> Result<(usize, sched::JoinHandle), String> {
    spawn_with(spec, args, None, None, false).map(|(p, h)| (p.pid, h))
}

/// Children a process will `wait` for: (parent pid, child, its thread).
static WAITABLE: SpinLock<Vec<(usize, Arc<Process>, sched::JoinHandle)>> = SpinLock::new(Vec::new());

/// Starts a program with the given standard input and output (default: the
/// console). `waited`: someone will collect its exit code (and report it), so
/// the kernel does not announce its exit.
pub fn spawn_with(spec: &str, args: &str, stdin: Option<Fd>, stdout: Option<Fd>, waited: bool) -> Result<(Arc<Process>, sched::JoinHandle), String> {
    let cwd = shell::cwd();
    let (name, image) = load_image(spec, &cwd)?;
    let name = name.as_str();
    let (proc, entry, sp) = create(name, &image, args, &cwd)?;
    proc.fds.with(|t| {
        if let Some(fd) = stdin {
            t[0] = Some(fd);
        }
        if let Some(fd) = stdout {
            t[1] = Some(fd);
        }
    });
    proc.waited.store(waited, Ordering::SeqCst);
    let pid = proc.pid;
    PROCS.with(|p| p.push(proc.clone()));
    let handle = sched::spawn_user(name, proc.clone(), move || enter_user(entry, sp));
    match handle {
        Some(h) => Ok((proc, h)),
        None => {
            proc.close_all();
            PROCS.with(|p| p.retain(|q| q.pid != pid));
            proc.destroy_space();
            Err(String::from("could not start a thread for the process"))
        }
    }
}

/// The exit code of a process that has ended.
pub fn exit_code(p: &Process) -> i64 {
    p.exit_code.load(Ordering::SeqCst)
}

/// Starts a child of the calling process. Unless it runs in the background,
/// the caller collects it with [`wait`]. Returns the pid.
pub fn spawn_child(spec: &str, args: &str, stdin: Option<Fd>, stdout: Option<Fd>, background: bool) -> Result<usize, String> {
    let (child, handle) = spawn_with(spec, args, stdin, stdout, !background)?;
    let pid = child.pid;
    if !background {
        // We hold the child and its thread, so this works even if it has already ended.
        let me = sched::current_process().map_or(0, |p| p.pid);
        WAITABLE.with(|w| w.push((me, child, handle)));
    }
    Ok(pid)
}

/// Waits for child `pid` of the calling process to end; returns its exit code (or -ECHILD).
pub fn wait(pid: usize) -> i64 {
    const ECHILD: i64 = -10;
    let me = sched::current_process().map_or(0, |p| p.pid);
    let entry = WAITABLE.with(|w| {
        let i = w.iter().position(|(parent, c, _)| *parent == me && c.pid == pid)?;
        Some(w.swap_remove(i))
    });
    let Some((_, child, handle)) = entry else { return ECHILD };
    handle.join_sleeping();
    exit_code(&child)
}

global_asm!(
    r#"
    .section .text.process,"ax"
    .global nova_enter_user
    // nova_enter_user(rip: rdi, rsp: rsi) -> !: drop to ring 3 with an iretq frame.
    nova_enter_user:
        cli
        swapgs
        push {ss}
        push rsi
        push 0x202
        push {cs}
        push rdi
        xor eax, eax
        xor ebx, ebx
        xor ecx, ecx
        xor edx, edx
        xor esi, esi
        xor edi, edi
        xor ebp, ebp
        xor r8d, r8d
        xor r9d, r9d
        xor r10d, r10d
        xor r11d, r11d
        xor r12d, r12d
        xor r13d, r13d
        xor r14d, r14d
        xor r15d, r15d
        iretq
    "#,
    ss = const USER_DS as u32,
    cs = const USER_CS as u32,
);

unsafe extern "sysv64" {
    fn nova_enter_user(rip: u64, rsp: u64) -> !;
}

fn enter_user(entry: u64, sp: u64) -> ! {
    // SAFETY: entry/sp describe the freshly built user image; the thread is
    // already running on the process's page tables.
    unsafe { nova_enter_user(entry, sp) }
}

/// Ends the calling process: reports it, gives its memory back, and ends the thread.
pub fn exit_current(code: i64) -> ! {
    nova_x86::cli();
    let proc = sched::current_process().expect("exit_current outside a process");
    if !proc.waited.load(Ordering::SeqCst) {
        shell::announce(&alloc::format!("[process {} ({}) exited with code {code}]", proc.pid, proc.name), false);
    }
    proc.exit_code.store(code, Ordering::SeqCst);
    finish(&proc)
}

/// A fault in user mode: kill the process and say why. The kernel carries on.
pub fn user_fault(f: &crate::exceptions::ExceptionFrame) -> ! {
    let vector = f.vector as u8;
    let (mnemonic, description) = exc::name(vector);
    let proc = sched::current_process();
    let (pid, name) = proc.as_ref().map_or((0, "?"), |p| (p.pid, p.name.as_str()));
    let extra = if vector == exc::PAGE_FAULT { alloc::format!(", address {:#x}", nova_x86::read_cr2()) } else { String::new() };
    shell::announce(&alloc::format!("[process {pid} ({name}) killed: {mnemonic} {description} at {:#x}{extra}]", f.rip), true);
    match proc {
        Some(p) => {
            p.exit_code.store(128 + vector as i64, Ordering::SeqCst);
            finish(&p)
        }
        None => nova_x86::halt_forever(),
    }
}

/// Common tail of every way a process can end. Interrupts are off, so this
/// thread cannot be preempted into the page tables it is about to free.
fn finish(proc: &Arc<Process>) -> ! {
    // Back onto the kernel's own page tables before freeing the process's.
    let kroot = vm::root();
    crate::cpu::this_cpu().cr3.store(kroot, Ordering::SeqCst);
    // SAFETY: the kernel root maps everything the kernel is running on.
    unsafe { nova_x86::write_cr3(kroot) };
    proc.destroy_space();
    PROCS.with(|p| p.retain(|q| q.pid != proc.pid));
    // Children nobody will wait for any more.
    let orphans: Vec<_> = WAITABLE.with(|w| {
        let (gone, keep): (Vec<_>, Vec<_>) = core::mem::take(w).into_iter().partition(|(parent, ..)| *parent == proc.pid);
        *w = keep;
        gone
    });
    // Interrupts are off here, and closing pipe ends may wake threads: that's fine
    // (unpark only queues them).
    proc.close_all();
    drop(orphans);
    sched::exit()
}

// Keep Box in scope for the closure types used above.
#[allow(dead_code)]
type _Unused = Box<()>;
