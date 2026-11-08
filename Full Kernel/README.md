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

Next stages are in [docs/ROADMAP.md](docs/ROADMAP.md).

## Layout

| Path | What |
|---|---|
| `boot/` | **NovaBoot**, the UEFI bootloader (`x86_64-unknown-uefi`) |
| `kernel/` | **The kernel** (`x86_64-unknown-none`), plus `linker.ld` |
| `crates/bootinfo` | The boot protocol: `BootInfo`, memory map, framebuffer ([spec](docs/BOOT-PROTOCOL.md)) |
| `crates/elf` | Bounds-checked ELF64 reader |
| `crates/paging` | x86_64 4-level page-table builder |
| `crates/x86` | Port I/O, serial (16550), MSRs, CPUID |
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
