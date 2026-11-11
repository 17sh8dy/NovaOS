//! The virtual file system: one tree of paths made of several mounted file
//! systems. Paths are absolute and normalised lexically (`.` / `..`).
//!
//! A path belongs to the mount with the longest matching prefix; the rest of
//! the path is looked up inside that file system. Directory listings include
//! the mount points that sit directly below them.

use crate::path::{check_name, components};
use crate::{DirEntry, FileSystem, FsError, FsStats, NodeId, NodeKind, Result, Stat};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// A file or directory somewhere in the tree. Stays valid until the file is removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle {
    pub mount: usize,
    pub node: NodeId,
    pub kind: NodeKind,
}

struct Mount {
    comps: Vec<String>,
    fs: Box<dyn FileSystem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountInfo {
    pub path: String,
    pub fs_name: &'static str,
    pub stats: Option<FsStats>,
}

pub struct Vfs {
    mounts: Vec<Mount>,
}

impl Default for Vfs {
    fn default() -> Self {
        Self::new()
    }
}

impl Vfs {
    pub const fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    /// Mounts `fs` at `path`. The root (`/`) must be mounted first; other
    /// mount points are created as directories in their parent if missing.
    pub fn mount(&mut self, path: &str, fs: Box<dyn FileSystem>) -> Result<()> {
        let comps = components(path)?;
        if self.mounts.iter().any(|m| m.comps == comps) {
            return Err(FsError::Exists);
        }
        if !comps.is_empty() {
            if self.mounts.is_empty() {
                return Err(FsError::NotFound);
            }
            match self.mkdir(path) {
                Ok(()) | Err(FsError::Exists) => {}
                Err(e) => return Err(e),
            }
        }
        self.mounts.push(Mount { comps, fs });
        Ok(())
    }

    pub fn is_mounted(&self, path: &str) -> bool {
        components(path).is_ok_and(|c| self.mounts.iter().any(|m| m.comps == c))
    }

    pub fn mounts(&mut self) -> Vec<MountInfo> {
        self.mounts
            .iter_mut()
            .map(|m| MountInfo { path: crate::path::join(&m.comps), fs_name: m.fs.name(), stats: m.fs.stats().ok() })
            .collect()
    }

    /// Flushes every mounted file system.
    pub fn sync_all(&mut self) -> Result<()> {
        let mut result = Ok(());
        for m in &mut self.mounts {
            if let Err(e) = m.fs.sync() {
                result = Err(e);
            }
        }
        result
    }

    /// The mount that owns `comps` and how many leading components belong to the mount point.
    fn locate(&self, comps: &[String]) -> Result<(usize, usize)> {
        self.mounts
            .iter()
            .enumerate()
            .filter(|(_, m)| comps.starts_with(&m.comps))
            .max_by_key(|(_, m)| m.comps.len())
            .map(|(i, m)| (i, m.comps.len()))
            .ok_or(FsError::NotFound)
    }

    fn walk(&mut self, comps: &[String]) -> Result<Handle> {
        let (idx, skip) = self.locate(comps)?;
        let fs = &mut self.mounts[idx].fs;
        let mut node = fs.root();
        for c in &comps[skip..] {
            node = fs.lookup(node, c)?;
        }
        let kind = fs.stat(node)?.kind;
        Ok(Handle { mount: idx, node, kind })
    }

    /// Looks up `path`.
    pub fn resolve(&mut self, path: &str) -> Result<Handle> {
        let comps = components(path)?;
        self.walk(&comps)
    }

    /// The directory containing `path` and the final name.
    fn parent(&mut self, path: &str) -> Result<(Handle, String)> {
        let mut comps = components(path)?;
        let name = comps.pop().ok_or(FsError::InvalidArgument)?;
        check_name(&name)?;
        let dir = self.walk(&comps)?;
        if dir.kind != NodeKind::Dir {
            return Err(FsError::NotDir);
        }
        Ok((dir, name))
    }

    pub fn stat(&mut self, path: &str) -> Result<Stat> {
        let h = self.resolve(path)?;
        self.stat_handle(&h)
    }

