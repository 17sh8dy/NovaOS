# Nova Full Kernel: roadmap

Each stage ends with something that boots and is checked by `tools\test.ps1`.

| # | Stage | What "done" looks like |
|---|---|---|
| 1 | **Bootloader + handoff** ✅ | NovaBoot loads the ELF kernel, page tables + NX, exits UEFI; the kernel logs the memory map and draws the boot screen |
| 2 | CPU tables + exceptions ✅ | Own GDT + TSS (separate stacks for double fault / NMI), IDT with handlers that print a register dump; a deliberate fault in a test build is caught and reported, not a silent reset |
| 3 | Physical memory manager ✅ | Frame allocator over `Usable` (then reclaimed) regions; unit-tested on the host like `crates/paging` |
| 4 | Kernel virtual memory + heap ✅ (+ PS/2 mouse, pulled forward) | Kernel-owned page tables (drop the identity map), `alloc` support (Box, Vec) |
| 5 | Interrupts + time ✅ | ACPI (MADT) parsing, Local APIC + IOAPIC, timer tick, `hlt`-based idle |
| 6 | Keyboard + console ✅ | PS/2 keyboard (USB later), a scrolling text console on the framebuffer |
| 7 | Tasks ✅ | Kernel threads and a scheduler; then SMP (start the other cores) |
| 8 | User space ✅ | Ring 3, system calls, ELF programs (built into the kernel until stage 9 gives us a disk) |
| 9 | Storage + files | AHCI/NVMe driver, a filesystem, a VFS |
| 10 | Nova shell | First real Nova user interface running on Nova itself |

Later: USB (xHCI), networking, GPU, real-hardware testing on a USB stick.

## Stage 8 notes

* **Ring 3:** the GDT has user code/data segments (ring 3) and the SYSRET-compatible layout; the TSS's
  ring-0 stack and `PerCpu::kernel_rsp` are switched to each thread's kernel stack on every context
  switch. `SYSCALL` (`kernel/src/syscall.rs`) and interrupts/exceptions from user mode `swapgs` so the
  kernel's per-CPU `gs` is in place; the user's `gs` is restored on the way out. Returns use `iretq`
  (no SYSRET canonical-address pitfalls).
* **Processes** (`kernel/src/process.rs`): own page tables (kernel upper half shared, supervisor-only),
  ELF loader built on `crates/elf` (per-page permissions from the program headers, W^X enforced,
  addresses limited to 0x1000..0x1000_0000), 64 KiB stack with an unmapped guard page, `sbrk` heap.
  The scheduler switches CR3 per thread. On exit or kill, every frame and page table is returned
  (checked: free-frame count identical before and after ten more processes).
