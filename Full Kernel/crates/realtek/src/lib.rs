//! The Realtek RTL8168 / RTL8111 gigabit Ethernet family (the RTL8111H on the
//! ASUS PRIME B550-PLUS among many others) without the hardware: register
//! offsets, the 16-byte descriptor format, chip identification from the
//! TxConfig register, link status, and the commands for reaching the PHY.
//!
//! The kernel driver (`kernel/src/rtl8168.rs`) owns the registers and memory.
//! QEMU cannot emulate this chip, so everything that can be checked without
//! one is checked here.
//!
//! Sources: the RTL8169/8110 datasheet (register map, descriptor layout, which
//! the 8168 keeps in "C+ mode") and the behaviour of Linux's `r8169` driver
//! (chip ids, receive configuration, the OCP way of reaching the PHY on
//! RTL8168G and newer).

#![no_std]

// ---- Registers (offsets in the memory-mapped register block) --------------------------------

/// MAC address, 6 bytes (read only for us: we never change it).
pub const IDR0: u64 = 0x00;
/// Multicast filter, 8 bytes.
pub const MAR0: u64 = 0x08;
/// Transmit descriptor ring address (low, high).
pub const TX_DESC_LO: u64 = 0x20;
pub const TX_DESC_HI: u64 = 0x24;
/// Command: reset, receiver enable, transmitter enable (8 bits).
pub const CHIP_CMD: u64 = 0x37;
/// Transmit poll: "there are new descriptors" (8 bits).
pub const TX_POLL: u64 = 0x38;
pub const INTR_MASK: u64 = 0x3C;
pub const INTR_STATUS: u64 = 0x3E;
pub const TX_CONFIG: u64 = 0x40;
pub const RX_CONFIG: u64 = 0x44;
/// Config register write lock (8 bits).
pub const CFG9346: u64 = 0x50;
/// MII access to the PHY (older chips).
pub const PHYAR: u64 = 0x60;
/// Link status as the MAC sees it (8 bits).
pub const PHY_STATUS: u64 = 0x6C;
/// OCP access to the PHY (RTL8168G and newer).
pub const GPHY_OCP: u64 = 0xB8;
/// Largest frame the receiver accepts (16 bits).
pub const RX_MAX_SIZE: u64 = 0xDA;
/// C+ command (16 bits): VLAN untagging, receive checksum offload, ...
pub const CPLUS_CMD: u64 = 0xE0;
pub const RX_DESC_LO: u64 = 0xE4;
pub const RX_DESC_HI: u64 = 0xE8;
/// Largest frame we may transmit, in 128-byte units (8 bits).
pub const MAX_TX_PACKET: u64 = 0xEC;

pub const CMD_RESET: u8 = 1 << 4;
pub const CMD_RX_ENABLE: u8 = 1 << 3;
pub const CMD_TX_ENABLE: u8 = 1 << 2;
/// TX_POLL: normal-priority queue has work.
pub const TX_POLL_NPQ: u8 = 1 << 6;
pub const CFG9346_UNLOCK: u8 = 0xC0;
pub const CFG9346_LOCK: u8 = 0x00;
/// C+ command bits we turn off: we want frames exactly as they arrived.
pub const CPLUS_RX_VLAN: u16 = 1 << 6;
pub const CPLUS_RX_CHECKSUM: u16 = 1 << 5;

// ---- Receive and transmit configuration ---------------------------------------------------------

pub const RX_ACCEPT_BROADCAST: u32 = 1 << 3;
pub const RX_ACCEPT_MULTICAST: u32 = 1 << 2;
pub const RX_ACCEPT_MY_PHYS: u32 = 1 << 1;
/// No limit on the receive DMA burst.
pub const RX_DMA_BURST: u32 = 7 << 8;
pub const RX_EARLY_OFF: u32 = 1 << 11;
pub const RX_MULTI_EN: u32 = 1 << 14;
pub const RX128_INT_EN: u32 = 1 << 15;
/// No limit on the transmit DMA burst.
pub const TX_DMA_BURST: u32 = 7 << 8;
/// Standard inter-frame gap.
pub const TX_IFG: u32 = 3 << 24;

/// The receive configuration: our address, broadcasts (DHCP, ARP) and multicast; and, on the
/// newer chips Linux treats the same way, the fetch settings they need.
pub fn rx_config(xid: u16) -> u32 {
    let accept = RX_ACCEPT_BROADCAST | RX_ACCEPT_MULTICAST | RX_ACCEPT_MY_PHYS;
    let fetch = if is_8168g_or_newer(xid) { RX128_INT_EN | RX_MULTI_EN | RX_EARLY_OFF } else { 0 };
    accept | RX_DMA_BURST | fetch
}

pub fn tx_config() -> u32 {
    TX_IFG | TX_DMA_BURST
}

// ---- Descriptors ---------------------------------------------------------------------------------

