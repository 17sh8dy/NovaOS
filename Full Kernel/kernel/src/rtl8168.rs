//! Realtek RTL8168 / RTL8111 gigabit Ethernet (the ASUS PRIME B550-PLUS has an
//! RTL8111H). Polled, like the virtio-net driver: [`Rtl8168::recv`] and
//! [`Rtl8168::send`] are called from the network thread.
//!
//! What it does: a software reset, two rings of 16 descriptors ("C+ mode"),
//! 2 KiB buffers, receive accepts our address, broadcasts and multicast. It
//! never writes the EEPROM, the MAC address, Wake-on-LAN or any setting that
//! outlives a power cycle. If the link is down after start-up it checks the PHY
//! and, only if the firmware left it powered down, powers it up and restarts
//! auto-negotiation (registers in RAM).
//!
//! **Not testable in QEMU** (it emulates no RTL8168): every step logs what it
//! saw, so a photo of the boot log on the real PC tells what happened.
//! Pure logic (descriptors, chip ids, PHY commands) is host-tested in `crates/realtek`.

use crate::pci::{self, PciDevice};
use crate::{bootdata, interrupts, kprintln, pmm, vm};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};
use nova_realtek::{self as rt, RxStatus};

const RING: usize = 16;
const BUF: usize = 2048;

fn direct() -> u64 {
    bootdata::info().physical_memory_offset
}

pub struct Rtl8168 {
    pub mac: [u8; 6],
    regs: u64,
    xid: u16,
    new_phy: bool,
    /// Descriptor rings (virtual addresses; one page each, 256-byte aligned).
    rx_ring: u64,
    tx_ring: u64,
    rx_bufs: Vec<u64>,
    tx_bufs: Vec<u64>,
    rx_next: usize,
    tx_next: usize,
    tx_clean: usize,
    pub rx_errors: u64,
}

impl Rtl8168 {
    fn r8(&self, off: u64) -> u8 {
        // SAFETY: inside the NIC's mapped register block.
        unsafe { ((self.regs + off) as *const u8).read_volatile() }
    }
    fn r16(&self, off: u64) -> u16 {
        // SAFETY: as above.
        unsafe { ((self.regs + off) as *const u16).read_volatile() }
    }
    fn r32(&self, off: u64) -> u32 {
        // SAFETY: as above.
        unsafe { ((self.regs + off) as *const u32).read_volatile() }
    }
    fn w8(&self, off: u64, v: u8) {
        // SAFETY: as above.
        unsafe { ((self.regs + off) as *mut u8).write_volatile(v) }
    }
    fn w16(&self, off: u64, v: u16) {
        // SAFETY: as above.
        unsafe { ((self.regs + off) as *mut u16).write_volatile(v) }
    }
    fn w32(&self, off: u64, v: u32) {
        // SAFETY: as above.
        unsafe { ((self.regs + off) as *mut u32).write_volatile(v) }
    }

    /// Reads PHY register `reg` (None if the PHY does not answer).
    fn phy_read(&self, reg: u8) -> Option<u16> {
        let port = if self.new_phy { rt::GPHY_OCP } else { rt::PHYAR };
        self.w32(port, rt::phy_read_cmd(self.new_phy, reg));
        let mut value = None;
        interrupts::wait_until(10, || {
            let v = self.r32(port);
            if v & rt::PHY_FLAG != 0 {
                value = Some(v as u16);
                true
            } else {
                false
            }
        });
        value
    }

    fn phy_write(&self, reg: u8, value: u16) -> bool {
        let port = if self.new_phy { rt::GPHY_OCP } else { rt::PHYAR };
        self.w32(port, rt::phy_write_cmd(self.new_phy, reg, value));
        interrupts::wait_until(10, || self.r32(port) & rt::PHY_FLAG == 0)
    }

    pub fn link(&self) -> rt::Link {
        rt::link(self.r8(rt::PHY_STATUS))
    }

