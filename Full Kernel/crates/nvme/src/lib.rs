//! NVM Express without the hardware: building submission-queue entries,
//! reading completion-queue entries, the head/tail/phase bookkeeping of a
//! queue pair, and parsing the IDENTIFY data structures.
//!
//! The kernel driver (`kernel/src/nvme.rs`) owns the registers and memory;
//! everything here is unit-tested on the host.
//!
//! Reference: NVM Express Base Specification 2.0 (sections 3.1 registers,
//! 4.1 queues, 5 admin commands) and the NVM Command Set Specification.

#![no_std]

#[cfg(test)]
extern crate std;

// ---- Controller registers (offsets into BAR0) ---------------------------------

pub const REG_CAP: usize = 0x00;
pub const REG_VS: usize = 0x08;
pub const REG_INTMS: usize = 0x0C;
pub const REG_CC: usize = 0x14;
pub const REG_CSTS: usize = 0x1C;
pub const REG_AQA: usize = 0x24;
pub const REG_ASQ: usize = 0x28;
pub const REG_ACQ: usize = 0x30;
/// Doorbells start here; their spacing comes from CAP.DSTRD.
pub const REG_DOORBELLS: usize = 0x1000;

pub const CSTS_RDY: u32 = 1 << 0;
pub const CSTS_CFS: u32 = 1 << 1;

/// The fields of the CAP register the driver needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Largest queue the controller supports, in entries (MQES + 1).
    pub max_queue_entries: u32,
    /// Worst-case time to become ready / not ready, in milliseconds.
    pub timeout_ms: u64,
    /// Doorbell stride in bytes (4 << DSTRD).
    pub doorbell_stride: usize,
    /// Supports the NVM command set.
    pub nvm_command_set: bool,
    /// Smallest / largest memory page size, in bytes.
    pub min_page: u64,
    pub max_page: u64,
}

impl Capabilities {
    pub fn parse(cap: u64) -> Self {
        Self {
            max_queue_entries: (cap & 0xFFFF) as u32 + 1,
            timeout_ms: ((cap >> 24) & 0xFF) * 500,
            doorbell_stride: 4 << ((cap >> 32) & 0xF),
            nvm_command_set: (cap >> 37) & 1 != 0,
            min_page: 1 << (12 + ((cap >> 48) & 0xF)),
            max_page: 1 << (12 + ((cap >> 52) & 0xF)),
        }
    }

    /// Byte offset of a queue's doorbell: submission tail (`completion` false) or completion head.
    pub fn doorbell(&self, queue: u16, completion: bool) -> usize {
        REG_DOORBELLS + (2 * queue as usize + completion as usize) * self.doorbell_stride
    }
}

/// The controller configuration (CC) value that enables it: NVM command set,
/// 4 KiB pages, round-robin arbitration, 64-byte submission and 16-byte
/// completion entries.
pub const fn cc_enable() -> u32 {
    // CSS (bits 4..6) = 0 = NVM, MPS (7..10) = 0 = 4 KiB, AMS (11..13) = 0 = round robin.
    1 // EN
        | (6 << 16) // IOSQES = 2^6
        | (4 << 20) // IOCQES = 2^4
}

/// Admin queue attributes: both admin queues `entries` long.
pub const fn aqa(entries: u32) -> u32 {
    (entries - 1) << 16 | (entries - 1)
}

// ---- Commands -------------------------------------------------------------------

pub const ADMIN_DELETE_SQ: u8 = 0x00;
pub const ADMIN_CREATE_SQ: u8 = 0x01;
pub const ADMIN_DELETE_CQ: u8 = 0x04;
pub const ADMIN_CREATE_CQ: u8 = 0x05;
pub const ADMIN_IDENTIFY: u8 = 0x06;

pub const IO_FLUSH: u8 = 0x00;
pub const IO_WRITE: u8 = 0x01;
pub const IO_READ: u8 = 0x02;

