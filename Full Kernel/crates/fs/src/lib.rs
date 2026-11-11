//! Files for Nova: the pieces that do not need hardware.
//!
//! * [`novafs`]: **NovaFS**, the on-disk file system (4 KiB blocks, inodes with
//!   direct/indirect/double-indirect blocks, directories, bitmaps, and a
//!   consistency checker).
//! * [`tmpfs`]: a file system that lives in RAM (for `/tmp`).
//! * [`vfs`]: the virtual file system: mount points, path resolution and the
//!   operations the rest of the kernel uses, over anything that implements
//!   [`FileSystem`].
//!
//! Disks are reached through the [`BlockDevice`] trait, so everything here is
//! unit-tested on the host against a RAM disk.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

pub mod novafs;
pub mod path;
pub mod tmpfs;
pub mod vfs;

/// Size of a file system block, and of the unit [`BlockDevice`] transfers.
pub const BLOCK_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    Exists,
    NotDir,
    IsDir,
    NotEmpty,
    NoSpace,
    NameTooLong,
    InvalidName,
    InvalidArgument,
    /// The disk reported an error.
    Io,
    /// The on-disk structures are inconsistent.
    Corrupt,
    Unsupported,
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "no such file or directory",
            Self::Exists => "already exists",
            Self::NotDir => "not a directory",
            Self::IsDir => "is a directory",
            Self::NotEmpty => "directory not empty",
            Self::NoSpace => "no space left on the device",
            Self::NameTooLong => "name too long",
            Self::InvalidName => "invalid name",
            Self::InvalidArgument => "invalid argument",
            Self::Io => "disk I/O error",
            Self::Corrupt => "file system is corrupt",
            Self::Unsupported => "not supported",
        })
    }
}

pub type Result<T> = core::result::Result<T, FsError>;

/// A disk addressed in [`BLOCK_SIZE`] blocks.
pub trait BlockDevice {
    fn block_count(&self) -> u64;
    fn read_block(&mut self, block: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<()>;
    fn write_block(&mut self, block: u64, buf: &[u8; BLOCK_SIZE]) -> Result<()>;
    /// Makes sure everything written so far is on the disk.
    fn flush(&mut self) -> Result<()>;
}

/// A file or directory within one file system.
pub type NodeId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub kind: NodeKind,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub kind: NodeKind,
    pub node: NodeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsStats {
    pub total_bytes: u64,
    pub free_bytes: u64,
}

/// What a file system offers the VFS. Names are single path components.
pub trait FileSystem: Send {
    fn name(&self) -> &'static str;
    fn root(&self) -> NodeId;
    fn lookup(&mut self, dir: NodeId, name: &str) -> Result<NodeId>;
    fn stat(&mut self, node: NodeId) -> Result<Stat>;
    fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, node: NodeId, offset: u64, data: &[u8]) -> Result<usize>;
    fn truncate(&mut self, node: NodeId, size: u64) -> Result<()>;
    fn create(&mut self, dir: NodeId, name: &str, kind: NodeKind) -> Result<NodeId>;
    /// Removes a file, or an empty directory.
    fn unlink(&mut self, dir: NodeId, name: &str) -> Result<()>;
    fn readdir(&mut self, dir: NodeId) -> Result<Vec<DirEntry>>;
    fn sync(&mut self) -> Result<()>;
    fn stats(&mut self) -> Result<FsStats>;
}

/// A [`BlockDevice`] in RAM (for tests and for formatting scratch images).
pub struct RamDisk {
    blocks: Vec<[u8; BLOCK_SIZE]>,
    /// Counts writes, so tests can check what touched the disk.
    pub writes: u64,
}

impl RamDisk {
    pub fn new(blocks: usize) -> Self {
        Self { blocks: alloc::vec![[0u8; BLOCK_SIZE]; blocks], writes: 0 }
    }
}

impl BlockDevice for RamDisk {
    fn block_count(&self) -> u64 {
        self.blocks.len() as u64
    }

    fn read_block(&mut self, block: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<()> {
        *buf = *self.blocks.get(block as usize).ok_or(FsError::Io)?;
        Ok(())
    }

    fn write_block(&mut self, block: u64, buf: &[u8; BLOCK_SIZE]) -> Result<()> {
        *self.blocks.get_mut(block as usize).ok_or(FsError::Io)? = *buf;
        self.writes += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
