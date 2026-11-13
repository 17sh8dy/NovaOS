# Nova Full Kernel: roadmap

Each stage ends with something that boots and is checked by `tools\test.ps1`.

| # | Stage | What "done" looks like |
|---|---|---|
| 1 | **Bootloader + handoff** ✅ | NovaBoot loads the ELF kernel, page tables + NX, exits UEFI; the kernel logs the memory map and draws the boot screen |
| 2 | CPU tables + exceptions ✅ | Own GDT + TSS (separate stacks for double fault / NMI), IDT with handlers that print a register dump; a deliberate fault in a test build is caught and reported, not a silent reset |
| 3 | Physical memory manager ✅ | Frame allocator over `Usable` (then reclaimed) regions; unit-tested on the host like `crates/paging` |
| 4 | Kernel virtual memory + heap ✅ (+ PS/2 mouse, pulled forward) | Kernel-owned page tables (drop the identity map), `alloc` support (Box, Vec) |
| 5 | Interrupts + time ✅ | ACPI (MADT) parsing, Local APIC + IOAPIC, timer tick, `hlt`-based idle |
| 6 | Keyboard + console ✅ | PS/2 keyboard (USB keyboards: see below), a scrolling text console on the framebuffer |
| 7 | Tasks ✅ | Kernel threads and a scheduler; then SMP (start the other cores) |
| 8 | User space ✅ | Ring 3, system calls, ELF programs (built into the kernel until stage 9 gives us a disk) |
| 9 | Storage + files ✅ | AHCI + NVMe drivers, a filesystem, a VFS |
| 9b | Real-hardware input ✅ | xHCI USB: keyboards + mice (boot protocol), hot plug, works with no PS/2 controller |
| 10 | Nova desktop + shell ✅ | 10a: the Nova desktop (windows, taskbar, launcher; Terminal, Files, System). 10b: the shell is a user program (`nsh`) with pipes and background jobs |

Later: USB hubs and storage, networking, GPU, real-hardware testing on a USB stick.

## Stage 10b notes: the shell as a user program

* **`user/src/bin/nsh.rs`**: the shell people type into is now an ordinary ring-3 program (no heap, fixed
  buffers). Built-ins `cd pwd echo run spawn help exit`; any other word runs a program (`/bin/NAME`, a
  built-in image, or a path) and waits for it; `a | b | c` connects programs with pipes; a trailing `&`
  runs it in the background; anything that is not a program goes to the kernel's system commands.
  New programs `grep [-v] TEXT` and `wc` read standard input, so pipelines are useful.
