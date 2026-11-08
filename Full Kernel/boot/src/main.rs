//! NovaBoot — the UEFI bootloader for the Nova kernel.
//!
//! What it does, in order:
//! 1. Reads `\NOVA\KERNEL.ELF` from the volume NovaBoot itself was loaded from.
//! 2. Validates the ELF and copies its segments into freshly allocated pages.
//! 3. Finds the framebuffer (GOP) and the ACPI RSDP.
//! 4. Builds the kernel's page tables:
//!    * identity map of low physical memory (so this code keeps running
//!      across the CR3 switch),
//!    * the same memory again at `layout::PHYSICAL_MEMORY_OFFSET`,
//!    * the kernel image at its linked higher-half address,
//!    * a kernel stack with an unmapped guard page below it.
//! 5. Exits UEFI boot services, turns the UEFI memory map into the Nova one,
//!    switches CR3 and jumps to the kernel with a `BootInfo` pointer in rdi.
//!
//! See docs/BOOT-PROTOCOL.md for the contract with the kernel.

#![no_std]
#![no_main]

mod console;
mod uefi;

use core::convert::Infallible;
use core::ffi::c_void;
use core::fmt;
use core::mem::size_of;
use core::ptr;
use nova_bootinfo::{self as bootinfo, BootInfo, Framebuffer, MemoryKind, MemoryRegion, layout};
use nova_elf::{Elf, ElfError, PF_W, PF_X};
use nova_paging::{self as paging, MapError, Mapper, NO_EXECUTE, Table, WRITABLE};
use uefi::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const KERNEL_PATH: &str = "\\NOVA\\KERNEL.ELF";
const PAGE: u64 = 4096;
/// Refuse kernels bigger than this (file or memory image).
const MAX_KERNEL_BYTES: u64 = 64 << 20;
/// Spare memory-map entries to allow for, since allocating memory can split
/// regions between our probe and the final map.
const MAP_SLACK: usize = 64;

enum BootError {
    Efi(&'static str, Status),
    Elf(ElfError),
    Map(MapError),
    Other(&'static str),
}

impl From<ElfError> for BootError {
    fn from(e: ElfError) -> Self {
        Self::Elf(e)
    }
}
impl From<MapError> for BootError {
    fn from(e: MapError) -> Self {
        Self::Map(e)
    }
}

impl fmt::Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Efi(what, s) => write!(f, "{what} failed: {} ({s:#x})", status_name(*s)),
            Self::Elf(e) => write!(f, "{KERNEL_PATH} is not a valid kernel: {e}"),
            Self::Map(e) => write!(f, "building page tables: {e}"),
            Self::Other(msg) => f.write_str(msg),
        }
    }
}

fn check(status: Status, what: &'static str) -> Result<(), BootError> {
    if is_error(status) { Err(BootError::Efi(what, status)) } else { Ok(()) }
}

fn pages_for(bytes: u64) -> u64 {
    bytes.div_ceil(PAGE)
}

struct Firmware {
    image: Handle,
    st: *mut SystemTable,
}

impl Firmware {
    fn bs(&self) -> &BootServices {
        // SAFETY: valid until ExitBootServices, and only used before it.
        unsafe { &*(*self.st).boot_services }
    }

    /// Allocates zeroed, page-aligned memory (marked LoaderData).
    fn alloc_pages(&self, pages: u64, what: &'static str) -> Result<u64, BootError> {
        let mut addr = 0u64;
        // SAFETY: plain boot-services call with valid out-pointer.
        let s = unsafe { (self.bs().allocate_pages)(ALLOCATE_ANY_PAGES, LOADER_DATA, pages as usize, &mut addr) };
        check(s, what)?;
        // SAFETY: firmware just gave us this memory; UEFI identity-maps RAM.
        unsafe { ptr::write_bytes(addr as *mut u8, 0, (pages * PAGE) as usize) };
        Ok(addr)
    }

    fn free_pages(&self, addr: u64, pages: u64) {
        // SAFETY: memory came from alloc_pages and is no longer referenced.
        unsafe { (self.bs().free_pages)(addr, pages as usize) };
    }

