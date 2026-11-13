//! xHCI device contexts (xHCI 1.2, section 6.2).
//!
//! A device context is a slot context followed by 31 endpoint contexts; an
//! *input* context puts an input-control context (which contexts to add or
//! drop) in front. Each context is 32 bytes, or 64 when the controller says
//! so (HCCPARAMS1.CSZ); only the first 32 bytes carry fields.

use crate::Speed;

pub const EP_CONTROL: u32 = 4;
pub const EP_INTERRUPT_IN: u32 = 7;

/// Byte offsets inside an input context.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// 32 or 64.
    pub context_size: usize,
}

impl Layout {
    pub fn new(big: bool) -> Self {
        Self { context_size: if big { 64 } else { 32 } }
    }

    /// Input control context (add/drop flags).
    pub fn control(&self) -> usize {
        0
    }

    /// Slot context inside an *input* context.
    pub fn input_slot(&self) -> usize {
        self.context_size
    }

    /// Endpoint context `dci` (1..=31) inside an *input* context.
    pub fn input_endpoint(&self, dci: u8) -> usize {
        self.context_size * (1 + dci as usize)
    }

    /// Endpoint context `dci` inside an *output* device context.
    pub fn output_endpoint(&self, dci: u8) -> usize {
        self.context_size * dci as usize
    }

    /// Bytes of a whole input context.
    pub fn input_size(&self) -> usize {
        self.context_size * 33
    }
}

/// Input control context: drop flags (dword 0), add flags (dword 1).
pub fn input_control(add: u32) -> [u32; 8] {
    let mut c = [0u32; 8];
    c[1] = add;
    c
}

/// A slot context for a device attached to a root port (route string 0).
pub fn slot(speed: Speed, root_port: u8, context_entries: u8) -> [u32; 8] {
    let mut s = [0u32; 8];
    s[0] = speed.id() << 20 | (context_entries as u32) << 27;
    s[1] = (root_port as u32) << 16;
    s
}

/// Endpoint 0 (control) context.
pub fn control_endpoint(max_packet: u16, ring: u64) -> [u32; 8] {
    let mut e = [0u32; 8];
    e[1] = 3 << 1 | EP_CONTROL << 3 | (max_packet as u32) << 16; // CErr = 3
    e[2] = ring as u32 | 1; // dequeue cycle state = 1
    e[3] = (ring >> 32) as u32;
    e[4] = 8; // average TRB length
    e
}

/// An interrupt IN endpoint context.
pub fn interrupt_in_endpoint(max_packet: u16, interval: u8, ring: u64) -> [u32; 8] {
    let mut e = [0u32; 8];
    e[0] = (interval as u32) << 16;
    e[1] = 3 << 1 | EP_INTERRUPT_IN << 3 | (max_packet as u32 & 0x7FF) << 16;
    e[2] = ring as u32 | 1;
    e[3] = (ring >> 32) as u32;
    e[4] = (max_packet as u32) | (max_packet as u32) << 16; // average TRB length, max ESIT payload
    e
}

/// Endpoint state (dword 0 bits 0..2) of an output endpoint context.
pub fn endpoint_state(dword0: u32) -> u32 {
    dword0 & 7
}

/// Changes the max packet size field of a control endpoint context.
pub fn set_max_packet(e: &mut [u32; 8], max_packet: u16) {
    e[1] = (e[1] & 0xFFFF) | (max_packet as u32) << 16;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let l = Layout::new(false);
        assert_eq!((l.input_slot(), l.input_endpoint(1), l.input_endpoint(3), l.output_endpoint(3)), (32, 64, 128, 96));
        assert_eq!(l.input_size(), 1056);
        let l = Layout::new(true);
        assert_eq!((l.input_slot(), l.input_endpoint(1), l.input_size()), (64, 128, 2112));
    }

    #[test]
    fn slot_context() {
        let s = slot(Speed::High, 5, 1);
        assert_eq!(s[0] >> 20 & 0xF, 3);
        assert_eq!(s[0] >> 27, 1);
        assert_eq!(s[1] >> 16 & 0xFF, 5);
    }

    #[test]
    fn endpoint_contexts() {
        let e = control_endpoint(64, 0x1_0000_3000);
        assert_eq!(e[1] >> 3 & 7, EP_CONTROL);
        assert_eq!(e[1] >> 1 & 3, 3);
        assert_eq!(e[1] >> 16, 64);
        assert_eq!((e[2], e[3]), (0x3001, 1));
        let mut e2 = e;
        set_max_packet(&mut e2, 8);
        assert_eq!((e2[1] >> 16, e2[1] & 0xFFFF), (8, e[1] & 0xFFFF));
        let i = interrupt_in_endpoint(8, 6, 0x4000);
        assert_eq!(i[0] >> 16 & 0xFF, 6);
        assert_eq!(i[1] >> 3 & 7, EP_INTERRUPT_IN);
        assert_eq!(i[4], 8 | 8 << 16);
        assert_eq!(input_control(0b1001)[1], 9);
        assert_eq!(endpoint_state(0x1_0003), 3);
    }
}
