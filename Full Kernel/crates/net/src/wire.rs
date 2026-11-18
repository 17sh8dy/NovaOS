//! Packet formats: Ethernet II, ARP, IPv4, ICMP echo and UDP, with the
//! Internet checksum. Parsers are bounds-checked and return `None` for
//! anything malformed (wrong length, bad checksum, fragments); builders return
//! ready-to-send bytes.

use alloc::vec::Vec;

pub type Mac = [u8; 6];
pub type Ip = [u8; 4];

pub const BROADCAST_MAC: Mac = [0xFF; 6];
pub const BROADCAST_IP: Ip = [255; 4];
pub const NO_IP: Ip = [0; 4];

pub const ETH_IPV4: u16 = 0x0800;
pub const ETH_ARP: u16 = 0x0806;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

/// Smallest Ethernet frame without the FCS (shorter ones are padded).
pub const MIN_FRAME: usize = 60;
/// Largest frame we send or accept (no VLAN tag, no jumbo frames).
pub const MAX_FRAME: usize = 1514;

fn be16(d: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([d[at], d[at + 1]])
}

/// The Internet checksum (RFC 1071) over `parts` joined together. Only the last part may have an odd length.
pub fn checksum(parts: &[&[u8]]) -> u16 {
    let mut sum: u32 = 0;
    for p in parts {
        let mut chunks = p.chunks_exact(2);
        for c in &mut chunks {
            sum += u16::from_be_bytes([c[0], c[1]]) as u32;
        }
        if let [last] = chunks.remainder() {
            sum += (*last as u32) << 8;
        }
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Parses "a.b.c.d".
pub fn parse_ip(s: &str) -> Option<Ip> {
    let mut ip = [0u8; 4];
    let mut parts = s.split('.');
    for b in &mut ip {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *b = p.parse().ok()?;
    }
    parts.next().is_none().then_some(ip)
}

pub fn ip_text(ip: Ip) -> alloc::string::String {
    alloc::format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

pub fn mac_text(m: Mac) -> alloc::string::String {
    alloc::format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
}

/// Are `a` and `b` on the same network under `mask`?
pub fn same_subnet(a: Ip, b: Ip, mask: Ip) -> bool {
    (0..4).all(|i| a[i] & mask[i] == b[i] & mask[i])
}

// ---- Ethernet ---------------------------------------------------------------------------------

pub struct Eth<'a> {
    pub dst: Mac,
    pub src: Mac,
    pub ethertype: u16,
    pub payload: &'a [u8],
}

pub fn parse_eth(frame: &[u8]) -> Option<Eth<'_>> {
    if frame.len() < 14 || frame.len() > MAX_FRAME + 4 {
        return None;
    }
    Some(Eth { dst: frame[0..6].try_into().ok()?, src: frame[6..12].try_into().ok()?, ethertype: be16(frame, 12), payload: &frame[14..] })
}

pub fn build_eth(dst: Mac, src: Mac, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(14 + payload.len().max(MIN_FRAME - 14));
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    f.extend_from_slice(&ethertype.to_be_bytes());
    f.extend_from_slice(payload);
    if f.len() < MIN_FRAME {
        f.resize(MIN_FRAME, 0);
    }
    f
}

// ---- ARP --------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arp {
    /// 1 = request, 2 = reply.
    pub op: u16,
    pub sha: Mac,
    pub spa: Ip,
    pub tha: Mac,
    pub tpa: Ip,
}

pub fn parse_arp(d: &[u8]) -> Option<Arp> {
    // Ethernet (1) / IPv4 (0x0800), hardware size 6, protocol size 4.
    if d.len() < 28 || be16(d, 0) != 1 || be16(d, 2) != ETH_IPV4 || d[4] != 6 || d[5] != 4 {
        return None;
    }
    let op = be16(d, 6);
    if op != 1 && op != 2 {
        return None;
    }
    Some(Arp { op, sha: d[8..14].try_into().ok()?, spa: d[14..18].try_into().ok()?, tha: d[18..24].try_into().ok()?, tpa: d[24..28].try_into().ok()? })
}

pub fn build_arp(a: &Arp) -> Vec<u8> {
    let mut d = Vec::with_capacity(28);
    d.extend_from_slice(&1u16.to_be_bytes());
    d.extend_from_slice(&ETH_IPV4.to_be_bytes());
    d.extend_from_slice(&[6, 4]);
    d.extend_from_slice(&a.op.to_be_bytes());
    d.extend_from_slice(&a.sha);
    d.extend_from_slice(&a.spa);
    d.extend_from_slice(&a.tha);
    d.extend_from_slice(&a.tpa);
    d
}

// ---- IPv4 -------------------------------------------------------------------------------------

pub struct Ipv4<'a> {
    pub src: Ip,
    pub dst: Ip,
    pub proto: u8,
    pub ttl: u8,
    pub payload: &'a [u8],
}

