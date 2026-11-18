//! DHCP client messages (RFC 2131 / 2132): building DISCOVER and REQUEST, and
//! parsing OFFER, ACK and NAK. The timing and state machine live in `stack.rs`.

use crate::wire::{Ip, Mac, NO_IP};
use alloc::vec::Vec;

pub const SERVER_PORT: u16 = 67;
pub const CLIENT_PORT: u16 = 68;

const MAGIC: [u8; 4] = [99, 130, 83, 99];
const HEADER: usize = 240;

const MSG_DISCOVER: u8 = 1;
const MSG_OFFER: u8 = 2;
const MSG_REQUEST: u8 = 3;
const MSG_ACK: u8 = 5;
const MSG_NAK: u8 = 6;

/// What a server granted us.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub ip: Ip,
    pub mask: Ip,
    pub router: Option<Ip>,
    pub dns: Option<Ip>,
    pub server: Ip,
    /// Seconds (0xFFFFFFFF = forever).
    pub lease_secs: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    Offer(Lease),
    Ack(Lease),
    Nak,
}

fn base(mac: Mac, xid: u32, msg_type: u8) -> Vec<u8> {
    let mut d = alloc::vec![0u8; HEADER];
    d[0] = 1; // BOOTREQUEST
    d[1] = 1; // Ethernet
    d[2] = 6;
    d[4..8].copy_from_slice(&xid.to_be_bytes());
    d[10] = 0x80; // flags: please broadcast the reply (we have no address yet)
    d[28..34].copy_from_slice(&mac);
    d[236..240].copy_from_slice(&MAGIC);
    d.extend_from_slice(&[53, 1, msg_type]);
    d
}

fn finish(mut d: Vec<u8>) -> Vec<u8> {
    // Parameters we want: subnet mask, router, DNS server, lease time.
    d.extend_from_slice(&[55, 4, 1, 3, 6, 51]);
    d.push(255);
    d
}

pub fn build_discover(mac: Mac, xid: u32) -> Vec<u8> {
    finish(base(mac, xid, MSG_DISCOVER))
}

pub fn build_request(mac: Mac, xid: u32, ip: Ip, server: Ip) -> Vec<u8> {
    let mut d = base(mac, xid, MSG_REQUEST);
    d.extend_from_slice(&[50, 4]);
    d.extend_from_slice(&ip);
    d.extend_from_slice(&[54, 4]);
    d.extend_from_slice(&server);
    finish(d)
}