    fn handle_protocol<T>(&self, handle: Handle, guid: &Guid, what: &'static str) -> Result<*mut T, BootError> {
        let mut out: *mut c_void = ptr::null_mut();
        // SAFETY: valid handle from the firmware, valid out-pointer.
        check(unsafe { (self.bs().handle_protocol)(handle, guid, &mut out) }, what)?;
        Ok(out.cast())
    }

    fn locate_protocol<T>(&self, guid: &Guid, what: &'static str) -> Result<*mut T, BootError> {
        let mut out: *mut c_void = ptr::null_mut();
        // SAFETY: valid out-pointer.
        check(unsafe { (self.bs().locate_protocol)(guid, ptr::null_mut(), &mut out) }, what)?;
        Ok(out.cast())
    }

    /// Reads the whole kernel file into fresh pages. Returns (address, size).
    fn read_kernel(&self) -> Result<(u64, u64), BootError> {
        let image: *mut LoadedImage = self.handle_protocol(self.image, &LOADED_IMAGE_GUID, "finding NovaBoot's own image")?;
        // SAFETY: the firmware returned a valid LoadedImage.
        let device = unsafe { (*image).device_handle };
        let fs: *mut SimpleFileSystem = self.handle_protocol(device, &SIMPLE_FILE_SYSTEM_GUID, "opening the boot volume")?;

        let mut root: *mut File = ptr::null_mut();
        // SAFETY: valid protocol pointer and out-pointer.
        check(unsafe { ((*fs).open_volume)(fs, &mut root) }, "opening the boot volume root")?;

        let mut name = [0u16; 64];
        for (slot, c) in name.iter_mut().zip(KERNEL_PATH.encode_utf16()) {
            *slot = c;
        }

        let mut file: *mut File = ptr::null_mut();
        // SAFETY: valid directory handle, NUL-terminated name, out-pointer.
        let status = unsafe { ((*root).open)(root, &mut file, name.as_ptr(), FILE_MODE_READ, 0) };
        if is_error(status) {
            unsafe { ((*root).close)(root) };
            return Err(BootError::Efi("opening \\NOVA\\KERNEL.ELF", status));
        }

        let result = (|| {
            let mut size = 0u64;
            // Seeking to u64::MAX moves to end-of-file; the position is the size.
            unsafe {
                check(((*file).set_position)(file, u64::MAX), "seeking the kernel file")?;
                check(((*file).get_position)(file, &mut size), "sizing the kernel file")?;
                check(((*file).set_position)(file, 0), "rewinding the kernel file")?;
            }
            if size == 0 {
                return Err(BootError::Other("\\NOVA\\KERNEL.ELF is empty"));
            }
            if size > MAX_KERNEL_BYTES {
                return Err(BootError::Other("\\NOVA\\KERNEL.ELF is larger than 64 MiB"));
            }
            let pages = pages_for(size);
            let buf = self.alloc_pages(pages, "allocating memory for the kernel file")?;
            let mut done = 0u64;
            while done < size {
                let mut chunk = (size - done) as usize;
                // SAFETY: buffer has room for `size` bytes.
                let s = unsafe { ((*file).read)(file, &mut chunk, (buf + done) as *mut u8) };
                if is_error(s) {
                    self.free_pages(buf, pages);
                    return Err(BootError::Efi("reading the kernel file", s));
                }
                if chunk == 0 {
                    self.free_pages(buf, pages);
                    return Err(BootError::Other("the kernel file ended early"));
                }
                done += chunk as u64;
            }
            Ok((buf, size))
        })();

        unsafe {
            ((*file).close)(file);
            ((*root).close)(root);
        }
        result
    }

