//! NVMe disks (the M.2 / PCIe SSDs most PCs boot from today).
//!
//! An NVMe controller is driven entirely through queues in memory: the driver
//! writes 64-byte commands into a *submission queue*, tells the controller
//! the new tail through a doorbell register, and the controller writes
//! 16-byte results into a *completion queue*, flipping a phase bit each lap so
//! new entries can be told from old ones. We set up the admin queue pair
//! (IDENTIFY, queue creation) and one I/O queue pair, and poll for
//! completions (interrupts off), one command at a time under the kernel's
//! sleeping mutex, like the AHCI driver.
//!
//! Every namespace with 512-byte or 4 KiB blocks and no metadata becomes a
//! disk of 4 KiB blocks for the file system. Command encoding, queue
//! bookkeeping and IDENTIFY parsing are in `crates/nvme` (host-tested).

use crate::kmutex::KMutex;
use crate::{bootdata, interrupts, pci, pmm, vm};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};
use nova_fs::{BLOCK_SIZE, BlockDevice, FsError};
use nova_nvme::*;

/// Entries per queue: one 4 KiB page of 64-byte submissions.
const QUEUE_ENTRIES: u16 = 64;
/// How long one I/O command may take.
const COMMAND_TIMEOUT_MS: u64 = 5000;

struct Queue {
    id: u16,
    pair: QueuePair,
    sq: u64, // virtual
    cq: u64, // virtual
}

struct Controller {
    regs: u64,
    cap: Capabilities,
    admin: Queue,
    io: Option<Queue>,
    /// One page for data (I/O and IDENTIFY), virtual + physical.
    buf_virt: u64,
    buf_phys: u64,
    address: String,
}

struct Namespace {
    controller: usize,
    nsid: u32,
    blocks: u64,
    block_size: u32,
}

struct State {
    controllers: Vec<Controller>,
    namespaces: Vec<Namespace>,
}

static STATE: KMutex<State> = KMutex::new(State { controllers: Vec::new(), namespaces: Vec::new() });

fn rd32(base: u64, off: usize) -> u32 {
    // SAFETY: `base` is the controller's mapped register block.
    unsafe { ((base + off as u64) as *const u32).read_volatile() }
}

fn wr32(base: u64, off: usize, v: u32) {
    // SAFETY: as above.
    unsafe { ((base + off as u64) as *mut u32).write_volatile(v) }
}

fn rd64(base: u64, off: usize) -> u64 {
    rd32(base, off) as u64 | (rd32(base, off + 4) as u64) << 32
}

fn wr64(base: u64, off: usize, v: u64) {
    wr32(base, off, v as u32);
    wr32(base, off + 4, (v >> 32) as u32);
}

fn direct() -> u64 {
    bootdata::info().physical_memory_offset
}

impl Controller {
    /// Submits `cmd` on `admin` or the I/O queue and waits for its completion.
    fn run(&mut self, admin: bool, cmd: impl FnOnce(u16) -> Command, timeout_ms: u64) -> Result<Completion, &'static str> {
        let regs = self.regs;
        let cap = self.cap;
        let q = if admin { &mut self.admin } else { self.io.as_mut().ok_or("no I/O queue")? };
        let cid = q.pair.cid();
        let cmd = cmd(cid);
        let (slot, tail) = q.pair.push();
        // SAFETY: the submission queue is a page we own; `slot` < QUEUE_ENTRIES.
        unsafe {
            let p = (q.sq + slot as u64 * 64) as *mut u32;
            for (i, d) in cmd.0.iter().enumerate() {
                p.add(i).write_volatile(*d);
            }
        }
        fence(Ordering::SeqCst);
        wr32(regs, cap.doorbell(q.id, false), tail as u32);

        let mut result = None;
        let done = interrupts::wait_until(timeout_ms, || {
            // SAFETY: the completion queue is a page we own; `cq_head` < QUEUE_ENTRIES.
            let d = unsafe {
                let p = (q.cq + q.pair.cq_head as u64 * 16) as *const u32;
                [p.read_volatile(), p.add(1).read_volatile(), p.add(2).read_volatile(), p.add(3).read_volatile()]
            };
            let c = Completion::parse(d);
            if !q.pair.is_new(&c) {
                return false;
            }
            let head = q.pair.pop();
            wr32(regs, cap.doorbell(q.id, true), head as u32);
            if c.cid == cid {
                result = Some(c);
                true
            } else {
                false // a late answer to a command that timed out earlier
            }
        });
        if rd32(regs, REG_CSTS) & CSTS_CFS != 0 {
            return Err("the controller reported a fatal error");
        }
        match result {
            Some(c) if c.ok() => Ok(c),
            Some(c) => Err(c.describe()),
            None if !done => Err("the controller did not answer in time"),
            None => Err("lost completion"),
        }
    }

    fn data(&self) -> &[u8] {
        // SAFETY: the data buffer is one page we own.
        unsafe { core::slice::from_raw_parts(self.buf_virt as *const u8, BLOCK_SIZE) }
    }

    fn data_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above.
        unsafe { core::slice::from_raw_parts_mut(self.buf_virt as *mut u8, BLOCK_SIZE) }
    }

    /// Clean shutdown notification (so the drive does not count an unsafe power loss).
    fn shutdown(&mut self) {
        let cc = rd32(self.regs, REG_CC);
        wr32(self.regs, REG_CC, (cc & !(3 << 14)) | 1 << 14); // SHN = normal
        let regs = self.regs;
        interrupts::wait_until(self.cap.timeout_ms.clamp(100, 5000), || (rd32(regs, REG_CSTS) >> 2) & 3 == 2);
    }
}

