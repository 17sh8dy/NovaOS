//! Transfer Request Blocks and rings (xHCI 1.2, sections 4.9 and 6.4).
//!
//! Every TRB is 16 bytes: two dwords of parameter, a status dword and a
//! control dword whose bit 0 is the *cycle bit*. The producer of a ring
//! writes TRBs with its current cycle state; the consumer knows a TRB is new
//! when its cycle bit matches the state it expects. Command and transfer rings
//! end in a Link TRB that points back to the start and tells the controller to
//! flip its cycle state; the event ring is a fixed segment the controller
//! wraps by itself.

// TRB types (control dword bits 10..15).
pub const NORMAL: u32 = 1;
pub const SETUP: u32 = 2;
pub const DATA: u32 = 3;
pub const STATUS: u32 = 4;
pub const LINK: u32 = 6;
pub const ENABLE_SLOT: u32 = 9;
pub const DISABLE_SLOT: u32 = 10;
pub const ADDRESS_DEVICE: u32 = 11;
pub const CONFIGURE_ENDPOINT: u32 = 12;
pub const EVALUATE_CONTEXT: u32 = 13;
pub const RESET_ENDPOINT: u32 = 14;
pub const SET_TR_DEQUEUE: u32 = 16;
pub const NOOP_COMMAND: u32 = 23;
pub const TRANSFER_EVENT: u32 = 32;
pub const COMMAND_COMPLETION: u32 = 33;
pub const PORT_STATUS_CHANGE: u32 = 34;
pub const HOST_CONTROLLER_EVENT: u32 = 37;

// Completion codes.
pub const CC_SUCCESS: u8 = 1;
pub const CC_STALL: u8 = 6;
pub const CC_SHORT_PACKET: u8 = 13;

const CYCLE: u32 = 1 << 0;
const TOGGLE_CYCLE: u32 = 1 << 1; // Link TRB
const ISP: u32 = 1 << 2; // interrupt on short packet
const IOC: u32 = 1 << 5; // interrupt on completion
const IDT: u32 = 1 << 6; // immediate data (Setup TRB)

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trb(pub [u32; 4]);

impl Trb {
    fn new(kind: u32) -> Self {
        Self([0, 0, 0, kind << 10])
    }

    fn with_pointer(mut self, p: u64) -> Self {
        self.0[0] = p as u32;
        self.0[1] = (p >> 32) as u32;
        self
    }

    pub fn kind(&self) -> u32 {
        (self.0[3] >> 10) & 0x3F
    }

    pub fn cycle(&self) -> bool {
        self.0[3] & CYCLE != 0
    }

    pub fn with_cycle(mut self, cycle: bool) -> Self {
        self.0[3] = (self.0[3] & !CYCLE) | cycle as u32;
        self
    }

    pub fn pointer(&self) -> u64 {
        self.0[0] as u64 | (self.0[1] as u64) << 32
    }

    // ---- Commands ----

    pub fn enable_slot() -> Self {
        Self::new(ENABLE_SLOT)
    }

    pub fn disable_slot(slot: u8) -> Self {
        let mut t = Self::new(DISABLE_SLOT);
        t.0[3] |= (slot as u32) << 24;
        t
    }

    pub fn address_device(input_context: u64, slot: u8) -> Self {
        let mut t = Self::new(ADDRESS_DEVICE).with_pointer(input_context);
        t.0[3] |= (slot as u32) << 24;
        t
    }

    pub fn configure_endpoint(input_context: u64, slot: u8) -> Self {
        let mut t = Self::new(CONFIGURE_ENDPOINT).with_pointer(input_context);
        t.0[3] |= (slot as u32) << 24;
        t
    }

    pub fn evaluate_context(input_context: u64, slot: u8) -> Self {
        let mut t = Self::new(EVALUATE_CONTEXT).with_pointer(input_context);
        t.0[3] |= (slot as u32) << 24;
        t
    }

    pub fn reset_endpoint(slot: u8, dci: u8) -> Self {
        let mut t = Self::new(RESET_ENDPOINT);
        t.0[3] |= (slot as u32) << 24 | (dci as u32) << 16;
        t
    }

    /// Moves an endpoint's dequeue pointer (after a reset, to skip what was queued).
    pub fn set_tr_dequeue(slot: u8, dci: u8, pointer: u64, cycle: bool) -> Self {
        let mut t = Self::new(SET_TR_DEQUEUE).with_pointer(pointer | cycle as u64);
        t.0[3] |= (slot as u32) << 24 | (dci as u32) << 16;
        t
    }

    pub fn noop_command() -> Self {
        Self::new(NOOP_COMMAND)
    }

