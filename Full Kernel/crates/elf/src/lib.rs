//! A small ELF64 reader: just enough to load a statically linked x86_64
//! kernel. Every offset and size is bounds-checked against the file, so a
//! corrupt kernel file produces an error instead of a wild copy.

#![no_std]

#[cfg(test)]
extern crate std;

use core::fmt;

pub const PT_LOAD: u32 = 1;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

const EHDR_SIZE: usize = 64;
const PHDR_SIZE: usize = 56;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 0x3E;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfError {
    TooSmall,
    BadMagic,
    Not64Bit,
    NotLittleEndian,
    NotExecutable,
    WrongMachine,
    BadProgramHeaders,
    SegmentOutOfFile,
    SegmentFileLargerThanMemory,
    SegmentAddressOverflow,
    NoLoadSegments,
    EntryNotInCode,
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooSmall => "file is too small to be ELF",
            Self::BadMagic => "missing ELF magic",
            Self::Not64Bit => "not a 64-bit ELF",
            Self::NotLittleEndian => "not little-endian",
            Self::NotExecutable => "not an executable (ET_EXEC)",
            Self::WrongMachine => "not an x86_64 binary",
            Self::BadProgramHeaders => "program header table is invalid",
            Self::SegmentOutOfFile => "a segment points outside the file",
            Self::SegmentFileLargerThanMemory => "a segment's file size exceeds its memory size",
            Self::SegmentAddressOverflow => "a segment's address range overflows",
            Self::NoLoadSegments => "no loadable segments",
            Self::EntryNotInCode => "entry point is not inside an executable segment",
        })
    }
}

/// A loadable segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub vaddr: u64,
    pub offset: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub flags: u32,
}

impl Segment {
    pub fn end(&self) -> u64 {
        self.vaddr + self.memsz
    }
    pub fn writable(&self) -> bool {
        self.flags & PF_W != 0
    }
    pub fn executable(&self) -> bool {
        self.flags & PF_X != 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Elf<'a> {
    data: &'a [u8],
    pub entry: u64,
    phoff: usize,
    phentsize: usize,
    phnum: usize,
}

fn u16_at(d: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([d[off], d[off + 1]])
}
fn u32_at(d: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(d[off..off + 4].try_into().unwrap())
}
fn u64_at(d: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(d[off..off + 8].try_into().unwrap())
}

impl<'a> Elf<'a> {
    /// Parses and fully validates an ELF64 x86_64 executable.
    pub fn parse(data: &'a [u8]) -> Result<Self, ElfError> {
        if data.len() < EHDR_SIZE {
            return Err(ElfError::TooSmall);
        }
        if data[0..4] != [0x7F, b'E', b'L', b'F'] {
            return Err(ElfError::BadMagic);
        }
        if data[4] != 2 {
            return Err(ElfError::Not64Bit);
        }
        if data[5] != 1 {
            return Err(ElfError::NotLittleEndian);
        }
        if u16_at(data, 16) != ET_EXEC {
            return Err(ElfError::NotExecutable);
        }
        if u16_at(data, 18) != EM_X86_64 {
            return Err(ElfError::WrongMachine);
        }

        let entry = u64_at(data, 24);
        let phoff = u64_at(data, 32);
        let phentsize = u16_at(data, 54) as usize;
        let phnum = u16_at(data, 56) as usize;

        if phentsize < PHDR_SIZE || phnum == 0 {
            return Err(ElfError::BadProgramHeaders);
        }
        let table_end = (phnum as u64)
            .checked_mul(phentsize as u64)
            .and_then(|size| size.checked_add(phoff))
            .ok_or(ElfError::BadProgramHeaders)?;
        if table_end > data.len() as u64 {
            return Err(ElfError::BadProgramHeaders);
        }

        let elf = Elf { data, entry, phoff: phoff as usize, phentsize, phnum };
        elf.validate()?;
        Ok(elf)
    }

