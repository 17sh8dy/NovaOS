//! virtio-net: the network card QEMU (and most hypervisors) provide, virtio 1.0
//! over PCI ("modern" interface, memory-mapped registers found through the
//! device's vendor capabilities). Polled: [`VirtioNet::recv`] and
//! [`VirtioNet::send`] are called from the network thread.
//!
//! Two split virtqueues of 16 buffers each (receive = queue 0, transmit = queue 1).
//! No offloads are negotiated: every frame is a plain Ethernet frame behind a
//! 12-byte virtio header of zeros. Reference: Virtual I/O Device (VIRTIO)
//! Specification 1.1, sections 2.6 (split virtqueues), 4.1 (PCI) and 5.1 (network).

use crate::pci::{self, PciDevice};
use crate::{bootdata, pmm, vm};
use alloc::vec::Vec;
use core::sync::atomic::{Ordering, fence};

const QSIZE: u16 = 16;
const BUF: usize = 2048;
const HDR: usize = 12;

const CAP_COMMON: u8 = 1;
const CAP_NOTIFY: u8 = 2;
const CAP_DEVICE: u8 = 4;

// Device status bits.
const ACKNOWLEDGE: u8 = 1;
const DRIVER: u8 = 2;
const DRIVER_OK: u8 = 4;
const FEATURES_OK: u8 = 8;

const F_MAC: u32 = 1 << 5;
/// Feature bit 32 (VIRTIO_F_VERSION_1) as bit 0 of the high feature word.
const F_VERSION_1_HI: u32 = 1;

// Common configuration register offsets.
const DEV_FEATURE_SELECT: u64 = 0;
const DEV_FEATURE: u64 = 4;
const DRV_FEATURE_SELECT: u64 = 8;
const DRV_FEATURE: u64 = 12;
const STATUS: u64 = 20;
const QUEUE_SELECT: u64 = 22;
const QUEUE_SIZE: u64 = 24;
const QUEUE_ENABLE: u64 = 28;
const QUEUE_NOTIFY_OFF: u64 = 30;
const QUEUE_DESC: u64 = 32;
const QUEUE_DRIVER: u64 = 40;
const QUEUE_DEVICE: u64 = 48;

const DESC_WRITE: u16 = 2;

fn direct() -> u64 {
    bootdata::info().physical_memory_offset
}

// SAFETY (all register helpers): `a` is inside a device register block or DMA memory we mapped/own.
fn r8(a: u64) -> u8 {
    unsafe { (a as *const u8).read_volatile() }
}
fn r16(a: u64) -> u16 {
    unsafe { (a as *const u16).read_volatile() }
}
fn r32(a: u64) -> u32 {
    unsafe { (a as *const u32).read_volatile() }
}
fn w8(a: u64, v: u8) {
    unsafe { (a as *mut u8).write_volatile(v) }
}
fn w16(a: u64, v: u16) {
    unsafe { (a as *mut u16).write_volatile(v) }
}
fn w32(a: u64, v: u32) {
    unsafe { (a as *mut u32).write_volatile(v) }
}
fn w64(a: u64, v: u64) {
    w32(a, v as u32);
    w32(a + 4, (v >> 32) as u32);
}

struct Queue {
    desc: u64,
    avail: u64,
    used: u64,
    last_used: u16,
    avail_idx: u16,
    notify: u64,
    /// Physical address of each buffer.
    bufs: Vec<u64>,
    /// Transmit only: which buffers the device still owns.
    busy: [bool; QSIZE as usize],
}

impl Queue {
    fn buf_virt(&self, i: usize) -> u64 {
        direct() + self.bufs[i]
    }

    /// The next finished buffer: (descriptor index, bytes the device wrote).
    fn take_used(&mut self) -> Option<(u16, u32)> {
        fence(Ordering::SeqCst);
        if r16(self.used + 2) == self.last_used {
            return None;
        }
        let slot = (self.last_used % QSIZE) as u64;
        let id = r32(self.used + 4 + 8 * slot) as u16;
        let len = r32(self.used + 8 + 8 * slot);
        self.last_used = self.last_used.wrapping_add(1);
        Some((id % QSIZE, len))
    }
}

pub struct VirtioNet {
    pub mac: [u8; 6],
    rx: Queue,
    tx: Queue,
    rx_index: u16,
    tx_index: u16,
}

struct Cap {
    bar: u8,
    offset: u32,
    length: u32,
    /// Notify capability only: bytes between queues' notify registers.
    multiplier: u32,
}

