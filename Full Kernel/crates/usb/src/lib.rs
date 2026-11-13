//! USB without the hardware.
//!
//! * [`trb`]: xHCI Transfer Request Blocks (commands, transfers, events) and
//!   the producer/consumer bookkeeping of the rings they live on.
//! * [`context`]: xHCI slot and endpoint contexts.
//! * [`desc`]: standard USB descriptors (device, configuration, interface,
//!   endpoint) and finding HID boot keyboards and mice in a configuration.
//! * [`hid`]: HID boot-protocol reports: keyboard reports become PS/2 scancode
//!   set 1 bytes (so the kernel's existing keyboard pipeline handles layout and
//!   modifiers), with key repeat; mouse reports become [`nova_ps2::MouseEvent`]s.
//!
//! The xHCI driver (`kernel/src/xhci.rs`) owns registers and memory; everything
//! here is unit-tested on the host.
//!
//! Reference: eXtensible Host Controller Interface for USB, revision 1.2;
//! USB 2.0 chapter 9; Device Class Definition for HID 1.11 appendix B;
//! HID Usage Tables (keyboard page 0x07).

#![no_std]

#[cfg(test)]
extern crate std;

pub mod context;
pub mod desc;
pub mod hid;
pub mod trb;

/// Port speeds as the xHCI reports them (PORTSC bits 10..13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Full,
    Low,
    High,
    Super,
    SuperPlus,
}

impl Speed {
    pub fn from_portsc(id: u32) -> Option<Self> {
        Some(match id {
            1 => Self::Full,
            2 => Self::Low,
            3 => Self::High,
            4 => Self::Super,
            5 => Self::SuperPlus,
            _ => return None,
        })
    }

    /// The value for the slot context's speed field.
    pub fn id(self) -> u32 {
        match self {
            Self::Full => 1,
            Self::Low => 2,
            Self::High => 3,
            Self::Super => 4,
            Self::SuperPlus => 5,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "full speed (12 Mb/s)",
            Self::Low => "low speed (1.5 Mb/s)",
            Self::High => "high speed (480 Mb/s)",
            Self::Super => "SuperSpeed (5 Gb/s)",
            Self::SuperPlus => "SuperSpeed+ (10 Gb/s)",
        }
    }

    /// Max packet size of endpoint 0 to start with, before the device
    /// descriptor tells us the real one.
    pub fn default_control_packet(self) -> u16 {
        match self {
            Self::Low | Self::Full => 8,
            Self::High => 64,
            Self::Super | Self::SuperPlus => 512,
        }
    }

    pub fn is_super(self) -> bool {
        matches!(self, Self::Super | Self::SuperPlus)
    }
}

/// The endpoint context interval for a periodic endpoint: the polling period
/// is 2^interval * 125 microseconds.
///
/// Full/low-speed devices give `b_interval` in milliseconds (1..255); high
/// and super speed give an exponent (1..16, period 2^(b_interval-1) microframes).
pub fn xhci_interval(speed: Speed, b_interval: u8) -> u8 {
    match speed {
        Speed::Low | Speed::Full => {
            let frames = (b_interval.max(1) as u32) * 8; // in 125 us units
            (31 - frames.leading_zeros()).clamp(3, 10) as u8
        }
        _ => b_interval.clamp(1, 16) - 1,
    }
}

/// Device Context Index of an endpoint: 1 for the control endpoint, then
/// 2 * number + (1 for IN).
pub fn dci(endpoint_address: u8) -> u8 {
    let n = endpoint_address & 0x0F;
    if n == 0 { 1 } else { n * 2 + (endpoint_address >> 7) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals() {
        // A full-speed keyboard asking for 10 ms gets 8 ms (2^6 * 125 us).
        assert_eq!(xhci_interval(Speed::Full, 10), 6);
        assert_eq!(xhci_interval(Speed::Low, 1), 3);
        assert_eq!(xhci_interval(Speed::Full, 255), 10);
        assert_eq!(xhci_interval(Speed::Full, 0), 3);
        // High speed: bInterval 4 => 2^3 microframes = 1 ms.
        assert_eq!(xhci_interval(Speed::High, 4), 3);
        assert_eq!(xhci_interval(Speed::High, 7), 6);
        assert_eq!(xhci_interval(Speed::Super, 0), 0);
    }

    #[test]
    fn context_indices() {
        assert_eq!(dci(0x00), 1);
        assert_eq!(dci(0x81), 3);
        assert_eq!(dci(0x01), 2);
        assert_eq!(dci(0x82), 5);
        assert_eq!(dci(0x8F), 31);
    }

    #[test]
    fn speeds() {
        assert_eq!(Speed::from_portsc(3), Some(Speed::High));
        assert_eq!(Speed::from_portsc(0), None);
        assert_eq!(Speed::Full.default_control_packet(), 8);
        assert_eq!(Speed::Super.default_control_packet(), 512);
        assert_eq!(Speed::Low.id(), 2);
    }
}
