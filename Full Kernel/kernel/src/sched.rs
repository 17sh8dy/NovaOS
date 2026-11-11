//! Kernel threads and the scheduler.
//!
//! * **Threads** each have their own 32 KiB kernel stack (with an unmapped
//!   guard page below it, so an overflow is a reported page fault).
//! * **Scheduling** is preemptive round-robin from one global ready queue:
//!   every CPU's timer tick ends the running thread's time slice and picks the
//!   next ready thread; a CPU with nothing to run sleeps (`hlt`) in its idle
//!   thread and looks again on its next tick.
//! * **Switching** saves only the callee-saved registers and the stack pointer
//!   ([`switch_context`]); everything else was already saved by the interrupt
//!   entry or by the compiler at the call site.
//!
//! The one subtle rule: a thread that is being switched *away from* must not
//! become visible to other CPUs (ready queue, sleepers, or freed) until the
//! switch has finished, because until then it is still executing on its own
//! stack. So the leaving CPU only records "I just left thread X"
//! (`PerCpu::prev`), and the code that runs next on that CPU
//! ([`finish_switch`]) files X where it belongs.
//!
//! Kernel stacks are never unmapped: dead threads' stacks go to a pool and are
//! reused. (Unmapping on a multi-CPU kernel needs TLB shootdown IPIs; that
//! arrives when something needs to give address space back.)

use crate::cpu::{self, PerCpu};
use crate::interrupts::{self, TICK_HZ};
use crate::log::SpinLock;
use crate::{pmm, vm};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use nova_paging::{NO_EXECUTE, WRITABLE};

const STACK_SIZE: u64 = 32 * 1024;
const GUARD: u64 = 4096;
const STACK_SLOT: u64 = STACK_SIZE + GUARD;
/// Virtual window the kernel stacks live in (one slot = guard page + stack).
const STACK_BASE: u64 = 0xFFFF_B000_0000_0000;
const STACK_WINDOW: u64 = 1 << 30;

/// Timer ticks a thread may run before another ready thread takes over.
pub const SLICE_TICKS: u32 = 1;

const NONE: usize = usize::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ready,
    Running,
    Sleeping,
    Dead,
}

/// A kernel stack: `base` is the lowest mapped address; the guard page sits below it.
#[derive(Clone, Copy)]
pub struct Stack {
    base: u64,
}

impl Stack {
    pub fn top(&self) -> u64 {
        self.base + STACK_SIZE
    }
}

struct Thread {
    #[allow(dead_code)] // for debugging / future thread listings
    name: String,
    state: State,
    idle: bool,
    saved_rsp: u64,
    wake_tick: u64,
    stack: Option<Stack>,
    done: Arc<AtomicBool>,
}

struct Sched {
    threads: Vec<Option<Box<Thread>>>,
    free_ids: Vec<usize>,
    ready: VecDeque<usize>,
    sleepers: Vec<usize>,
    stack_pool: Vec<Stack>,
    next_slot: u64,
    stacks_created: usize,
}

impl Sched {
    const fn new() -> Self {
        Self {
            threads: Vec::new(),
            free_ids: Vec::new(),
            ready: VecDeque::new(),
            sleepers: Vec::new(),
            stack_pool: Vec::new(),
            next_slot: 0,
            stacks_created: 0,
        }
    }

    /// A stack from the pool, or a freshly mapped one.
    fn alloc_stack(&mut self) -> Option<Stack> {
        if let Some(s) = self.stack_pool.pop() {
            return Some(s);
        }
        let slot = self.next_slot * STACK_SLOT;
        if slot + STACK_SLOT > STACK_WINDOW {
            return None;
        }
        let base = STACK_BASE + slot + GUARD; // the page below `base` stays unmapped
        for page in (0..STACK_SIZE).step_by(4096) {
            let frame = pmm::alloc_frame()?;
            vm::map_page(base + page, frame, WRITABLE | NO_EXECUTE).ok()?;
        }
        self.next_slot += 1;
        self.stacks_created += 1;
        Some(Stack { base })
    }