/// The virtio vendor capabilities of `d`, by type.
fn capability(d: &PciDevice, kind: u8) -> Option<Cap> {
    let (b, dv, f) = (d.bus, d.device, d.function);
    let mut ptr = (pci::read32(b, dv, f, 0x34) & 0xFF) as u8;
    for _ in 0..48 {
        if ptr < 0x40 {
            return None;
        }
        let d0 = pci::read32(b, dv, f, ptr);
        if d0 & 0xFF == 0x09 && (d0 >> 24) as u8 == kind {
            let d1 = pci::read32(b, dv, f, ptr + 4);
            return Some(Cap {
                bar: d1 as u8,
                offset: pci::read32(b, dv, f, ptr + 8),
                length: pci::read32(b, dv, f, ptr + 12),
                multiplier: if kind == CAP_NOTIFY { pci::read32(b, dv, f, ptr + 16) } else { 0 },
            });
        }
        ptr = (d0 >> 8) as u8;
    }
    None
}

fn map(d: &PciDevice, c: &Cap) -> Result<u64, &'static str> {
    let base = pci::bar_memory(d, c.bar).ok_or("a virtio register BAR is not memory-mapped")?;
    vm::map_mmio(base + c.offset as u64, c.length.max(16) as u64).map_err(|_| "could not map the virtio registers")
}

/// Is this PCI device a virtio network card (transitional or modern id)?
pub fn is_virtio_net(d: &PciDevice) -> bool {
    d.vendor == 0x1AF4 && (d.device_id == 0x1041 || d.device_id == 0x1000)
}

impl VirtioNet {
    pub fn probe(d: &PciDevice) -> Result<Self, &'static str> {
        let common_cap = capability(d, CAP_COMMON).ok_or("no virtio 1.0 interface (legacy-only device)")?;
        let notify_cap = capability(d, CAP_NOTIFY).ok_or("no notify capability")?;
        let device_cap = capability(d, CAP_DEVICE).ok_or("no device configuration")?;
        pci::enable_dma(d);
        pci::disable_intx(d);
        let common = map(d, &common_cap)?;
        let notify_base = map(d, &notify_cap)?;
        let device = map(d, &device_cap)?;

        // Reset, then walk through the status handshake.
        w8(common + STATUS, 0);
        for _ in 0..1000 {
            if r8(common + STATUS) == 0 {
                break;
            }
        }
        w8(common + STATUS, ACKNOWLEDGE);
        w8(common + STATUS, ACKNOWLEDGE | DRIVER);

        w32(common + DEV_FEATURE_SELECT, 0);
        let lo = r32(common + DEV_FEATURE);
        w32(common + DEV_FEATURE_SELECT, 1);
        let hi = r32(common + DEV_FEATURE);
        if hi & F_VERSION_1_HI == 0 {
            return Err("the device does not offer VIRTIO_F_VERSION_1");
        }
        let want_lo = lo & F_MAC;
        w32(common + DRV_FEATURE_SELECT, 0);
        w32(common + DRV_FEATURE, want_lo);
        w32(common + DRV_FEATURE_SELECT, 1);
        w32(common + DRV_FEATURE, F_VERSION_1_HI);
        w8(common + STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK);
        if r8(common + STATUS) & FEATURES_OK == 0 {
            return Err("the device refused our features");
        }

        let mac = if want_lo & F_MAC != 0 {
            core::array::from_fn(|i| r8(device + i as u64))
        } else {
            [0x02, 0x4E, 0x4F, 0x56, 0x41, 0x01] // locally administered "NOVA"
        };

        let mut rx = Self::queue(common, notify_base, notify_cap.multiplier, 0)?;
        let tx = Self::queue(common, notify_base, notify_cap.multiplier, 1)?;

        // Give every receive buffer to the device.
        for i in 0..QSIZE {
            w64(rx.desc + 16 * i as u64, rx.bufs[i as usize]);
            w32(rx.desc + 16 * i as u64 + 8, BUF as u32);
            w16(rx.desc + 16 * i as u64 + 12, DESC_WRITE);
            w16(rx.avail + 4 + 2 * i as u64, i);
        }
        fence(Ordering::SeqCst);
        rx.avail_idx = QSIZE;
        w16(rx.avail + 2, rx.avail_idx);
        for i in 0..QSIZE {
            w64(tx.desc + 16 * i as u64, tx.bufs[i as usize]);
        }

