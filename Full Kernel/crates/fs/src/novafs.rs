//! NovaFS: Nova's own on-disk file system.
//!
//! ```text
//! block 0            superblock (magic, geometry, free counts, checksum)
//! block 1..          inode bitmap, then block bitmap
//!   ...              inode table (128-byte inodes, 32 per block)
//! data_start..       data blocks, indirect blocks, directory blocks
//! ```
//!
//! * Blocks are 4 KiB. An inode holds 12 direct block numbers, one indirect
//!   and one double-indirect block, so a file can reach ~4 GiB; holes read as
//!   zeros.
//! * A directory is an ordinary file of 64-byte entries (inode, kind, name up
//!   to 58 bytes). `.` and `..` are not stored; the VFS resolves them.
//! * Inode 0 is never used; the root directory is inode 1.
//! * There is **no journal**. Operations are ordered so a crash can leak
//!   blocks or inodes (which [`NovaFs::fsck`] reports) but should not leave a
//!   directory entry pointing at free storage. Call `sync` to push everything
//!   to the disk.

use crate::path::{MAX_NAME, check_name};
use crate::{BLOCK_SIZE, BlockDevice, DirEntry, FileSystem, FsError, FsStats, NodeId, NodeKind, Result, Stat};
use alloc::string::String;
use alloc::vec::Vec;

const MAGIC: [u8; 8] = *b"NOVAFS01";
const VERSION: u32 = 1;
const INODE_SIZE: usize = 128;
const INODES_PER_BLOCK: usize = BLOCK_SIZE / INODE_SIZE;
const DIRECT: usize = 12;
const PTRS: usize = BLOCK_SIZE / 4;
const DIRENT_SIZE: usize = 64;
const ROOT_INO: u32 = 1;
const BITS_PER_BLOCK: u64 = (BLOCK_SIZE * 8) as u64;
/// At most this many inodes, however big the disk (keeps formatting fast).
const MAX_INODES: u64 = 262_144;

#[allow(dead_code)] // documents the on-disk value of an unused inode
const KIND_FREE: u8 = 0;
const KIND_FILE: u8 = 1;
const KIND_DIR: u8 = 2;

/// The largest file, in blocks.
pub const MAX_FILE_BLOCKS: u64 = (DIRECT + PTRS + PTRS * PTRS) as u64;

type Block = [u8; BLOCK_SIZE];

fn put_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_u64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn get_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn get_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn get_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[derive(Clone, Copy, Debug, Default)]
struct Superblock {
    total_blocks: u64,
    inode_count: u32,
    ibitmap_start: u64,
    ibitmap_blocks: u32,
    bbitmap_start: u64,
    bbitmap_blocks: u32,
    itable_start: u64,
    itable_blocks: u32,
    data_start: u64,
    free_blocks: u64,
    free_inodes: u32,
}

const SB_USED: usize = 100;

impl Superblock {
    fn encode(&self, b: &mut Block) {
        b.fill(0);
        b[0..8].copy_from_slice(&MAGIC);
        put_u32(b, 8, VERSION);
        put_u32(b, 12, BLOCK_SIZE as u32);
        put_u64(b, 16, self.total_blocks);
        put_u32(b, 24, self.inode_count);
        put_u64(b, 32, self.ibitmap_start);
        put_u32(b, 40, self.ibitmap_blocks);
        put_u64(b, 48, self.bbitmap_start);
        put_u32(b, 56, self.bbitmap_blocks);
        put_u64(b, 64, self.itable_start);
        put_u32(b, 72, self.itable_blocks);
        put_u64(b, 80, self.data_start);
        put_u64(b, 88, self.free_blocks);
        put_u32(b, 96, self.free_inodes);
        let crc = crc32(&b[..SB_USED]);
        put_u32(b, BLOCK_SIZE - 4, crc);
    }

    fn decode(b: &Block) -> Result<Self> {
        if b[0..8] != MAGIC || get_u32(b, 8) != VERSION || get_u32(b, 12) != BLOCK_SIZE as u32 {
            return Err(FsError::Corrupt);
        }
        if get_u32(b, BLOCK_SIZE - 4) != crc32(&b[..SB_USED]) {
            return Err(FsError::Corrupt);
        }
        Ok(Self {
            total_blocks: get_u64(b, 16),
            inode_count: get_u32(b, 24),
            ibitmap_start: get_u64(b, 32),
            ibitmap_blocks: get_u32(b, 40),
            bbitmap_start: get_u64(b, 48),
            bbitmap_blocks: get_u32(b, 56),
            itable_start: get_u64(b, 64),
            itable_blocks: get_u32(b, 72),
            data_start: get_u64(b, 80),
            free_blocks: get_u64(b, 88),
            free_inodes: get_u32(b, 96),
        })
    }