fn new_queue(id: u16) -> Option<(Queue, u64, u64)> {
    let sq = pmm::alloc_dma_frame(false)?;
    let Some(cq) = pmm::alloc_dma_frame(false) else {
        let _ = pmm::free_frame(sq);
        return None;
    };
    Some((Queue { id, pair: QueuePair::new(QUEUE_ENTRIES), sq: direct() + sq, cq: direct() + cq }, sq, cq))
}

/// A started controller, its usable namespaces and its model name.
type Started = (Controller, Vec<(u32, NamespaceInfo)>, String);

/// Brings up one controller; returns it with the namespaces it found.
fn start(dev: &pci::PciDevice) -> Result<Started, &'static str> {
    let bar = pci::bar_memory(dev, 0).ok_or("no register BAR")?;
    pci::enable_dma(dev);
    pci::disable_intx(dev);
    let regs = vm::map_mmio(bar, 0x1000).map_err(|_| "could not map the registers")?;
    let cap = Capabilities::parse(rd64(regs, REG_CAP));
    vm::map_mmio(bar, 0x1000 + 4 * cap.doorbell_stride as u64).map_err(|_| "could not map the doorbells")?;
    if !cap.nvm_command_set {
        return Err("no NVM command set");
    }
    if cap.min_page > 4096 {
        return Err("needs pages larger than 4 KiB");
    }
    let timeout = cap.timeout_ms.clamp(500, 30_000);

    // Disable (reset) the controller, so the admin queues can be set up.
    if rd32(regs, REG_CC) & 1 != 0 {
        wr32(regs, REG_CC, rd32(regs, REG_CC) & !1);
    }
    if !interrupts::wait_until(timeout, || rd32(regs, REG_CSTS) & CSTS_RDY == 0) {
        return Err("would not reset");
    }

    let (admin, asq, acq) = new_queue(0).ok_or("no memory for the admin queue")?;
    let buf_phys = pmm::alloc_dma_frame(false).ok_or("no memory for a data buffer")?;
    wr32(regs, REG_INTMS, !0); // no interrupts: we poll
    wr32(regs, REG_AQA, aqa(QUEUE_ENTRIES as u32));
    wr64(regs, REG_ASQ, asq);
    wr64(regs, REG_ACQ, acq);
    wr32(regs, REG_CC, cc_enable());
    if !interrupts::wait_until(timeout, || rd32(regs, REG_CSTS) & (CSTS_RDY | CSTS_CFS) != 0) || rd32(regs, REG_CSTS) & CSTS_CFS != 0 {
        return Err("did not become ready");
    }

    let mut c = Controller { regs, cap, admin, io: None, buf_virt: direct() + buf_phys, buf_phys, address: dev.address() };

    c.run(true, |cid| Command::identify(cid, IDENTIFY_CONTROLLER, 0, buf_phys), 2000).map_err(|_| "IDENTIFY CONTROLLER failed")?;
    let info = ControllerInfo::parse(c.data()).ok_or("bad IDENTIFY data")?;
    let model = String::from(info.model());

    // Active namespaces (NVMe 1.1+); older controllers: try 1..=N.
    let mut ids: Vec<u32> = Vec::new();
    if c.run(true, |cid| Command::identify(cid, IDENTIFY_ACTIVE_NAMESPACES, 0, buf_phys), 2000).is_ok() {
        ids.extend(active_namespaces(c.data()));
    } else {
        ids.extend(1..=info.namespaces.min(16));
    }

    // One I/O queue pair.
    let (io, sq, cq) = new_queue(1).ok_or("no memory for the I/O queue")?;
    c.run(true, |cid| Command::create_cq(cid, 1, QUEUE_ENTRIES as u32, cq), 2000).map_err(|_| "could not create the I/O completion queue")?;
    c.run(true, |cid| Command::create_sq(cid, 1, QUEUE_ENTRIES as u32, sq, 1), 2000).map_err(|_| "could not create the I/O submission queue")?;
    c.io = Some(io);

    let mut namespaces = Vec::new();
    for nsid in ids {
        if c.run(true, |cid| Command::identify(cid, IDENTIFY_NAMESPACE, nsid, buf_phys), 2000).is_err() {
            continue;
        }
        match NamespaceInfo::parse(c.data()) {
            Some(ns) if ns.blocks == 0 => {}
            Some(ns) if ns.metadata != 0 => crate::kprintln!("nvme: {} namespace {nsid}: blocks with metadata are not supported, skipped", c.address),
            Some(ns) if ns.block_size as usize > BLOCK_SIZE => crate::kprintln!("nvme: {} namespace {nsid}: {}-byte blocks are not supported, skipped", c.address, ns.block_size),
            Some(ns) => namespaces.push((nsid, ns)),
            None => crate::kprintln!("nvme: {} namespace {nsid}: unknown block format, skipped", c.address),
        }
    }
    Ok((c, namespaces, model))
}

