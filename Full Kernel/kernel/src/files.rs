//! The kernel's file system: one global VFS, disk discovery and mounting at
//! boot, formatting, and the error numbers system calls report.
//!
//! At boot the root (`/`) is the first disk that holds a NovaFS. If no disk
//! does, `/` is a RAM file system (nothing is saved) until `format` is used.
//! `/tmp` is always a RAM file system.

use crate::ahci::{self, AhciDisk, DiskInfo};
use crate::kmutex::KMutex;
use crate::pci;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use nova_fs::novafs::NovaFs;
use nova_fs::tmpfs::TmpFs;
use nova_fs::vfs::Vfs;
use core::sync::atomic::{AtomicUsize, Ordering};
use nova_fs::FsError;

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

/// The disks found at boot.
pub fn disks() -> Vec<(usize, String, u64)> {
    DISKS.lock().iter().map(|d| (d.index, d.model.clone(), d.sectors)).collect()
}

/// Finds the PCI devices and disks and mounts the file systems.
pub fn init() {
    let devices = pci::enumerate();
    for d in &devices {
        // Bridges and the like are noise; show the devices that matter for storage and I/O.
        if matches!(d.class, 0x01 | 0x02 | 0x03 | 0x0C) {
            crate::kprintln!("pci: {} {:04x}:{:04x} {}", d.address(), d.vendor, d.device_id, d.class_name());
        }
    }
    let found = ahci::init(&devices);
    for d in &found {
        crate::kprintln!("ahci: disk {}: {} ({} MiB)", d.index, d.model, (d.sectors * 512) >> 20);
    }
    *DISKS.lock() = found;

    let mut vfs = VFS.lock();
    let mut root_from = None;
    for (index, ..) in disks() {
        if let Ok(fs) = NovaFs::mount(AhciDisk::new(index)) {
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
    if !disks().iter().any(|(i, ..)| *i == index) {
        return Err(FsError::NotFound);
    }
    let fs = NovaFs::format(AhciDisk::new(index))?;
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

/// Runs the NovaFS consistency checker on disk `index` (read-only on the disk; the
/// disk must not be the mounted root, so this checks a second view of the same
/// device only when nothing is writing: callers sync first).
pub fn fsck_disk(index: usize) -> Result<Vec<String>, FsError> {
    let mut fs = NovaFs::mount(AhciDisk::new(index))?;
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