    /// The regions must tile the disk in order, with no overlap and no overflow.
    fn validate(&self, device_blocks: u64) -> Result<()> {
        let ok = self.total_blocks <= device_blocks
            && self.ibitmap_start == 1
            && self.bbitmap_start == self.ibitmap_start + self.ibitmap_blocks as u64
            && self.itable_start == self.bbitmap_start + self.bbitmap_blocks as u64
            && self.data_start == self.itable_start + self.itable_blocks as u64
            && self.data_start < self.total_blocks
            && self.inode_count as usize == self.itable_blocks as usize * INODES_PER_BLOCK
            && self.ibitmap_blocks as u64 * BITS_PER_BLOCK >= self.inode_count as u64
            && self.bbitmap_blocks as u64 * BITS_PER_BLOCK >= self.total_blocks
            && self.free_blocks <= self.total_blocks
            && self.free_inodes <= self.inode_count;
        if ok { Ok(()) } else { Err(FsError::Corrupt) }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Inode {
    kind: u8,
    links: u16,
    size: u64,
    direct: [u32; DIRECT],
    indirect: u32,
    dindirect: u32,
}

impl Inode {
    fn encode(&self, b: &mut [u8]) {
        b[..INODE_SIZE].fill(0);
        b[0] = self.kind;
        put_u16(b, 2, self.links);
        put_u64(b, 8, self.size);
        for (i, d) in self.direct.iter().enumerate() {
            put_u32(b, 16 + 4 * i, *d);
        }
        put_u32(b, 64, self.indirect);
        put_u32(b, 68, self.dindirect);
    }

    fn decode(b: &[u8]) -> Self {
        let mut direct = [0u32; DIRECT];
        for (i, d) in direct.iter_mut().enumerate() {
            *d = get_u32(b, 16 + 4 * i);
        }
        Self { kind: b[0], links: get_u16(b, 2), size: get_u64(b, 8), direct, indirect: get_u32(b, 64), dindirect: get_u32(b, 68) }
    }

    fn node_kind(&self) -> Option<NodeKind> {
        match self.kind {
            KIND_FILE => Some(NodeKind::File),
            KIND_DIR => Some(NodeKind::Dir),
            _ => None,
        }
    }
}

/// A mounted NovaFS on a block device.
pub struct NovaFs<B: BlockDevice> {
    dev: B,
    sb: Superblock,
    sb_dirty: bool,
    /// Where the next block allocation starts looking.
    hint: u64,
}

impl<B: BlockDevice> NovaFs<B> {
    /// Creates an empty file system on `dev`, destroying whatever was there.
    pub fn format(mut dev: B) -> Result<Self> {
        let total = dev.block_count();
        if total < 16 {
            return Err(FsError::InvalidArgument);
        }
        let inodes = ((total / 4).clamp(64, MAX_INODES) / INODES_PER_BLOCK as u64 * INODES_PER_BLOCK as u64) as u32;
        let ibitmap_blocks = (inodes as u64).div_ceil(BITS_PER_BLOCK) as u32;
        let bbitmap_blocks = total.div_ceil(BITS_PER_BLOCK) as u32;
        let itable_blocks = inodes / INODES_PER_BLOCK as u32;
        let ibitmap_start = 1;
        let bbitmap_start = ibitmap_start + ibitmap_blocks as u64;
        let itable_start = bbitmap_start + bbitmap_blocks as u64;
        let data_start = itable_start + itable_blocks as u64;
        if data_start + 2 >= total {
            return Err(FsError::NoSpace);
        }
        let sb = Superblock {
            total_blocks: total,
            inode_count: inodes,
            ibitmap_start,
            ibitmap_blocks,
            bbitmap_start,
            bbitmap_blocks,
            itable_start,
            itable_blocks,
            data_start,
            free_blocks: total - data_start,
            free_inodes: inodes - 2, // inode 0 (unused) and the root
        };

        // Bitmaps are written whole, with the metadata and the padding past the end marked used.
        let mut block: Block = [0; BLOCK_SIZE];
        for i in 0..ibitmap_blocks as u64 {
            block.fill(0);
            for bit in 0..BITS_PER_BLOCK {
                let ino = i * BITS_PER_BLOCK + bit;
                if ino < 2 || ino >= inodes as u64 {
                    block[(bit / 8) as usize] |= 1 << (bit % 8);
                }
            }
            dev.write_block(ibitmap_start + i, &block)?;
        }
        for i in 0..bbitmap_blocks as u64 {
            block.fill(0);
            for bit in 0..BITS_PER_BLOCK {
                let blk = i * BITS_PER_BLOCK + bit;
                if blk < data_start || blk >= total {
                    block[(bit / 8) as usize] |= 1 << (bit % 8);
                }
            }
            dev.write_block(bbitmap_start + i, &block)?;
        }
        let mut fs = Self { dev, sb, sb_dirty: true, hint: data_start };
        fs.write_inode(ROOT_INO, &Inode { kind: KIND_DIR, links: 1, ..Default::default() })?;
        fs.commit()?;
        Ok(fs)
    }

    /// Opens an existing NovaFS. `Corrupt` if the superblock is missing or damaged.
    pub fn mount(mut dev: B) -> Result<Self> {
        let mut block: Block = [0; BLOCK_SIZE];
        dev.read_block(0, &mut block)?;
        let sb = Superblock::decode(&block)?;
        sb.validate(dev.block_count())?;
        Ok(Self { hint: sb.data_start, dev, sb, sb_dirty: false })
    }

    /// Gives the device back (after a `sync`).
    pub fn into_device(self) -> B {
        self.dev
    }

    pub fn device(&mut self) -> &mut B {
        &mut self.dev
    }

    fn commit(&mut self) -> Result<()> {
        if self.sb_dirty {
            let mut block: Block = [0; BLOCK_SIZE];
            self.sb.encode(&mut block);
            self.dev.write_block(0, &block)?;
            self.sb_dirty = false;
        }
        Ok(())
    }

    // ---- bitmaps -----------------------------------------------------

    fn bitmap_get(&mut self, start: u64, index: u64) -> Result<bool> {
        let mut b: Block = [0; BLOCK_SIZE];
        self.dev.read_block(start + index / BITS_PER_BLOCK, &mut b)?;
        let bit = index % BITS_PER_BLOCK;
        Ok(b[(bit / 8) as usize] & (1 << (bit % 8)) != 0)
    }

    fn bitmap_set(&mut self, start: u64, index: u64, value: bool) -> Result<()> {
        let mut b: Block = [0; BLOCK_SIZE];
        let blk = start + index / BITS_PER_BLOCK;
        self.dev.read_block(blk, &mut b)?;
        let bit = index % BITS_PER_BLOCK;
        let mask = 1u8 << (bit % 8);
        if value {
            b[(bit / 8) as usize] |= mask;
        } else {
            b[(bit / 8) as usize] &= !mask;
        }
        self.dev.write_block(blk, &b)
    }

    /// Finds a clear bit in a bitmap, starting at `from`, wrapping around.
    fn bitmap_find_free(&mut self, start: u64, blocks: u32, from: u64) -> Result<Option<u64>> {
        let mut b: Block = [0; BLOCK_SIZE];
        let nblocks = blocks as u64;
        let first = (from / BITS_PER_BLOCK) % nblocks;
        for step in 0..nblocks {
            let bi = (first + step) % nblocks;
            self.dev.read_block(start + bi, &mut b)?;
            let skip = if step == 0 { (from % BITS_PER_BLOCK) as usize / 8 } else { 0 };
            for (byte_i, &byte) in b.iter().enumerate().skip(skip) {
                if byte != 0xFF {
                    let bit = (!byte).trailing_zeros() as u64;
                    return Ok(Some(bi * BITS_PER_BLOCK + byte_i as u64 * 8 + bit));
                }
            }
            if step == nblocks - 1 && skip > 0 {
                // Wrapped back to the first block: check the part before `from`.
                for (byte_i, &byte) in b.iter().enumerate().take(skip) {
                    if byte != 0xFF {
                        let bit = (!byte).trailing_zeros() as u64;
                        return Ok(Some(bi * BITS_PER_BLOCK + byte_i as u64 * 8 + bit));
                    }
                }
            }
        }
        Ok(None)
    }

    fn alloc_block(&mut self) -> Result<u32> {
        let (start, blocks) = (self.sb.bbitmap_start, self.sb.bbitmap_blocks);
        let found = self.bitmap_find_free(start, blocks, self.hint)?.ok_or(FsError::NoSpace)?;
        if found >= self.sb.total_blocks || found < self.sb.data_start || found > u32::MAX as u64 {
            return Err(FsError::Corrupt);
        }
        self.bitmap_set(start, found, true)?;
        self.sb.free_blocks = self.sb.free_blocks.saturating_sub(1);
        self.sb_dirty = true;
        self.hint = found + 1;
        // New blocks start zeroed, so holes and fresh indirect blocks read as zeros.
        self.dev.write_block(found, &[0; BLOCK_SIZE])?;
        Ok(found as u32)
    }

    fn free_block(&mut self, blk: u32) -> Result<()> {
        let b = blk as u64;
        if b < self.sb.data_start || b >= self.sb.total_blocks {
            return Err(FsError::Corrupt);
        }
        self.bitmap_set(self.sb.bbitmap_start, b, false)?;
        self.sb.free_blocks += 1;
        self.sb_dirty = true;
        self.hint = self.hint.min(b);
        Ok(())
    }

    fn alloc_inode(&mut self) -> Result<u32> {
        let (start, blocks) = (self.sb.ibitmap_start, self.sb.ibitmap_blocks);
        let found = self.bitmap_find_free(start, blocks, 0)?.ok_or(FsError::NoSpace)?;
        if found >= self.sb.inode_count as u64 {
            return Err(FsError::Corrupt);
        }
        self.bitmap_set(start, found, true)?;
        self.sb.free_inodes = self.sb.free_inodes.saturating_sub(1);
        self.sb_dirty = true;
        Ok(found as u32)
    }

    fn free_inode(&mut self, ino: u32) -> Result<()> {
        self.bitmap_set(self.sb.ibitmap_start, ino as u64, false)?;
        self.sb.free_inodes += 1;
        self.sb_dirty = true;
        Ok(())
    }

    // ---- inodes ------------------------------------------------------

    fn inode_location(&self, ino: u32) -> Result<(u64, usize)> {
        if ino == 0 || ino >= self.sb.inode_count {
            return Err(FsError::Corrupt);
        }
        Ok((self.sb.itable_start + (ino as usize / INODES_PER_BLOCK) as u64, (ino as usize % INODES_PER_BLOCK) * INODE_SIZE))
    }

    fn read_inode(&mut self, ino: u32) -> Result<Inode> {
        let (blk, off) = self.inode_location(ino)?;
        let mut b: Block = [0; BLOCK_SIZE];
        self.dev.read_block(blk, &mut b)?;
        Ok(Inode::decode(&b[off..off + INODE_SIZE]))
    }

    fn write_inode(&mut self, ino: u32, inode: &Inode) -> Result<()> {
        let (blk, off) = self.inode_location(ino)?;
        let mut b: Block = [0; BLOCK_SIZE];
        self.dev.read_block(blk, &mut b)?;
        inode.encode(&mut b[off..off + INODE_SIZE]);
        self.dev.write_block(blk, &b)
    }

    /// Reads an inode that must be in use.
    fn live_inode(&mut self, ino: u32) -> Result<Inode> {
        let inode = self.read_inode(ino)?;
        if inode.node_kind().is_none() {
            return Err(FsError::NotFound);
        }
        Ok(inode)
    }

    // ---- block mapping ----------------------------------------------

    /// Entry `idx` of the pointer table in block `table`; allocates the entry if asked.
    fn table_entry(&mut self, table: u32, idx: usize, alloc: bool) -> Result<Option<u32>> {
        let mut b: Block = [0; BLOCK_SIZE];
        self.dev.read_block(table as u64, &mut b)?;
        let e = get_u32(&b, idx * 4);
        if e != 0 || !alloc {
            return Ok((e != 0).then_some(e));
        }
        let new = self.alloc_block()?;
        // alloc_block may have rewritten other blocks, but not this table: re-read to be safe.
        self.dev.read_block(table as u64, &mut b)?;
        put_u32(&mut b, idx * 4, new);
        self.dev.write_block(table as u64, &b)?;
        Ok(Some(new))
    }

    /// An inode-level pointer slot: returns its block, allocating if asked.
    fn slot(&mut self, slot: &mut u32, alloc: bool) -> Result<Option<u32>> {
        if *slot == 0 && alloc {
            *slot = self.alloc_block()?;
        }
        Ok((*slot != 0).then_some(*slot))
    }

    /// The disk block holding logical block `lbn` of the file (allocated if `alloc`).
    /// The inode is updated in memory; the caller writes it back.
    fn bmap(&mut self, inode: &mut Inode, lbn: u64, alloc: bool) -> Result<Option<u32>> {
        if lbn >= MAX_FILE_BLOCKS {
            return Err(FsError::NoSpace);
        }
        let l = lbn as usize;
        if l < DIRECT {
            return self.slot(&mut inode.direct[l], alloc);
        }
        let l = l - DIRECT;
        if l < PTRS {
            let Some(ind) = self.slot(&mut inode.indirect, alloc)? else { return Ok(None) };
            return self.table_entry(ind, l, alloc);
        }
        let l = l - PTRS;
        let Some(dbl) = self.slot(&mut inode.dindirect, alloc)? else { return Ok(None) };
        let Some(ind) = self.table_entry(dbl, l / PTRS, alloc)? else { return Ok(None) };
        self.table_entry(ind, l % PTRS, alloc)
    }

    /// Frees entries `from..` of a pointer table of *data* blocks; true if the table is now empty.
    fn free_table_entries(&mut self, table: u32, from: usize) -> Result<bool> {
        let mut b: Block = [0; BLOCK_SIZE];
        self.dev.read_block(table as u64, &mut b)?;
        for i in from..PTRS {
            let e = get_u32(&b, i * 4);
            if e != 0 {
                self.free_block(e)?;
                put_u32(&mut b, i * 4, 0);
            }
        }
        self.dev.write_block(table as u64, &b)?;
        Ok((0..PTRS).all(|i| get_u32(&b, i * 4) == 0))
    }

    /// Frees every block of the file at logical block `first` and beyond.
    fn free_from(&mut self, inode: &mut Inode, first: u64) -> Result<()> {
        let first = first.min(MAX_FILE_BLOCKS) as usize;
        for i in first.min(DIRECT)..DIRECT {
            if inode.direct[i] != 0 {
                self.free_block(inode.direct[i])?;
                inode.direct[i] = 0;
            }
        }
        // Single indirect: covers file blocks DIRECT .. DIRECT + PTRS.
        if inode.indirect != 0 {
            let from = first.saturating_sub(DIRECT);
            if from < PTRS && self.free_table_entries(inode.indirect, from)? {
                self.free_block(inode.indirect)?;
                inode.indirect = 0;
            }
        }
        // Double indirect: covers DIRECT + PTRS ..
        if inode.dindirect != 0 {
            let rel = first.saturating_sub(DIRECT + PTRS);
            let (hi, lo) = (rel / PTRS, rel % PTRS);
            let mut b: Block = [0; BLOCK_SIZE];
            self.dev.read_block(inode.dindirect as u64, &mut b)?;
            for i in hi..PTRS {
                let child = get_u32(&b, i * 4);
                if child == 0 {
                    continue;
                }
                let from = if i == hi { lo } else { 0 };
                if self.free_table_entries(child, from)? {
                    self.free_block(child)?;
                    put_u32(&mut b, i * 4, 0);
                }
            }
            self.dev.write_block(inode.dindirect as u64, &b)?;
            if (0..PTRS).all(|i| get_u32(&b, i * 4) == 0) {
                self.free_block(inode.dindirect)?;
                inode.dindirect = 0;
            }
        }
        Ok(())
    }

    // ---- file data ---------------------------------------------------

    fn read_inode_data(&mut self, inode: &mut Inode, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if offset >= inode.size {
            return Ok(0);
        }
        let len = buf.len().min((inode.size - offset) as usize);
        let mut done = 0;
        let mut blockbuf: Block = [0; BLOCK_SIZE];
        while done < len {
            let pos = offset + done as u64;
            let (lbn, within) = (pos / BLOCK_SIZE as u64, (pos % BLOCK_SIZE as u64) as usize);
            let n = (BLOCK_SIZE - within).min(len - done);
            match self.bmap(inode, lbn, false)? {
                Some(blk) => {
                    self.dev.read_block(blk as u64, &mut blockbuf)?;
                    buf[done..done + n].copy_from_slice(&blockbuf[within..within + n]);
                }
                None => buf[done..done + n].fill(0), // a hole
            }
            done += n;
        }
        Ok(len)
    }

    fn write_inode_data(&mut self, ino: u32, inode: &mut Inode, offset: u64, data: &[u8]) -> Result<usize> {
        let end = offset.checked_add(data.len() as u64).ok_or(FsError::InvalidArgument)?;
        if end.div_ceil(BLOCK_SIZE as u64) > MAX_FILE_BLOCKS {
            return Err(FsError::NoSpace);
        }
        let mut done = 0;
        let mut result = Ok(());
        let mut blockbuf: Block = [0; BLOCK_SIZE];
        while done < data.len() {
            let pos = offset + done as u64;
            let (lbn, within) = (pos / BLOCK_SIZE as u64, (pos % BLOCK_SIZE as u64) as usize);
            let n = (BLOCK_SIZE - within).min(data.len() - done);
            let blk = match self.bmap(inode, lbn, true) {
                Ok(Some(b)) => b,
                Ok(None) => unreachable!("bmap with alloc always returns a block"),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            };
            if n == BLOCK_SIZE {
                blockbuf.copy_from_slice(&data[done..done + n]);
            } else {
                self.dev.read_block(blk as u64, &mut blockbuf)?;
                blockbuf[within..within + n].copy_from_slice(&data[done..done + n]);
            }
            self.dev.write_block(blk as u64, &blockbuf)?;
            done += n;
        }
        if done > 0 {
            inode.size = inode.size.max(offset + done as u64);
            self.write_inode(ino, inode)?;
        }
        self.commit()?;
        match result {
            Err(e) if done == 0 => Err(e),
            _ => Ok(done),
        }
    }

    // ---- directories -------------------------------------------------

    /// Calls `f(slot_offset, inode_number, kind, name)` for every used entry;
    /// stops early when `f` returns true.
    fn dir_scan(&mut self, dir: &mut Inode, mut f: impl FnMut(u64, u32, u8, &str) -> bool) -> Result<()> {
        let mut pos = 0u64;
        let mut chunk: Block = [0; BLOCK_SIZE];
        while pos < dir.size {
            let n = self.read_inode_data(dir, pos, &mut chunk)?;
            if n == 0 {
                break;
            }
            for slot in 0..n / DIRENT_SIZE {
                let e = &chunk[slot * DIRENT_SIZE..(slot + 1) * DIRENT_SIZE];
                let ino = get_u32(e, 0);
                if ino == 0 {
                    continue;
                }
                let len = e[5] as usize;
                if len == 0 || len > MAX_NAME {
                    return Err(FsError::Corrupt);
                }
                let name = core::str::from_utf8(&e[6..6 + len]).map_err(|_| FsError::Corrupt)?;
                if f(pos + (slot * DIRENT_SIZE) as u64, ino, e[4], name) {
                    return Ok(());
                }
            }
            pos += n as u64;
        }
        Ok(())
    }

    fn dir_find(&mut self, dir: &mut Inode, name: &str) -> Result<Option<(u64, u32, u8)>> {
        let mut found = None;
        self.dir_scan(dir, |off, ino, kind, n| {
            if n == name {
                found = Some((off, ino, kind));
                true
            } else {
                false
            }
        })?;
        Ok(found)
    }

    fn dir_add(&mut self, dir_ino: u32, dir: &mut Inode, name: &str, ino: u32, kind: u8) -> Result<()> {
        // Reuse the first free slot, else append.
        let mut free_slot = None;
        let mut pos = 0u64;
        let mut chunk: Block = [0; BLOCK_SIZE];
        'scan: while pos < dir.size {
            let n = self.read_inode_data(dir, pos, &mut chunk)?;
            if n == 0 {
                break;
            }
            for slot in 0..n / DIRENT_SIZE {
                if get_u32(&chunk, slot * DIRENT_SIZE) == 0 {
                    free_slot = Some(pos + (slot * DIRENT_SIZE) as u64);
                    break 'scan;
                }
            }
            pos += n as u64;
        }
        let at = free_slot.unwrap_or(dir.size);
        let mut e = [0u8; DIRENT_SIZE];
        put_u32(&mut e, 0, ino);
        e[4] = kind;
        e[5] = name.len() as u8;
        e[6..6 + name.len()].copy_from_slice(name.as_bytes());
        let n = self.write_inode_data(dir_ino, dir, at, &e)?;
        if n != DIRENT_SIZE {
            return Err(FsError::NoSpace);
        }
        Ok(())
    }

    fn dir_is_empty(&mut self, dir: &mut Inode) -> Result<bool> {
        let mut empty = true;
        self.dir_scan(dir, |_, _, _, _| {
            empty = false;
            true
        })?;
        Ok(empty)
    }

    // ---- public operations (also exposed through FileSystem) ----------

    fn dir_inode(&mut self, node: NodeId) -> Result<(u32, Inode)> {
        let ino = u32::try_from(node).map_err(|_| FsError::NotFound)?;
        let inode = self.live_inode(ino)?;
        if inode.kind != KIND_DIR {
            return Err(FsError::NotDir);
        }
        Ok((ino, inode))
    }

    fn file_inode(&mut self, node: NodeId) -> Result<(u32, Inode)> {
        let ino = u32::try_from(node).map_err(|_| FsError::NotFound)?;
        let inode = self.live_inode(ino)?;
        if inode.kind != KIND_FILE {
            return Err(FsError::IsDir);
        }
        Ok((ino, inode))
    }

    /// Checks the whole file system for inconsistencies; returns a description
    /// of each problem found (empty = healthy).
    pub fn fsck(&mut self) -> Result<Vec<String>> {
        let mut problems: Vec<String> = Vec::new();
        let sb = self.sb;
        let total = sb.total_blocks as usize;
        // owner[b]: 0 = unowned, else inode number that references block b.
        let mut owner: Vec<u32> = alloc::vec![0; total];
        let mut inode_used: Vec<bool> = alloc::vec![false; sb.inode_count as usize];
        let mut counted_free_inodes = 0u32;

        for ino in 1..sb.inode_count {
            let used = self.bitmap_get(sb.ibitmap_start, ino as u64)?;
            if !used {
                counted_free_inodes += 1;
                continue;
            }
            inode_used[ino as usize] = true;
            let mut inode = self.read_inode(ino)?;
            if inode.node_kind().is_none() {
                problems.push(alloc::format!("inode {ino} is marked used but has no valid type"));
                continue;
            }
            // Every block this inode references.
            let mut refs: Vec<u32> = Vec::new();
            let blocks = inode.size.div_ceil(BLOCK_SIZE as u64);
            for lbn in 0..blocks.min(MAX_FILE_BLOCKS) {
                if let Some(b) = self.bmap(&mut inode, lbn, false)? {
                    refs.push(b);
                }
            }
            // The pointer blocks themselves (they can exist past `size` after a truncate-grow, so
            // collect them structurally).
            if inode.indirect != 0 {
                refs.push(inode.indirect);
            }
            if inode.dindirect != 0 {
                refs.push(inode.dindirect);
                let mut b: Block = [0; BLOCK_SIZE];
                self.dev.read_block(inode.dindirect as u64, &mut b)?;
                for i in 0..PTRS {
                    let c = get_u32(&b, i * 4);
                    if c != 0 {
                        refs.push(c);
                    }
                }
            }
            for &b in &refs {
                let bi = b as usize;
                if bi < sb.data_start as usize || bi >= total {
                    problems.push(alloc::format!("inode {ino} references block {b}, outside the data area"));
                } else if owner[bi] != 0 {
                    problems.push(alloc::format!("block {b} is used by both inode {} and inode {ino}", owner[bi]));
                } else {
                    owner[bi] = ino;
                }
            }
        }
        if counted_free_inodes != sb.free_inodes {
            problems.push(alloc::format!("superblock says {} free inodes, bitmap says {counted_free_inodes}", sb.free_inodes));
        }

        // Block bitmap vs ownership.
        let mut free_blocks = 0u64;
        for (b, &owned_by) in owner.iter().enumerate().skip(sb.data_start as usize) {
            let used = self.bitmap_get(sb.bbitmap_start, b as u64)?;
            if !used {
                free_blocks += 1;
            }
            match (used, owned_by != 0) {
                (true, false) => problems.push(alloc::format!("block {b} is marked used but nothing references it (leak)")),
                (false, true) => problems.push(alloc::format!("block {b} is in use by inode {owned_by} but marked free")),
                _ => {}
            }
        }
        if free_blocks != sb.free_blocks {
            problems.push(alloc::format!("superblock says {} free blocks, bitmap says {free_blocks}", sb.free_blocks));
        }

        // Directory tree: every entry names a used inode of the right kind; count links.
        let mut links: Vec<u32> = alloc::vec![0; sb.inode_count as usize];
        let mut stack = alloc::vec![ROOT_INO];
        let mut visited = alloc::vec![false; sb.inode_count as usize];
        links[ROOT_INO as usize] = 1; // the root has no parent entry
        while let Some(dir_ino) = stack.pop() {
            if core::mem::replace(&mut visited[dir_ino as usize], true) {
                problems.push(alloc::format!("directory {dir_ino} is reachable twice (loop or hard link)"));
                continue;
            }
            let mut dir = self.read_inode(dir_ino)?;
            let mut entries: Vec<(u32, u8, String)> = Vec::new();
            self.dir_scan(&mut dir, |_, ino, kind, name| {
                entries.push((ino, kind, String::from(name)));
                false
            })?;
            for (ino, kind, name) in entries {
                if ino >= sb.inode_count || !inode_used[ino as usize] {
                    problems.push(alloc::format!("entry '{name}' in directory {dir_ino} points at unused inode {ino}"));
                    continue;
                }
                let child = self.read_inode(ino)?;
                if child.kind != kind {
                    problems.push(alloc::format!("entry '{name}' in directory {dir_ino} has the wrong type for inode {ino}"));
                }
                links[ino as usize] += 1;
                if child.kind == KIND_DIR {
                    stack.push(ino);
                }
            }
        }
        for ino in 1..sb.inode_count {
            if !inode_used[ino as usize] {
                continue;
            }
            let inode = self.read_inode(ino)?;
            if links[ino as usize] == 0 {
                problems.push(alloc::format!("inode {ino} is in use but no directory entry names it (orphan)"));
            } else if inode.links as u32 != links[ino as usize] {
                problems.push(alloc::format!("inode {ino} says {} links but {} entries name it", inode.links, links[ino as usize]));
            }
        }
        Ok(problems)
    }
}

impl<B: BlockDevice + Send> FileSystem for NovaFs<B> {
    fn name(&self) -> &'static str {
        "novafs"
    }

