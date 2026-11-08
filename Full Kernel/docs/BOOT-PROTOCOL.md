# Nova boot protocol (v1)

The contract between NovaBoot and the kernel. The source of truth is
`crates/bootinfo/src/lib.rs`; this file explains it.

## Files on the boot volume

| Path | Contents |
|---|---|
| `\EFI\BOOT\BOOTX64.EFI` | NovaBoot (the UEFI default boot path) |
| `\NOVA\KERNEL.ELF` | The kernel: ELF64, x86_64, `ET_EXEC`, at most 64 MiB |

The kernel must be linked at or above `0xFFFF_FFFF_8000_0000` and its entry
point must lie in an executable segment. NovaBoot refuses anything else, with
a message, and returns to the firmware after 10 seconds.

## Machine state at kernel entry

```rust
unsafe extern "sysv64" fn _start(info: *const BootInfo) -> !
```

* 64-bit long mode, ring 0, **interrupts disabled**.
* `rdi` = virtual address of the `BootInfo` (in the direct map).
* `rsp` = `0xFFFF_FFFF_7000_0000 - 8` (as if called; ABI-aligned). 128 KiB stack
  with an **unmapped guard page** below it.
* CR3 = NovaBoot's page tables (physical address in `BootInfo::page_table_root`).
* `EFER.NXE` set when the CPU supports NX (`BootInfo::nx_enabled`).
* UEFI boot services are **exited**. Runtime services are not set up (no
  `SetVirtualAddressMap` call yet).
* GDT and IDT are still the **firmware's**. They live in
  `FirmwareReclaimable` memory: load your own before reusing that memory.

## Virtual memory layout

| Virtual range | Maps | Flags |
|---|---|---|
| `0` .. `N` | identity map of physical memory (2 MiB pages) | RW, executable |
| `0xFFFF_8000_0000_0000` .. `+N` | the same physical memory (the direct map) | RW, NX |
| `0xFFFF_FFFF_7000_0000 - 128 KiB` .. | kernel stack | RW, NX |
| `0xFFFF_FFFF_8000_0000` .. | kernel image, per-segment permissions | R-X / R-- / RW- |

`N` = all RAM, at least 4 GiB (to cover the framebuffer, LAPIC and IOAPIC),
rounded up to 1 GiB, at most 512 GiB. It is in `BootInfo::physical_memory_mapped`.
The identity map only exists so NovaBoot survives the CR3 switch; the kernel
should drop it once it has its own tables (it will become user space).

All memory is mapped write-back; MMIO (LAPIC, IOAPIC, framebuffer) should be
remapped with proper cache attributes by the kernel's memory manager.

## BootInfo

Every address **inside** `BootInfo` is physical: add `physical_memory_offset`.
Check `info.is_valid()` (magic `"NOVABOOT"`, version, struct size) first.

| Field | Meaning |
|---|---|
| `physical_memory_offset`, `physical_memory_mapped` | the direct map |
| `memory_map_addr`, `memory_map_len` | `[MemoryRegion]`, sorted, same-kind neighbours merged |
| `framebuffer` | linear 32-bit framebuffer; `addr == 0` means none. Use `Framebuffer::pack` for pixel values |
| `kernel_phys`, `kernel_virt`, `kernel_pages` | where the kernel image is |
| `stack_phys`, `stack_pages`, `stack_top` | the initial stack |
| `page_table_root` | the PML4 in CR3 |
| `rsdp_addr` | ACPI RSDP (2.0 preferred), 0 if none |

## Memory kinds

| Kind | From UEFI | Kernel may use it |
|---|---|---|
| `Usable` | ConventionalMemory | yes |
| `FirmwareReclaimable` | BootServicesCode/Data | after loading its own GDT/IDT |
| `Loader` | LoaderCode/Data | after it no longer needs the BootInfo, boot page tables or initial stack (the kernel image is also here) |
| `AcpiReclaimable` | ACPIReclaimMemory | after parsing ACPI |
| `FirmwareRuntime` | RuntimeServicesCode/Data | never |
| `AcpiNvs`, `Reserved`, `Mmio`, `Bad` | the rest | never |
