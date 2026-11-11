# Nova OS — Full Kernel

The real-operating-system version of Nova OS: our own bootloader, our own
kernel, running directly on x86_64 hardware (or a virtual machine). It shares
no code with the desktop-environment app in the parent folder; that app is
untouched and keeps running on top of Windows.

Written in Rust with **no third-party crates**, so every byte that boots is ours.

## Status

**Stage 1: bootloader — done.** NovaBoot (a UEFI application) loads the kernel,
builds its page tables, exits the firmware and jumps to it. The kernel validates
what it was handed, reports the machine over serial and draws its boot screen.

```
UEFI firmware ──> NovaBoot (BOOTX64.EFI) ──> Nova kernel (KERNEL.ELF)
                   reads \NOVA\KERNEL.ELF      higher half: 0xFFFFFFFF80000000
                   checks the ELF              reads BootInfo + memory map
                   framebuffer, ACPI RSDP      serial log on COM1
                   page tables (NX)            draws the boot screen
                   ExitBootServices, jump      halts (stage 2 picks up here)
```

**Stage 2: CPU tables + exceptions — done.** The kernel loads its own GDT, TSS and
IDT. A CPU fault now stops with a register dump on the serial log *and* a crash screen
instead of silently resetting the machine; double fault, NMI and machine check run on
their own stacks. See the fault-test builds below.

**Stage 3: physical memory manager — done.** A bitmap frame allocator hands out 4 KiB
frames from the boot memory map (including the firmware's reclaimed scratch memory) and is
checked at every boot by writing and verifying real memory.

**Stage 4: virtual memory + heap — done.** The kernel runs on its own page tables (code is
write-protected, data and heap are no-execute, the bootloader's identity map is gone), has given
back the bootloader's memory, and has a heap (`Box`, `Vec`, `String`). **PS/2 mouse** support came
with it: boot in QEMU and move the mouse; a pointer follows it and a read-out shows its position.
(Keyboard is next, with the console.)

**Stage 5: interrupts + timer — done.** The kernel reads the firmware's ACPI tables, switches
from the legacy PICs to the local APIC and IO-APIC, runs a calibrated 100 Hz timer, takes the
mouse by interrupt, and idles with `hlt` instead of spinning. The pointer read-out shows uptime.

**Stage 7: threads + SMP — done** (ahead of stage 6, the keyboard + console). Kernel threads with
guard-paged stacks, a preemptive scheduler, and all CPUs started (4 in the default QEMU setup, see
`run.ps1 -Cpus`). A crash report says which CPU and thread it happened on.

Next stages are in [docs/ROADMAP.md](docs/ROADMAP.md).

## Layout

| Path | What |
|---|---|
| `boot/` | **NovaBoot**, the UEFI bootloader (`x86_64-unknown-uefi`) |
| `kernel/` | **The kernel** (`x86_64-unknown-none`), plus `linker.ld` |
| `crates/bootinfo` | The boot protocol: `BootInfo`, memory map, framebuffer ([spec](docs/BOOT-PROTOCOL.md)) |
| `crates/elf` | Bounds-checked ELF64 reader |
| `crates/paging` | x86_64 4-level page-table builders (boot pool + dynamic) |
| `crates/pmm` | Bitmap physical frame allocator |
| `crates/heap` | Kernel heap allocator (free list + coalescing) |
| `crates/acpi` | ACPI table parser (RSDP, XSDT/RSDT, MADT) |
| `crates/apic` | Local APIC / IO-APIC register encodings, timer maths |
| `crates/ps2` | PS/2 mouse packet decoder + pointer tracking |
| `crates/x86` | Port I/O, serial (16550), MSRs, CPUID, GDT/TSS/IDT data, exception decoding |
| `tools/` | Build, run and test scripts |

## Requirements

* Rust (stable). `rust-toolchain.toml` pulls in the two bare-metal targets automatically.
* QEMU: `winget install SoftwareFreedomConservancy.QEMU` (ships the UEFI firmware we boot with).
* Python 3 (for the run and test scripts; standard library only).
* **Windows Hypervisor Platform** enabled (QEMU runs with `-accel whpx`, see below).

## Use it

```powershell
.\tools\build.ps1     # build -> build\esp  (the boot volume)
.\tools\run.ps1       # build + boot in a QEMU window; the kernel log prints here
.\tools\test.ps1      # everything: unit tests, clippy, build, 3 headless boots
python tools\boot_test.py --memory 6G   # one headless boot; screenshot -> build\boot_test.png
```

`run.ps1 -Memory 2G` changes guest RAM; `-NoBuild` boots the existing build.

### See a crash screen on purpose

```powershell
.	oolsuild.ps1 -Fault pf      # div | ud | gp | pf | df | stack | nmi | panic
.	oolsun.ps1 -NoBuild         # boots, draws the boot screen, then faults
.	oolsuild.ps1                # back to the normal kernel
```

### Why WHPX and not plain QEMU emulation

On this PC, QEMU 11.1's software CPU (TCG) crashes with `0xC0000409` as soon as
the guest CPU starts, even with no guest at all
(`qemu-system-x86_64 -display none -nodefaults`). Paused VMs and WHPX both work,
so it is a QEMU/Windows build problem, not Nova. WHPX is also much faster (a
boot takes ~1.3 s). To use TCG on another machine: `$env:NOVA_ACCEL = 'tcg'`.

## Booting a real PC (not tried yet)

The contents of `build\esp` are a complete UEFI boot volume. Copy them to a
FAT32 USB stick (`EFI\BOOT\BOOTX64.EFI` + `NOVA\KERNEL.ELF`), turn **Secure
Boot off**, and boot from the stick. Output goes to the screen and to COM1.
This has only been run in QEMU so far.

## Rules

* **No third-party crates** in `boot/` or `kernel/`; this is meant to be ours.
* Logic that can be tested on the host lives in `crates/` with unit tests.
* Every change must keep `tools\test.ps1` green; a UI/boot change is only done
  when it has been seen booting (the boot test takes a screenshot).
