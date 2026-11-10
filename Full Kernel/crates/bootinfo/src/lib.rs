//! The Nova boot protocol.
//!
//! NovaBoot (the UEFI bootloader) fills in a [`BootInfo`] and jumps to the
//! kernel entry point with a pointer to it in `rdi` (System V ABI):
//!
//! ```text
//! extern "sysv64" fn _start(info: &'static BootInfo) -> !
//! ```
//!
//! Rules (see docs/BOOT-PROTOCOL.md for the full text):
//! * Every address *inside* `BootInfo` is a **physical** address. Add
//!   [`BootInfo::physical_memory_offset`] to reach it from the kernel.
//! * The `info` pointer itself is already a virtual (higher-half) address.
//! * Interrupts are disabled. UEFI boot services have been exited.
//! * The GDT and IDT are still the firmware's and live in memory marked
//!   [`MemoryKind::FirmwareReclaimable`]; the kernel must load its own before
//!   reusing that memory.

#![no_std]

#[cfg(test)]
extern crate std;

/// "NOVABOOT" in little-endian ASCII.
pub const MAGIC: u64 = u64::from_le_bytes(*b"NOVABOOT");
/// Bumped whenever the layout of [`BootInfo`] changes.
pub const VERSION: u32 = 1;

/// The fixed virtual memory layout NovaBoot sets up.
pub mod layout {
    /// All physical memory is mapped here (higher-half direct map).
    pub const PHYSICAL_MEMORY_OFFSET: u64 = 0xFFFF_8000_0000_0000;
    /// The kernel image must be linked at or above this address.
    pub const KERNEL_BASE: u64 = 0xFFFF_FFFF_8000_0000;
    /// Top of the initial kernel stack (exclusive). The page below the
    /// stack's lowest page is left unmapped as a guard.
    pub const KERNEL_STACK_TOP: u64 = 0xFFFF_FFFF_7000_0000;
    /// Size of the initial kernel stack.
    pub const KERNEL_STACK_PAGES: u64 = 32; // 128 KiB
    /// Most physical memory NovaBoot will direct-map (one PDPT).
    pub const MAX_DIRECT_MAP: u64 = 512 << 30;
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    /// `size_of::<BootInfo>()` as the bootloader saw it.
    pub size: u32,

    /// Virtual address where physical address 0 is mapped.
    pub physical_memory_offset: u64,
    /// How many bytes of physical memory are mapped at that offset.
    pub physical_memory_mapped: u64,

    /// Physical address of a `[MemoryRegion; memory_map_len]` array,
    /// sorted by start address, adjacent same-kind regions merged.
    pub memory_map_addr: u64,
    pub memory_map_len: u64,

    pub framebuffer: Framebuffer,

    /// Where the kernel image lives (physical) and where it is mapped.
    pub kernel_phys: u64,
    pub kernel_virt: u64,
    pub kernel_pages: u64,

    /// The initial stack (physical base, page count, virtual top).
    pub stack_phys: u64,
    pub stack_pages: u64,
    pub stack_top: u64,

    /// Physical address of the PML4 that is loaded in CR3.
    pub page_table_root: u64,

    /// Physical address of the ACPI RSDP, or 0 if the firmware had none.
    pub rsdp_addr: u64,

    /// UEFI firmware revision (from the system table).
    pub firmware_revision: u32,
    /// 1 if NX (no-execute) is enabled and used in the page tables.
    pub nx_enabled: u32,
}

impl BootInfo {
    /// True if the magic, version and size all match this build.
    pub fn is_valid(&self) -> bool {
        self.magic == MAGIC
            && self.version == VERSION
            && self.size as usize == core::mem::size_of::<BootInfo>()
    }

    /// Converts a physical address from this structure into a pointer.
    pub fn phys_to_virt(&self, phys: u64) -> u64 {
        phys + self.physical_memory_offset
    }

    /// The memory map as a slice.
    ///
    /// # Safety
    /// The `BootInfo` must have come from NovaBoot and the direct map must
    /// still be in place.
    pub unsafe fn memory_map(&self) -> &[MemoryRegion] {
        let ptr = self.phys_to_virt(self.memory_map_addr) as *const MemoryRegion;
        // SAFETY: guaranteed by the caller / boot protocol.
        unsafe { core::slice::from_raw_parts(ptr, self.memory_map_len as usize) }
    }
}

/// A linear 32-bit framebuffer. `addr == 0` means there is none.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Framebuffer {
    pub addr: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    /// Pixels (not bytes) per row.
    pub stride: u32,
    pub bytes_per_pixel: u32,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub _reserved: u32,
}

impl Framebuffer {
    pub fn is_present(&self) -> bool {
        self.addr != 0 && self.width != 0 && self.height != 0 && self.bytes_per_pixel == 4
    }