pub const IDENTIFY_NAMESPACE: u32 = 0x00;
pub const IDENTIFY_CONTROLLER: u32 = 0x01;
pub const IDENTIFY_ACTIVE_NAMESPACES: u32 = 0x02;

/// One 64-byte submission queue entry, as 16 little-endian dwords.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Command(pub [u32; 16]);

impl Command {
    /// A command with opcode, command id and namespace; everything else zero.
    pub fn new(opcode: u8, cid: u16, nsid: u32) -> Self {
        let mut c = [0u32; 16];
        c[0] = opcode as u32 | (cid as u32) << 16; // FUSE = 0, PSDT = 0 (PRPs)
        c[1] = nsid;
        Self(c)
    }

    pub fn opcode(&self) -> u8 {
        self.0[0] as u8
    }

    pub fn cid(&self) -> u16 {
        (self.0[0] >> 16) as u16
    }

    /// Sets the data pointer to one physical page (PRP entry 1). Enough for any
    /// transfer that fits in one memory page and starts on its boundary.
    pub fn with_page(mut self, phys: u64) -> Self {
        self.0[6] = phys as u32;
        self.0[7] = (phys >> 32) as u32;
        self
    }

    /// Sets PRP entry 2 (the second page, or a PRP list).
    pub fn with_prp2(mut self, phys: u64) -> Self {
        self.0[8] = phys as u32;
        self.0[9] = (phys >> 32) as u32;
        self
    }

    pub fn prp1(&self) -> u64 {
        self.0[6] as u64 | (self.0[7] as u64) << 32
    }

    /// IDENTIFY with the given CNS value, into a 4 KiB page.
    pub fn identify(cid: u16, cns: u32, nsid: u32, page: u64) -> Self {
        let mut c = Self::new(ADMIN_IDENTIFY, cid, nsid).with_page(page);
        c.0[10] = cns;
        c
    }

    /// Create I/O completion queue `qid` of `entries`, physically contiguous at
    /// `phys`, with interrupts off (the driver polls).
    pub fn create_cq(cid: u16, qid: u16, entries: u32, phys: u64) -> Self {
        let mut c = Self::new(ADMIN_CREATE_CQ, cid, 0).with_page(phys);
        c.0[10] = (entries - 1) << 16 | qid as u32;
        c.0[11] = 1; // PC = physically contiguous, IEN = 0
        c
    }

    /// Create I/O submission queue `qid` feeding completion queue `cqid`.
    pub fn create_sq(cid: u16, qid: u16, entries: u32, phys: u64, cqid: u16) -> Self {
        let mut c = Self::new(ADMIN_CREATE_SQ, cid, 0).with_page(phys);
        c.0[10] = (entries - 1) << 16 | qid as u32;
        c.0[11] = (cqid as u32) << 16 | 1; // PC, priority = urgent (ignored for round robin)
        c
    }

    /// Read or write `blocks` logical blocks starting at `lba`.
    pub fn rw(write: bool, cid: u16, nsid: u32, lba: u64, blocks: u16, page: u64) -> Self {
        let mut c = Self::new(if write { IO_WRITE } else { IO_READ }, cid, nsid).with_page(page);
        c.0[10] = lba as u32;
        c.0[11] = (lba >> 32) as u32;
        c.0[12] = (blocks - 1) as u32;
        c
    }

    pub fn flush(cid: u16, nsid: u32) -> Self {
        Self::new(IO_FLUSH, cid, nsid)
    }
}

/// One 16-byte completion queue entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    pub result: u32,
    pub sq_head: u16,
    pub sq_id: u16,
    pub cid: u16,
    pub phase: bool,
    /// Status code type (bits 9..11 of the status field) and code (bits 1..8).
    pub status_type: u8,
    pub status_code: u8,
}

impl Completion {
    pub fn parse(d: [u32; 4]) -> Self {
        let status = (d[3] >> 16) as u16;
        Self {
            result: d[0],
            sq_head: d[2] as u16,
            sq_id: (d[2] >> 16) as u16,
            cid: d[3] as u16,
            phase: status & 1 != 0,
            status_code: (status >> 1) as u8,
            status_type: ((status >> 9) & 0x7) as u8,
        }
    }

