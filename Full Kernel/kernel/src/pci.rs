//! PCI: finding the devices on the bus, using the classic port-I/O
//! configuration mechanism (ports 0xCF8/0xCFC), which every x86 PC supports
//! for the standard 256-byte configuration header.

use alloc::vec::Vec;
use nova_x86::{inl, outl};

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

#[derive(Clone, Copy, Debug)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
}

impl PciDevice {
    pub fn address(&self) -> alloc::string::String {
        alloc::format!("{:02x}:{:02x}.{}", self.bus, self.device, self.function)
    }

    /// A human name for the device class, where we know it.
    pub fn class_name(&self) -> &'static str {
        match (self.class, self.subclass, self.prog_if) {
            (0x01, 0x06, 0x01) => "SATA controller (AHCI)",
            (0x01, 0x08, 0x02) => "NVM Express controller",
            (0x01, 0x01, _) => "IDE controller",
            (0x01, _, _) => "storage controller",
            (0x02, _, _) => "network controller",
            (0x03, _, _) => "display controller",
            (0x06, 0x00, _) => "host bridge",
            (0x06, 0x01, _) => "ISA bridge",
            (0x06, _, _) => "bridge",
            (0x0C, 0x03, 0x30) => "USB controller (xHCI)",
            (0x0C, 0x03, _) => "USB controller",
            (0x0C, 0x05, _) => "SMBus controller",
            _ => "device",
        }
    }
}

fn config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000 | (bus as u32) << 16 | (device as u32) << 11 | (function as u32) << 8 | (offset as u32 & 0xFC)
}

pub fn read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    // SAFETY: the kernel owns the PCI configuration ports.
    unsafe {
        outl(CONFIG_ADDRESS, config_address(bus, device, function, offset));
        inl(CONFIG_DATA)
    }
}

pub fn write32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    // SAFETY: as above.
    unsafe {
        outl(CONFIG_ADDRESS, config_address(bus, device, function, offset));
        outl(CONFIG_DATA, value);
    }
}

/// Finds every PCI function on buses 0..=255.
pub fn enumerate() -> Vec<PciDevice> {
    let mut found = Vec::new();
    for bus in 0..=255u8 {
        for device in 0..32u8 {
            let id = read32(bus, device, 0, 0);
            if id & 0xFFFF == 0xFFFF {
                continue;
            }
            let multi = read32(bus, device, 0, 0x0C) >> 16 & 0x80 != 0;
            for function in 0..if multi { 8 } else { 1 } {
                let id = read32(bus, device, function, 0);
                if id & 0xFFFF == 0xFFFF {
                    continue;
                }
                let class = read32(bus, device, function, 0x08);
                found.push(PciDevice {
                    bus,
                    device,
                    function,
                    vendor: id as u16,
                    device_id: (id >> 16) as u16,
                    class: (class >> 24) as u8,
                    subclass: (class >> 16) as u8,
                    prog_if: (class >> 8) as u8,
                });
            }
        }
    }
    found
}

/// The memory-mapped base address of BAR `index` (0..=5), if it is a memory BAR.
pub fn bar_memory(d: &PciDevice, index: u8) -> Option<u64> {
    let off = 0x10 + index * 4;
    let lo = read32(d.bus, d.device, d.function, off);
    if lo & 1 != 0 {
        return None; // an I/O-port BAR
    }
    let mut base = (lo & !0xF) as u64;
    if (lo >> 1) & 3 == 2 {
        base |= (read32(d.bus, d.device, d.function, off + 4) as u64) << 32; // 64-bit BAR
    }
    (base != 0).then_some(base)
}

/// Turns on memory-space decoding and bus mastering (DMA) for a device.
pub fn enable_dma(d: &PciDevice) {
    let cmd = read32(d.bus, d.device, d.function, 0x04);
    write32(d.bus, d.device, d.function, 0x04, (cmd | 0x6) & 0xFFFF); // memory space + bus master
}