    fn insert(&mut self, t: Thread) -> usize {
        if let Some(id) = self.free_ids.pop() {
            self.threads[id] = Some(Box::new(t));
            id
        } else {
            self.threads.push(Some(Box::new(t)));
            self.threads.len() - 1
        }
    }

    fn thread(&mut self, id: usize) -> &mut Thread {
        self.threads[id].as_mut().expect("scheduler: no such thread")
    }
}

static SCHED: SpinLock<Sched> = SpinLock::new(Sched::new());
/// Set once [`init`] has run; the timer tick does nothing before that.
static READY: AtomicBool = AtomicBool::new(false);
static SWITCHES: AtomicU64 = AtomicU64::new(0);

// ---- context switch ----------------------------------------------------

unsafe extern "sysv64" {
    /// Saves the callee-saved registers on the current stack, stores the stack
    /// pointer in `*old_rsp`, switches to `new_rsp` and restores from there.
    fn switch_context(old_rsp: *mut u64, new_rsp: u64);
    /// Where a brand-new thread's first `switch_context` returns to.
    fn nova_thread_start();
}

global_asm!(
    r#"
    .section .text.sched,"ax"
    .global switch_context
    switch_context:
        push rbp
        push rbx
        push r12
        push r13
        push r14
        push r15
        mov [rdi], rsp
        mov rsp, rsi
        pop r15
        pop r14
        pop r13
        pop r12
        pop rbx
        pop rbp
        ret

    .global nova_thread_start
    nova_thread_start:
        mov rdi, r12
        call {main}
        ud2
    "#,
    main = sym thread_main
);

/// First code a new thread runs (on its own stack, interrupts off).
extern "sysv64" fn thread_main(arg: u64) -> ! {
    finish_switch();
    nova_x86::sti();
    // SAFETY: `arg` is the `Box<Box<dyn FnOnce>>` that `spawn` leaked for us.
    let f = unsafe { Box::from_raw(arg as *mut Box<dyn FnOnce() + Send>) };
    (*f)();
    exit()
}

/// Builds the initial stack of a new thread so that the first switch to it
/// "returns" into `nova_thread_start` with `arg` in r12.
fn prepare_stack(stack: &Stack, arg: u64) -> u64 {
    // Layout, low to high: r15 r14 r13 r12 rbx rbp | return address | 2 pad qwords.
    // After the six pops and the `ret`, rsp = top - 16: 16-byte aligned as
    // the stub's `call` needs.
    let rsp = stack.top() - 9 * 8;
    // SAFETY: the stack is mapped and unused.
    unsafe {
        let frame = rsp as *mut u64;
        for i in 0..9 {
            frame.add(i).write(0);
        }
        frame.add(3).write(arg); // r12
        frame.add(6).write(nova_thread_start as *const () as usize as u64);
    }
    rsp
}

/// After a switch, on the CPU that did it: files the thread that was just
/// left (ready queue, sleepers, or freed if it exited).
fn finish_switch() {
    let prev = cpu::this_cpu().prev.swap(NONE, Ordering::SeqCst);
    if prev == NONE {
        return;
    }
    SCHED.with(|s| match s.thread(prev).state {
        State::Ready => s.ready.push_back(prev),
        State::Sleeping => s.sleepers.push(prev),
        State::Dead => {
            let t = s.threads[prev].take().expect("dead thread vanished");
            if let Some(stack) = t.stack {
                s.stack_pool.push(stack);
            }
            s.free_ids.push(prev);
        }
        State::Running => {}
    });
}