/// Owned by the NIC (it clears this when done).
pub const OWN: u32 = 1 << 31;
/// End of ring: the NIC goes back to descriptor 0 after this one.
pub const EOR: u32 = 1 << 30;
/// First and last segment of a frame.
pub const FS: u32 = 1 << 29;
pub const LS: u32 = 1 << 28;
/// Receive error summary (CRC, runt, too long, FIFO overflow...).
pub const RX_RES: u32 = 1 << 21;
pub const LEN_MASK: u32 = 0x3FFF;
/// Rings must start on a 256-byte boundary.
pub const RING_ALIGN: u64 = 256;
pub const DESC_BYTES: u64 = 16;

/// A receive descriptor's first word: give a `buf_len`-byte buffer to the NIC.
pub fn rx_desc(buf_len: u32, last_in_ring: bool) -> u32 {
    OWN | if last_in_ring { EOR } else { 0 } | (buf_len & LEN_MASK & !7)
}

/// A transmit descriptor's first word: one whole frame of `len` bytes.
pub fn tx_desc(len: usize, last_in_ring: bool) -> u32 {
    OWN | FS | LS | if last_in_ring { EOR } else { 0 } | (len as u32 & LEN_MASK)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RxStatus {
    /// Still the NIC's.
    Pending,
    /// A good frame of this many bytes (the 4-byte CRC already taken off).
    Frame(usize),
    /// A bad frame (or one split over several buffers, which we never set up for): drop it.
    Error,
}

/// What a receive descriptor's first word says.
pub fn rx_status(opts1: u32) -> RxStatus {
    if opts1 & OWN != 0 {
        return RxStatus::Pending;
    }
    if opts1 & (FS | LS) != (FS | LS) || opts1 & RX_RES != 0 {
        return RxStatus::Error;
    }
    match (opts1 & LEN_MASK) as usize {
        len if len >= 4 + 14 => RxStatus::Frame(len - 4),
        _ => RxStatus::Error,
    }
}

// ---- Identification --------------------------------------------------------------------------

/// PCI device ids of the RTL8168/8111 family (vendor 0x10EC) this driver takes.
pub fn is_supported(vendor: u16, device: u16) -> bool {
    vendor == 0x10EC && matches!(device, 0x8168 | 0x8161)
}

/// The chip's version id: bits 20..31 of TxConfig, as Linux's `r8169` reads it.
pub fn xid(tx_config: u32) -> u16 {
    ((tx_config >> 20) & 0xFCF) as u16
}

/// RTL8168G and every later 8168 reach the PHY through OCP registers, not PHYAR.
pub fn is_8168g_or_newer(xid: u16) -> bool {
    let x = xid & 0x7CF;
    (0x4C0..0x600).contains(&x)
}

/// A name for the log.
pub fn chip_name(xid: u16) -> &'static str {
    match xid & 0x7CF {
        0x540 | 0x541 => "RTL8168H/8111H",
        0x54A | 0x54B => "RTL8168FP/8117",
        0x5C8 => "RTL8411B",
        0x4C0 | 0x4C1 => "RTL8168G/8111G",
        0x509 => "RTL8168GU/8111GU",
        0x488 | 0x481 | 0x480 => "RTL8168F/8111F",
        0x2C8 | 0x2C1 | 0x2C2 => "RTL8168E/8111E",
        0x288 | 0x281 | 0x280 => "RTL8168D/8111D",
        x if (0x600..0x700).contains(&x) => "RTL8125 (2.5G, needs another driver)",
        _ => "RTL8168 family (unknown version)",
    }
}

// ---- Link ------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub up: bool,
    pub mbps: u16,
    pub full_duplex: bool,
}

/// The PHY_STATUS register.
pub fn link(phy_status: u8) -> Link {
    let mbps = if phy_status & 0x10 != 0 {
        1000
    } else if phy_status & 0x08 != 0 {
        100
    } else if phy_status & 0x04 != 0 {
        10
    } else {
        0
    };
    Link { up: phy_status & 0x02 != 0, mbps, full_duplex: phy_status & 0x01 != 0 }
}

// ---- Reaching the PHY ------------------------------------------------------------------------

/// Standard MII registers.
pub const MII_BMCR: u8 = 0;
pub const MII_BMSR: u8 = 1;
/// BMCR: power down, auto-negotiation enable, restart auto-negotiation.
pub const BMCR_POWER_DOWN: u16 = 1 << 11;
pub const BMCR_ANENABLE: u16 = 1 << 12;
pub const BMCR_ANRESTART: u16 = 1 << 9;
/// BMSR: link up (latched low), auto-negotiation complete.
pub const BMSR_LINK: u16 = 1 << 2;
pub const BMSR_AN_DONE: u16 = 1 << 5;

/// The busy/ready flag of both PHYAR and GPHY_OCP.
pub const PHY_FLAG: u32 = 1 << 31;