pub struct DiskInfo {
    pub index: usize,
    pub model: String,
    pub bytes: u64,
}

/// Finds NVMe controllers and their namespaces.
pub fn init(devices: &[pci::PciDevice]) -> Vec<DiskInfo> {
    let mut disks = Vec::new();
    let mut st = STATE.lock();
    for dev in devices.iter().filter(|d| d.class == 0x01 && d.subclass == 0x08 && d.prog_if == 0x02) {
        match start(dev) {
            Ok((c, namespaces, model)) => {
                let ci = st.controllers.len();
                let vs = rd32(c.regs, REG_VS);
                crate::kprintln!("nvme: {} {model}: NVMe {}.{}, {} namespace(s)", c.address, vs >> 16, (vs >> 8) & 0xFF, namespaces.len());
                st.controllers.push(c);
                for (nsid, ns) in namespaces {
                    let name = if nsid == 1 { model.clone() } else { alloc::format!("{model} (namespace {nsid})") };
                    disks.push(DiskInfo { index: st.namespaces.len(), model: name, bytes: ns.blocks * ns.block_size as u64 });
                    st.namespaces.push(Namespace { controller: ci, nsid, blocks: ns.blocks, block_size: ns.block_size });
                }
            }
            Err(e) => crate::kprintln!("nvme: {}: {e}", dev.address()),
        }
    }
    disks
}

/// Tells every controller the power is about to go (call after the last write).
pub fn shutdown() {
    for c in STATE.lock().controllers.iter_mut() {
        c.shutdown();
    }
}

/// A namespace, as the file system sees it.
pub struct NvmeDisk {
    index: usize,
}

impl NvmeDisk {
    pub fn new(index: usize) -> Self {
        Self { index }
    }
}

fn io_error(what: &str, block: u64, e: &str) -> FsError {
    crate::kprintln!("nvme: {what} of block {block} failed: {e}");
    FsError::Io
}

impl NvmeDisk {
    /// Reads into `into`, or (when `into` is None) writes `from`.
    fn transfer(&mut self, block: u64, into: Option<&mut [u8; BLOCK_SIZE]>, from: Option<&[u8; BLOCK_SIZE]>) -> nova_fs::Result<()> {
        let write = into.is_none();
        let mut st = STATE.lock();
        let State { controllers, namespaces } = &mut *st;
        let ns = namespaces.get(self.index).ok_or(FsError::Io)?;
        let per = (BLOCK_SIZE as u32 / ns.block_size) as u64;
        if (block + 1) * per > ns.blocks {
            return Err(FsError::Io);
        }
        let (nsid, page) = (ns.nsid, controllers[ns.controller].buf_phys);
        let c = &mut controllers[ns.controller];
        if let Some(src) = from {
            c.data_mut().copy_from_slice(src);
        }
        c.run(false, |cid| Command::rw(write, cid, nsid, block * per, per as u16, page), COMMAND_TIMEOUT_MS)
            .map_err(|e| io_error(if write { "write" } else { "read" }, block, e))?;
        if let Some(dst) = into {
            dst.copy_from_slice(c.data());
        }
        Ok(())
    }
}

impl BlockDevice for NvmeDisk {
    fn block_count(&self) -> u64 {
        STATE.lock().namespaces.get(self.index).map_or(0, |ns| ns.blocks * ns.block_size as u64 / BLOCK_SIZE as u64)
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8; BLOCK_SIZE]) -> nova_fs::Result<()> {
        self.transfer(block, Some(buf), None)
    }

    fn write_block(&mut self, block: u64, buf: &[u8; BLOCK_SIZE]) -> nova_fs::Result<()> {
        self.transfer(block, None, Some(buf))
    }

    fn flush(&mut self) -> nova_fs::Result<()> {
        let mut st = STATE.lock();
        let State { controllers, namespaces } = &mut *st;
        let ns = namespaces.get(self.index).ok_or(FsError::Io)?;
        let nsid = ns.nsid;
        controllers[ns.controller].run(false, |cid| Command::flush(cid, nsid), COMMAND_TIMEOUT_MS).map(|_| ()).map_err(|e| io_error("flush", 0, e))
    }
}