    fn framebuffer(&self) -> Framebuffer {
        let gop: *mut GraphicsOutput = match self.locate_protocol(&GRAPHICS_OUTPUT_GUID, "graphics output") {
            Ok(gop) => gop,
            Err(_) => {
                println!("Screen: no graphics output protocol; booting without a framebuffer");
                return Framebuffer::default();
            }
        };
        // SAFETY: the GOP mode and info structures are owned by the firmware.
        let (mode, info) = unsafe { (&*(*gop).mode, &*(*(*gop).mode).info) };
        let (red, green, blue) = match info.pixel_format {
            PIXEL_RGB => (0x0000_00FF, 0x0000_FF00, 0x00FF_0000),
            PIXEL_BGR => (0x00FF_0000, 0x0000_FF00, 0x0000_00FF),
            PIXEL_BITMASK => (info.red_mask, info.green_mask, info.blue_mask),
            _ => {
                println!("Screen: the display has no linear framebuffer (blt-only); booting without one");
                return Framebuffer::default();
            }
        };
        Framebuffer {
            addr: mode.frame_buffer_base,
            size: mode.frame_buffer_size as u64,
            width: info.horizontal_resolution,
            height: info.vertical_resolution,
            stride: info.pixels_per_scan_line,
            bytes_per_pixel: 4,
            red_mask: red,
            green_mask: green,
            blue_mask: blue,
            _reserved: 0,
        }
    }

    fn rsdp(&self) -> u64 {
        // SAFETY: the configuration table array is owned by the firmware.
        let tables = unsafe {
            let st = &*self.st;
            if st.configuration_table.is_null() {
                return 0;
            }
            core::slice::from_raw_parts(st.configuration_table, st.number_of_table_entries)
        };
        let find = |guid: Guid| tables.iter().find(|t| t.vendor_guid == guid).map(|t| t.vendor_table as u64);
        find(ACPI_20_TABLE_GUID).or_else(|| find(ACPI_10_TABLE_GUID)).unwrap_or(0)
    }

    /// Asks for the memory map size. Returns (bytes needed, descriptor size).
    fn memory_map_size(&self) -> Result<(usize, usize), BootError> {
        let (mut size, mut key, mut desc_size, mut version) = (0usize, 0usize, 0usize, 0u32);
        // SAFETY: a zero-sized query with a null buffer is the documented way to get the size.
        let s = unsafe { (self.bs().get_memory_map)(&mut size, ptr::null_mut(), &mut key, &mut desc_size, &mut version) };
        if s != BUFFER_TOO_SMALL && is_error(s) {
            return Err(BootError::Efi("reading the memory map size", s));
        }
        if desc_size < size_of::<MemoryDescriptor>() {
            return Err(BootError::Other("the firmware reported a bad memory descriptor size"));
        }
        Ok((size, desc_size))
    }

    /// Highest end address of any RAM-like region (not MMIO, not reserved).
    fn highest_ram_address(&self) -> Result<u64, BootError> {
        let (size, desc_size) = self.memory_map_size()?;
        let cap = size + MAP_SLACK * desc_size;
        let pages = pages_for(cap as u64);
        let buf = self.alloc_pages(pages, "allocating a memory map buffer")?;
        let (mut len, mut key, mut ds, mut version) = (cap, 0usize, 0usize, 0u32);
        // SAFETY: buffer has `cap` bytes.
        let s = unsafe { (self.bs().get_memory_map)(&mut len, buf as *mut u8, &mut key, &mut ds, &mut version) };
        let result = check(s, "reading the memory map").map(|()| {
            let mut top = 0u64;
            for i in 0..len / ds {
                // SAFETY: entries are `ds` bytes apart inside the buffer.
                let d = unsafe { ptr::read_unaligned((buf as usize + i * ds) as *const MemoryDescriptor) };
                if matches!(d.ty, 1..=7 | 9 | 10 | 14) {
                    top = top.max(d.physical_start + d.number_of_pages * PAGE);
                }
            }
            top
        });
        self.free_pages(buf, pages);
        result
    }
}

/// The loaded kernel image.
struct Kernel {
    phys: u64,
    virt: u64,
    pages: u64,
    entry: u64,
    /// The raw file, freed once the page tables are built.
    file_addr: u64,
    file_pages: u64,
}

fn flags_name(flags: u32) -> &'static str {
    match (flags & 4 != 0, flags & PF_W != 0, flags & PF_X != 0) {
        (_, false, true) => "R-X",
        (_, true, true) => "RWX",
        (_, true, false) => "RW-",
        (_, false, false) => "R--",
    }
}

