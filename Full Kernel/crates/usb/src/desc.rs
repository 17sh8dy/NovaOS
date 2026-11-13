//! Standard USB descriptors (USB 2.0 chapter 9) and the requests that fetch them.

pub const GET_DESCRIPTOR: u8 = 6;
pub const SET_CONFIGURATION: u8 = 9;
/// HID class requests (HID 1.11 section 7.2).
pub const HID_SET_IDLE: u8 = 0x0A;
pub const HID_SET_PROTOCOL: u8 = 0x0B;

pub const DEVICE: u8 = 1;
pub const CONFIGURATION: u8 = 2;
pub const STRING: u8 = 3;
pub const INTERFACE: u8 = 4;
pub const ENDPOINT: u8 = 5;

pub const CLASS_HID: u8 = 3;
pub const CLASS_HUB: u8 = 9;

/// The fields of a device descriptor we use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    pub usb_version: u16,
    pub class: u8,
    /// Endpoint 0 max packet size in bytes (already decoded for SuperSpeed's exponent form).
    pub max_packet0: u16,
    pub vendor: u16,
    pub product: u16,
    pub manufacturer_string: u8,
    pub product_string: u8,
    pub configurations: u8,
}

impl Device {
    /// Parses at least the first 8 bytes (enough for `max_packet0`); the rest
    /// of the fields are zero if fewer than 18 bytes are given.
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 8 || b[1] != DEVICE || b[0] < 8 {
            return None;
        }
        let usb_version = u16::from_le_bytes([b[2], b[3]]);
        let mp = b[7];
        let max_packet0 = if usb_version >= 0x0300 { 1u16 << mp.min(15) } else { mp as u16 };
        let full = b.len() >= 18;
        let w = |i: usize| if full { u16::from_le_bytes([b[i], b[i + 1]]) } else { 0 };
        Some(Self {
            usb_version,
            class: b[4],
            max_packet0,
            vendor: w(8),
            product: w(10),
            manufacturer_string: if full { b[14] } else { 0 },
            product_string: if full { b[15] } else { 0 },
            configurations: if full { b[17] } else { 0 },
        })
    }
}

/// What kind of HID boot device an interface is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootKind {
    Keyboard,
    Mouse,
}

/// A HID boot-protocol interface with its interrupt IN endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootInterface {
    pub kind: BootKind,
    pub interface: u8,
    pub endpoint: u8,
    pub max_packet: u16,
    pub interval: u8,
}

/// The parts of a configuration we use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub value: u8,
    pub total_length: u16,
    pub boot: [Option<BootInterface>; 4],
    /// The device (or one of its interfaces) is a hub.
    pub hub: bool,
}

/// The total length of a configuration from its first 9 bytes.
pub fn config_total_length(b: &[u8]) -> Option<u16> {
    (b.len() >= 9 && b[1] == CONFIGURATION).then(|| u16::from_le_bytes([b[2], b[3]]))
}

/// Walks a whole configuration descriptor (with its interfaces and endpoints).
pub fn parse_config(b: &[u8]) -> Option<Config> {
    let total = config_total_length(b)?;
    let b = &b[..(total as usize).min(b.len())];
    let mut cfg = Config { value: b[5], total_length: total, boot: [None; 4], hub: false };
    let mut found = 0;
    // The interface we are inside: (number, alternate, class, subclass, protocol).
    let mut current: Option<(u8, u8, u8, u8, u8)> = None;
    let mut taken_for_current = false;
    let mut i = 0;
    while i + 2 <= b.len() {
        let len = b[i] as usize;
        if len < 2 || i + len > b.len() {
            break;
        }
        let d = &b[i..i + len];
        match d[1] {
            INTERFACE if len >= 9 => {
                current = Some((d[2], d[3], d[5], d[6], d[7]));
                taken_for_current = false;
                if d[5] == CLASS_HUB {
                    cfg.hub = true;
                }
            }
            ENDPOINT if len >= 7 => {
                if let Some((number, alt, class, subclass, protocol)) = current {
                    let is_interrupt_in = d[2] & 0x80 != 0 && d[3] & 3 == 3;
                    let kind = match (class, subclass, protocol) {
                        (CLASS_HID, 1, 1) => Some(BootKind::Keyboard),
                        (CLASS_HID, 1, 2) => Some(BootKind::Mouse),
                        _ => None,
                    };
                    if let (Some(kind), true, 0, false) = (kind, is_interrupt_in, alt, taken_for_current) {
                        if found < cfg.boot.len() {
                            cfg.boot[found] = Some(BootInterface {
                                kind,
                                interface: number,
                                endpoint: d[2],
                                max_packet: u16::from_le_bytes([d[4], d[5]]) & 0x7FF,
                                interval: d[6],
                            });
                            found += 1;
                        }
                        taken_for_current = true;
                    }
                }
            }
            _ => {}
        }
        i += len;
    }
    Some(cfg)
}

