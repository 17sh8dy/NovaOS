//! SATA disks through an AHCI controller.
//!
//! The controller keeps, per port, a *command list* (32 slots) and a *received
//! FIS* area in memory, and each command is a *command table* holding the ATA
//! command and a list of buffers (PRDT) to DMA to or from. We use one slot and
//! one 4 KiB buffer per port, issue a command by setting a bit in `PxCI`, and
//! poll until the controller clears it. (Interrupts would let the CPU do other
//! work meanwhile; polling is simpler and, with the kernel's sleeping mutex
//! around it, does not block other threads.)
//!
//! A disk is exposed to the file system as a `BlockDevice` of 4 KiB blocks
//! (8 sectors each).

use crate::kmutex::KMutex;
use crate::{bootdata, interrupts, pci, pmm, vm};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};
use nova_fs::{BLOCK_SIZE, BlockDevice, FsError};

// HBA registers.
const HBA_CAP: usize = 0x00;
const HBA_GHC: usize = 0x04;
const HBA_PI: usize = 0x0C;
const GHC_AE: u32 = 1 << 31;
const CAP_S64A: u32 = 1 << 31;

// Per-port registers (offsets from the port's base).
const P_CLB: usize = 0x00;
const P_CLBU: usize = 0x04;
const P_FB: usize = 0x08;
const P_FBU: usize = 0x0C;
const P_IS: usize = 0x10;
const P_CMD: usize = 0x18;
const P_TFD: usize = 0x20;
const P_SIG: usize = 0x24;
const P_SSTS: usize = 0x28;
const P_SERR: usize = 0x30;
const P_CI: usize = 0x38;

const CMD_ST: u32 = 1 << 0;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;
const TFD_ERR: u32 = 1 << 0;
const TFD_DRQ: u32 = 1 << 3;
const TFD_BSY: u32 = 1 << 7;
const IS_TFES: u32 = 1 << 30;
const SIG_SATA: u32 = 0x0000_0101;

// ATA commands.
const ATA_IDENTIFY: u8 = 0xEC;
const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_FLUSH_EXT: u8 = 0xEA;

const SECTOR: usize = 512;
const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / SECTOR) as u64;

/// Layout of a port's command memory frame.
const FIS_OFFSET: usize = 0x400;
const TABLE_OFFSET: usize = 0x800;

struct Port {
    /// Register block of this port (virtual address).
    regs: u64,
    /// Command list / FIS / command table frame, and the data buffer frame (virtual addresses).
    cmd_virt: u64,
    cmd_phys: u64,
    buf_virt: u64,
    buf_phys: u64,
    sectors: u64,
    model: String,
}

struct Controller {
    ports: Vec<Port>,
}

static CONTROLLER: KMutex<Controller> = KMutex::new(Controller { ports: Vec::new() });

fn rd(base: u64, off: usize) -> u32 {
    // SAFETY: `base` is a mapped HBA register block.
    unsafe { ((base + off as u64) as *const u32).read_volatile() }
}

fn wr(base: u64, off: usize, v: u32) {
    // SAFETY: as above.
    unsafe { ((base + off as u64) as *mut u32).write_volatile(v) }
}

/// Spins until `cond` holds or about `ms` milliseconds have passed.
fn wait_until(ms: u64, mut cond: impl FnMut() -> bool) -> bool {
    let ticks = (ms * interrupts::TICK_HZ as u64).div_ceil(1000).max(1);
    let deadline = interrupts::ticks() + ticks;
    let mut spins = 0u64;
    loop {
        if cond() {
            return true;
        }
        spins += 1;
        // Without timer ticks (very early boot) fall back to a spin count.
        if interrupts::ticks() >= deadline && spins > 1000 || spins > 200_000_000 {
            return false;
        }
        core::hint::spin_loop();
    }
}

