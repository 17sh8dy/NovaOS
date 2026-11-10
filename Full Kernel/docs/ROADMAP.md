# Nova Full Kernel: roadmap

Each stage ends with something that boots and is checked by `tools\test.ps1`.

| # | Stage | What "done" looks like |
|---|---|---|
| 1 | **Bootloader + handoff** ✅ | NovaBoot loads the ELF kernel, page tables + NX, exits UEFI; the kernel logs the memory map and draws the boot screen |
| 2 | CPU tables + exceptions ✅ | Own GDT + TSS (separate stacks for double fault / NMI), IDT with handlers that print a register dump; a deliberate fault in a test build is caught and reported, not a silent reset |
| 3 | Physical memory manager | Frame allocator over `Usable` (then reclaimed) regions; unit-tested on the host like `crates/paging` |
| 4 | Kernel virtual memory + heap | Kernel-owned page tables (drop the identity map), `alloc` support (Box, Vec) |
| 5 | Interrupts + time | ACPI (MADT) parsing, Local APIC + IOAPIC, timer tick, `hlt`-based idle |
| 6 | Keyboard + console | PS/2 keyboard (USB later), a scrolling text console on the framebuffer |
| 7 | Tasks | Kernel threads and a scheduler; then SMP (start the other cores) |
| 8 | User space | Ring 3, system calls, ELF programs loaded from disk |
| 9 | Storage + files | AHCI/NVMe driver, a filesystem, a VFS |
| 10 | Nova shell | First real Nova user interface running on Nova itself |

Later: USB (xHCI), networking, GPU, real-hardware testing on a USB stick.

## Stage 2 notes

* `kernel/src/cpu.rs` installs our GDT/TSS/IDT; `exceptions.rs` has the 32 stubs and
  the register dump; `crash.rs` draws it (serial + screen); pure logic (descriptor
  encoding, error-code decoding) is unit-tested in `crates/x86`.
* IST stacks: #DF, NMI, #MC each get 16 KiB. A kernel stack overflow hits the guard
  page and is reported as #DF on IST 1 (`toolsuild.ps1 -Fault stack` shows it).
* Test kernels: `toolsuild.ps1 -Fault div|ud|gp|pf|df|stack|nmi|panic`, then `run.ps1 -NoBuild`.
  `test.ps1` builds and boots all of them.
* Gaps: no backtrace in the dump (needs frame pointers); IST stacks have no guard pages
  until stage 4; vectors 32-255 are not-present gates (a stray interrupt shows up as #NP/#GP);
  no `swapgs` / ring-3 handling yet (stage 8).

## Known gaps in stage 1

* Not yet booted on a real PC (QEMU + OVMF only).
* The framebuffer and MMIO are mapped write-back; needs PAT/cache attributes.
* UEFI runtime services are left unmapped (no `SetVirtualAddressMap`).
* QEMU's TCG emulator crashes on this Windows build; boots use WHPX (see README).