    fn root(&self) -> NodeId {
        ROOT_INO as NodeId
    }

    fn lookup(&mut self, dir: NodeId, name: &str) -> Result<NodeId> {
        let (_, mut d) = self.dir_inode(dir)?;
        match self.dir_find(&mut d, name)? {
            Some((_, ino, _)) => Ok(ino as NodeId),
            None => Err(FsError::NotFound),
        }
    }

    fn stat(&mut self, node: NodeId) -> Result<Stat> {
        let ino = u32::try_from(node).map_err(|_| FsError::NotFound)?;
        let inode = self.live_inode(ino)?;
        Ok(Stat { kind: inode.node_kind().ok_or(FsError::Corrupt)?, size: inode.size })
    }

    fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let (_, mut inode) = self.file_inode(node)?;
        self.read_inode_data(&mut inode, offset, buf)
    }

    fn write(&mut self, node: NodeId, offset: u64, data: &[u8]) -> Result<usize> {
        let (ino, mut inode) = self.file_inode(node)?;
        self.write_inode_data(ino, &mut inode, offset, data)
    }

    fn truncate(&mut self, node: NodeId, size: u64) -> Result<()> {
        let (ino, mut inode) = self.file_inode(node)?;
        if size.div_ceil(BLOCK_SIZE as u64) > MAX_FILE_BLOCKS {
            return Err(FsError::NoSpace);
        }
        if size < inode.size {
            self.free_from(&mut inode, size.div_ceil(BLOCK_SIZE as u64))?;
            // Zero the tail of the new last block so growing again reads zeros.
            let within = (size % BLOCK_SIZE as u64) as usize;
            if within != 0
                && let Some(blk) = self.bmap(&mut inode, size / BLOCK_SIZE as u64, false)?
            {
                let mut b: Block = [0; BLOCK_SIZE];
                self.dev.read_block(blk as u64, &mut b)?;
                b[within..].fill(0);
                self.dev.write_block(blk as u64, &b)?;
            }
        }
        inode.size = size;
        self.write_inode(ino, &inode)?;
        self.commit()
    }

    fn create(&mut self, dir: NodeId, name: &str, kind: NodeKind) -> Result<NodeId> {
        check_name(name)?;
        let (dir_ino, mut d) = self.dir_inode(dir)?;
        if self.dir_find(&mut d, name)?.is_some() {
            return Err(FsError::Exists);
        }
        let ino = self.alloc_inode()?;
        let k = if kind == NodeKind::Dir { KIND_DIR } else { KIND_FILE };
        self.write_inode(ino, &Inode { kind: k, links: 1, ..Default::default() })?;
        if let Err(e) = self.dir_add(dir_ino, &mut d, name, ino, k) {
            // Roll back so a failed create leaks nothing.
            let _ = self.write_inode(ino, &Inode::default());
            let _ = self.free_inode(ino);
            let _ = self.commit();
            return Err(e);
        }
        self.commit()?;
        Ok(ino as NodeId)
    }

    fn unlink(&mut self, dir: NodeId, name: &str) -> Result<()> {
        check_name(name)?;
        let (dir_ino, mut d) = self.dir_inode(dir)?;
        let (slot, ino, _) = self.dir_find(&mut d, name)?.ok_or(FsError::NotFound)?;
        let mut target = self.live_inode(ino)?;
        if target.kind == KIND_DIR && !self.dir_is_empty(&mut target)? {
            return Err(FsError::NotEmpty);
        }
        // Remove the name first: a crash after this leaks storage (fsck reports it) but never
        // leaves a name pointing at freed storage.
        self.write_inode_data(dir_ino, &mut d, slot, &[0u8; DIRENT_SIZE])?;
        self.free_from(&mut target, 0)?;
        self.write_inode(ino, &Inode::default())?;
        self.free_inode(ino)?;
        self.commit()
    }

    fn readdir(&mut self, dir: NodeId) -> Result<Vec<DirEntry>> {
        let (_, mut d) = self.dir_inode(dir)?;
        let mut out = Vec::new();
        self.dir_scan(&mut d, |_, ino, kind, name| {
            out.push(DirEntry { name: String::from(name), kind: if kind == KIND_DIR { NodeKind::Dir } else { NodeKind::File }, node: ino as NodeId });
            false
        })?;
        Ok(out)
    }

    fn sync(&mut self) -> Result<()> {
        self.sb_dirty = true;
        self.commit()?;
        self.dev.flush()
    }

    fn stats(&mut self) -> Result<FsStats> {
        Ok(FsStats {
            total_bytes: (self.sb.total_blocks - self.sb.data_start) * BLOCK_SIZE as u64,
            free_bytes: self.sb.free_blocks * BLOCK_SIZE as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RamDisk;
    use alloc::vec;
    use std::collections::BTreeMap;

    fn fresh(blocks: usize) -> NovaFs<RamDisk> {
        NovaFs::format(RamDisk::new(blocks)).unwrap()
    }

    fn clean(fs: &mut NovaFs<RamDisk>) {
        let problems = fs.fsck().unwrap();
        assert!(problems.is_empty(), "fsck: {problems:#?}");
    }

    fn make(fs: &mut NovaFs<RamDisk>, dir: NodeId, name: &str, data: &[u8]) -> NodeId {
        let n = fs.create(dir, name, NodeKind::File).unwrap();
        assert_eq!(fs.write(n, 0, data).unwrap(), data.len());
        n
    }

    fn read_all(fs: &mut NovaFs<RamDisk>, n: NodeId) -> Vec<u8> {
        let size = fs.stat(n).unwrap().size as usize;
        let mut v = vec![0u8; size];
        assert_eq!(fs.read(n, 0, &mut v).unwrap(), size);
        v
    }

    #[test]
    fn formats_mounts_and_starts_empty_and_clean() {
        let mut fs = fresh(1024);
        assert!(fs.readdir(fs.root()).unwrap().is_empty());
        clean(&mut fs);
        let free = fs.stats().unwrap().free_bytes;
        let dev = fs.into_device();
        let mut fs2 = NovaFs::mount(dev).unwrap();
        assert_eq!(fs2.stats().unwrap().free_bytes, free);
        clean(&mut fs2);
    }

    #[test]
    fn refuses_to_mount_garbage_or_a_damaged_superblock() {
        assert_eq!(NovaFs::mount(RamDisk::new(64)).err(), Some(FsError::Corrupt));
        let mut dev = fresh(256).into_device();
        let mut b = [0u8; BLOCK_SIZE];
        dev.read_block(0, &mut b).unwrap();
        b[30] ^= 1; // flip a bit inside the checksummed area
        dev.write_block(0, &b).unwrap();
        assert_eq!(NovaFs::mount(dev).err(), Some(FsError::Corrupt));
        assert_eq!(NovaFs::format(RamDisk::new(8)).err(), Some(FsError::InvalidArgument));
    }

    #[test]
    fn files_round_trip_and_survive_remount() {
        let mut fs = fresh(1024);
        let root = fs.root();
        let n = make(&mut fs, root, "hello.txt", b"hello, nova");
        assert_eq!(fs.stat(n).unwrap(), Stat { kind: NodeKind::File, size: 11 });
        let mut fs = NovaFs::mount(fs.into_device()).unwrap();
        let n = fs.lookup(fs.root(), "hello.txt").unwrap();
        assert_eq!(read_all(&mut fs, n), b"hello, nova");
        clean(&mut fs);
    }

    #[test]
    fn writes_and_reads_at_offsets_across_block_boundaries() {
        let mut fs = fresh(2048);
        let root = fs.root();
        let n = fs.create(root, "f", NodeKind::File).unwrap();
        let pattern: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        fs.write(n, 4090, &pattern).unwrap(); // straddles blocks 0, 1, 2 and leaves a hole before
        assert_eq!(fs.stat(n).unwrap().size, 4090 + 10_000);
        let mut back = vec![0u8; 10_000];
        assert_eq!(fs.read(n, 4090, &mut back).unwrap(), 10_000);
        assert_eq!(back, pattern);
        let mut head = [1u8; 4090];
        fs.read(n, 0, &mut head).unwrap();
        assert!(head.iter().all(|&b| b == 0), "the hole reads as zeros");
        // Reads past the end return what exists, then nothing.
        let mut tail = [0u8; 100];
        assert_eq!(fs.read(n, 4090 + 9_950, &mut tail).unwrap(), 50);
        assert_eq!(fs.read(n, 1 << 40, &mut tail).unwrap(), 0);
        clean(&mut fs);
    }

    #[test]
    fn large_files_use_indirect_and_double_indirect_blocks() {
        let mut fs = fresh(8192); // 32 MiB
        let root = fs.root();
        let n = fs.create(root, "big", NodeKind::File).unwrap();
        // (The root directory has taken its first block by now; that block stays after deletes.)
        let free_empty = fs.stats().unwrap().free_bytes;
        // One block in each region: direct, single indirect, double indirect (far out).
        for lbn in [0u64, 11, 12, 1035, 1036, 1036 + 1024, 1036 + 5 * 1024 + 7] {
            let data = vec![(lbn % 200) as u8 + 1; BLOCK_SIZE];
            fs.write(n, lbn * BLOCK_SIZE as u64, &data).unwrap();
        }
        for lbn in [0u64, 11, 12, 1035, 1036, 1036 + 1024, 1036 + 5 * 1024 + 7] {
            let mut b = vec![0u8; BLOCK_SIZE];
            fs.read(n, lbn * BLOCK_SIZE as u64, &mut b).unwrap();
            assert!(b.iter().all(|&x| x == (lbn % 200) as u8 + 1), "block {lbn}");
        }
        // Holes between them are zeros and cost nothing.
        let mut b = vec![9u8; BLOCK_SIZE];
        fs.read(n, 500 * BLOCK_SIZE as u64, &mut b).unwrap();
        assert!(b.iter().all(|&x| x == 0));
        clean(&mut fs);
        // Deleting returns every block, including the pointer blocks.
        fs.unlink(root, "big").unwrap();
        assert_eq!(fs.stats().unwrap().free_bytes, free_empty);
        clean(&mut fs);
    }

    #[test]
    fn truncate_shrinks_zeroes_the_tail_and_frees_blocks() {
        let mut fs = fresh(4096);
        let root = fs.root();
        let n = fs.create(root, "t", NodeKind::File).unwrap();
        let free0 = fs.stats().unwrap().free_bytes;
        fs.write(n, 0, &vec![0xAB; 3 * BLOCK_SIZE + 100]).unwrap();
        fs.truncate(n, 5000).unwrap();
        assert_eq!(fs.stat(n).unwrap().size, 5000);
        // Growing again must read zeros where the old data was.
        fs.truncate(n, 9000).unwrap();
        let data = read_all(&mut fs, n);
        assert!(data[..5000].iter().all(|&b| b == 0xAB));
        assert!(data[5000..].iter().all(|&b| b == 0), "stale data leaked back after shrink+grow");
        fs.truncate(n, 0).unwrap();
        assert_eq!(fs.stats().unwrap().free_bytes, free0, "truncate(0) frees everything");
        clean(&mut fs);
    }

    #[test]
    fn truncating_a_double_indirect_file_frees_only_the_tail() {
        let mut fs = fresh(8192);
        let root = fs.root();
        let n = fs.create(root, "d", NodeKind::File).unwrap();
        let far = (DIRECT + PTRS + 3 * PTRS) as u64 * BLOCK_SIZE as u64;
        fs.write(n, 0, b"head").unwrap();
        fs.write(n, far, b"far").unwrap();
        fs.truncate(n, (DIRECT + PTRS + 2 * PTRS) as u64 * BLOCK_SIZE as u64).unwrap();
        let mut b = [0u8; 4];
        fs.read(n, 0, &mut b).unwrap();
        assert_eq!(&b, b"head");
        assert_eq!(fs.stat(n).unwrap().size, (DIRECT + PTRS + 2 * PTRS) as u64 * BLOCK_SIZE as u64);
        clean(&mut fs);
        fs.truncate(n, 4).unwrap();
        clean(&mut fs);
    }

    #[test]
    fn directories_nest_list_and_reject_misuse() {
        let mut fs = fresh(1024);
        let root = fs.root();
        let docs = fs.create(root, "docs", NodeKind::Dir).unwrap();
        let a = fs.create(docs, "a.txt", NodeKind::File).unwrap();
        fs.write(a, 0, b"A").unwrap();
        let deep = fs.create(docs, "deep", NodeKind::Dir).unwrap();
        fs.create(deep, "b", NodeKind::File).unwrap();

        let mut names: Vec<String> = fs.readdir(docs).unwrap().into_iter().map(|e| e.name).collect();
        names.sort();
        assert_eq!(names, ["a.txt", "deep"]);
        assert_eq!(fs.lookup(docs, "deep").unwrap(), deep);
        assert_eq!(fs.lookup(docs, "nope"), Err(FsError::NotFound));
        assert_eq!(fs.create(docs, "a.txt", NodeKind::File), Err(FsError::Exists));
        assert_eq!(fs.lookup(a, "x"), Err(FsError::NotDir));
        assert_eq!(fs.read(docs, 0, &mut [0u8; 4]), Err(FsError::IsDir));
        assert_eq!(fs.unlink(root, "docs"), Err(FsError::NotEmpty));
        assert_eq!(fs.create(docs, "bad/name", NodeKind::File), Err(FsError::InvalidName));
        assert_eq!(fs.create(docs, &"x".repeat(59), NodeKind::File), Err(FsError::NameTooLong));
        clean(&mut fs);
    }

    #[test]
    fn unlink_frees_everything_and_slots_are_reused() {
        let mut fs = fresh(1024);
        let root = fs.root();
        // Warm up: the directory's first block is allocated by the first create and is kept.
        make(&mut fs, root, "warmup", b"x");
        fs.unlink(root, "warmup").unwrap();
        let free0 = fs.stats().unwrap().free_bytes;
        for round in 0..3 {
            for i in 0..40 {
                make(&mut fs, root, &alloc::format!("file{i}"), &vec![round as u8 + 1; 5000]);
            }
            for i in 0..40 {
                fs.unlink(root, &alloc::format!("file{i}")).unwrap();
            }
            assert_eq!(fs.stats().unwrap().free_bytes, free0, "round {round}: blocks leaked");
            clean(&mut fs);
        }
        // The directory did not grow without bound: freed entries are reused.
        assert!(fs.stat(root).unwrap().size <= 40 * DIRENT_SIZE as u64 + DIRENT_SIZE as u64);
    }

    #[test]
    fn running_out_of_space_fails_cleanly_without_leaking() {
        let mut fs = fresh(64);
        let root = fs.root();
        let n = fs.create(root, "huge", NodeKind::File).unwrap();
        let big = vec![7u8; 64 * BLOCK_SIZE];
        // Writes as much as fits, then reports NoSpace on the next attempt.
        let written = fs.write(n, 0, &big).unwrap();
        assert!(written > 0 && written < big.len());
        assert_eq!(fs.write(n, written as u64, &big[..BLOCK_SIZE]), Err(FsError::NoSpace));
        assert_eq!(read_all(&mut fs, n).len(), written);
        fs.unlink(root, "huge").unwrap();
        clean(&mut fs);
        assert!(fs.create(root, "again", NodeKind::File).is_ok(), "space came back");
    }

    #[test]
    fn running_out_of_inodes_is_an_error_not_corruption() {
        let mut fs = fresh(256); // 64 inodes
        let root = fs.root();
        let mut made = 0;
        loop {
            match fs.create(root, &alloc::format!("n{made}"), NodeKind::File) {
                Ok(_) => made += 1,
                Err(e) => {
                    assert_eq!(e, FsError::NoSpace);
                    break;
                }
            }
        }
        assert!(made > 20);
        clean(&mut fs);
    }

    #[test]
    fn fsck_notices_deliberate_damage() {
        let mut fs = fresh(512);
        let root = fs.root();
        make(&mut fs, root, "a", b"some data");
        // Leak a block: mark one more block used in the bitmap behind the file system's back.
        let leak = fs.sb.data_start + 100;
        fs.bitmap_set(fs.sb.bbitmap_start, leak, true).unwrap();
        let problems = fs.fsck().unwrap();
        assert!(problems.iter().any(|p| p.contains("leak")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("free blocks")), "{problems:?}");
        // Orphan an inode: allocate one that no directory names.
        let mut fs = fresh(512);
        let ino = fs.alloc_inode().unwrap();
        fs.write_inode(ino, &Inode { kind: KIND_FILE, links: 1, ..Default::default() }).unwrap();
        fs.commit().unwrap();
        assert!(fs.fsck().unwrap().iter().any(|p| p.contains("orphan")));
    }

    /// Random operations checked against an in-memory model; fsck after each batch.
    #[test]
    fn random_workout_matches_a_model_and_stays_consistent() {
        let mut fs = fresh(4096);
        let root = fs.root();
        let mut model: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut rng = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for step in 0..600 {
            let name = alloc::format!("f{}", next() % 24);
            match next() % 5 {
                0 | 1 => {
                    // write at a random offset, creating the file if needed
                    let node = match fs.lookup(root, &name) {
                        Ok(n) => n,
                        Err(_) => {
                            model.insert(name.clone(), Vec::new());
                            fs.create(root, &name, NodeKind::File).unwrap()
                        }
                    };
                    let off = (next() % 20_000) as usize;
                    let len = (next() % 6_000) as usize + 1;
                    let data: Vec<u8> = (0..len).map(|i| ((step + i) % 256) as u8).collect();
                    let m = model.get_mut(&name).unwrap();
                    if m.len() < off + len {
                        m.resize(off + len, 0);
                    }
                    m[off..off + len].copy_from_slice(&data);
                    assert_eq!(fs.write(node, off as u64, &data).unwrap(), len);
                }
                2 => {
                    if let Some(m) = model.get_mut(&name) {
                        let node = fs.lookup(root, &name).unwrap();
                        let new = (next() % 25_000) as usize;
                        m.resize(new, 0);
                        fs.truncate(node, new as u64).unwrap();
                    }
                }
                3 => {
                    if model.remove(&name).is_some() {
                        fs.unlink(root, &name).unwrap();
                    } else {
                        assert_eq!(fs.unlink(root, &name), Err(FsError::NotFound));
                    }
                }
                _ => {
                    if let Some(m) = model.get(&name) {
                        let node = fs.lookup(root, &name).unwrap();
                        assert_eq!(&read_all(&mut fs, node), m, "step {step}: {name}");
                    }
                }
            }
            if step % 100 == 99 {
                clean(&mut fs);
            }
        }
        for (name, m) in &model {
            let node = fs.lookup(root, name).unwrap();
            assert_eq!(&read_all(&mut fs, node), m);
        }
        clean(&mut fs);
        // And the whole thing survives a remount.
        let mut fs = NovaFs::mount(fs.into_device()).unwrap();
        for (name, m) in &model {
            let node = fs.lookup(fs.root(), name).unwrap();
            assert_eq!(&read_all(&mut fs, node), m);
        }
        clean(&mut fs);
    }
}