/// Picks the next thread for this CPU and switches to it. Interrupts must be off.
/// If the current thread can keep running and nothing else is ready, returns at once.
fn schedule() {
    let cpu: &PerCpu = cpu::this_cpu();
    let target = SCHED.with(|s| {
        let cur = cpu.current.load(Ordering::SeqCst);
        let cur_runnable = s.thread(cur).state == State::Running;
        let next = match s.ready.pop_front() {
            Some(n) => n,
            None if cur_runnable => return None,
            None => cpu.idle.load(Ordering::SeqCst),
        };
        if next == cur {
            return None;
        }
        // The thread we leave is filed by finish_switch once we are off its stack.
        if s.thread(cur).idle {
            cpu.prev.store(NONE, Ordering::SeqCst);
        } else {
            if cur_runnable {
                s.thread(cur).state = State::Ready;
            }
            cpu.prev.store(cur, Ordering::SeqCst);
        }
        s.thread(next).state = State::Running;
        cpu.current.store(next, Ordering::SeqCst);
        cpu.slice.store(0, Ordering::SeqCst);
        let old = &mut s.thread(cur).saved_rsp as *mut u64;
        let new = s.thread(next).saved_rsp;
        Some((old, new))
    });
    if let Some((old, new)) = target {
        SWITCHES.fetch_add(1, Ordering::Relaxed);
        // SAFETY: `old` points into a live Thread; `new` is a stack prepared by
        // prepare_stack or saved by a previous switch_context.
        unsafe { switch_context(old, new) };
        finish_switch();
    }
}

// ---- public API --------------------------------------------------------

/// Sets up scheduling on the boot CPU: the running code becomes thread 0
/// ("main") and a separate idle thread is created for the boot CPU.
pub fn init() {
    let cpu = cpu::this_cpu();
    let idle_id = SCHED.with(|s| {
        s.insert(Thread {
            name: String::from("main"),
            state: State::Running,
            idle: false,
            saved_rsp: 0,
            wake_tick: 0,
            stack: None,
            done: Arc::new(AtomicBool::new(false)),
        });
        create(s, "idle/0", true, Box::new(idle_loop)).expect("no memory for the boot CPU's idle thread").0
    });
    cpu.current.store(0, Ordering::SeqCst);
    cpu.idle.store(idle_id, Ordering::SeqCst);
    READY.store(true, Ordering::SeqCst);
}

/// Called on an application processor once it can take interrupts: its boot
/// context becomes that CPU's idle thread.
pub fn init_ap() {
    let cpu = cpu::this_cpu();
    let id = SCHED.with(|s| {
        s.insert(Thread {
            name: alloc::format!("idle/{}", cpu.id),
            state: State::Running,
            idle: true,
            saved_rsp: 0,
            wake_tick: 0,
            stack: None,
            done: Arc::new(AtomicBool::new(false)),
        })
    });
    cpu.current.store(id, Ordering::SeqCst);
    cpu.idle.store(id, Ordering::SeqCst);
}

fn create(s: &mut Sched, name: &str, idle: bool, f: Box<dyn FnOnce() + Send>) -> Option<(usize, Arc<AtomicBool>)> {
    let stack = s.alloc_stack()?;
    let arg = Box::into_raw(Box::new(f)) as u64;
    let done = Arc::new(AtomicBool::new(false));
    let id = s.insert(Thread {
        name: String::from(name),
        state: if idle { State::Running } else { State::Ready },
        idle,
        saved_rsp: prepare_stack(&stack, arg),
        wake_tick: 0,
        stack: Some(stack),
        done: done.clone(),
    });
    if !idle {
        s.ready.push_back(id);
    }
    Some((id, done))
}

/// Waits for a thread to finish.
pub struct JoinHandle {
    done: Arc<AtomicBool>,
}