/// Parses a reply to our `xid`. Anything else (other transaction, other client, junk) is `None`.
pub fn parse(d: &[u8], xid: u32, mac: Mac) -> Option<Msg> {
    if d.len() < HEADER || d[0] != 2 || d[236..240] != MAGIC || d[4..8] != xid.to_be_bytes() || d[28..34] != mac {
        return None;
    }
    let yiaddr: Ip = d[16..20].try_into().ok()?;
    let siaddr: Ip = d[20..24].try_into().ok()?;
    let (mut kind, mut mask, mut router, mut dns, mut server, mut secs) = (0u8, None, None, None, None, 0xFFFF_FFFFu32);
    let mut i = HEADER;
    while i < d.len() {
        let code = d[i];
        i += 1;
        match code {
            0 => continue,
            255 => break,
            _ => {}
        }
        let len = *d.get(i)? as usize;
        let data = d.get(i + 1..i + 1 + len)?;
        i += 1 + len;
        let first_ip = || -> Option<Ip> { data.get(..4)?.try_into().ok() };
        match code {
            53 if len == 1 => kind = data[0],
            1 => mask = first_ip(),
            3 => router = first_ip(),
            6 => dns = first_ip(),
            54 => server = first_ip(),
            51 if len == 4 => secs = u32::from_be_bytes(data.try_into().ok()?),
            _ => {}
        }
    }
    let lease = || Lease {
        ip: yiaddr,
        mask: mask.unwrap_or([255, 255, 255, 0]),
        router,
        dns,
        server: server.unwrap_or(siaddr),
        lease_secs: secs,
    };
    match kind {
        MSG_OFFER if yiaddr != NO_IP => Some(Msg::Offer(lease())),
        MSG_ACK if yiaddr != NO_IP => Some(Msg::Ack(lease())),
        MSG_NAK => Some(Msg::Nak),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const MAC: Mac = [0x52, 0x54, 0, 0x12, 0x34, 0x56];

    /// What a server sends (the test plays the server).
    pub fn reply(kind: u8, xid: u32, mac: Mac, yiaddr: Ip) -> Vec<u8> {
        let mut d = alloc::vec![0u8; HEADER];
        d[0] = 2;
        d[1] = 1;
        d[2] = 6;
        d[4..8].copy_from_slice(&xid.to_be_bytes());
        d[16..20].copy_from_slice(&yiaddr);
        d[20..24].copy_from_slice(&[10, 0, 2, 2]);
        d[28..34].copy_from_slice(&mac);
        d[236..240].copy_from_slice(&MAGIC);
        d.extend_from_slice(&[53, 1, kind]);
        d.extend_from_slice(&[54, 4, 10, 0, 2, 2]);
        d.extend_from_slice(&[51, 4, 0, 0, 0x0e, 0x10]); // 3600 s
        d.extend_from_slice(&[1, 4, 255, 255, 255, 0]);
        d.extend_from_slice(&[0, 0]); // padding is allowed between options
        d.extend_from_slice(&[3, 4, 10, 0, 2, 2]);
        d.extend_from_slice(&[6, 8, 10, 0, 2, 3, 8, 8, 8, 8]); // two DNS servers: we keep the first
        d.push(255);
        d
    }

    #[test]
    fn discover_and_request_layout() {
        let d = build_discover(MAC, 0xdead_beef);
        assert_eq!((d[0], d[1], d[2]), (1, 1, 6));
        assert_eq!(&d[4..8], &0xdead_beefu32.to_be_bytes());
        assert_eq!(&d[28..34], &MAC);
        assert_eq!(&d[236..240], &MAGIC);
        assert_eq!(&d[240..243], &[53, 1, 1]);
        assert_eq!(*d.last().unwrap(), 255);
        let r = build_request(MAC, 1, [10, 0, 2, 15], [10, 0, 2, 2]);
        assert_eq!(&r[240..243], &[53, 1, 3]);
        assert!(r.windows(6).any(|w| w == [50, 4, 10, 0, 2, 15]), "requested address");
        assert!(r.windows(6).any(|w| w == [54, 4, 10, 0, 2, 2]), "server identifier");
    }

    #[test]
    fn parses_offer_ack_and_nak() {
        let want = Lease { ip: [10, 0, 2, 15], mask: [255, 255, 255, 0], router: Some([10, 0, 2, 2]), dns: Some([10, 0, 2, 3]), server: [10, 0, 2, 2], lease_secs: 3600 };
        assert_eq!(parse(&reply(MSG_OFFER, 9, MAC, [10, 0, 2, 15]), 9, MAC), Some(Msg::Offer(want.clone())));
        assert_eq!(parse(&reply(MSG_ACK, 9, MAC, [10, 0, 2, 15]), 9, MAC), Some(Msg::Ack(want)));
        assert_eq!(parse(&reply(MSG_NAK, 9, MAC, NO_IP), 9, MAC), Some(Msg::Nak));
    }

    #[test]
    fn ignores_other_transactions_clients_and_junk() {
        let r = reply(MSG_OFFER, 9, MAC, [10, 0, 2, 15]);
        assert_eq!(parse(&r, 10, MAC), None, "another transaction id");
        assert_eq!(parse(&r, 9, [1; 6]), None, "another client");
        assert_eq!(parse(&r[..100], 9, MAC), None);
        let mut bad = r.clone();
        bad[236] = 0; // no magic cookie
        assert_eq!(parse(&bad, 9, MAC), None);
        let mut cut = r.clone();
        cut.pop(); // the end marker
        cut.extend_from_slice(&[3, 200, 1, 2]); // an option that claims to run past the end
        assert_eq!(parse(&cut, 9, MAC), None);
        assert_eq!(parse(&reply(MSG_OFFER, 9, MAC, NO_IP), 9, MAC), None, "an offer of nothing");
    }
}
