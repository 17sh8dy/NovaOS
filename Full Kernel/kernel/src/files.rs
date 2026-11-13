//! The kernel's file system: one global VFS, disk discovery and mounting at
//! boot, formatting, and the error numbers system calls report.
//!
//! At boot the root (`/`) is the first disk that holds a NovaFS. If no disk
//! does, `/` is a RAM file system (nothing is saved) until `format` is used.
//! `/tmp` is always a RAM file system.

use crate::ahci::{self, AhciDisk};
use crate::kmutex::KMutex;
use crate::nvme::{self, NvmeDisk};
use crate::pci;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use nova_fs::novafs::NovaFs;
use nova_fs::tmpfs::TmpFs;
use nova_fs::vfs::Vfs;
use core::sync::atomic::{AtomicUsize, Ordering};
use nova_fs::{BLOCK_SIZE, BlockDevice, FsError};

/// Which driver a disk belongs to, and its index there.
#[derive(Clone, Copy, Debug)]
pub enum Bus {
    Sata(usize),
    Nvme(usize),
}

impl Bus {
    pub fn name(self) -> &'static str {
        match self {
            Bus::Sata(_) => "SATA",
            Bus::Nvme(_) => "NVMe",
        }
    }
}

/// One disk of any kind. Disks are numbered SATA first, then NVMe.
pub struct DiskInfo {
    pub bus: Bus,
    pub model: String,
    pub bytes: u64,
}

/// A disk, whatever its driver, as the file system sees it.
pub enum Disk {
    Sata(AhciDisk),
    Nvme(NvmeDisk),
}

impl Disk {
    fn open(index: usize) -> Option<Self> {
        Some(match DISKS.lock().get(index)?.bus {
            Bus::Sata(i) => Disk::Sata(AhciDisk::new(i)),
            Bus::Nvme(i) => Disk::Nvme(NvmeDisk::new(i)),
        })
    }
}

impl BlockDevice for Disk {
    fn block_count(&self) -> u64 {
        match self {
            Disk::Sata(d) => d.block_count(),
            Disk::Nvme(d) => d.block_count(),
        }
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8; BLOCK_SIZE]) -> nova_fs::Result<()> {
        match self {
            Disk::Sata(d) => d.read_block(block, buf),
            Disk::Nvme(d) => d.read_block(block, buf),
        }
    }

    fn write_block(&mut self, block: u64, buf: &[u8; BLOCK_SIZE]) -> nova_fs::Result<()> {
        match self {
            Disk::Sata(d) => d.write_block(block, buf),
            Disk::Nvme(d) => d.write_block(block, buf),
        }
    }

    fn flush(&mut self) -> nova_fs::Result<()> {
        match self {
            Disk::Sata(d) => d.flush(),
            Disk::Nvme(d) => d.flush(),
        }
    }
}

pub static VFS: KMutex<Vfs> = KMutex::new(Vfs::new());
static DISKS: KMutex<Vec<DiskInfo>> = KMutex::new(Vec::new());
/// Which disk `/` lives on (usize::MAX = none: `/` is a RAM file system).
static ROOT_DISK: AtomicUsize = AtomicUsize::new(usize::MAX);

pub fn root_disk() -> Option<usize> {
    match ROOT_DISK.load(Ordering::SeqCst) {
        usize::MAX => None,
        i => Some(i),
    }
}

/// How much RAM `/tmp` (and a RAM-only `/`) may hold.
const RAM_FS_BYTES: u64 = 32 << 20;

/// The disks found at boot: (number, model, bytes, bus name).
pub fn disks() -> Vec<(usize, String, u64, &'static str)> {
    DISKS.lock().iter().enumerate().map(|(i, d)| (i, d.model.clone(), d.bytes, d.bus.name())).collect()
}

/// Finds the disks on the PCI devices and mounts the file systems.
pub fn init(devices: &[pci::PciDevice]) {
    let mut found = Vec::new();
    for d in ahci::init(devices) {
        crate::kprintln!("ahci: disk {}: {} ({} MiB)", found.len(), d.model, (d.sectors * 512) >> 20);
        found.push(DiskInfo { bus: Bus::Sata(d.index), model: d.model, bytes: d.sectors * 512 });
    }
    for d in nvme::init(devices) {
        crate::kprintln!("nvme: disk {}: {} ({} MiB)", found.len(), d.model, d.bytes >> 20);
        found.push(DiskInfo { bus: Bus::Nvme(d.index), model: d.model, bytes: d.bytes });
    }
    *DISKS.lock() = found;

    let mut vfs = VFS.lock();
    let mut root_from = None;
    for (index, ..) in disks() {
        let Some(disk) = Disk::open(index) else { continue };
        if let Ok(fs) = NovaFs::mount(disk) {
            let _ = vfs.mount("/", Box::new(fs));
            root_from = Some(index);
            break;
        }
    }
    match root_from {
        Some(i) => {
            ROOT_DISK.store(i, Ordering::SeqCst);
            crate::kprintln!("vfs: mounted NovaFS from disk {i} at /");
        }
        None => {
            let _ = vfs.mount("/", Box::new(TmpFs::new(RAM_FS_BYTES)));
            crate::kprintln!("vfs: no NovaFS on any disk; / is a RAM file system (use 'format DISK erase' to create one)");
        }
    }
    let _ = vfs.mount("/tmp", Box::new(TmpFs::new(RAM_FS_BYTES)));
}

/// Creates a fresh NovaFS on disk `index` and mounts it at `/`, replacing
/// whatever was mounted there. Destroys everything on that disk.
pub fn format_disk(index: usize) -> Result<(), FsError> {
    let disk = Disk::open(index).ok_or(FsError::NotFound)?;
    let fs = NovaFs::format(disk)?;
    let mut vfs = VFS.lock();
    *vfs = Vfs::new();
    vfs.mount("/", Box::new(fs))?;
    vfs.mount("/tmp", Box::new(TmpFs::new(RAM_FS_BYTES)))?;
    ROOT_DISK.store(index, Ordering::SeqCst);
    Ok(())
}

/// Flushes every file system to its disk.
pub fn sync() -> Result<(), FsError> {
    VFS.lock().sync_all()
}

/// Flushes everything and tells the disks the power is going (before a reboot or halt).
pub fn shutdown() {
    let _ = sync();
    nvme::shutdown();
}

/// Runs the NovaFS consistency checker on disk `index` (read-only on the disk; the
/// disk must not be the mounted root, so this checks a second view of the same
/// device only when nothing is writing: callers sync first).
pub fn fsck_disk(index: usize) -> Result<Vec<String>, FsError> {
    let mut fs = NovaFs::mount(Disk::open(index).ok_or(FsError::NotFound)?)?;
    fs.fsck()
}

/// The (negative) error number a system call returns for a file system error.
pub fn errno(e: FsError) -> i64 {
    -match e {
        FsError::NotFound => 2,
        FsError::Io => 5,
        FsError::Exists => 17,
        FsError::NotDir => 20,
        FsError::IsDir => 21,
        FsError::InvalidName | FsError::InvalidArgument => 22,
        FsError::NoSpace => 28,
        FsError::NameTooLong => 36,
        FsError::NotEmpty => 39,
        FsError::Unsupported => 38,
        FsError::Corrupt => 117,
    }
}

/// Makes `path` absolute by joining it onto `cwd` if it is relative.
pub fn absolute(cwd: &str, path: &str) -> String {
    if path.starts_with('/') {
        String::from(path)
    } else if cwd == "/" {
        alloc::format!("/{path}")
    } else {
        alloc::format!("{cwd}/{path}")
    }
}