fn load_kernel(fw: &Firmware) -> Result<(Kernel, Elf<'static>), BootError> {
    let (file_addr, file_size) = fw.read_kernel()?;
    // SAFETY: read_kernel filled `file_size` bytes; the pages stay allocated
    // until we free them below (and `Elf` is only used before that).
    let bytes: &'static [u8] = unsafe { core::slice::from_raw_parts(file_addr as *const u8, file_size as usize) };
    let elf = Elf::parse(bytes)?;
    println!("Kernel: {KERNEL_PATH} ({file_size} bytes), entry {:#x}", elf.entry);

    let (lo, hi) = elf.load_span();
    if lo < layout::KERNEL_BASE {
        return Err(BootError::Other("the kernel is not linked in the higher half (0xFFFFFFFF80000000+)"));
    }
    if hi - lo > MAX_KERNEL_BYTES {
        return Err(BootError::Other("the kernel's memory image is larger than 64 MiB"));
    }
    let pages = (hi - lo) / PAGE;
    let phys = fw.alloc_pages(pages, "allocating memory for the kernel")?;
    for seg in elf.segments() {
        println!("  segment {:#x} +{:#07x} {}", seg.vaddr, seg.memsz, flags_name(seg.flags));
        let data = elf.segment_data(&seg);
        // SAFETY: the destination lies inside the `pages` we just allocated
        // (load_span covers every segment); the source is inside the file.
        unsafe { ptr::copy_nonoverlapping(data.as_ptr(), (phys + (seg.vaddr - lo)) as *mut u8, data.len()) };
        // The rest (.bss) is already zero: alloc_pages zeroes.
    }
    println!("Kernel: {pages} pages at physical {phys:#x}");
    Ok((Kernel { phys, virt: lo, pages, entry: elf.entry, file_addr, file_pages: pages_for(file_size) }, elf))
}

#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(image: Handle, st: *mut SystemTable) -> Status {
    console::init(st);
    let fw = Firmware { image, st };
    match boot(&fw) {
        Ok(never) => match never {},
        Err(e) => {
            println!();
            println!("NovaBoot: cannot start Nova: {e}");
            if console::firmware_active() {
                println!("Returning to the firmware in 10 seconds.");
                // SAFETY: boot services are still active.
                unsafe { (fw.bs().stall)(10_000_000) };
                match e {
                    BootError::Efi(_, s) => s,
                    _ => LOAD_ERROR,
                }
            } else {
                nova_x86::halt_forever()
            }
        }
    }
}