impl Port {
    /// Issues one command in slot 0 and waits for it. `data` is the number of
    /// bytes the command moves through the data buffer (0 for none).
    fn command(&mut self, ata: u8, lba: u64, count: u16, write: bool, data: usize) -> Result<(), &'static str> {
        if !wait_until(1000, || rd(self.regs, P_TFD) & (TFD_BSY | TFD_DRQ) == 0) {
            return Err("the drive is busy");
        }
        let table = self.cmd_virt + TABLE_OFFSET as u64;
        // SAFETY: the command frame is ours (allocated in `init`), 4 KiB, in the direct map.
        unsafe {
            // Command header, slot 0: FIS length 5 dwords, direction, one PRDT entry (if any data).
            let hdr = self.cmd_virt as *mut u32;
            hdr.write_volatile(5 | (write as u32) << 6 | ((data > 0) as u32) << 16);
            hdr.add(1).write_volatile(0); // bytes transferred: filled in by the HBA
            hdr.add(2).write_volatile((self.cmd_phys + TABLE_OFFSET as u64) as u32);
            hdr.add(3).write_volatile(((self.cmd_phys + TABLE_OFFSET as u64) >> 32) as u32);

            // Command table: the H2D register FIS, then the PRDT at +0x80.
            core::ptr::write_bytes(table as *mut u8, 0, 0x90);
            let fis = table as *mut u8;
            fis.write(0x27); // register FIS, host to device
            fis.add(1).write(0x80); // this FIS carries a command
            fis.add(2).write(ata);
            fis.add(4).write(lba as u8);
            fis.add(5).write((lba >> 8) as u8);
            fis.add(6).write((lba >> 16) as u8);
            fis.add(7).write(0x40); // LBA addressing
            fis.add(8).write((lba >> 24) as u8);
            fis.add(9).write((lba >> 32) as u8);
            fis.add(10).write((lba >> 40) as u8);
            fis.add(12).write(count as u8);
            fis.add(13).write((count >> 8) as u8);
            if data > 0 {
                let prd = (table + 0x80) as *mut u32;
                prd.write_volatile(self.buf_phys as u32);
                prd.add(1).write_volatile((self.buf_phys >> 32) as u32);
                prd.add(2).write_volatile(0);
                prd.add(3).write_volatile((data - 1) as u32);
            }
        }
        wr(self.regs, P_IS, !0);
        wr(self.regs, P_SERR, !0);
        fence(Ordering::SeqCst);
        wr(self.regs, P_CI, 1);
        let mut failed = false;
        let done = wait_until(5000, || {
            if rd(self.regs, P_IS) & IS_TFES != 0 {
                failed = true;
                return true;
            }
            rd(self.regs, P_CI) & 1 == 0
        });
        if failed || rd(self.regs, P_TFD) & TFD_ERR != 0 {
            // Recover the port: stop and restart command processing.
            self.restart();
            return Err("the drive reported an error");
        }
        if !done {
            self.restart();
            return Err("the drive did not answer in time");
        }
        Ok(())
    }

    fn stop(&mut self) -> bool {
        let cmd = rd(self.regs, P_CMD);
        wr(self.regs, P_CMD, cmd & !(CMD_ST | CMD_FRE));
        wait_until(500, || rd(self.regs, P_CMD) & (CMD_CR | CMD_FR) == 0)
    }

    fn start(&mut self) {
        wait_until(500, || rd(self.regs, P_CMD) & CMD_CR == 0);
        let cmd = rd(self.regs, P_CMD);
        wr(self.regs, P_CMD, cmd | CMD_FRE);
        wr(self.regs, P_CMD, rd(self.regs, P_CMD) | CMD_ST);
    }

    fn restart(&mut self) {
        self.stop();
        wr(self.regs, P_SERR, !0);
        wr(self.regs, P_IS, !0);
        self.start();
    }

    fn data(&self) -> &[u8] {
        // SAFETY: the buffer frame is ours and 4 KiB.
        unsafe { core::slice::from_raw_parts(self.buf_virt as *const u8, BLOCK_SIZE) }
    }

    fn data_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above.
        unsafe { core::slice::from_raw_parts_mut(self.buf_virt as *mut u8, BLOCK_SIZE) }
    }
}

pub struct DiskInfo {
    pub index: usize,
    pub model: String,
    pub sectors: u64,
}

/// A frame below 4 GiB (or anywhere, if the controller does 64-bit DMA), zeroed.
fn dma_frame(s64a: bool, direct: u64) -> Option<u64> {
    let f = pmm::alloc_frame()?;
    if !s64a && f >= 1 << 32 {
        let _ = pmm::free_frame(f);
        return None;
    }
    // SAFETY: a frame we own, in the direct map.
    unsafe { core::ptr::write_bytes((direct + f) as *mut u8, 0, 4096) };
    Some(f)
}