* **System calls:** exit, write, read (a keyboard line, via the shell's editor), getpid, yield,
  sleep_ms, uptime_ms, sbrk. Every user pointer is checked by walking the process's page tables
  (`copy_from_user` / `copy_to_user`): kernel addresses, unmapped pages and wrapping ranges give
  -EFAULT, never a kernel fault.
* **User faults** (#PF, #GP, #UD...) kill only the process and print why; the kernel carries on.
* **Programs** are `user/src/bin/*.rs` (no_std, tiny runtime in `user/src/lib.rs`), linked at 0x400000
  by `user/linker.ld`, built by `tools\build_user.ps1` (own RUSTFLAGS: the kernel's code model would
  break them) into `kernel/user_bin` (git-ignored) and embedded in the kernel (no file system yet).
  Shell: `progs`, `run NAME`, `spawn NAME...`, `ps`. Programs: hello, greet (reads the keyboard),
  counter, memtest (private memory), crash (killed), evil (bad syscalls, then `cli`: killed).
* `boot_test.py --user` runs all of that through the emulated keyboard, including two processes
  interleaving and the no-leak check.
* Gaps: no fork/exec/wait/pipes/signals, no file system (programs are built in), one thread per
  process, user stack does not grow, no ASLR, no resource limits besides the 64 MiB heap cap, new
  kernel PML4 slots created after a process exists are not seen by it, and an NMI landing between
  `swapgs` and `iretq` could run with the wrong `gs` (needs a safer NMI path).

## Stage 2 notes

* `kernel/src/cpu.rs` installs our GDT/TSS/IDT; `exceptions.rs` has the 32 stubs and
  the register dump; `crash.rs` draws it (serial + screen); pure logic (descriptor
  encoding, error-code decoding) is unit-tested in `crates/x86`.
* IST stacks: #DF, NMI, #MC each get 16 KiB. A kernel stack overflow hits the guard
  page and is reported as #DF on IST 1 (`tools\build.ps1 -Fault stack` shows it).
* Test kernels: `tools\build.ps1 -Fault div|ud|gp|pf|df|stack|nmi|panic`, then `run.ps1 -NoBuild`.
  `test.ps1` builds and boots all of them.
* Gaps: no backtrace in the dump (needs frame pointers); IST stacks have no guard pages
  until stage 4; vectors 32-255 are not-present gates (a stray interrupt shows up as #NP/#GP);
  no `swapgs` / ring-3 handling yet (stage 8).

## Stage 3 notes

* `crates/pmm` is a bitmap frame allocator (1 bit per 4 KiB frame, host-tested: 15 tests);
  `kernel/src/pmm.rs` is the global instance + a self-test.
* It frees `Usable` and `FirmwareReclaimable` memory above 1 MiB. Still held back: low memory
  (<1 MiB), `Loader` (page tables, boot info, stack, kernel image), ACPI (until parsed), runtime/MMIO.
* The bitmap lives in the first usable region above 1 MiB (8 KiB for 256 MiB, 256 KiB for 6 GiB).
* Boot self-test: allocates up to 512 MiB, writes an address pattern into each frame, verifies,
  frees, checks nothing leaked, then checks the boot info + memory map are untouched.
* Gaps: freeing a *reserved* frame is not detected (the bitmap can't tell reserved from
  allocated); no per-CPU caches / locking granularity yet; `Loader` memory is reclaimed in stage 4
  once the kernel has its own page tables; ACPI-reclaimable in stage 5.

## Stage 4 notes

* `kernel/src/vm.rs` builds the kernel's own page tables and switches CR3: direct map of all RAM
  (2 MiB pages, 4 KiB at ragged edges, NX), the framebuffer **write-combining** (PAT entry 1),
  the kernel image per section (text R-X, rodata R--, data RW-), the boot stack with its guard
  page. The bootloader's identity map is gone: a null or low pointer is a #PF.
* Before switching, vm.rs checks that the new tables map the running code, stack, boot data and
  frame bitmap, and refuses to switch otherwise (a miss would be a triple fault).
* `kernel/src/bootdata.rs` copies BootInfo + memory map into the kernel first, because they live in
  `Loader` memory, which is then reclaimed (old page tables, boot data, loader image) and verified
  by running the pmm self-test again afterwards.
* `crates/paging/src/dynamic.rs`: page-table builder over any `TableMemory` (frame allocator +
  direct map in the kernel, a Vec on the host); `crates/heap`: first-fit allocator with coalescing,
  randomised host test; `kernel/src/heap.rs`: the `#[global_allocator]`, grows by mapping frames at
  `0xFFFF_A000_0000_0000` (1 GiB window).
* **PS/2 mouse** (polled): `crates/ps2` decodes packets (3- or 4-byte, wheel) and tracks the pointer;
  `kernel/src/ps2.rs` talks to the i8042; `cursor.rs` draws an arrow with save-under. The keyboard
  port is left disabled (stage 6). `tools/boot_test.py --mouse` drives it through QEMU and checks the
  exact pointer position and button events.
* More fault kinds: `null`, `lowmem` (identity map really gone), `ro` (kernel code is write-protected),
  `nx` (heap is not executable).
* Gaps: heap never shrinks; no unmap/free of table pages; ACPI memory still reserved (stage 5); heap
  and IST stacks have no guard pages; the mouse is polled (busy loop) until interrupts exist; USB mice
  and keyboards need xHCI (later); real PCs with USB-only input may have no i8042 at all.

## Stage 5 notes

* `crates/acpi` (RSDP -> XSDT/RSDT -> MADT, checksum-verified, 7 host tests) and `crates/apic`
  (IO-APIC redirection entries, LAPIC registers, timer maths, 4 host tests); `kernel/src/acpi.rs`,
  `apic.rs`, `interrupts.rs` do the hardware side.
* All 256 vectors have stubs (generated by `kernel/build.rs`); 0-31 are faults, 32+ go to
  `interrupts::dispatch`, which runs the registered handler, then sends the EOI. Unowned vectors are
  counted and logged as strays; vector `0xFF` is the APIC spurious vector (no EOI).
* Legacy 8259 PICs are remapped out of the way and masked. The local APIC runs in xAPIC (MMIO) mode;
  its registers and the IO-APIC are mapped **uncached** (`vm::map_mmio`).
* The APIC timer is measured against the PIT once, then runs at 100 Hz. Boot self-test: 10 ticks
  (+-2) during a 100 ms PIT wait. The main loop sleeps with `sti; hlt`; `boot_test.py --mouse`
  checks ~100 wake-ups per second and 0 stray interrupts.
* Spinlocks now disable interrupts while held (a handler that logs while the interrupted code holds
  the log lock would deadlock a single CPU).
* The PS/2 mouse is interrupt driven (ISA IRQ 12 -> IO-APIC -> vector 0x2C) with a 64-event queue;
  ACPI interrupt-source overrides decide the GSI/polarity/trigger. Falls back to polling if there is
  no usable APIC.
* New fault kind `stray`: a software interrupt on an unowned vector is logged and boot continues.
* Gaps: boot CPU only (stage 7 starts the others); no x2APIC mode; no HPET/TSC clock yet (the tick
  counter is the only time source); ACPI tables stay in reserved memory (we keep using them);
  tables outside the direct map are reported as unreadable rather than mapped on demand; MSI/PCI
  interrupts and a real keyboard IRQ come with their drivers.

## Stage 6 notes

* `crates/kbd`: scancode set 1 decoder (E0 prefix, Pause, fake-shift bytes), US layout, shift / caps /
  ctrl / alt / num lock tracking, keypad; 12 host tests. `crates/console`: character grid (wrap,
  scroll, tab, backspace, per-row dirty tracking) and a line editor (cursor editing, kill line/word,
  history with draft); 15 host tests.
* `kernel/src/ps2.rs` now drives both devices: the controller's translation bit gives set 1; IRQ 1 and
  IRQ 12 share one handler that drains the single output buffer and routes bytes by the status bit.
  Each device is optional (a PC may have only one).
* `kernel/src/gui.rs`: compact splash + console panel + status bar + mouse pointer behind one lock;
  only changed console rows are repainted. `font.rs` got lowercase and the missing punctuation.
* `kernel/src/shell.rs`: the shell is a kernel thread that `park`s until the keyboard interrupt
  `unpark`s it (new in `sched.rs`; no lost wake-ups). Commands: help clear echo uptime mem cpus
  threads sleep version panic reboot halt. Every console line is mirrored to serial as `con: ...`.
* `tools/boot_test.py --shell` types through QEMU's keyboard (real PS/2 scancodes) and checks the
  replies, backspace editing, history (Up) and Ctrl+C.
* Gaps: US layout only, no key repeat control or LEDs, no scrollback (output that scrolls off is gone),
  single-row line editing, no clipboard/selection, USB keyboards need xHCI (later), the shell runs
  built-ins only (stage 8 adds programs).

## Stage 7 notes

* `kernel/src/sched.rs`: kernel threads (`spawn`, `join`, `yield_now`, `sleep_ms`, `exit`), each with a
  32 KiB stack and an unmapped guard page; preemptive round-robin from one ready queue, one tick
  (10 ms) per slice; idle CPUs `hlt` in a per-CPU idle thread. A thread that was just switched away
  from is only made visible to other CPUs by `finish_switch` on the *next* thread's side (it is still
  running on its own stack until then). Dead threads' stacks go to a pool; kernel mappings are never
  unmapped (that needs TLB shootdown IPIs, added when something needs it).
* `kernel/src/cpu.rs`: per-CPU GDT/TSS/IST stacks and a `PerCpu` record via the `gs` base
  (`this_cpu()`); the IDT is shared.
* `kernel/src/smp.rs`: real-mode trampoline copied to physical 0x8000 (real -> protected -> long
  mode on a temporary page table that identity-maps that page and shares the kernel's upper half),
  INIT + STARTUP IPIs one CPU at a time, each AP then switches to the kernel's tables, builds its own
  tables, starts its APIC timer at the boot CPU's measured rate and idles in the scheduler.
* Fatal errors stop the other CPUs with an NMI IPI first; reports name the CPU and thread.
* The IRQ dispatcher now sends the EOI *before* the handler (the timer handler may switch threads and
  not return for a while). Fine for edge-triggered interrupts; level-triggered ones need care later.
* Tests: `sched::self_test` (64 short threads twice with stack reuse, `sleep_ms(50)` = 5 ticks, twice
  as many never-yielding busy threads as CPUs: all must run, all CPUs must be used, atomic and
  spinlock counters must be exact); fault kinds `tdiv` / `tstack` inside a thread; QEMU runs with
  `-smp 4` (`NOVA_CPUS` / `run.ps1 -Cpus` / `boot_test.py --cpus`).
* Gaps: one global run queue and lock (no per-CPU queues, no CPU affinity, no priorities); no
  blocking primitives (mutex, condvar, channels) beyond yield/sleep/join; idle CPUs wait for their
  next tick instead of being woken by an IPI; xAPIC only; AP start-up is serial and gives up after
  one failure; not tried on real hardware (many cores, x2APIC, odd firmware).

## Known gaps in stage 1

* Not yet booted on a real PC (QEMU + OVMF only).
* The framebuffer and MMIO are mapped write-back; needs PAT/cache attributes.
* UEFI runtime services are left unmapped (no `SetVirtualAddressMap`).
* QEMU's TCG emulator crashes on this Windows build; boots use WHPX (see README).