/// Parses an IPv4 packet. Fragments, options-only garbage and bad checksums give `None`
/// (padding after the packet's own length, which Ethernet adds, is cut off).
pub fn parse_ipv4(d: &[u8]) -> Option<Ipv4<'_>> {
    if d.len() < 20 || d[0] >> 4 != 4 {
        return None;
    }
    let ihl = (d[0] & 15) as usize * 4;
    let total = be16(d, 2) as usize;
    if ihl < 20 || total < ihl || total > d.len() {
        return None;
    }
    if checksum(&[&d[..ihl]]) != 0 {
        return None;
    }
    let frag = be16(d, 6);
    if frag & 0x3FFF != 0 {
        return None; // more-fragments flag or a fragment offset: reassembly is not supported
    }
    Some(Ipv4 { src: d[12..16].try_into().ok()?, dst: d[16..20].try_into().ok()?, proto: d[9], ttl: d[8], payload: &d[ihl..total] })
}

pub fn build_ipv4(src: Ip, dst: Ip, proto: u8, id: u16, ttl: u8, payload: &[u8]) -> Vec<u8> {
    let total = 20 + payload.len();
    let mut h = Vec::with_capacity(total);
    h.extend_from_slice(&[0x45, 0]);
    h.extend_from_slice(&(total as u16).to_be_bytes());
    h.extend_from_slice(&id.to_be_bytes());
    h.extend_from_slice(&0x4000u16.to_be_bytes()); // don't fragment
    h.extend_from_slice(&[ttl, proto, 0, 0]);
    h.extend_from_slice(&src);
    h.extend_from_slice(&dst);
    let c = checksum(&[&h]);
    h[10..12].copy_from_slice(&c.to_be_bytes());
    h.extend_from_slice(payload);
    h
}

// ---- ICMP -------------------------------------------------------------------------------------

pub const ICMP_ECHO_REPLY: u8 = 0;
pub const ICMP_ECHO_REQUEST: u8 = 8;

pub struct Icmp<'a> {
    pub kind: u8,
    pub code: u8,
    pub id: u16,
    pub seq: u16,
    pub payload: &'a [u8],
}

pub fn parse_icmp(d: &[u8]) -> Option<Icmp<'_>> {
    if d.len() < 8 || checksum(&[d]) != 0 {
        return None;
    }
    Some(Icmp { kind: d[0], code: d[1], id: be16(d, 4), seq: be16(d, 6), payload: &d[8..] })
}

pub fn build_icmp_echo(kind: u8, id: u16, seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut d = Vec::with_capacity(8 + payload.len());
    d.extend_from_slice(&[kind, 0, 0, 0]);
    d.extend_from_slice(&id.to_be_bytes());
    d.extend_from_slice(&seq.to_be_bytes());
    d.extend_from_slice(payload);
    let c = checksum(&[&d]);
    d[2..4].copy_from_slice(&c.to_be_bytes());
    d
}

// ---- UDP --------------------------------------------------------------------------------------

pub struct Udp<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: &'a [u8],
}

fn pseudo(src: Ip, dst: Ip, proto: u8, len: usize) -> [u8; 12] {
    let mut p = [0u8; 12];
    p[0..4].copy_from_slice(&src);
    p[4..8].copy_from_slice(&dst);
    p[9] = proto;
    p[10..12].copy_from_slice(&(len as u16).to_be_bytes());
    p
}

/// Parses a UDP datagram carried from `src` to `dst` (a checksum of 0 means "none" and is accepted).
pub fn parse_udp(src: Ip, dst: Ip, d: &[u8]) -> Option<Udp<'_>> {
    if d.len() < 8 {
        return None;
    }
    let len = be16(d, 4) as usize;
    if len < 8 || len > d.len() {
        return None;
    }
    let d = &d[..len];
    if be16(d, 6) != 0 && checksum(&[&pseudo(src, dst, PROTO_UDP, len), d]) != 0 {
        return None;
    }
    Some(Udp { src_port: be16(d, 0), dst_port: be16(d, 2), payload: &d[8..] })
}