        w8(common + STATUS, ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK);
        let nic = Self { mac, rx, tx, rx_index: 0, tx_index: 1 };
        nic.rx.notify_queue(nic.rx_index);
        Ok(nic)
    }

    /// Allocates and registers queue `index`.
    fn queue(common: u64, notify_base: u64, multiplier: u32, index: u16) -> Result<Queue, &'static str> {
        w16(common + QUEUE_SELECT, index);
        if r16(common + QUEUE_SIZE) == 0 {
            return Err("the device has no such queue");
        }
        w16(common + QUEUE_SIZE, QSIZE.min(r16(common + QUEUE_SIZE)));
        if r16(common + QUEUE_SIZE) != QSIZE {
            return Err("the device's queue is smaller than 16 entries");
        }
        // One page: descriptors, then the available ring, then the used ring.
        let ring = pmm::alloc_dma_frame(false).ok_or("out of memory")?;
        let (desc, avail, used) = (direct() + ring, direct() + ring + 512, direct() + ring + 1024);
        let mut bufs = Vec::new();
        for _ in 0..(QSIZE as usize * BUF).div_ceil(4096) {
            let f = pmm::alloc_dma_frame(false).ok_or("out of memory")?;
            for k in 0..4096 / BUF {
                bufs.push(f + (k * BUF) as u64);
            }
        }
        bufs.truncate(QSIZE as usize);
        w64(common + QUEUE_DESC, ring);
        w64(common + QUEUE_DRIVER, ring + 512);
        w64(common + QUEUE_DEVICE, ring + 1024);
        let notify = notify_base + r16(common + QUEUE_NOTIFY_OFF) as u64 * multiplier as u64;
        w16(common + QUEUE_ENABLE, 1);
        Ok(Queue { desc, avail, used, last_used: 0, avail_idx: 0, notify, bufs, busy: [false; QSIZE as usize] })
    }

    /// The next received frame, if any.
    pub fn recv(&mut self) -> Option<Vec<u8>> {
        let (id, len) = self.rx.take_used()?;
        let len = len as usize;
        let frame = if (HDR..=BUF).contains(&len) {
            // SAFETY: the device wrote `len` bytes into buffer `id`, which is ours and BUF bytes long.
            let data = unsafe { core::slice::from_raw_parts((self.rx.buf_virt(id as usize) + HDR as u64) as *const u8, len - HDR) };
            Some(data.to_vec())
        } else {
            None
        };
        // Back to the device either way.
        let slot = (self.rx.avail_idx % QSIZE) as u64;
        w16(self.rx.avail + 4 + 2 * slot, id);
        fence(Ordering::SeqCst);
        self.rx.avail_idx = self.rx.avail_idx.wrapping_add(1);
        w16(self.rx.avail + 2, self.rx.avail_idx);
        fence(Ordering::SeqCst);
        self.rx.notify_queue(self.rx_index);
        frame.or_else(|| self.recv())
    }

    /// Queues a frame for transmission. False if the queue is full (the frame is dropped).
    pub fn send(&mut self, frame: &[u8]) -> bool {
        if frame.len() + HDR > BUF {
            return false;
        }
        while let Some((id, _)) = self.tx.take_used() {
            self.tx.busy[id as usize] = false;
        }
        let Some(i) = self.tx.busy.iter().position(|b| !b) else { return false };
        self.tx.busy[i] = true;
        let virt = self.tx.buf_virt(i);
        // SAFETY: buffer `i` is ours (BUF bytes) and the device is not using it (busy[i] was false).
        unsafe {
            core::ptr::write_bytes(virt as *mut u8, 0, HDR);
            core::ptr::copy_nonoverlapping(frame.as_ptr(), (virt + HDR as u64) as *mut u8, frame.len());
        }
        w32(self.tx.desc + 16 * i as u64 + 8, (HDR + frame.len()) as u32);
        w16(self.tx.desc + 16 * i as u64 + 12, 0);
        let slot = (self.tx.avail_idx % QSIZE) as u64;
        w16(self.tx.avail + 4 + 2 * slot, i as u16);
        fence(Ordering::SeqCst);
        self.tx.avail_idx = self.tx.avail_idx.wrapping_add(1);
        w16(self.tx.avail + 2, self.tx.avail_idx);
        fence(Ordering::SeqCst);
        self.tx.notify_queue(self.tx_index);
        true
    }
}

impl Queue {
    /// Rings the doorbell: writing the queue's index to its notify address.
    fn notify_queue(&self, index: u16) {
        w16(self.notify, index);
    }
}