impl JoinHandle {
    pub fn is_finished(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    pub fn join(self) {
        while !self.is_finished() {
            yield_now();
            core::hint::spin_loop();
        }
    }
}

/// Starts a new kernel thread running `f`. It becomes ready immediately and
/// may run on any CPU.
pub fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Option<JoinHandle> {
    let r = SCHED.with(|s| create(s, name, false, Box::new(f)));
    r.map(|(_, done)| JoinHandle { done })
}

/// Gives the CPU to another ready thread, if there is one.
pub fn yield_now() {
    if !READY.load(Ordering::SeqCst) {
        return;
    }
    let were_on = nova_x86::interrupts_enabled();
    nova_x86::cli();
    schedule();
    if were_on {
        nova_x86::sti();
    }
}

/// Sleeps for at least `ms` milliseconds (rounded up to whole timer ticks).
pub fn sleep_ms(ms: u64) {
    let ticks = (ms * TICK_HZ as u64).div_ceil(1000).max(1);
    let were_on = nova_x86::interrupts_enabled();
    nova_x86::cli();
    let wake = interrupts::ticks() + ticks;
    let cpu = cpu::this_cpu();
    SCHED.with(|s| {
        let t = s.thread(cpu.current.load(Ordering::SeqCst));
        t.state = State::Sleeping;
        t.wake_tick = wake;
    });
    schedule(); // not runnable: switches to another thread or this CPU's idle thread
    if were_on {
        nova_x86::sti();
    }
}

/// Ends the calling thread.
pub fn exit() -> ! {
    nova_x86::cli();
    let cpu = cpu::this_cpu();
    SCHED.with(|s| {
        let t = s.thread(cpu.current.load(Ordering::SeqCst));
        t.state = State::Dead;
        t.done.store(true, Ordering::SeqCst);
    });
    schedule();
    unreachable!("a dead thread was scheduled again");
}

fn idle_loop() {
    loop {
        nova_x86::sti_hlt();
    }
}

/// Wakes sleepers whose time has come (called from the boot CPU's tick).
pub fn wake_sleepers(now: u64) {
    if !READY.load(Ordering::SeqCst) {
        return;
    }
    SCHED.with(|s| {
        let mut i = 0;
        while i < s.sleepers.len() {
            let id = s.sleepers[i];
            if s.thread(id).wake_tick <= now {
                s.sleepers.swap_remove(i);
                s.thread(id).state = State::Ready;
                s.ready.push_back(id);
            } else {
                i += 1;
            }
        }
    });
}

/// The per-CPU timer tick: ends the time slice and preempts if someone is waiting.
/// Runs in the timer interrupt handler.
pub fn tick() {
    if !READY.load(Ordering::SeqCst) {
        return;
    }
    let cpu = cpu::this_cpu();
    let cur = cpu.current.load(Ordering::SeqCst);
    if cur == NONE {
        return;
    }
    let used = cpu.slice.fetch_add(1, Ordering::SeqCst) + 1;
    let waiting = SCHED.with(|s| !s.ready.is_empty());
    if waiting && (cur == cpu.idle.load(Ordering::SeqCst) || used >= SLICE_TICKS) {
        schedule();
    }
}

/// The id of the thread running on this CPU (for logs and crash reports).
#[allow(dead_code)]
pub fn current_id() -> usize {
    cpu::this_cpu().current.load(Ordering::SeqCst)
}

/// A stack for a CPU's own boot context (application processors run on one
/// before they have a scheduler thread).
pub fn alloc_cpu_stack() -> Option<Stack> {
    SCHED.with(|s| s.alloc_stack())
}

pub struct Stats {
    pub threads: usize,
    pub stacks_created: usize,
    pub stacks_pooled: usize,
    pub switches: u64,
}

pub fn stats() -> Stats {
    SCHED.with(|s| Stats {
        threads: s.threads.iter().flatten().count(),
        stacks_created: s.stacks_created,
        stacks_pooled: s.stack_pool.len(),
        switches: SWITCHES.load(Ordering::Relaxed),
    })
}

// ---- self-test ---------------------------------------------------------

static STOP: AtomicBool = AtomicBool::new(false);
static SHARED: AtomicU64 = AtomicU64::new(0);
static LOCKED: SpinLock<u64> = SpinLock::new(0);
static CPU_MASK: AtomicU64 = AtomicU64::new(0);

pub struct SelfTest {
    pub spinners: usize,
    pub cpus_used: u32,
    pub switches: u64,
    pub sleep_ticks: u64,
    pub stacks_created: usize,
}

/// Exercises threads, preemption, sleeping, locking and (if `cpus` > 1)
/// parallel execution. `cpus` is how many CPUs are online.
pub fn self_test(cpus: usize) -> Result<SelfTest, &'static str> {
    let switches_before = stats().switches;