pub fn build_udp(src: Ip, dst: Ip, src_port: u16, dst_port: u16, payload: &[u8]) -> Vec<u8> {
    let len = 8 + payload.len();
    let mut d = Vec::with_capacity(len);
    d.extend_from_slice(&src_port.to_be_bytes());
    d.extend_from_slice(&dst_port.to_be_bytes());
    d.extend_from_slice(&(len as u16).to_be_bytes());
    d.extend_from_slice(&[0, 0]);
    d.extend_from_slice(payload);
    let mut c = checksum(&[&pseudo(src, dst, PROTO_UDP, len), &d]);
    if c == 0 {
        c = 0xFFFF;
    }
    d[6..8].copy_from_slice(&c.to_be_bytes());
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_matches_the_rfc_example() {
        // RFC 1071 section 3: 00 01 f2 03 f4 f5 f6 f7 sums to ddf2 (so the checksum is 220d).
        assert_eq!(checksum(&[&[0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7]]), 0x220d);
        // Odd length and split across parts give the same answer as one slice.
        assert_eq!(checksum(&[&[1, 2, 3]]), checksum(&[&[1, 2], &[3]]));
        // A header with its own checksum included sums to zero.
        let h = build_ipv4([10, 0, 0, 1], [10, 0, 0, 2], PROTO_UDP, 7, 64, b"");
        assert_eq!(checksum(&[&h]), 0);
        // The classic worked example: 45 00 00 73 00 00 40 00 40 11 b8 61 c0 a8 00 01 c0 a8 00 c7.
        let real = [0x45, 0, 0, 0x73, 0, 0, 0x40, 0, 0x40, 0x11, 0xb8, 0x61, 0xc0, 0xa8, 0, 1, 0xc0, 0xa8, 0, 0xc7];
        assert_eq!(checksum(&[&real]), 0);
    }

    #[test]
    fn ip_text_round_trip() {
        assert_eq!(parse_ip("10.0.2.15"), Some([10, 0, 2, 15]));
        for bad in ["", "1.2.3", "1.2.3.4.5", "256.1.1.1", "a.b.c.d", "1..2.3", "1.2.3.-4", "1.2.3.4 "] {
            assert_eq!(parse_ip(bad), None, "{bad:?}");
        }
        assert_eq!(ip_text([192, 168, 1, 9]), "192.168.1.9");
        assert_eq!(mac_text([0x52, 0x54, 0, 0x12, 0x34, 0x56]), "52:54:00:12:34:56");
        assert!(same_subnet([10, 0, 2, 15], [10, 0, 2, 2], [255, 255, 255, 0]));
        assert!(!same_subnet([10, 0, 2, 15], [10, 0, 3, 2], [255, 255, 255, 0]));
    }

    #[test]
    fn ethernet_pads_short_frames() {
        let f = build_eth(BROADCAST_MAC, [1, 2, 3, 4, 5, 6], ETH_ARP, &[9; 28]);
        assert_eq!(f.len(), MIN_FRAME);
        let e = parse_eth(&f).unwrap();
        assert_eq!((e.dst, e.src, e.ethertype, e.payload.len()), (BROADCAST_MAC, [1, 2, 3, 4, 5, 6], ETH_ARP, 46));
        assert!(parse_eth(&[0; 13]).is_none());
    }

    #[test]
    fn arp_round_trip_and_rejects() {
        let a = Arp { op: 1, sha: [1; 6], spa: [10, 0, 2, 15], tha: [0; 6], tpa: [10, 0, 2, 2] };
        let b = build_arp(&a);
        assert_eq!(b.len(), 28);
        assert_eq!(parse_arp(&b), Some(a));
        let mut bad = b.clone();
        bad[7] = 9; // an opcode that is neither request nor reply
        assert_eq!(parse_arp(&bad), None);
        assert_eq!(parse_arp(&b[..27]), None);
    }

    #[test]
    fn ipv4_round_trip_padding_and_rejects() {
        let p = build_ipv4([10, 0, 2, 15], [10, 0, 2, 2], PROTO_ICMP, 5, 64, b"abc");
        let mut padded = p.clone();
        padded.extend_from_slice(&[0; 20]); // Ethernet padding
        let ip = parse_ipv4(&padded).unwrap();
        assert_eq!((ip.src, ip.dst, ip.proto, ip.ttl, ip.payload), ([10, 0, 2, 15], [10, 0, 2, 2], PROTO_ICMP, 64, &b"abc"[..]));
        let mut bad = p.clone();
        bad[8] ^= 1; // flips the TTL: the header checksum no longer matches
        assert!(parse_ipv4(&bad).is_none());
        let mut frag = p.clone();
        frag[6] = 0x20; // more fragments
        let c = {
            frag[10] = 0;
            frag[11] = 0;
            checksum(&[&frag[..20]])
        };
        frag[10..12].copy_from_slice(&c.to_be_bytes());
        assert!(parse_ipv4(&frag).is_none(), "fragments are dropped");
        assert!(parse_ipv4(&p[..19]).is_none());
        let mut short = p.clone();
        short[2..4].copy_from_slice(&200u16.to_be_bytes()); // claims more than there is
        assert!(parse_ipv4(&short).is_none());
    }

    #[test]
    fn icmp_and_udp_round_trip() {
        let e = build_icmp_echo(ICMP_ECHO_REQUEST, 0x1234, 7, b"hello");
        let i = parse_icmp(&e).unwrap();
        assert_eq!((i.kind, i.id, i.seq, i.payload), (ICMP_ECHO_REQUEST, 0x1234, 7, &b"hello"[..]));
        let mut bad = e.clone();
        bad[9] ^= 0xFF;
        assert!(parse_icmp(&bad).is_none());

        let (a, b) = ([10, 0, 2, 15], [10, 0, 2, 3]);
        let u = build_udp(a, b, 5000, 53, b"query");
        let p = parse_udp(a, b, &u).unwrap();
        assert_eq!((p.src_port, p.dst_port, p.payload), (5000, 53, &b"query"[..]));
        assert!(parse_udp(a, [10, 0, 2, 4], &u).is_none(), "the pseudo header covers the addresses");
        let mut none = u.clone();
        none[6] = 0;
        none[7] = 0;
        assert!(parse_udp(a, b, &none).is_some(), "a zero checksum means none");
    }
}