    pub fn chip(&self) -> &'static str {
        rt::chip_name(self.xid)
    }

    pub fn probe(d: &PciDevice) -> Result<Self, String> {
        // The register block: BAR 2 on the PCIe chips (BAR 0 is I/O ports), BAR 1 on old ones.
        let (base, size) = pci::bar_memory_size(d, 2).or_else(|| pci::bar_memory_size(d, 1)).ok_or("no memory-mapped register BAR")?;
        if size < 256 {
            return Err(alloc::format!("register BAR too small ({size} bytes)"));
        }
        pci::enable_dma(d);
        pci::disable_intx(d);
        let regs = vm::map_mmio(base, 256).map_err(|_| "could not map the registers")?;
        let mut nic = Self {
            mac: [0; 6],
            regs,
            xid: 0,
            new_phy: false,
            rx_ring: 0,
            tx_ring: 0,
            rx_bufs: Vec::new(),
            tx_bufs: Vec::new(),
            rx_next: 0,
            tx_next: 0,
            tx_clean: 0,
            rx_errors: 0,
        };
        let txc = nic.r32(rt::TX_CONFIG);
        if txc == 0xFFFF_FFFF {
            return Err(String::from("the registers read as all ones (device asleep or not decoding)"));
        }
        nic.xid = rt::xid(txc);
        nic.new_phy = rt::is_8168g_or_newer(nic.xid);
        kprintln!("rtl8168: {} {:04x}:{:04x}: {} (version id {:#05x}), registers at {base:#x}", d.address(), d.vendor, d.device_id, nic.chip(), nic.xid);
        if nic.chip().starts_with("RTL8125") {
            return Err(String::from("this is an RTL8125 (2.5G), which needs its own driver"));
        }

        // Quiet: no interrupts (we poll), nothing pending.
        nic.w16(rt::INTR_MASK, 0);
        nic.w16(rt::INTR_STATUS, 0xFFFF);

        // Software reset; the chip clears the bit when done.
        nic.w8(rt::CHIP_CMD, rt::CMD_RESET);
        if !interrupts::wait_until(100, || nic.r8(rt::CHIP_CMD) & rt::CMD_RESET == 0) {
            return Err(String::from("the chip did not come out of reset"));
        }

        for i in 0..6 {
            nic.mac[i] = nic.r8(rt::IDR0 + i as u64);
        }
        if nic.mac == [0; 6] || nic.mac == [0xFF; 6] {
            return Err(String::from("no MAC address in the chip"));
        }

        // Descriptor rings and buffers.
        let (rx_ring, tx_ring) = (pmm::alloc_dma_frame(false).ok_or("out of memory")?, pmm::alloc_dma_frame(false).ok_or("out of memory")?);
        nic.rx_ring = direct() + rx_ring;
        nic.tx_ring = direct() + tx_ring;
        for bufs in [&mut nic.rx_bufs, &mut nic.tx_bufs] {
            while bufs.len() < RING {
                let f = pmm::alloc_dma_frame(false).ok_or("out of memory")?;
                for k in 0..4096 / BUF {
                    bufs.push(f + (k * BUF) as u64);
                }
            }
            bufs.truncate(RING);
        }
        for i in 0..RING {
            let d = nic.rx_ring + i as u64 * rt::DESC_BYTES;
            // SAFETY: descriptor `i` of our receive ring.
            unsafe {
                ((d + 8) as *mut u64).write_volatile(nic.rx_bufs[i]);
                ((d + 4) as *mut u32).write_volatile(0);
            }
            fence(Ordering::SeqCst);
            // SAFETY: as above; the ownership bit goes last.
            unsafe { (d as *mut u32).write_volatile(rt::rx_desc(BUF as u32, i == RING - 1)) };
            let t = nic.tx_ring + i as u64 * rt::DESC_BYTES;
            // SAFETY: descriptor `i` of our transmit ring: not the NIC's (no OWN).
            unsafe {
                (t as *mut u32).write_volatile(if i == RING - 1 { rt::EOR } else { 0 });
                ((t + 4) as *mut u32).write_volatile(0);
                ((t + 8) as *mut u64).write_volatile(nic.tx_bufs[i]);
            }
        }
        fence(Ordering::SeqCst);

        // Configure (config registers unlocked only for this).
        nic.w8(rt::CFG9346, rt::CFG9346_UNLOCK);
        nic.w8(rt::MAX_TX_PACKET, 0x3F);
        nic.w16(rt::RX_MAX_SIZE, BUF as u16);
        let cplus = nic.r16(rt::CPLUS_CMD) & !(rt::CPLUS_RX_VLAN | rt::CPLUS_RX_CHECKSUM);
        nic.w16(rt::CPLUS_CMD, cplus);
        nic.w32(rt::TX_DESC_HI, (tx_ring >> 32) as u32);
        nic.w32(rt::TX_DESC_LO, tx_ring as u32);
        nic.w32(rt::RX_DESC_HI, (rx_ring >> 32) as u32);
        nic.w32(rt::RX_DESC_LO, rx_ring as u32);
        nic.w8(rt::CHIP_CMD, rt::CMD_TX_ENABLE | rt::CMD_RX_ENABLE);
        nic.w32(rt::RX_CONFIG, rt::rx_config(nic.xid));
        nic.w32(rt::TX_CONFIG, rt::tx_config());
        // All multicast groups (IPv4 needs none yet; this keeps the filter simple).
        nic.w32(rt::MAR0, 0xFFFF_FFFF);
        nic.w32(rt::MAR0 + 4, 0xFFFF_FFFF);
        nic.w8(rt::CFG9346, rt::CFG9346_LOCK);
        let _ = nic.r8(rt::CHIP_CMD); // flush posted writes

        nic.bring_up_phy();
        Ok(nic)
    }

    /// Waits for the link; if it stays down, looks at the PHY and wakes it if it was powered down.
    fn bring_up_phy(&self) {
        let bmcr = self.phy_read(rt::MII_BMCR);
        let bmsr = self.phy_read(rt::MII_BMSR);
        kprintln!(
            "rtl8168: PHY via {}: BMCR {}, BMSR {}",
            if self.new_phy { "OCP" } else { "PHYAR" },
            bmcr.map_or(String::from("no answer"), |v| alloc::format!("{v:#06x}")),
            bmsr.map_or(String::from("no answer"), |v| alloc::format!("{v:#06x}"))
        );
        if let Some(b) = bmcr
            && b & rt::BMCR_POWER_DOWN != 0
        {
            let ok = self.phy_write(rt::MII_BMCR, rt::BMCR_ANENABLE | rt::BMCR_ANRESTART);
            kprintln!("rtl8168: the PHY was powered down: powering it up and restarting auto-negotiation ({})", if ok { "done" } else { "no answer" });
        }
        // Auto-negotiation takes a few seconds; DHCP keeps retrying meanwhile, so don't wait long here.
        interrupts::wait_until(3000, || self.link().up);
        let l = self.link();
        if l.up {
            kprintln!("rtl8168: link up, {} Mb/s, {} duplex", l.mbps, if l.full_duplex { "full" } else { "half" });
        } else {
            kprintln!("rtl8168: no link yet (cable plugged in? the other end on?); PHY_STATUS {:#04x}", self.r8(rt::PHY_STATUS));
        }
    }

    /// The next good received frame, if any (bad ones are dropped and counted).
    pub fn recv(&mut self) -> Option<Vec<u8>> {
        loop {
            let i = self.rx_next;
            let d = self.rx_ring + i as u64 * rt::DESC_BYTES;
            fence(Ordering::SeqCst);
            // SAFETY: descriptor `i` of our receive ring.
            let opts1 = unsafe { (d as *const u32).read_volatile() };
            let frame = match rt::rx_status(opts1) {
                RxStatus::Pending => return None,
                RxStatus::Frame(len) if len <= BUF => {
                    // SAFETY: the NIC wrote `len` bytes into buffer `i` (ours, BUF bytes long).
                    Some(unsafe { core::slice::from_raw_parts((direct() + self.rx_bufs[i]) as *const u8, len) }.to_vec())
                }
                _ => {
                    self.rx_errors += 1;
                    None
                }
            };
            // Hand the buffer back.
            fence(Ordering::SeqCst);
            // SAFETY: as above.
            unsafe { (d as *mut u32).write_volatile(rt::rx_desc(BUF as u32, i == RING - 1)) };
            self.rx_next = (i + 1) % RING;
            if frame.is_some() {
                return frame;
            }
        }
    }

    /// Queues one frame. False if the ring is full or the frame too big (it is dropped).
    pub fn send(&mut self, frame: &[u8]) -> bool {
        if frame.len() > BUF || frame.is_empty() {
            return false;
        }
        // Reclaim what the NIC has finished sending.
        while self.tx_clean != self.tx_next {
            // SAFETY: descriptor of our transmit ring.
            let opts1 = unsafe { ((self.tx_ring + self.tx_clean as u64 * rt::DESC_BYTES) as *const u32).read_volatile() };
            if opts1 & rt::OWN != 0 {
                break;
            }
            self.tx_clean = (self.tx_clean + 1) % RING;
        }
        if (self.tx_next + 1) % RING == self.tx_clean {
            return false;
        }
        let i = self.tx_next;
        // SAFETY: buffer `i` is ours and the NIC is done with it (checked above).
        unsafe { core::ptr::copy_nonoverlapping(frame.as_ptr(), (direct() + self.tx_bufs[i]) as *mut u8, frame.len()) };
        let d = self.tx_ring + i as u64 * rt::DESC_BYTES;
        fence(Ordering::SeqCst);
        // SAFETY: descriptor `i` of our transmit ring; the ownership bit is in this write.
        unsafe { (d as *mut u32).write_volatile(rt::tx_desc(frame.len(), i == RING - 1)) };
        fence(Ordering::SeqCst);
        self.tx_next = (i + 1) % RING;
        self.w8(rt::TX_POLL, rt::TX_POLL_NPQ);
        true
    }
}