    fn raw_segment(&self, index: usize) -> Option<Result<Segment, ElfError>> {
        let base = self.phoff + index * self.phentsize;
        let d = self.data;
        if u32_at(d, base) != PT_LOAD {
            return None;
        }
        let seg = Segment {
            flags: u32_at(d, base + 4),
            offset: u64_at(d, base + 8),
            vaddr: u64_at(d, base + 16),
            filesz: u64_at(d, base + 32),
            memsz: u64_at(d, base + 40),
        };
        let file_end = seg.offset.checked_add(seg.filesz);
        Some(if file_end.is_none_or(|end| end > d.len() as u64) {
            Err(ElfError::SegmentOutOfFile)
        } else if seg.filesz > seg.memsz {
            Err(ElfError::SegmentFileLargerThanMemory)
        } else if seg.vaddr.checked_add(seg.memsz).is_none() {
            Err(ElfError::SegmentAddressOverflow)
        } else {
            Ok(seg)
        })
    }

    fn validate(&self) -> Result<(), ElfError> {
        let mut any = false;
        let mut entry_ok = false;
        for i in 0..self.phnum {
            if let Some(seg) = self.raw_segment(i) {
                let seg = seg?;
                any = true;
                if seg.executable() && self.entry >= seg.vaddr && self.entry < seg.end() {
                    entry_ok = true;
                }
            }
        }
        if !any {
            return Err(ElfError::NoLoadSegments);
        }
        if !entry_ok {
            return Err(ElfError::EntryNotInCode);
        }
        Ok(())
    }

    /// The validated `PT_LOAD` segments.
    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        // validate() already proved every load segment is Ok.
        (0..self.phnum).filter_map(|i| self.raw_segment(i).and_then(Result::ok))
    }