    pub fn ok(&self) -> bool {
        self.status_type == 0 && self.status_code == 0
    }

    /// A short description of a failure.
    pub fn describe(&self) -> &'static str {
        match (self.status_type, self.status_code) {
            (0, 0) => "success",
            (0, 0x01) => "invalid opcode",
            (0, 0x02) => "invalid field",
            (0, 0x04) => "data transfer error",
            (0, 0x06) => "internal error",
            (0, 0x0B) => "invalid namespace",
            (0, 0x80) => "LBA out of range",
            (0, 0x81) => "capacity exceeded",
            (0, 0x82) => "namespace not ready",
            (1, 0x01) => "invalid completion queue",
            (1, 0x02) => "invalid queue id",
            (1, 0x03) => "invalid queue size",
            (2, _) => "media error",
            (3, _) => "path error",
            (0, _) => "generic error",
            (1, _) => "command-specific error",
            _ => "vendor-specific error",
        }
    }
}

/// Bookkeeping for one submission/completion queue pair (the memory itself
/// belongs to the driver).
#[derive(Debug)]
pub struct QueuePair {
    pub entries: u16,
    /// Next submission slot to fill.
    pub sq_tail: u16,
    /// Next completion slot to look at.
    pub cq_head: u16,
    /// The phase bit a *new* completion will carry (flips each time round).
    pub phase: bool,
    next_cid: u16,
}

impl QueuePair {
    pub const fn new(entries: u16) -> Self {
        Self { entries, sq_tail: 0, cq_head: 0, phase: true, next_cid: 0 }
    }

    /// Slot for the next command and the new tail to write to the doorbell.
    pub fn push(&mut self) -> (u16, u16) {
        let slot = self.sq_tail;
        self.sq_tail = (self.sq_tail + 1) % self.entries;
        (slot, self.sq_tail)
    }

    /// A fresh command identifier.
    pub fn cid(&mut self) -> u16 {
        self.next_cid = self.next_cid.wrapping_add(1);
        self.next_cid
    }

    /// Whether `c` (read from slot `cq_head`) is a new completion.
    pub fn is_new(&self, c: &Completion) -> bool {
        c.phase == self.phase
    }

    /// Consumes the completion at `cq_head`; returns the new head for the doorbell.
    pub fn pop(&mut self) -> u16 {
        self.cq_head += 1;
        if self.cq_head == self.entries {
            self.cq_head = 0;
            self.phase = !self.phase;
        }
        self.cq_head
    }
}

// ---- IDENTIFY data ----------------------------------------------------------------

fn text(bytes: &[u8]) -> [u8; 40] {
    let mut out = [b' '; 40];
    let n = bytes.len().min(40);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u32_at(b, off) as u64 | (u32_at(b, off + 4) as u64) << 32
}

/// What we use from the IDENTIFY CONTROLLER structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerInfo {
    serial: [u8; 40],
    serial_len: usize,
    model: [u8; 40],
    model_len: usize,
    /// Largest transfer, as a power of two of the minimum page size (0 = no limit).
    pub mdts: u8,
    /// Number of namespaces (highest valid namespace id).
    pub namespaces: u32,
}

impl ControllerInfo {
    pub fn parse(page: &[u8]) -> Option<Self> {
        if page.len() < 4096 {
            return None;
        }
        let trim = |b: &[u8]| b.iter().rposition(|&c| c != b' ' && c != 0).map_or(0, |i| i + 1);
        Some(Self {
            serial: text(&page[4..24]),
            serial_len: trim(&page[4..24]),
            model: text(&page[24..64]),
            model_len: trim(&page[24..64]),
            mdts: page[77],
            namespaces: u32_at(page, 516),
        })
    }

    pub fn model(&self) -> &str {
        core::str::from_utf8(&self.model[..self.model_len]).unwrap_or("NVMe disk")
    }