/// Decodes a string descriptor (UTF-16LE) into `out`, replacing anything
/// outside printable ASCII with '?'. Returns the number of bytes written.
pub fn string(b: &[u8], out: &mut [u8]) -> usize {
    if b.len() < 2 || b[1] != STRING {
        return 0;
    }
    let len = (b[0] as usize).min(b.len());
    let mut n = 0;
    for c in b[2..len].chunks_exact(2) {
        if n == out.len() {
            break;
        }
        let u = u16::from_le_bytes([c[0], c[1]]);
        out[n] = if (0x20..0x7F).contains(&u) { u as u8 } else { b'?' };
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    /// QEMU's usb-kbd high-speed descriptors, abridged.
    fn keyboard_config() -> Vec<u8> {
        let mut c = vec![9, 2, 0, 0, 1, 1, 0, 0xA0, 50];
        c.extend([9, 4, 0, 0, 1, 3, 1, 1, 0]); // interface 0: HID boot keyboard
        c.extend([9, 0x21, 0x11, 1, 0, 1, 0x22, 63, 0]); // HID descriptor
        c.extend([7, 5, 0x81, 3, 8, 0, 7]); // EP 1 IN interrupt, 8 bytes, bInterval 7
        let n = c.len() as u16;
        c[2..4].copy_from_slice(&n.to_le_bytes());
        c
    }

    #[test]
    fn device_descriptor() {
        let d = [18, 1, 0x00, 0x02, 0, 0, 0, 64, 0x27, 0x06, 0x01, 0x00, 0, 0, 1, 2, 3, 1];
        let dev = Device::parse(&d).unwrap();
        assert_eq!((dev.usb_version, dev.max_packet0, dev.vendor, dev.product, dev.product_string, dev.configurations), (0x200, 64, 0x0627, 1, 2, 1));
        // First 8 bytes only.
        assert_eq!(Device::parse(&d[..8]).unwrap().max_packet0, 64);
        // SuperSpeed encodes 512 as 2^9.
        let mut ss = d;
        ss[3] = 3;
        ss[7] = 9;
        assert_eq!(Device::parse(&ss).unwrap().max_packet0, 512);
        assert!(Device::parse(&d[..7]).is_none());
        assert!(Device::parse(&[18, 2, 0, 0, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn finds_the_boot_keyboard() {
        let c = keyboard_config();
        assert_eq!(config_total_length(&c), Some(c.len() as u16));
        let cfg = parse_config(&c).unwrap();
        assert_eq!(cfg.value, 1);
        assert!(!cfg.hub);
        assert_eq!(cfg.boot[0], Some(BootInterface { kind: BootKind::Keyboard, interface: 0, endpoint: 0x81, max_packet: 8, interval: 7 }));
        assert_eq!(cfg.boot[1], None);
    }

    #[test]
    fn composite_keyboard_and_mouse() {
        let mut c = keyboard_config();
        c.extend([9, 4, 1, 0, 1, 3, 1, 2, 0]); // interface 1: boot mouse
        c.extend([7, 5, 0x02, 3, 8, 0, 10]); // an OUT endpoint first: ignored
        c.extend([7, 5, 0x82, 3, 4, 0, 10]);
        c.extend([9, 4, 2, 0, 1, 3, 0, 0, 0]); // interface 2: HID but not boot
        c.extend([7, 5, 0x83, 3, 8, 0, 10]);
        let n = c.len() as u16;
        c[2..4].copy_from_slice(&n.to_le_bytes());
        let cfg = parse_config(&c).unwrap();
        assert_eq!(cfg.boot[1].map(|b| (b.kind, b.interface, b.endpoint, b.max_packet)), Some((BootKind::Mouse, 1, 0x82, 4)));
        assert_eq!(cfg.boot[2], None);
    }

    #[test]
    fn hub_and_truncated() {
        let mut c = vec![9, 2, 25, 0, 1, 1, 0, 0xE0, 0];
        c.extend([9, 4, 0, 0, 1, 9, 0, 0, 0]);
        c.extend([7, 5, 0x81, 3, 1, 0, 12]);
        assert!(parse_config(&c).unwrap().hub);
        // A descriptor claiming to run past the end stops the walk instead of panicking.
        let mut bad = keyboard_config();
        let len = bad.len();
        bad[len - 7] = 40;
        assert_eq!(parse_config(&bad).unwrap().boot[0], None);
        assert!(parse_config(&[9, 2]).is_none());
    }

    #[test]
    fn string_descriptor() {
        let s = [10, 3, b'Q', 0, b'E', 0, b'M', 0, 0xE9, 0];
        let mut out = [0u8; 16];
        let n = string(&s, &mut out);
        assert_eq!(&out[..n], b"QEM?");
        assert_eq!(string(&[2, 1], &mut out), 0);
    }
}