    /// The bytes a segment copies from the file.
    pub fn segment_data(&self, seg: &Segment) -> &'a [u8] {
        &self.data[seg.offset as usize..(seg.offset + seg.filesz) as usize]
    }

    /// The page-aligned virtual range `[start, end)` covering every segment.
    pub fn load_span(&self) -> (u64, u64) {
        let mut lo = u64::MAX;
        let mut hi = 0;
        for seg in self.segments() {
            lo = lo.min(seg.vaddr);
            hi = hi.max(seg.end());
        }
        (lo & !0xFFF, (hi + 0xFFF) & !0xFFF)
    }

    /// The union of the flags of every segment that touches the 4 KiB page
    /// starting at `page`. 0 if none does.
    pub fn page_flags(&self, page: u64) -> u32 {
        let page_end = page + 0x1000;
        self.segments()
            .filter(|s| s.memsz > 0 && s.vaddr < page_end && s.end() > page)
            .fold(0, |acc, s| acc | s.flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    struct Seg {
        vaddr: u64,
        data: Vec<u8>,
        memsz: u64,
        flags: u32,
    }

    /// Builds a minimal ELF64 executable in memory.
    fn build(entry: u64, segs: &[Seg]) -> Vec<u8> {
        let phoff = EHDR_SIZE;
        let data_start = phoff + segs.len() * PHDR_SIZE;
        let mut out = vec![0u8; data_start];
        out[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        out[4] = 2;
        out[5] = 1;
        out[6] = 1;
        out[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
        out[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        out[24..32].copy_from_slice(&entry.to_le_bytes());
        out[32..40].copy_from_slice(&(phoff as u64).to_le_bytes());
        out[52..54].copy_from_slice(&(EHDR_SIZE as u16).to_le_bytes());
        out[54..56].copy_from_slice(&(PHDR_SIZE as u16).to_le_bytes());
        out[56..58].copy_from_slice(&(segs.len() as u16).to_le_bytes());
        for (i, s) in segs.iter().enumerate() {
            let offset = out.len() as u64;
            out.extend_from_slice(&s.data);
            let p = phoff + i * PHDR_SIZE;
            out[p..p + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
            out[p + 4..p + 8].copy_from_slice(&s.flags.to_le_bytes());
            out[p + 8..p + 16].copy_from_slice(&offset.to_le_bytes());
            out[p + 16..p + 24].copy_from_slice(&s.vaddr.to_le_bytes());
            out[p + 24..p + 32].copy_from_slice(&s.vaddr.to_le_bytes());
            out[p + 32..p + 40].copy_from_slice(&(s.data.len() as u64).to_le_bytes());
            out[p + 40..p + 48].copy_from_slice(&s.memsz.to_le_bytes());
            out[p + 48..p + 56].copy_from_slice(&0x1000u64.to_le_bytes());
        }
        out
    }

    const BASE: u64 = 0xFFFF_FFFF_8000_0000;

    fn kernel_like() -> Vec<u8> {
        build(
            BASE + 0x10,
            &[
                Seg { vaddr: BASE, data: vec![0x90; 0x20], memsz: 0x20, flags: PF_R | PF_X },
                Seg { vaddr: BASE + 0x1000, data: vec![1, 2, 3], memsz: 0x10, flags: PF_R },
                Seg { vaddr: BASE + 0x2000, data: vec![7; 8], memsz: 0x3000, flags: PF_R | PF_W },
            ],
        )
    }

    #[test]
    fn parses_a_kernel_like_image() {
        let file = kernel_like();
        let elf = Elf::parse(&file).unwrap();
        assert_eq!(elf.entry, BASE + 0x10);
        let segs: Vec<_> = elf.segments().collect();
        assert_eq!(segs.len(), 3);
        assert_eq!(elf.segment_data(&segs[1]), &[1, 2, 3]);
        assert_eq!(elf.load_span(), (BASE, BASE + 0x5000));
        assert_eq!(elf.page_flags(BASE), PF_R | PF_X);
        assert_eq!(elf.page_flags(BASE + 0x1000), PF_R);
        assert_eq!(elf.page_flags(BASE + 0x4000), PF_R | PF_W); // .bss tail
        assert_eq!(elf.page_flags(BASE + 0x5000), 0);
    }

    #[test]
    fn shared_page_gets_union_of_flags() {
        let file = build(
            BASE,
            &[
                Seg { vaddr: BASE, data: vec![0; 0x800], memsz: 0x800, flags: PF_R | PF_X },
                Seg { vaddr: BASE + 0x800, data: vec![0; 8], memsz: 8, flags: PF_R | PF_W },
            ],
        );
        let elf = Elf::parse(&file).unwrap();
        assert_eq!(elf.page_flags(BASE), PF_R | PF_W | PF_X);
    }

    #[test]
    fn rejects_garbage_headers() {
        assert_eq!(Elf::parse(&[0; 10]).unwrap_err(), ElfError::TooSmall);
        let mut f = kernel_like();
        f[0] = 0;
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::BadMagic);
        let mut f = kernel_like();
        f[4] = 1;
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::Not64Bit);
        let mut f = kernel_like();
        f[5] = 2;
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::NotLittleEndian);
        let mut f = kernel_like();
        f[16] = 3; // ET_DYN
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::NotExecutable);
        let mut f = kernel_like();
        f[18] = 0x28; // ARM
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::WrongMachine);
    }

    #[test]
    fn rejects_program_table_past_end_of_file() {
        let mut f = kernel_like();
        f[56..58].copy_from_slice(&500u16.to_le_bytes());
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::BadProgramHeaders);
        let mut f = kernel_like();
        f[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::BadProgramHeaders);
    }

    #[test]
    fn rejects_segment_outside_file() {
        let mut f = kernel_like();
        let p = EHDR_SIZE + PHDR_SIZE; // second segment
        f[p + 32..p + 40].copy_from_slice(&0xFFFF_FFFFu64.to_le_bytes()); // filesz
        f[p + 40..p + 48].copy_from_slice(&0xFFFF_FFFFu64.to_le_bytes()); // memsz
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::SegmentOutOfFile);
    }

    #[test]
    fn rejects_filesz_bigger_than_memsz() {
        let f = build(BASE, &[Seg { vaddr: BASE, data: vec![0; 16], memsz: 8, flags: PF_X }]);
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::SegmentFileLargerThanMemory);
    }

    #[test]
    fn rejects_address_overflow() {
        let f = build(u64::MAX - 4, &[Seg { vaddr: u64::MAX - 8, data: vec![], memsz: 64, flags: PF_X }]);
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::SegmentAddressOverflow);
    }

    #[test]
    fn rejects_entry_outside_code() {
        let f = build(
            BASE + 0x1000,
            &[
                Seg { vaddr: BASE, data: vec![0; 16], memsz: 16, flags: PF_R | PF_X },
                Seg { vaddr: BASE + 0x1000, data: vec![0; 16], memsz: 16, flags: PF_R | PF_W },
            ],
        );
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::EntryNotInCode);
    }

    #[test]
    fn rejects_no_load_segments() {
        let mut f = kernel_like();
        for i in 0..3 {
            let p = EHDR_SIZE + i * PHDR_SIZE;
            f[p..p + 4].copy_from_slice(&6u32.to_le_bytes()); // PT_PHDR
        }
        assert_eq!(Elf::parse(&f).unwrap_err(), ElfError::NoLoadSegments);
    }
}