    pub fn stat_handle(&mut self, h: &Handle) -> Result<Stat> {
        self.mounts[h.mount].fs.stat(h.node)
    }

    /// Creates an empty file or directory; `Exists` if the name is taken.
    pub fn create(&mut self, path: &str, kind: NodeKind) -> Result<Handle> {
        let (dir, name) = self.parent(path)?;
        let node = self.mounts[dir.mount].fs.create(dir.node, &name, kind)?;
        Ok(Handle { mount: dir.mount, node, kind })
    }

    pub fn mkdir(&mut self, path: &str) -> Result<()> {
        self.create(path, NodeKind::Dir).map(|_| ())
    }

    /// Removes a file or an empty directory (not a mount point).
    pub fn remove(&mut self, path: &str) -> Result<()> {
        let comps = components(path)?;
        if self.mounts.iter().any(|m| m.comps == comps) {
            return Err(FsError::InvalidArgument);
        }
        let (dir, name) = self.parent(path)?;
        self.mounts[dir.mount].fs.unlink(dir.node, &name)
    }

    pub fn readdir(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let comps = components(path)?;
        let h = self.walk(&comps)?;
        if h.kind != NodeKind::Dir {
            return Err(FsError::NotDir);
        }
        let mut entries = self.mounts[h.mount].fs.readdir(h.node)?;
        // Mount points directly below this directory appear even if the parent file system has no entry.
        for m in &self.mounts {
            if m.comps.len() == comps.len() + 1 && m.comps.starts_with(&comps) {
                let name = &m.comps[comps.len()];
                if !entries.iter().any(|e| &e.name == name) {
                    entries.push(DirEntry { name: name.clone(), kind: NodeKind::Dir, node: u64::MAX });
                }
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    pub fn read_at(&mut self, h: &Handle, offset: u64, buf: &mut [u8]) -> Result<usize> {
        self.mounts[h.mount].fs.read(h.node, offset, buf)
    }

    pub fn write_at(&mut self, h: &Handle, offset: u64, data: &[u8]) -> Result<usize> {
        self.mounts[h.mount].fs.write(h.node, offset, data)
    }

    pub fn truncate(&mut self, h: &Handle, size: u64) -> Result<()> {
        self.mounts[h.mount].fs.truncate(h.node, size)
    }

    /// The whole contents of a file.
    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>> {
        let h = self.resolve(path)?;
        if h.kind == NodeKind::Dir {
            return Err(FsError::IsDir);
        }
        let size = self.stat_handle(&h)?.size as usize;
        let mut data = alloc::vec![0u8; size];
        let mut done = 0;
        while done < size {
            let n = self.read_at(&h, done as u64, &mut data[done..])?;
            if n == 0 {
                data.truncate(done);
                break;
            }
            done += n;
        }
        Ok(data)
    }

    /// Creates the file if needed and replaces its contents.
    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let h = match self.resolve(path) {
            Ok(h) if h.kind == NodeKind::Dir => return Err(FsError::IsDir),
            Ok(h) => h,
            Err(FsError::NotFound) => self.create(path, NodeKind::File)?,
            Err(e) => return Err(e),
        };
        self.truncate(&h, 0)?;
        let mut done = 0;
        while done < data.len() {
            let n = self.write_at(&h, done as u64, &data[done..])?;
            if n == 0 {
                return Err(FsError::NoSpace);
            }
            done += n;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::novafs::NovaFs;
    use crate::tmpfs::TmpFs;
    use crate::RamDisk;

    fn vfs() -> Vfs {
        let mut v = Vfs::new();
        v.mount("/", Box::new(NovaFs::format(RamDisk::new(1024)).unwrap())).unwrap();
        v.mount("/tmp", Box::new(TmpFs::new(1 << 20))).unwrap();
        v
    }

    fn names(v: &mut Vfs, path: &str) -> Vec<String> {
        v.readdir(path).unwrap().into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn paths_resolve_into_the_right_mount() {
        let mut v = vfs();
        v.write_file("/etc-motd", b"disk").unwrap();
        v.write_file("/tmp/scratch", b"ram").unwrap();
        assert_eq!(v.resolve("/etc-motd").unwrap().mount, 0);
        assert_eq!(v.resolve("/tmp/scratch").unwrap().mount, 1);
        assert_eq!(v.read_file("/tmp/./scratch").unwrap(), b"ram");
        assert_eq!(v.read_file("/tmp/../etc-motd").unwrap(), b"disk");
        let info = v.mounts();
        assert_eq!((info[0].path.as_str(), info[0].fs_name), ("/", "novafs"));
        assert_eq!((info[1].path.as_str(), info[1].fs_name), ("/tmp", "tmpfs"));
    }

    #[test]
    fn listings_include_mount_points() {
        let mut v = vfs();
        v.mkdir("/bin").unwrap();
        assert_eq!(names(&mut v, "/"), ["bin", "tmp"]);
        // Even a mount point whose directory the parent file system lacks.
        let mut v = Vfs::new();
        v.mount("/", Box::new(TmpFs::new(100))).unwrap();
        v.mount("/mnt/x", Box::new(TmpFs::new(100))).unwrap_err(); // parent /mnt missing
        v.mkdir("/mnt").unwrap();
        v.mount("/mnt/x", Box::new(TmpFs::new(100))).unwrap();
        assert_eq!(names(&mut v, "/mnt"), ["x"]);
    }

    #[test]
    fn mkdir_remove_and_errors() {
        let mut v = vfs();
        v.mkdir("/a").unwrap();
        v.write_file("/a/f", b"x").unwrap();
        assert_eq!(v.mkdir("/a"), Err(FsError::Exists));
        assert_eq!(v.remove("/a"), Err(FsError::NotEmpty));
        assert_eq!(v.remove("/tmp"), Err(FsError::InvalidArgument), "cannot remove a mount point");
        assert_eq!(v.read_file("/a"), Err(FsError::IsDir));
        assert_eq!(v.read_file("/a/f/x"), Err(FsError::NotDir));
        assert_eq!(v.read_file("/nope"), Err(FsError::NotFound));
        assert_eq!(v.write_file("/a", b"x"), Err(FsError::IsDir));
        assert_eq!(v.write_file("/missing/f", b"x"), Err(FsError::NotFound));
        assert_eq!(v.resolve("relative"), Err(FsError::InvalidArgument));
        v.remove("/a/f").unwrap();
        v.remove("/a").unwrap();
        assert_eq!(v.stat("/a"), Err(FsError::NotFound));
    }

    #[test]
    fn write_file_replaces_contents() {
        let mut v = vfs();
        v.write_file("/f", &[1u8; 9000]).unwrap();
        v.write_file("/f", b"short").unwrap();
        assert_eq!(v.read_file("/f").unwrap(), b"short");
        assert_eq!(v.stat("/f").unwrap().size, 5);
    }

    #[test]
    fn handles_read_and_write_at_offsets() {
        let mut v = vfs();
        let h = v.create("/tmp/h", NodeKind::File).unwrap();
        v.write_at(&h, 0, b"hello world").unwrap();
        v.write_at(&h, 6, b"NOVA!").unwrap();
        let mut buf = [0u8; 11];
        assert_eq!(v.read_at(&h, 0, &mut buf).unwrap(), 11);
        assert_eq!(&buf, b"hello NOVA!");
        v.truncate(&h, 5).unwrap();
        assert_eq!(v.read_file("/tmp/h").unwrap(), b"hello");
    }

    #[test]
    fn the_root_must_be_mounted_first() {
        let mut v = Vfs::new();
        assert_eq!(v.mount("/tmp", Box::new(TmpFs::new(1))), Err(FsError::NotFound));
        v.mount("/", Box::new(TmpFs::new(1))).unwrap();
        assert_eq!(v.mount("/", Box::new(TmpFs::new(1))), Err(FsError::Exists));
        assert!(v.is_mounted("/"));
        assert!(!v.is_mounted("/tmp"));
    }
}