    // ---- Transfers ----

    /// The Setup stage of a control transfer, the 8-byte request carried in the TRB itself.
    /// `data_in`: None = no data stage, Some(true) = IN, Some(false) = OUT.
    pub fn setup(request_type: u8, request: u8, value: u16, index: u16, length: u16, data_in: Option<bool>) -> Self {
        let mut t = Self::new(SETUP);
        t.0[0] = request_type as u32 | (request as u32) << 8 | (value as u32) << 16;
        t.0[1] = index as u32 | (length as u32) << 16;
        t.0[2] = 8;
        let trt = match data_in {
            None => 0,
            Some(false) => 2,
            Some(true) => 3,
        };
        t.0[3] |= IDT | trt << 16;
        t
    }

    pub fn data(buffer: u64, length: u32, data_in: bool) -> Self {
        let mut t = Self::new(DATA).with_pointer(buffer);
        t.0[2] = length & 0x1_FFFF;
        t.0[3] |= (data_in as u32) << 16;
        t
    }

    /// The Status stage: its direction is the opposite of the data stage (IN when there is none).
    pub fn status(data_in: Option<bool>) -> Self {
        let mut t = Self::new(STATUS);
        let status_in = data_in != Some(true);
        t.0[3] |= IOC | (status_in as u32) << 16;
        t
    }

    /// A Normal TRB (bulk/interrupt data) that raises an event when done or short.
    pub fn normal(buffer: u64, length: u32) -> Self {
        let mut t = Self::new(NORMAL).with_pointer(buffer);
        t.0[2] = length & 0x1_FFFF;
        t.0[3] |= IOC | ISP;
        t
    }

    pub fn link(to: u64) -> Self {
        let mut t = Self::new(LINK).with_pointer(to);
        t.0[3] |= TOGGLE_CYCLE;
        t
    }

    // ---- Events ----

    pub fn completion_code(&self) -> u8 {
        (self.0[2] >> 24) as u8
    }

    /// Transfer event: bytes NOT transferred. Command completion: the parameter.
    pub fn residual(&self) -> u32 {
        self.0[2] & 0xFF_FFFF
    }

    pub fn slot(&self) -> u8 {
        (self.0[3] >> 24) as u8
    }

    /// Transfer event: the endpoint's device context index.
    pub fn endpoint(&self) -> u8 {
        ((self.0[3] >> 16) & 0x1F) as u8
    }

    /// Port status change event: the (1-based) port.
    pub fn port(&self) -> u8 {
        (self.0[0] >> 24) as u8
    }
}

/// A short description of a completion code.
pub fn describe(code: u8) -> &'static str {
    match code {
        1 => "success",
        3 => "data buffer error",
        4 => "babble",
        5 => "USB transaction error",
        6 => "stall",
        9 => "no free slots",
        11 => "slot not enabled",
        12 => "endpoint not enabled",
        13 => "short packet",
        17 => "parameter error",
        19 => "context state error",
        21 => "event ring full",
        24 => "command ring stopped",
        25 => "command aborted",
        _ => "error",
    }
}

/// A command or transfer ring as its producer (the driver) sees it. The last
/// TRB slot holds the Link back to the start.
#[derive(Debug)]
pub struct Producer {
    /// Physical address of TRB 0.
    pub base: u64,
    /// TRB slots including the Link.
    pub len: usize,
    pub enqueue: usize,
    pub cycle: bool,
}

/// What to write after a [`Producer::push`]: the TRB, then (when the ring wraps) the Link.
#[derive(Debug, PartialEq, Eq)]
pub struct Push {
    pub index: usize,
    pub trb: Trb,
    /// Physical address of the TRB written (what an event will point back to).
    pub address: u64,
    pub link: Option<(usize, Trb)>,
}

impl Producer {
    pub const fn new(base: u64, len: usize) -> Self {
        Self { base, len, enqueue: 0, cycle: true }
    }

    pub fn push(&mut self, trb: Trb) -> Push {
        let index = self.enqueue;
        let trb = trb.with_cycle(self.cycle);
        let address = self.base + 16 * index as u64;
        self.enqueue += 1;
        let mut link = None;
        if self.enqueue == self.len - 1 {
            link = Some((self.len - 1, Trb::link(self.base).with_cycle(self.cycle)));
            self.enqueue = 0;
            self.cycle = !self.cycle;
        }
        Push { index, trb, address, link }
    }
}

/// The event ring as its consumer (the driver) sees it: one segment.
#[derive(Debug)]
pub struct Consumer {
    pub base: u64,
    pub len: usize,
    pub dequeue: usize,
    pub cycle: bool,
}

