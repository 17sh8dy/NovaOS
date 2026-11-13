//! The PC's battery-backed real-time clock (CMOS RTC): decoding what its
//! registers hold. Reading the ports is done by the kernel.

/// RTC register numbers (written to port 0x70, read from 0x71).
pub const SECONDS: u8 = 0x00;
pub const MINUTES: u8 = 0x02;
pub const HOURS: u8 = 0x04;
pub const DAY: u8 = 0x07;
pub const MONTH: u8 = 0x08;
pub const YEAR: u8 = 0x09;
pub const STATUS_A: u8 = 0x0A;
pub const STATUS_B: u8 = 0x0B;
/// Status A bit 7: an update is in progress (values may be half-written).
pub const UPDATING: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

fn bcd(v: u8) -> u8 {
    (v >> 4) * 10 + (v & 0x0F)
}

/// Decodes raw register values `[sec, min, hour, day, month, year]` using
/// status register B (bit 2: binary rather than BCD, bit 1: 24-hour clock).
/// Years are taken as 2000 + yy. Returns None for impossible values.
pub fn decode(raw: [u8; 6], status_b: u8) -> Option<DateTime> {
    let binary = status_b & 0x04 != 0;
    let h24 = status_b & 0x02 != 0;
    let conv = |v: u8| if binary { v } else { bcd(v) };
    let pm = raw[2] & 0x80 != 0;
    let mut hour = conv(raw[2] & 0x7F);
    if !h24 {
        hour %= 12;
        if pm {
            hour += 12;
        }
    }
    let t = DateTime { second: conv(raw[0]), minute: conv(raw[1]), hour, day: conv(raw[3]), month: conv(raw[4]), year: 2000 + conv(raw[5]) as u16 };
    let ok = t.second < 60 && t.minute < 60 && t.hour < 24 && (1..=31).contains(&t.day) && (1..=12).contains(&t.month);
    ok.then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcd_24_hour() {
        let t = decode([0x45, 0x30, 0x23, 0x12, 0x11, 0x26], 0x02).unwrap();
        assert_eq!(t, DateTime { year: 2026, month: 11, day: 12, hour: 23, minute: 30, second: 45 });
    }

    #[test]
    fn binary_12_hour() {
        // 12:05 AM is hour 0; 12:05 PM is hour 12; 1 PM is 13.
        assert_eq!(decode([0, 5, 12, 1, 1, 0], 0x04).unwrap().hour, 0);
        assert_eq!(decode([0, 5, 0x80 | 12, 1, 1, 0], 0x04).unwrap().hour, 12);
        assert_eq!(decode([0, 5, 0x80 | 1, 1, 1, 0], 0x04).unwrap().hour, 13);
        // BCD + 12-hour.
        assert_eq!(decode([0, 0, 0x80 | 0x11, 1, 1, 0], 0x00).unwrap().hour, 23);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode([0x99, 0, 0, 1, 1, 0], 0x02).is_none());
        assert!(decode([0, 0, 0, 0, 1, 0], 0x02).is_none());
        assert!(decode([0xFF; 6], 0x06).is_none(), "a floating bus reads 0xFF");
    }
}