    // 1. Many short threads: spawn, run, join; their stacks must be reused.
    let sum = Arc::new(AtomicU64::new(0));
    let mut created_after_wave = [0usize; 2];
    for wave in created_after_wave.iter_mut() {
        let handles: Vec<JoinHandle> = (0..64u64)
            .map(|i| {
                let sum = sum.clone();
                spawn("short", move || {
                    sum.fetch_add(i + 1, Ordering::SeqCst);
                })
                .ok_or("could not spawn a thread")
            })
            .collect::<Result<_, _>>()?;
        for h in handles {
            h.join();
        }
        *wave = stats().stacks_created;
    }
    if sum.load(Ordering::SeqCst) != 2 * (64 * 65 / 2) {
        return Err("short threads did not all run exactly once");
    }
    if created_after_wave[1] > created_after_wave[0] + 8 {
        return Err("dead threads' stacks are not being reused");
    }

    // 2. Sleeping: 50 ms must be about 5 ticks.
    let t0 = interrupts::ticks();
    sleep_ms(50);
    let sleep_ticks = interrupts::ticks() - t0;
    if !(5..=7).contains(&sleep_ticks) {
        return Err("sleep_ms(50) did not take about 50 ms");
    }

    // 3. Preemption + parallelism: twice as many busy threads as CPUs, none of
    //    which ever yields. Each must still get to run (time slicing), all
    //    CPUs must have been used, and the shared counters must be exact.
    STOP.store(false, Ordering::SeqCst);
    SHARED.store(0, Ordering::SeqCst);
    CPU_MASK.store(0, Ordering::SeqCst);
    LOCKED.with(|c| *c = 0);
    let n = (cpus * 2).max(2);
    let counts: Vec<Arc<AtomicU64>> = (0..n).map(|_| Arc::new(AtomicU64::new(0))).collect();
    let locked_counts: Vec<Arc<AtomicU64>> = (0..n).map(|_| Arc::new(AtomicU64::new(0))).collect();
    let mut handles = Vec::new();
    for i in 0..n {
        let (count, locked) = (counts[i].clone(), locked_counts[i].clone());
        handles.push(
            spawn("spinner", move || {
                let (mut local, mut locked_local) = (0u64, 0u64);
                while !STOP.load(Ordering::Relaxed) {
                    local += 1;
                    SHARED.fetch_add(1, Ordering::SeqCst);
                    if local % 256 == 0 {
                        LOCKED.with(|c| *c += 1);
                        locked_local += 1;
                        CPU_MASK.fetch_or(1u64 << cpu::this_cpu().id, Ordering::SeqCst);
                    }
                }
                count.store(local, Ordering::SeqCst);
                locked.store(locked_local, Ordering::SeqCst);
            })
            .ok_or("could not spawn a thread")?,
        );
    }
    sleep_ms(400);
    STOP.store(true, Ordering::SeqCst);
    for h in handles {
        h.join();
    }
    let total: u64 = counts.iter().map(|c| c.load(Ordering::SeqCst)).sum();
    let locked_total: u64 = locked_counts.iter().map(|c| c.load(Ordering::SeqCst)).sum();
    if counts.iter().any(|c| c.load(Ordering::SeqCst) == 0) {
        return Err("a busy thread never ran: no preemption");
    }
    if total != SHARED.load(Ordering::SeqCst) {
        return Err("atomic counter lost updates across threads/CPUs");
    }
    if locked_total != LOCKED.with(|c| *c) {
        return Err("spinlock-protected counter lost updates across threads/CPUs");
    }
    crate::kprintln!("sched: busy threads ran on CPUs (bit mask) {:#b}", CPU_MASK.load(Ordering::SeqCst));
    let cpus_used = CPU_MASK.load(Ordering::SeqCst).count_ones();
    if cpus_used as usize != cpus {
        return Err("busy threads did not run on every online CPU");
    }

    let s = stats();
    Ok(SelfTest { spinners: n, cpus_used, switches: s.switches - switches_before, sleep_ticks, stacks_created: s.stacks_created })
}