    /// Packs an 8-bit-per-channel colour into this framebuffer's pixel format.
    pub fn pack(&self, r: u8, g: u8, b: u8) -> u32 {
        channel(r, self.red_mask) | channel(g, self.green_mask) | channel(b, self.blue_mask)
    }
}

fn channel(value: u8, mask: u32) -> u32 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones().min(8);
    ((value as u32) >> (8 - bits)) << shift & mask
}

/// What a range of physical memory is.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    /// Free RAM.
    Usable = 1,
    Reserved = 2,
    /// ACPI tables; usable once the kernel has parsed them.
    AcpiReclaimable = 3,
    AcpiNvs = 4,
    /// In use by the handoff: kernel image, kernel stack, page tables,
    /// boot info, and the bootloader itself. Keep until the kernel has
    /// replaced them.
    Loader = 5,
    /// UEFI boot-services code/data, which includes the GDT/IDT still
    /// loaded at handoff. Usable once the kernel has its own GDT and IDT.
    FirmwareReclaimable = 6,
    /// UEFI runtime services. Never reuse.
    FirmwareRuntime = 7,
    Mmio = 8,
    Bad = 9,
}

impl MemoryKind {
    /// Maps a UEFI `EFI_MEMORY_TYPE` to a Nova memory kind.
    pub fn from_uefi(ty: u32) -> Self {
        match ty {
            7 => Self::Usable,                     // ConventionalMemory
            1 | 2 => Self::Loader,                 // LoaderCode / LoaderData
            3 | 4 => Self::FirmwareReclaimable,    // BootServicesCode / Data
            5 | 6 => Self::FirmwareRuntime,        // RuntimeServicesCode / Data
            8 => Self::Bad,                        // UnusableMemory
            9 => Self::AcpiReclaimable,            // ACPIReclaimMemory
            10 => Self::AcpiNvs,                   // ACPIMemoryNVS
            11 | 12 => Self::Mmio,                 // MemoryMappedIO / PortSpace
            _ => Self::Reserved,                   // PalCode, Persistent, OEM, ...
        }
    }

    fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Self::Usable,
            3 => Self::AcpiReclaimable,
            4 => Self::AcpiNvs,
            5 => Self::Loader,
            6 => Self::FirmwareReclaimable,
            7 => Self::FirmwareRuntime,
            8 => Self::Mmio,
            9 => Self::Bad,
            _ => Self::Reserved,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::Reserved => "reserved",
            Self::AcpiReclaimable => "acpi-reclaimable",
            Self::AcpiNvs => "acpi-nvs",
            Self::Loader => "loader",
            Self::FirmwareReclaimable => "firmware-reclaimable",
            Self::FirmwareRuntime => "firmware-runtime",
            Self::Mmio => "mmio",
            Self::Bad => "bad",
        }
    }
}

/// One range of physical memory. `kind` is stored raw so that reading a map
/// written by a different build can never produce an invalid enum.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRegion {
    pub start: u64,
    pub len: u64,
    kind: u32,
    _pad: u32,
}

impl MemoryRegion {
    pub const fn new(start: u64, len: u64, kind: MemoryKind) -> Self {
        Self { start, len, kind: kind as u32, _pad: 0 }
    }

    pub fn kind(&self) -> MemoryKind {
        MemoryKind::from_raw(self.kind)
    }

    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.len)
    }
}

/// Sorts regions by start address, drops empty ones and merges neighbours
/// of the same kind. Returns the new length; the tail is left untouched.
pub fn normalize(regions: &mut [MemoryRegion]) -> usize {
    // Insertion sort: no allocation, and UEFI maps are nearly sorted already.
    for i in 1..regions.len() {
        let mut j = i;
        while j > 0 && regions[j - 1].start > regions[j].start {
            regions.swap(j - 1, j);
            j -= 1;
        }
    }

    let mut out = 0;
    for i in 0..regions.len() {
        let region = regions[i];
        if region.len == 0 {
            continue;
        }
        if out > 0 {
            let prev = &mut regions[out - 1];
            if prev.kind == region.kind && prev.end() == region.start {
                prev.len += region.len;
                continue;
            }
        }
        regions[out] = region;
        out += 1;
    }
    out
}

