//! A file system that lives in RAM: nothing survives a reboot. Used for `/tmp`.

use crate::path::check_name;
use crate::{DirEntry, FileSystem, FsError, FsStats, NodeId, NodeKind, Result, Stat};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

struct Node {
    kind: NodeKind,
    data: Vec<u8>,
    children: BTreeMap<String, NodeId>,
}

pub struct TmpFs {
    /// `None` = a freed slot. Node 0 is the root.
    nodes: Vec<Option<Node>>,
    used_bytes: u64,
    limit_bytes: u64,
}

impl TmpFs {
    /// An empty file system holding at most `limit_bytes` of file data.
    pub fn new(limit_bytes: u64) -> Self {
        Self { nodes: alloc::vec![Some(Node { kind: NodeKind::Dir, data: Vec::new(), children: BTreeMap::new() })], used_bytes: 0, limit_bytes }
    }

    fn node(&mut self, id: NodeId) -> Result<&mut Node> {
        self.nodes.get_mut(id as usize).and_then(|n| n.as_mut()).ok_or(FsError::NotFound)
    }

    fn file(&mut self, id: NodeId) -> Result<&mut Node> {
        let n = self.node(id)?;
        if n.kind == NodeKind::Dir { Err(FsError::IsDir) } else { Ok(n) }
    }
}

impl FileSystem for TmpFs {
    fn name(&self) -> &'static str {
        "tmpfs"
    }

    fn root(&self) -> NodeId {
        0
    }

    fn lookup(&mut self, dir: NodeId, name: &str) -> Result<NodeId> {
        let d = self.node(dir)?;
        if d.kind != NodeKind::Dir {
            return Err(FsError::NotDir);
        }
        d.children.get(name).copied().ok_or(FsError::NotFound)
    }

    fn stat(&mut self, node: NodeId) -> Result<Stat> {
        let n = self.node(node)?;
        Ok(Stat { kind: n.kind, size: n.data.len() as u64 })
    }

    fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let n = self.file(node)?;
        let off = offset.min(n.data.len() as u64) as usize;
        let len = buf.len().min(n.data.len() - off);
        buf[..len].copy_from_slice(&n.data[off..off + len]);
        Ok(len)
    }

    fn write(&mut self, node: NodeId, offset: u64, data: &[u8]) -> Result<usize> {
        let end = offset.checked_add(data.len() as u64).ok_or(FsError::InvalidArgument)?;
        let (limit, used) = (self.limit_bytes, self.used_bytes);
        let n = self.file(node)?;
        let grow = end.saturating_sub(n.data.len() as u64);
        if used + grow > limit {
            return Err(FsError::NoSpace);
        }
        if end > n.data.len() as u64 {
            n.data.resize(end as usize, 0);
        }
        n.data[offset as usize..end as usize].copy_from_slice(data);
        self.used_bytes += grow;
        Ok(data.len())
    }

    fn truncate(&mut self, node: NodeId, size: u64) -> Result<()> {
        let (limit, used) = (self.limit_bytes, self.used_bytes);
        let n = self.file(node)?;
        let old = n.data.len() as u64;
        if size > old && used + (size - old) > limit {
            return Err(FsError::NoSpace);
        }
        n.data.resize(size as usize, 0);
        n.data.shrink_to_fit();
        self.used_bytes = used - old + size;
        Ok(())
    }

    fn create(&mut self, dir: NodeId, name: &str, kind: NodeKind) -> Result<NodeId> {
        check_name(name)?;
        let d = self.node(dir)?;
        if d.kind != NodeKind::Dir {
            return Err(FsError::NotDir);
        }
        if d.children.contains_key(name) {
            return Err(FsError::Exists);
        }
        let id = self.nodes.iter().position(|n| n.is_none()).unwrap_or(self.nodes.len());
        let node = Node { kind, data: Vec::new(), children: BTreeMap::new() };
        if id == self.nodes.len() {
            self.nodes.push(Some(node));
        } else {
            self.nodes[id] = Some(node);
        }
        self.node(dir)?.children.insert(String::from(name), id as NodeId);
        Ok(id as NodeId)
    }

    fn unlink(&mut self, dir: NodeId, name: &str) -> Result<()> {
        check_name(name)?;
        let id = self.lookup(dir, name)?;
        let target = self.node(id)?;
        if target.kind == NodeKind::Dir && !target.children.is_empty() {
            return Err(FsError::NotEmpty);
        }
        let freed = target.data.len() as u64;
        self.nodes[id as usize] = None;
        self.used_bytes -= freed;
        self.node(dir)?.children.remove(name);
        Ok(())
    }

    fn readdir(&mut self, dir: NodeId) -> Result<Vec<DirEntry>> {
        let children: Vec<(String, NodeId)> = {
            let d = self.node(dir)?;
            if d.kind != NodeKind::Dir {
                return Err(FsError::NotDir);
            }
            d.children.iter().map(|(k, v)| (k.clone(), *v)).collect()
        };
        children
            .into_iter()
            .map(|(name, node)| Ok(DirEntry { name, kind: self.node(node)?.kind, node }))
            .collect()
    }

    fn sync(&mut self) -> Result<()> {
        Ok(())
    }

    fn stats(&mut self) -> Result<FsStats> {
        Ok(FsStats { total_bytes: self.limit_bytes, free_bytes: self.limit_bytes - self.used_bytes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behaves_like_a_file_system() {
        let mut fs = TmpFs::new(1 << 20);
        let root = fs.root();
        let d = fs.create(root, "d", NodeKind::Dir).unwrap();
        let f = fs.create(d, "f", NodeKind::File).unwrap();
        assert_eq!(fs.write(f, 3, b"abc").unwrap(), 3);
        assert_eq!(fs.stat(f).unwrap().size, 6);
        let mut buf = [9u8; 8];
        assert_eq!(fs.read(f, 0, &mut buf).unwrap(), 6);
        assert_eq!(&buf[..6], b"\0\0\0abc");
        assert_eq!(fs.unlink(root, "d"), Err(FsError::NotEmpty));
        assert_eq!(fs.create(d, "f", NodeKind::File), Err(FsError::Exists));
        assert_eq!(fs.readdir(d).unwrap()[0].name, "f");
        fs.unlink(d, "f").unwrap();
        fs.unlink(root, "d").unwrap();
        assert!(fs.readdir(root).unwrap().is_empty());
        assert_eq!(fs.stats().unwrap().free_bytes, 1 << 20, "everything freed");
    }

    #[test]
    fn enforces_its_size_limit_and_reuses_slots() {
        let mut fs = TmpFs::new(100);
        let root = fs.root();
        let f = fs.create(root, "f", NodeKind::File).unwrap();
        assert_eq!(fs.write(f, 0, &[1; 100]).unwrap(), 100);
        assert_eq!(fs.write(f, 100, &[1]), Err(FsError::NoSpace));
        fs.truncate(f, 10).unwrap();
        assert_eq!(fs.stats().unwrap().free_bytes, 90);
        fs.unlink(root, "f").unwrap();
        let g = fs.create(root, "g", NodeKind::File).unwrap();
        assert_eq!(g, f, "the freed slot is reused");
        assert_eq!(fs.read(g, 0, &mut [0u8; 4]).unwrap(), 0, "and starts empty");
    }
}