/// Finds AHCI controllers and the SATA disks on them.
pub fn init(devices: &[pci::PciDevice]) -> Vec<DiskInfo> {
    let mut disks = Vec::new();
    let direct = bootdata::info().physical_memory_offset;
    let mut ctl = CONTROLLER.lock();
    for dev in devices.iter().filter(|d| d.class == 0x01 && d.subclass == 0x06 && d.prog_if == 0x01) {
        let Some(abar) = pci::bar_memory(dev, 5) else {
            crate::kprintln!("ahci: {} has no usable register BAR", dev.address());
            continue;
        };
        pci::enable_dma(dev);
        let Ok(hba) = vm::map_mmio(abar, 0x2000) else {
            crate::kprintln!("ahci: could not map the registers of {}", dev.address());
            continue;
        };
        wr(hba, HBA_GHC, rd(hba, HBA_GHC) | GHC_AE);
        let s64a = rd(hba, HBA_CAP) & CAP_S64A != 0;
        let implemented = rd(hba, HBA_PI);
        for p in 0..32 {
            if implemented & (1 << p) == 0 {
                continue;
            }
            let regs = hba + 0x100 + 0x80 * p as u64;
            // Something present and talking? (SStatus: DET = 3, IPM = 1)
            let ssts = rd(regs, P_SSTS);
            if ssts & 0xF != 3 || (ssts >> 8) & 0xF != 1 || rd(regs, P_SIG) != SIG_SATA {
                continue;
            }
            let (Some(cmd_phys), Some(buf_phys)) = (dma_frame(s64a, direct), dma_frame(s64a, direct)) else {
                crate::kprintln!("ahci: no DMA memory for port {p}");
                continue;
            };
            let mut port = Port { regs, cmd_virt: direct + cmd_phys, cmd_phys, buf_virt: direct + buf_phys, buf_phys, sectors: 0, model: String::new() };
            if !port.stop() {
                crate::kprintln!("ahci: port {p} would not stop");
                continue;
            }
            wr(regs, P_CLB, cmd_phys as u32);
            wr(regs, P_CLBU, (cmd_phys >> 32) as u32);
            wr(regs, P_FB, (cmd_phys + FIS_OFFSET as u64) as u32);
            wr(regs, P_FBU, ((cmd_phys + FIS_OFFSET as u64) >> 32) as u32);
            wr(regs, P_SERR, !0);
            wr(regs, P_IS, !0);
            port.start();
            if let Err(e) = port.command(ATA_IDENTIFY, 0, 0, false, SECTOR) {
                crate::kprintln!("ahci: port {p}: IDENTIFY failed: {e}");
                continue;
            }
            let id: Vec<u16> = port.data()[..SECTOR].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let lba48 = id[83] & (1 << 10) != 0;
            let sectors = if lba48 { id[100] as u64 | (id[101] as u64) << 16 | (id[102] as u64) << 32 | (id[103] as u64) << 48 } else { id[60] as u64 | (id[61] as u64) << 16 };
            let logical = if id[106] & (1 << 14) != 0 && id[106] & (1 << 15) == 0 && id[106] & (1 << 12) != 0 {
                (id[117] as u32 | (id[118] as u32) << 16) as usize * 2
            } else {
                SECTOR
            };
            let mut model = String::new();
            for w in &id[27..47] {
                model.push((w >> 8) as u8 as char);
                model.push(*w as u8 as char);
            }
            let model = String::from(model.trim());
            if logical != SECTOR {
                crate::kprintln!("ahci: port {p}: {model}: {logical}-byte sectors are not supported, skipped");
                continue;
            }
            if !lba48 {
                crate::kprintln!("ahci: port {p}: {model}: no 48-bit LBA, skipped");
                continue;
            }
            port.sectors = sectors;
            port.model = model.clone();
            disks.push(DiskInfo { index: ctl.ports.len(), model, sectors });
            ctl.ports.push(port);
        }
    }
    disks
}

/// A disk, as the file system sees it.
pub struct AhciDisk {
    index: usize,
}

impl AhciDisk {
    pub fn new(index: usize) -> Self {
        Self { index }
    }
}

fn io_error(what: &str, block: u64, e: &str) -> FsError {
    crate::kprintln!("ahci: {what} of block {block} failed: {e}");
    FsError::Io
}

impl BlockDevice for AhciDisk {
    fn block_count(&self) -> u64 {
        CONTROLLER.lock().ports.get(self.index).map_or(0, |p| p.sectors / SECTORS_PER_BLOCK)
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8; BLOCK_SIZE]) -> nova_fs::Result<()> {
        let mut ctl = CONTROLLER.lock();
        let port = ctl.ports.get_mut(self.index).ok_or(FsError::Io)?;
        if block >= port.sectors / SECTORS_PER_BLOCK {
            return Err(FsError::Io);
        }
        port.command(ATA_READ_DMA_EXT, block * SECTORS_PER_BLOCK, SECTORS_PER_BLOCK as u16, false, BLOCK_SIZE).map_err(|e| io_error("read", block, e))?;
        buf.copy_from_slice(port.data());
        Ok(())
    }

    fn write_block(&mut self, block: u64, buf: &[u8; BLOCK_SIZE]) -> nova_fs::Result<()> {
        let mut ctl = CONTROLLER.lock();
        let port = ctl.ports.get_mut(self.index).ok_or(FsError::Io)?;
        if block >= port.sectors / SECTORS_PER_BLOCK {
            return Err(FsError::Io);
        }
        port.data_mut().copy_from_slice(buf);
        port.command(ATA_WRITE_DMA_EXT, block * SECTORS_PER_BLOCK, SECTORS_PER_BLOCK as u16, true, BLOCK_SIZE).map_err(|e| io_error("write", block, e))
    }

    fn flush(&mut self) -> nova_fs::Result<()> {
        let mut ctl = CONTROLLER.lock();
        let port = ctl.ports.get_mut(self.index).ok_or(FsError::Io)?;
        port.command(ATA_FLUSH_EXT, 0, 0, false, 0).map_err(|e| io_error("flush", 0, e))
    }
}