impl Consumer {
    pub const fn new(base: u64, len: usize) -> Self {
        Self { base, len, dequeue: 0, cycle: true }
    }

    pub fn is_new(&self, t: &Trb) -> bool {
        t.cycle() == self.cycle
    }

    /// Moves past the current event; returns the new dequeue pointer for ERDP.
    pub fn advance(&mut self) -> u64 {
        self.dequeue += 1;
        if self.dequeue == self.len {
            self.dequeue = 0;
            self.cycle = !self.cycle;
        }
        self.base + 16 * self.dequeue as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_stage_layout() {
        // GET_DESCRIPTOR(device), 18 bytes IN.
        let t = Trb::setup(0x80, 6, 0x0100, 0, 18, Some(true));
        assert_eq!(t.kind(), SETUP);
        assert_eq!(t.0[0], 0x0100_0680);
        assert_eq!(t.0[1], 18 << 16);
        assert_eq!(t.0[2], 8);
        assert_eq!(t.0[3] >> 16 & 3, 3, "TRT = IN data stage");
        assert_ne!(t.0[3] & IDT, 0);
        assert_eq!(Trb::setup(0, 9, 1, 0, 0, None).0[3] >> 16 & 3, 0);
    }

    #[test]
    fn status_direction_is_opposite_of_data() {
        assert_eq!(Trb::status(Some(true)).0[3] >> 16 & 1, 0, "IN data -> OUT status");
        assert_eq!(Trb::status(Some(false)).0[3] >> 16 & 1, 1);
        assert_eq!(Trb::status(None).0[3] >> 16 & 1, 1, "no data -> IN status");
        assert_ne!(Trb::status(None).0[3] & IOC, 0);
    }

    #[test]
    fn commands() {
        let t = Trb::address_device(0x1000, 3);
        assert_eq!((t.kind(), t.pointer(), t.slot()), (ADDRESS_DEVICE, 0x1000, 3));
        let t = Trb::reset_endpoint(2, 3);
        assert_eq!((t.slot(), t.endpoint()), (2, 3));
        let t = Trb::set_tr_dequeue(2, 1, 0x8040, true);
        assert_eq!((t.kind(), t.pointer(), t.slot(), t.endpoint()), (SET_TR_DEQUEUE, 0x8041, 2, 1));
        assert_eq!(Trb::enable_slot().kind(), ENABLE_SLOT);
        let n = Trb::normal(0x2000, 8);
        assert_eq!((n.kind(), n.pointer(), n.0[2]), (NORMAL, 0x2000, 8));
    }

    #[test]
    fn events() {
        // Transfer event: success, 3 bytes left, endpoint 3, slot 2.
        let e = Trb([0x5000, 0, (CC_SUCCESS as u32) << 24 | 3, 2 << 24 | 3 << 16 | TRANSFER_EVENT << 10 | 1]);
        assert_eq!((e.kind(), e.completion_code(), e.residual(), e.slot(), e.endpoint()), (TRANSFER_EVENT, 1, 3, 2, 3));
        assert!(e.cycle());
        let p = Trb([5 << 24, 0, 1 << 24, PORT_STATUS_CHANGE << 10]);
        assert_eq!(p.port(), 5);
        assert_eq!(describe(6), "stall");
    }

    #[test]
    fn producer_wraps_through_the_link() {
        let mut r = Producer::new(0x10000, 4); // 3 usable slots + link
        let a = r.push(Trb::noop_command());
        assert_eq!((a.index, a.address, a.trb.cycle(), a.link), (0, 0x10000, true, None));
        r.push(Trb::noop_command());
        let c = r.push(Trb::noop_command());
        assert_eq!(c.index, 2);
        let (li, link) = c.link.expect("third push fills the ring");
        assert_eq!((li, link.kind(), link.pointer(), link.cycle()), (3, LINK, 0x10000, true));
        assert_ne!(link.0[3] & TOGGLE_CYCLE, 0);
        // Second lap: cycle bit flipped.
        let d = r.push(Trb::noop_command());
        assert_eq!((d.index, d.trb.cycle()), (0, false));
    }

    #[test]
    fn consumer_wraps_and_flips() {
        let mut e = Consumer::new(0x20000, 2);
        let fresh = Trb::noop_command().with_cycle(true);
        assert!(e.is_new(&fresh));
        assert_eq!(e.advance(), 0x20010);
        assert_eq!(e.advance(), 0x20000);
        assert!(!e.is_new(&fresh), "after wrapping, cycle-1 TRBs are old");
        assert!(e.is_new(&fresh.with_cycle(false)));
    }
}