    pub fn serial(&self) -> &str {
        core::str::from_utf8(&self.serial[..self.serial_len]).unwrap_or("")
    }
}

/// What we use from the IDENTIFY NAMESPACE structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamespaceInfo {
    /// Size in logical blocks.
    pub blocks: u64,
    /// Bytes per logical block in the format in use.
    pub block_size: u32,
    /// Metadata bytes per block (we only handle 0).
    pub metadata: u16,
}

impl NamespaceInfo {
    pub fn parse(page: &[u8]) -> Option<Self> {
        if page.len() < 4096 {
            return None;
        }
        let blocks = u64_at(page, 0);
        let flbas = page[26];
        // Low 4 bits plus (NVMe 2.0) bits 5..6 select the LBA format.
        let index = (flbas & 0xF) as usize | ((flbas as usize >> 5) & 0x3) << 4;
        if index > page[25] as usize || index >= 64 {
            return None;
        }
        let f = u32_at(page, 128 + index * 4);
        let lbads = (f >> 16) & 0xFF;
        if !(9..=16).contains(&lbads) {
            return None;
        }
        Some(Self { blocks, block_size: 1 << lbads, metadata: f as u16 })
    }
}

/// Active namespace ids from an IDENTIFY (CNS 2) page: a zero-terminated list.
pub fn active_namespaces(page: &[u8]) -> impl Iterator<Item = u32> + '_ {
    page.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).take_while(|&n| n != 0).take(1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    #[test]
    fn capabilities() {
        // MQES 2047, TO 15 (7.5 s), DSTRD 0, CSS bit 37, MPSMIN 0, MPSMAX 4.
        let cap = 2047u64 | 15 << 24 | 1 << 37 | 4 << 52;
        let c = Capabilities::parse(cap);
        assert_eq!(c.max_queue_entries, 2048);
        assert_eq!(c.timeout_ms, 7500);
        assert_eq!(c.doorbell_stride, 4);
        assert!(c.nvm_command_set);
        assert_eq!((c.min_page, c.max_page), (4096, 65536));
        assert_eq!(c.doorbell(0, false), 0x1000);
        assert_eq!(c.doorbell(0, true), 0x1004);
        assert_eq!(c.doorbell(1, false), 0x1008);
        // Stride 2 => 16 bytes.
        let c = Capabilities::parse(1 << 37 | 2 << 32);
        assert_eq!(c.doorbell(1, true), 0x1000 + 3 * 16);
    }

    #[test]
    fn controller_config() {
        assert_eq!(cc_enable(), 0x0046_0001);
        assert_eq!(aqa(32), 0x001F_001F);
    }

    #[test]
    fn read_command_layout() {
        let c = Command::rw(false, 7, 1, 0x1_2345_6789, 8, 0xABCD_E000);
        assert_eq!(c.opcode(), IO_READ);
        assert_eq!(c.cid(), 7);
        assert_eq!(c.0[1], 1);
        assert_eq!(c.prp1(), 0xABCD_E000);
        assert_eq!(c.0[10], 0x2345_6789);
        assert_eq!(c.0[11], 1);
        assert_eq!(c.0[12], 7, "number of blocks is zero-based");
        assert_eq!(Command::rw(true, 1, 1, 0, 1, 0).opcode(), IO_WRITE);
    }

    #[test]
    fn queue_creation_commands() {
        let cq = Command::create_cq(1, 1, 64, 0x5000);
        assert_eq!(cq.opcode(), ADMIN_CREATE_CQ);
        assert_eq!(cq.0[10], 63 << 16 | 1);
        assert_eq!(cq.0[11], 1, "contiguous, interrupts off");
        let sq = Command::create_sq(2, 1, 64, 0x6000, 1);
        assert_eq!(sq.opcode(), ADMIN_CREATE_SQ);
        assert_eq!(sq.0[11], 1 << 16 | 1);
        let id = Command::identify(3, IDENTIFY_CONTROLLER, 0, 0x7000);
        assert_eq!((id.opcode(), id.0[10], id.prp1()), (ADMIN_IDENTIFY, 1, 0x7000));
    }

    #[test]
    fn completion_parsing() {
        // Success, phase 1, cid 9, sq head 3, sq 1.
        let c = Completion::parse([0, 0, 1 << 16 | 3, 1 << 16 | 9]);
        assert!(c.ok() && c.phase);
        assert_eq!((c.cid, c.sq_head, c.sq_id), (9, 3, 1));
        // LBA out of range: SCT 0, SC 0x80, phase 0.
        let status: u32 = 0x80 << 1;
        let c = Completion::parse([0, 0, 0, status << 16 | 4]);
        assert!(!c.ok() && !c.phase);
        assert_eq!(c.describe(), "LBA out of range");
        // Media error.
        let c = Completion::parse([0, 0, 0, (2u32 << 9 | 0x81 << 1 | 1) << 16]);
        assert_eq!(c.describe(), "media error");
    }

    #[test]
    fn queue_wraps_and_flips_phase() {
        let mut q = QueuePair::new(4);
        let tails: std::vec::Vec<_> = (0..5).map(|_| q.push()).collect();
        assert_eq!(tails, vec![(0, 1), (1, 2), (2, 3), (3, 0), (0, 1)]);
        let new = |phase| Completion { result: 0, sq_head: 0, sq_id: 0, cid: 0, phase, status_type: 0, status_code: 0 };
        assert!(q.is_new(&new(true)) && !q.is_new(&new(false)));
        for _ in 0..3 {
            q.pop();
        }
        assert!(q.phase);
        assert_eq!(q.pop(), 0);
        assert!(!q.phase, "phase flips when the head wraps");
        assert!(q.is_new(&new(false)));
        assert_ne!(q.cid(), q.cid());
    }

    fn page() -> std::vec::Vec<u8> {
        vec![0u8; 4096]
    }

    #[test]
    fn identify_controller() {
        let mut p = page();
        p[4..24].copy_from_slice(b"NOVA0001            ");
        p[24..64].copy_from_slice(b"QEMU NVMe Ctrl                          ");
        p[77] = 7;
        p[516..520].copy_from_slice(&3u32.to_le_bytes());
        let c = ControllerInfo::parse(&p).unwrap();
        assert_eq!(c.model(), "QEMU NVMe Ctrl");
        assert_eq!(c.serial(), "NOVA0001");
        assert_eq!((c.mdts, c.namespaces), (7, 3));
        assert!(ControllerInfo::parse(&p[..100]).is_none());
    }

    #[test]
    fn identify_namespace() {
        let mut p = page();
        p[0..8].copy_from_slice(&131072u64.to_le_bytes());
        p[25] = 1; // two formats: 0 and 1
        p[26] = 1; // using format 1
        p[128..132].copy_from_slice(&(9u32 << 16).to_le_bytes()); // 512 B
        p[132..136].copy_from_slice(&(12u32 << 16).to_le_bytes()); // 4 KiB
        let n = NamespaceInfo::parse(&p).unwrap();
        assert_eq!(n, NamespaceInfo { blocks: 131072, block_size: 4096, metadata: 0 });
        p[26] = 0;
        assert_eq!(NamespaceInfo::parse(&p).unwrap().block_size, 512);
        p[26] = 5; // format 5 doesn't exist
        assert!(NamespaceInfo::parse(&p).is_none());
        p[26] = 0;
        p[128..132].copy_from_slice(&(9u32 << 16 | 8).to_le_bytes());
        assert_eq!(NamespaceInfo::parse(&p).unwrap().metadata, 8);
    }

    #[test]
    fn namespace_list() {
        let mut p = page();
        p[0..4].copy_from_slice(&1u32.to_le_bytes());
        p[4..8].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(active_namespaces(&p).collect::<std::vec::Vec<_>>(), vec![1, 4]);
        assert_eq!(active_namespaces(&page()).count(), 0);
    }
}