fn boot(fw: &Firmware) -> Result<Infallible, BootError> {
    // SAFETY: the system table is valid while boot services run.
    let st = unsafe { &*fw.st };
    unsafe {
        // The firmware's 5-minute watchdog would reset the machine under a
        // kernel that never returns.
        (fw.bs().set_watchdog_timer)(0, 0, 0, ptr::null());
        ((*st.con_out).clear_screen)(st.con_out);
    }

    println!("NovaBoot {VERSION} - Nova OS bootloader");
    print!("Firmware: ");
    console::print_ucs2(st.firmware_vendor);
    println!(" (revision {:#x})", st.firmware_revision);

    let framebuffer = fw.framebuffer();
    if framebuffer.is_present() {
        println!(
            "Screen: {}x{}, stride {}, framebuffer at {:#x}",
            framebuffer.width, framebuffer.height, framebuffer.stride, framebuffer.addr
        );
    }
    let rsdp = fw.rsdp();
    if rsdp != 0 {
        println!("ACPI: RSDP at {rsdp:#x}");
    } else {
        println!("ACPI: no RSDP found");
    }

    let (kernel, elf) = load_kernel(fw)?;
    let nx = nova_x86::has_nx();

    // Kernel stack: physical pages, mapped below KERNEL_STACK_TOP.
    let stack_pages = layout::KERNEL_STACK_PAGES;
    let stack_phys = fw.alloc_pages(stack_pages, "allocating the kernel stack")?;
    let stack_bottom = layout::KERNEL_STACK_TOP - stack_pages * PAGE;

    // How much physical memory to map: all RAM, and at least the low 4 GiB
    // (where the framebuffer, LAPIC and IOAPIC usually live).
    let ram_top = fw.highest_ram_address()?;
    let fb_top = framebuffer.addr + framebuffer.size;
    let direct = ram_top.max(fb_top).max(4 << 30).next_multiple_of(1 << 30).min(layout::MAX_DIRECT_MAP);

    // Boot info + Nova memory map, and a buffer for the final UEFI map.
    let (map_bytes, desc_size) = fw.memory_map_size()?;
    let region_cap = map_bytes / desc_size + MAP_SLACK;
    let raw_cap = region_cap * desc_size;
    let raw_pages = pages_for(raw_cap as u64);
    let raw_map = fw.alloc_pages(raw_pages, "allocating the final memory map buffer")?;
    let info_header = size_of::<BootInfo>().next_multiple_of(16);
    let info_pages = pages_for((info_header + region_cap * size_of::<MemoryRegion>()) as u64);
    let info_phys = fw.alloc_pages(info_pages, "allocating the boot info")?;
    let regions_phys = info_phys + info_header as u64;

    // Page tables.
    let pool_len = 1
        + 2 * paging::tables_for_large_range(direct)
        + paging::tables_for_small_range(kernel.pages)
        + paging::tables_for_small_range(stack_pages)
        + 4;
    let pool_phys = fw.alloc_pages(pool_len as u64, "allocating page tables")?;
    // SAFETY: freshly allocated, zeroed, page-aligned and identity-mapped.
    let pool = unsafe { core::slice::from_raw_parts_mut(pool_phys as *mut Table, pool_len) };
    let mut mapper = Mapper::new(pool, pool_phys)?;
    let nx_bit = if nx { NO_EXECUTE } else { 0 };

    mapper.map_range_2m(0, 0, direct, WRITABLE)?;
    mapper.map_range_2m(layout::PHYSICAL_MEMORY_OFFSET, 0, direct, WRITABLE | nx_bit)?;
    for i in 0..kernel.pages {
        let virt = kernel.virt + i * PAGE;
        let seg_flags = elf.page_flags(virt);
        if seg_flags == 0 {
            continue; // a gap between segments: leave it unmapped
        }
        let mut flags = 0;
        if seg_flags & PF_W != 0 {
            flags |= WRITABLE;
        }
        if seg_flags & PF_X == 0 {
            flags |= nx_bit;
        }
        mapper.map_4k(virt, kernel.phys + i * PAGE, flags)?;
    }
    for i in 0..stack_pages {
        mapper.map_4k(stack_bottom + i * PAGE, stack_phys + i * PAGE, WRITABLE | nx_bit)?;
    }

    // Prove the tables say what we think before betting the boot on them.
    let info_virt = layout::PHYSICAL_MEMORY_OFFSET + info_phys;
    let here = boot as *const () as u64;
    let checks = [
        (kernel.entry, kernel.phys + (kernel.entry - kernel.virt)),
        (layout::KERNEL_STACK_TOP - 8, stack_phys + stack_pages * PAGE - 8),
        (info_virt, info_phys),
        (here, here),
    ];
    for (virt, want) in checks {
        if mapper.translate(virt).map(|(p, _)| p) != Some(want) {
            return Err(BootError::Other("page table self-check failed"));
        }
    }
    println!(
        "Paging: {} GiB identity + direct map, kernel at {:#x}, NX {}, {}/{} tables",
        direct >> 30,
        kernel.virt,
        if nx { "on" } else { "off" },
        mapper.tables_used(),
        mapper.tables_total()
    );
    let cr3 = mapper.root();

    // SAFETY: info_phys has room for a BootInfo (info_pages covers it).
    unsafe {
        ptr::write(
            info_phys as *mut BootInfo,
            BootInfo {
                magic: bootinfo::MAGIC,
                version: bootinfo::VERSION,
                size: size_of::<BootInfo>() as u32,
                physical_memory_offset: layout::PHYSICAL_MEMORY_OFFSET,
                physical_memory_mapped: direct,
                memory_map_addr: regions_phys,
                memory_map_len: 0,
                framebuffer,
                kernel_phys: kernel.phys,
                kernel_virt: kernel.virt,
                kernel_pages: kernel.pages,
                stack_phys,
                stack_pages,
                stack_top: layout::KERNEL_STACK_TOP,
                page_table_root: cr3,
                rsdp_addr: rsdp,
                firmware_revision: st.firmware_revision,
                nx_enabled: nx as u32,
            },
        );
    }

    // The kernel file is no longer needed: segments are copied and the
    // page flags are in the tables. `elf` must not be used after this.
    let _ = elf;
    fw.free_pages(kernel.file_addr, kernel.file_pages);
    let entry = kernel.entry;

    println!("Exiting UEFI boot services...");

    // Final memory map + ExitBootServices. No printing or allocating from
    // here on: either would change the map and invalidate the key. If the
    // key is stale, ExitBootServices fails and we simply try again.
    let mut status = INVALID_PARAMETER;
    let mut map_size = 0usize;
    let mut desc = desc_size;
    for _ in 0..8 {
        let (mut key, mut version) = (0usize, 0u32);
        map_size = raw_cap;
        // SAFETY: raw_map has raw_cap bytes.
        let s = unsafe { (fw.bs().get_memory_map)(&mut map_size, raw_map as *mut u8, &mut key, &mut desc, &mut version) };
        if is_error(s) {
            status = s;
            continue;
        }
        // SAFETY: we hold the current map key.
        status = unsafe { (fw.bs().exit_boot_services)(fw.image, key) };
        if status == SUCCESS {
            break;
        }
    }
    if status != SUCCESS {
        return Err(BootError::Efi("ExitBootServices", status));
    }
    console::firmware_gone();
    nova_x86::cli();

    // Translate the UEFI map into the Nova one.
    let count = (map_size / desc).min(region_cap);
    // SAFETY: regions_phys has room for region_cap entries.
    let regions = unsafe { core::slice::from_raw_parts_mut(regions_phys as *mut MemoryRegion, region_cap) };
    for (i, region) in regions.iter_mut().enumerate().take(count) {
        // SAFETY: entries are `desc` bytes apart inside raw_map.
        let d = unsafe { ptr::read_unaligned((raw_map as usize + i * desc) as *const MemoryDescriptor) };
        *region = MemoryRegion::new(d.physical_start, d.number_of_pages * PAGE, MemoryKind::from_uefi(d.ty));
    }
    let len = bootinfo::normalize(&mut regions[..count]);
    // SAFETY: we wrote the BootInfo above.
    unsafe { (*(info_phys as *mut BootInfo)).memory_map_len = len as u64 };

    println!("NovaBoot: firmware exited; {len} memory regions; jumping to the kernel at {entry:#x}");

    if nx {
        // SAFETY: the CPU supports NX (checked); our tables use the bit.
        unsafe {
            let efer = nova_x86::rdmsr(nova_x86::MSR_EFER);
            nova_x86::wrmsr(nova_x86::MSR_EFER, efer | nova_x86::EFER_NXE);
        }
    }

    // SAFETY: the tables were self-checked above: this code is identity
    // mapped, the stack and entry point are mapped, and info_virt points at
    // the BootInfo.
    unsafe { jump_to_kernel(cr3, layout::KERNEL_STACK_TOP, entry, info_virt) }
}

/// Switches to the new page tables and stack, then jumps to the kernel.
///
/// The stack is set up as if the kernel had been `call`ed (a zero return
/// address), so it starts with the alignment the System V ABI expects.
unsafe fn jump_to_kernel(cr3: u64, stack_top: u64, entry: u64, info: u64) -> ! {
    unsafe {
        core::arch::asm!(
            "mov cr3, {cr3}",
            "mov rsp, {stack}",
            "xor ebp, ebp",
            "push 0",
            "jmp {entry}",
            cr3 = in(reg) cr3,
            stack = in(reg) stack_top,
            entry = in(reg) entry,
            in("rdi") info,
            options(noreturn)
        )
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("\nNovaBoot panic: {info}");
    nova_x86::halt_forever()
}