* **System calls 16-22:** `spawn(command, stdin_fd, stdout_fd, flags)` (5 arguments: `r8` is now read),
  `wait(pid)`, `pipe()`, `chdir`, `getcwd`, `readline(prompt)` (the Terminal's line editor *with history*,
  like a TTY in cooked mode; Ctrl+C = -EINTR) and `ctl(command)`: runs one of the kernel's system commands
  (`disks format mem ps install fscheck usb reboot ...`, i.e. the old kernel shell's built-ins) for the
  caller, resolving paths against the caller's working directory. They print to the console, so `ctl`
  output cannot be piped yet.
* **Processes:** one descriptor table per process (`Fd::Console | File | PipeIn | PipeOut`, 0-2 start as the
  console; a child's 0/1 can be pipe ends); the working directory can change and is inherited; children a
  parent `wait`s for are registered (with their thread handle, so a child that ends at once still reports
  its code) and the kernel no longer announces *their* exits, the shell does. `pipe.rs`: 4 KiB buffer,
  readers/writers sleep (`park`) and wake each other; an end closes when its last descriptor does, so EOF
  and broken pipes work. Reading the console returns end-of-file on Ctrl+C, so `grep x` typed by mistake
  can be stopped.
* **Terminal:** a kernel thread starts `nsh` (logs `shell: ready (nsh, a user program, is process N)`) and
  starts a new one when it exits; if `nsh` can't run, the old in-kernel shell loop takes over.
* **Tests:** the existing shell, disk and user-space tests now all run through `nsh` unchanged (except: `ps`
  lists the shell itself; `install` copies 13 programs). `boot_test.py --nsh`: pipelines of 2 and 3
  programs with exact counts, `cd` inherited by a child, `cd` errors, a background job, Ctrl+C ending a
  program reading the keyboard, `exit` restarting the shell.
* Gaps: no signals (Ctrl+C can't stop a program that isn't reading the keyboard), no redirection (`>`,
  `<`), no environment variables or `$?`, built-ins can't be in a pipeline, `ctl` commands print straight to
  the console, a background program reading the keyboard competes with the shell.

## Stage 10a notes: the Nova desktop

* **`crates/gfx`** (6 host tests): `Canvas` (0x00RRGGBB pixels in memory: clipped fill, blit, text, console
  cells, gradient, star), `Rect` maths, and the 5x7 font (moved here from the kernel).
* **`crates/desktop`** (24 host tests): the window manager and compositor as pure logic. Windows with title
  bar, minimise and close buttons and a soft shadow; stacking order and focus (click to focus + raise);
  dragging by the title bar (a window can't be dragged out of reach); a taskbar with the Nova launcher
  (menu), one button per open/minimised window (click: focus, minimise if already in front, restore) and a
  clock; the pointer. Every change records **damage**; `compose(area)` redraws exactly that rectangle
  (wallpaper, windows bottom to top, taskbar, menu, pointer); a test checks that composing in pieces gives
  the same picture as all at once. `apps.rs`: the Files view (folder list with `..`, folders first, keyboard
  and click navigation, scrolling, a wrapped text preview of the selected file) and the System info page;
  they *ask* for data (`Request`) and the kernel answers, so they are host-tested too.
* **Kernel:** `gui.rs` keeps a back buffer the size of the screen; after each change the damaged rectangles
  are composed and copied to the (write-combining) framebuffer (`Screen::present`, a straight row copy when
  the pixel layout matches). The console API the shell uses is unchanged: its grid is now drawn into the
  Terminal window. `desktop.rs` is the boot thread's loop: mouse to the desktop, clicks to apps, disk access
  for Files *outside* the GUI lock, the System page refreshed every second, the clock from the CMOS RTC
  (`rtc.rs`, decoding host-tested in `crates/x86`). Keyboard focus: `ps2` sends keys to the shell only while
  the Terminal window is focused, otherwise to the desktop (Files uses arrows/Enter/Backspace; the wheel
  scrolls it).
* **Tests:** `boot_test.py --desktop` uses the desktop through QEMU's mouse and keyboard: launcher -> Files,
  browse with the keyboard, preview a file, check typing into Files does not reach the shell, drag Files by
  its title bar to an exact position, close it (focus returns to the Terminal), open System, click-to-focus,
  minimise and restore from the taskbar; every click's position is checked against the kernel's log; it
  saves `desktop_files.png` and `desktop_system.png`. Run with PS/2 and with USB-only input.
* Gaps: no window resizing or maximise; one Terminal (closing it hides it; the shell keeps running); no
  hover effects, no double-click, no drag-and-drop, no Alt+Tab; the RTC is shown as-is (no time zones); apps
  are kernel code (user-space GUI programs need a window system-call API, later); fonts are the 5x7 bitmap.

## NVMe + USB notes (after stage 9, before stage 10)

* **NVMe** (`kernel/src/nvme.rs`, logic in `crates/nvme`, 9 host tests): resets the controller, admin
  queue pair, IDENTIFY controller / active namespace list / namespace, one I/O queue pair (64 entries,
  physically contiguous, interrupts off, polled by phase bit, timeouts, fatal-status check). Namespaces
  with 512 B or 4 KiB blocks and no metadata become disks; a 4 KiB file-system block is one PRP page.
  `reboot`/`halt` send the NVMe shutdown notification after syncing. Disks are numbered SATA first,
  then NVMe (`files::Disk` hides which driver a disk uses). Tests: storage phases 1 + 2 on the NVMe
  disk, with 512 B and with 4 KiB sectors.
* **xHCI** (`kernel/src/xhci.rs`, logic in `crates/usb`, 25 host tests): BIOS handoff (USB Legacy
  Support capability, SMIs off), Supported Protocol capability tells USB 2 from USB 3 ports, halt +
  reset, device-context array + scratchpad pages, command ring, one event-ring segment, 32-bit-DMA
  controllers respected. Per device: port reset (USB 2), Enable Slot, Address Device, endpoint-0 packet
  size from the first 8 bytes of the device descriptor (Evaluate Context), device/config/string
  descriptors, Set Configuration, SET_PROTOCOL(boot), SET_IDLE, Configure Endpoint with interrupt-IN
  endpoints, 8 transfers kept queued. Stalls/errors: Reset Endpoint + Set TR Dequeue. Polled: during
  start-up and from the `usb` thread every 10 ms (no MSI yet). Port Status Change events attach and
  detach devices (held keys are released on unplug).
* **Input:** keyboard reports become PS/2 set 1 scancodes into a per-keyboard `nova_kbd::Pipeline`
  (so layout and modifiers are shared code); USB keyboards don't repeat keys themselves, so
  `crates/usb` does typematic repeat (500 ms, then ~30/s). Mouse reports go into the PS/2 mouse queue.
  The shell starts if there is a PS/2 keyboard *or* a USB controller (a keyboard may be plugged later).
  The ISO key next to left Shift (`\ |`) was missing from the layout; added.
* **Tests:** `boot_test.py --usb` plugs QEMU's usb-kbd + usb-mouse in (QEMU then routes input to them),
  runs the mouse + shell (+ user program) tests through them, checks the `usb` command, then unplugs and
  replugs the keyboard over QMP and types again. `--no-ps2` boots a q35 with `i8042=off`: USB is the only
  input. Also at 6 GiB (DMA buffers may sit above 4 GiB).
* Gaps: **hubs** (a device behind a hub is reported and ignored; desktop front panels, docks and some
  laptops' internal ports use hubs), USB mass storage, report-protocol HID (gaming mice, keyboards
  without a boot interface), interrupts/MSI for both drivers, more than one NVMe I/O queue, NVMe
  namespaces with metadata, PRP lists (transfers > 4 KiB), both drivers untried on real hardware.

## Stage 9 notes

* **Bus + disk:** `kernel/src/pci.rs` enumerates PCI through the classic 0xCF8/0xCFC ports (works on any
  x86 PC). `kernel/src/ahci.rs` drives SATA disks: per port a command list, a received-FIS area and a
  command table; IDENTIFY, READ/WRITE DMA EXT (LBA48) and FLUSH CACHE EXT, one slot, polled completion
  with timeouts and port restart on error. 512-byte-sector disks only. `kmutex.rs` is a sleeping mutex
  (yields instead of spinning, keeps interrupts on) so a slow disk never blocks timer/keyboard/mouse.
* **`crates/fs`** (pure logic, 25 host tests over a RAM disk): **NovaFS** (4 KiB blocks; inodes with 12
  direct + indirect + double-indirect blocks, files up to ~4 GiB, sparse holes; directories as 64-byte
  entry files; inode/block bitmaps; superblock with CRC; no journal: operations are ordered so a crash
  leaks storage rather than dangling names), an **fsck** that cross-checks bitmaps, block ownership,
  directory entries and link counts (a random-workout test compares against an in-memory model and runs
  fsck throughout), **TmpFs** (RAM) and the **VFS** (mount table, longest-prefix path resolution,
  `.`/`..` handled lexically, mount points appear in listings).
* **Boot:** the first disk with a NovaFS is mounted at `/`; otherwise `/` is RAM until you `format`.
  `/tmp` is always RAM. `format DISK erase` needs the explicit word and refuses while processes run;
  nothing writes to a disk unless asked to.
* **Syscalls:** open (write/create/truncate flags), close, seek, read/write on files, readdir, stat,
  mkdir, unlink, getargs; per-process descriptor table (3..) and working directory. Programs can be run
  from the disk (`run /bin/hello`, or just `run hello` after `install`).
* **Shell:** `pwd cd ls cat write mkdir rm stat cp mounts/df disks format install sync fscheck`.
* **Tests** (`test.ps1`): phase 1 formats a blank virtual disk, installs programs, writes files, runs a
  program from disk, runs `fsdemo` and `fsbig` (a 5 MiB file through double-indirect blocks, verified
  byte for byte), fsck clean; phase 2 boots again on the *same* disk image and checks that everything
  persisted, then fsck again. QEMU gets a second SATA disk (`build\disk.img`, 64 MiB).
* Gaps: **NVMe not done** (many real PCs have only NVMe), no AHCI interrupts/NCQ, no partition table
  (the whole disk is the file system), no FAT/ext support to read other systems' disks, no rename, no
  permissions/timestamps, no journal, no block cache (every access goes to the disk), writes are not
  flushed except by `sync`/shutdown, and a file system on a real disk is untested.

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