/// Total bytes of the given kind.
pub fn total_of(regions: &[MemoryRegion], kind: MemoryKind) -> u64 {
    regions.iter().filter(|r| r.kind() == kind).map(|r| r.len).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    #[test]
    fn magic_spells_novaboot() {
        assert_eq!(&MAGIC.to_le_bytes(), b"NOVABOOT");
    }

    #[test]
    fn layout_is_canonical_and_ordered() {
        use layout::*;
        // Higher half, and the kernel/stack windows sit above the direct map.
        const {
            assert!(PHYSICAL_MEMORY_OFFSET >= 0xFFFF_8000_0000_0000);
            assert!(PHYSICAL_MEMORY_OFFSET + MAX_DIRECT_MAP <= KERNEL_STACK_TOP - KERNEL_STACK_PAGES * 4096);
            assert!(KERNEL_STACK_TOP <= KERNEL_BASE);
            assert!(KERNEL_STACK_TOP.is_multiple_of(4096));
        }
    }

    #[test]
    fn uefi_types_map_to_kinds() {
        assert_eq!(MemoryKind::from_uefi(7), MemoryKind::Usable);
        assert_eq!(MemoryKind::from_uefi(2), MemoryKind::Loader);
        assert_eq!(MemoryKind::from_uefi(4), MemoryKind::FirmwareReclaimable);
        assert_eq!(MemoryKind::from_uefi(6), MemoryKind::FirmwareRuntime);
        assert_eq!(MemoryKind::from_uefi(9), MemoryKind::AcpiReclaimable);
        assert_eq!(MemoryKind::from_uefi(0x8000_0001), MemoryKind::Reserved);
    }

    #[test]
    fn kind_round_trips_through_raw() {
        for ty in 0..16 {
            let kind = MemoryKind::from_uefi(ty);
            assert_eq!(MemoryRegion::new(0, 1, kind).kind(), kind);
        }
    }

    #[test]
    fn unknown_raw_kind_reads_as_reserved() {
        let mut r = MemoryRegion::new(0, 4096, MemoryKind::Usable);
        r.kind = 77;
        assert_eq!(r.kind(), MemoryKind::Reserved);
    }

    #[test]
    fn normalize_sorts_merges_and_drops_empty() {
        let u = MemoryKind::Usable;
        let l = MemoryKind::Loader;
        let mut map = vec![
            MemoryRegion::new(0x3000, 0x1000, u),
            MemoryRegion::new(0x0000, 0x1000, u),
            MemoryRegion::new(0x9000, 0x0000, u), // empty
            MemoryRegion::new(0x1000, 0x2000, u), // merges with 0x0000
            MemoryRegion::new(0x4000, 0x1000, l), // different kind: kept apart
            MemoryRegion::new(0x6000, 0x1000, l), // gap: kept apart
        ];
        let n = normalize(&mut map);
        assert_eq!(
            &map[..n],
            &[
                MemoryRegion::new(0x0000, 0x4000, u),
                MemoryRegion::new(0x4000, 0x1000, l),
                MemoryRegion::new(0x6000, 0x1000, l),
            ]
        );
        assert_eq!(total_of(&map[..n], u), 0x4000);
    }

    #[test]
    fn normalize_handles_empty_and_single() {
        let mut none: [MemoryRegion; 0] = [];
        assert_eq!(normalize(&mut none), 0);
        let mut one = [MemoryRegion::new(5, 0, MemoryKind::Usable)];
        assert_eq!(normalize(&mut one), 0);
    }

    #[test]
    fn pack_rgb_and_bgr() {
        let rgb = Framebuffer { red_mask: 0xFF, green_mask: 0xFF00, blue_mask: 0xFF_0000, ..Default::default() };
        assert_eq!(rgb.pack(0x11, 0x22, 0x33), 0x0033_2211);
        let bgr = Framebuffer { red_mask: 0xFF_0000, green_mask: 0xFF00, blue_mask: 0xFF, ..Default::default() };
        assert_eq!(bgr.pack(0x11, 0x22, 0x33), 0x0011_2233);
    }

    #[test]
    fn pack_narrow_masks_keep_high_bits() {
        // 5-6-5 style masks.
        let fb = Framebuffer { red_mask: 0xF800, green_mask: 0x07E0, blue_mask: 0x001F, ..Default::default() };
        assert_eq!(fb.pack(0xFF, 0xFF, 0xFF), 0xFFFF);
        assert_eq!(fb.pack(0x80, 0, 0), 0x8000);
        assert_eq!(fb.pack(0, 0, 0), 0);
    }

    #[test]
    fn framebuffer_presence() {
        let mut fb = Framebuffer { addr: 0x8000_0000, width: 800, height: 600, stride: 800, bytes_per_pixel: 4, ..Default::default() };
        assert!(fb.is_present());
        fb.addr = 0;
        assert!(!fb.is_present());
    }

    #[test]
    fn boot_info_validity() {
        let mut info = BootInfo {
            magic: MAGIC,
            version: VERSION,
            size: core::mem::size_of::<BootInfo>() as u32,
            physical_memory_offset: 0,
            physical_memory_mapped: 0,
            memory_map_addr: 0,
            memory_map_len: 0,
            framebuffer: Framebuffer::default(),
            kernel_phys: 0,
            kernel_virt: 0,
            kernel_pages: 0,
            stack_phys: 0,
            stack_pages: 0,
            stack_top: 0,
            page_table_root: 0,
            rsdp_addr: 0,
            firmware_revision: 0,
            nx_enabled: 0,
        };
        assert!(info.is_valid());
        info.version += 1;
        assert!(!info.is_valid());
    }
}
