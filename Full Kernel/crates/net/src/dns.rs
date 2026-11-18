//! A DNS client's messages (RFC 1035): one A-record question out, the first
//! IPv4 answer back. Name compression is skipped, never followed, so a hostile
//! response cannot loop us.

use crate::wire::Ip;
use alloc::vec::Vec;

pub const PORT: u16 = 53;

const TYPE_A: u16 = 1;
const CLASS_IN: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Addr(Ip),
    /// The name does not exist (rcode 3).
    NoSuchName,
    /// The server refused or failed (the rcode), or answered without an address.
    Failed(u8),
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(at)?, *d.get(at + 1)?]))
}

/// A question for the A record of `name` (recursion desired). `None` for an invalid name.
pub fn build_query(id: u16, name: &str) -> Option<Vec<u8>> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() || name.len() > 253 {
        return None;
    }
    let mut d = Vec::with_capacity(18 + name.len());
    d.extend_from_slice(&id.to_be_bytes());
    d.extend_from_slice(&0x0100u16.to_be_bytes()); // standard query, RD
    d.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // one question
    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 || !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return None;
        }
        d.push(label.len() as u8);
        d.extend_from_slice(label.as_bytes());
    }
    d.push(0);
    d.extend_from_slice(&TYPE_A.to_be_bytes());
    d.extend_from_slice(&CLASS_IN.to_be_bytes());
    Some(d)
}

/// The offset just after the (possibly compressed) name that starts at `at`.
fn skip_name(d: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let len = *d.get(at)?;
        match len {
            0 => return Some(at + 1),
            l if l & 0xC0 == 0xC0 => {
                d.get(at + 1)?;
                return Some(at + 2);
            }
            l if l & 0xC0 != 0 => return None,
            l => at += 1 + l as usize,
        }
    }
}

/// The meaning of a response to our query `id`. `None` if it is not an answer to it.
pub fn parse_response(id: u16, d: &[u8]) -> Option<Answer> {
    if d.len() < 12 || be16(d, 0)? != id {
        return None;
    }
    let flags = be16(d, 2)?;
    if flags & 0x8000 == 0 {
        return None; // a query, not a response
    }
    match flags & 15 {
        0 => {}
        3 => return Some(Answer::NoSuchName),
        rcode => return Some(Answer::Failed(rcode as u8)),
    }
    let (questions, answers) = (be16(d, 4)? as usize, be16(d, 6)? as usize);
    let mut at = 12;
    for _ in 0..questions {
        at = skip_name(d, at)? + 4;
    }
    for _ in 0..answers {
        at = skip_name(d, at)?;
        let (kind, class, rdlen) = (be16(d, at)?, be16(d, at + 2)?, be16(d, at + 8)? as usize);
        let rdata = d.get(at + 10..at + 10 + rdlen)?;
        at += 10 + rdlen;
        if kind == TYPE_A && class == CLASS_IN && rdlen == 4 {
            return Some(Answer::Addr(rdata.try_into().ok()?));
        }
    }
    Some(Answer::Failed(0))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A response to `build_query(id, "example.com")`: a CNAME first, then the A record, names compressed.
    pub fn response(id: u16, rcode: u16, addr: Option<Ip>) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&id.to_be_bytes());
        d.extend_from_slice(&(0x8180u16 | rcode).to_be_bytes());
        d.extend_from_slice(&[0, 1, 0, if addr.is_some() { 2 } else { 0 }, 0, 0, 0, 0]);
        d.extend_from_slice(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0, 0, 1, 0, 1]);
        if let Some(a) = addr {
            // CNAME (type 5) to "www.example.com", owner name = pointer to offset 12.
            d.extend_from_slice(&[0xC0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 6, 3, b'w', b'w', b'w', 0xC0, 12]);
            // A record, owner = pointer to the question name.
            d.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
            d.extend_from_slice(&a);
        }
        d
    }

    #[test]
    fn query_layout_and_invalid_names() {
        let q = build_query(0xBEEF, "example.com").unwrap();
        assert_eq!(&q[..4], &[0xBE, 0xEF, 1, 0]);
        assert_eq!(&q[12..], &[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0, 0, 1, 0, 1]);
        assert_eq!(build_query(1, "example.com."), build_query(1, "example.com"), "a trailing dot is fine");
        for bad in ["", ".", "a..b", "has space.com", "-.\u{e9}.com", &"a".repeat(64), &["abcdefgh."; 40].concat()] {
            assert_eq!(build_query(1, bad), None, "{bad:?}");
        }
        assert!(build_query(1, &"a".repeat(63)).is_some());
    }

    #[test]
    fn answers_through_a_cname_and_compression() {
        assert_eq!(parse_response(7, &response(7, 0, Some([93, 184, 216, 34]))), Some(Answer::Addr([93, 184, 216, 34])));
        assert_eq!(parse_response(7, &response(7, 3, None)), Some(Answer::NoSuchName));
        assert_eq!(parse_response(7, &response(7, 2, None)), Some(Answer::Failed(2)));
        assert_eq!(parse_response(7, &response(7, 0, None)), Some(Answer::Failed(0)), "no address in the answer");
    }

    #[test]
    fn other_ids_queries_and_junk_are_ignored() {
        let r = response(7, 0, Some([1, 2, 3, 4]));
        assert_eq!(parse_response(8, &r), None);
        let mut q = r.clone();
        q[2] &= 0x7F; // QR bit cleared: a query
        assert_eq!(parse_response(7, &q), None);
        assert_eq!(parse_response(7, &r[..11]), None);
        // Every truncation is safe, and a name that points at itself does not loop.
        for n in 0..r.len() {
            let _ = parse_response(7, &r[..n]);
        }
        let mut looped = r.clone();
        looped[29] = 0xC0; // the CNAME owner now points at offset 29, itself
        looped[30] = 29;
        let _ = parse_response(7, &looped);
        // A reserved label type is refused.
        let mut odd = r.clone();
        odd[12] = 0x80;
        assert_eq!(parse_response(7, &odd), None);
    }
}