/// Where standard PHY register `reg` lives in OCP space (RTL8168G and newer).
fn ocp_address(reg: u8) -> u32 {
    0xA400 + (reg as u32 & 0x1F) * 2
}

/// The word to write to PHYAR / GPHY_OCP to read PHY register `reg`. Then wait for
/// [`PHY_FLAG`] to be **set** and take the low 16 bits.
pub fn phy_read_cmd(new_chip: bool, reg: u8) -> u32 {
    if new_chip { ocp_address(reg) << 15 } else { (reg as u32 & 0x1F) << 16 }
}

/// The word to write to PHYAR / GPHY_OCP to write `value` to PHY register `reg`. Then wait for
/// [`PHY_FLAG`] to **clear**.
pub fn phy_write_cmd(new_chip: bool, reg: u8, value: u16) -> u32 {
    PHY_FLAG | value as u32 | if new_chip { ocp_address(reg) << 15 } else { (reg as u32 & 0x1F) << 16 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors() {
        assert_eq!(rx_desc(2048, false), 0x8000_0800);
        assert_eq!(rx_desc(2048, true), 0xC000_0800, "the last one wraps the ring");
        assert_eq!(rx_desc(2051, false) & LEN_MASK, 2048, "buffer sizes are multiples of 8");
        assert_eq!(tx_desc(60, false), 0xB000_003C);
        assert_eq!(tx_desc(1514, true), 0xF000_05EA);
    }

    #[test]
    fn receive_status() {
        assert_eq!(rx_status(rx_desc(2048, false)), RxStatus::Pending);
        // The NIC filled it: FS | LS, 64 bytes including the CRC.
        assert_eq!(rx_status(FS | LS | 64), RxStatus::Frame(60));
        assert_eq!(rx_status(EOR | FS | LS | 1518), RxStatus::Frame(1514), "the ring flag does not matter");
        assert_eq!(rx_status(FS | LS | RX_RES | 64), RxStatus::Error);
        assert_eq!(rx_status(FS | 2048), RxStatus::Error, "a frame split over buffers");
        assert_eq!(rx_status(FS | LS | 10), RxStatus::Error, "shorter than a header and a CRC");
    }

    #[test]
    fn identification() {
        assert!(is_supported(0x10EC, 0x8168));
        assert!(is_supported(0x10EC, 0x8161));
        assert!(!is_supported(0x10EC, 0x8125), "the 2.5G RTL8125 is different");
        assert!(!is_supported(0x8086, 0x8168));
        // TxConfig of an RTL8111H: id 0x541 in bits 20..31.
        let x = xid(0x5410_0000 | TX_IFG);
        assert_eq!(x & 0x7CF, 0x541);
        assert_eq!(chip_name(x), "RTL8168H/8111H");
        assert!(is_8168g_or_newer(x));
        assert!(!is_8168g_or_newer(0x2C1), "RTL8168E uses PHYAR");
        assert!(!is_8168g_or_newer(0x641), "RTL8125 is not ours");
        assert_eq!(chip_name(0x123), "RTL8168 family (unknown version)");
    }

    #[test]
    fn receive_configuration() {
        let old = rx_config(0x2C1);
        assert_eq!(old & 0xF, RX_ACCEPT_BROADCAST | RX_ACCEPT_MULTICAST | RX_ACCEPT_MY_PHYS);
        assert_eq!(old & RX_MULTI_EN, 0);
        assert_eq!(old & 1, 0, "never promiscuous");
        let new = rx_config(0x541);
        assert_ne!(new & RX_MULTI_EN, 0);
        assert_ne!(new & RX128_INT_EN, 0);
        assert_eq!(tx_config(), 0x0300_0700);
    }

    #[test]
    fn link_status() {
        assert_eq!(link(0x13), Link { up: true, mbps: 1000, full_duplex: true });
        assert_eq!(link(0x0B), Link { up: true, mbps: 100, full_duplex: true });
        assert_eq!(link(0x00), Link { up: false, mbps: 0, full_duplex: false });
    }

    #[test]
    fn phy_commands() {
        // OCP (RTL8168G+): BMCR is at 0xA400, BMSR at 0xA402, shifted to bit 15.
        assert_eq!(phy_read_cmd(true, MII_BMCR), 0xA400 << 15);
        assert_eq!(phy_read_cmd(true, MII_BMSR), 0xA402 << 15);
        assert_eq!(phy_write_cmd(true, MII_BMCR, 0x1200), PHY_FLAG | 0xA400 << 15 | 0x1200);
        // PHYAR (older): register in bits 16..20.
        assert_eq!(phy_read_cmd(false, MII_BMSR), 1 << 16);
        assert_eq!(phy_write_cmd(false, MII_BMCR, 0x1200), 0x8000_1200);
    }
}
