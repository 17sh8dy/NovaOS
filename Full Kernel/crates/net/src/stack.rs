//! The network stack as a state machine: frames in, frames out, a clock.
//!
//! It never touches hardware. The kernel feeds it every received frame
//! ([`Stack::handle_frame`]), calls [`Stack::poll`] regularly with the time in
//! milliseconds, sends whatever frames come back, and reads [`Event`]s.
//!
//! * **DHCP client:** discover, offer, request, ack; retransmits; renews by starting again at 75% of the lease.
//! * **ARP:** answers requests for our address, learns mappings, queues packets until the next hop's
//!   address is known (3 requests, then gives up).
//! * **ICMP:** answers echo requests (so the PC can be pinged) and sends them ([`Stack::ping`]).
//! * **DNS:** asks the DHCP-provided server for A records ([`Stack::resolve`]) with retries.
//! * **UDP/TCP:** received UDP is handed to the clients above; TCP is the next step.

use crate::dhcp::{self, Lease, Msg};
use crate::dns::{self, Answer};
use crate::wire::*;
use alloc::string::String;
use alloc::vec::Vec;

/// Something the kernel should know or show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// DHCP finished: this is our address now.
    Leased(Lease),
    /// DHCP got no answer (it tries again later).
    LeaseFailed,
    PingReply { from: Ip, seq: u16, ms: u64, ttl: u8, bytes: usize },
    /// A name lookup finished: the address, or why not.
    Resolved { name: String, result: Result<Ip, &'static str> },
    /// A packet waited for ARP and nobody answered.
    Unreachable { ip: Ip },
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Stats {
    pub frames_in: u64,
    pub frames_out: u64,
    pub dropped: u64,
}

const PING_ID: u16 = 0x4E56;
const ARP_TTL_MS: u64 = 120_000;
const ARP_RETRY_MS: u64 = 1_000;
const ARP_TRIES: u8 = 3;
const DNS_RETRY_MS: u64 = 1_500;
const DNS_TRIES: u8 = 3;
const DHCP_TRIES: u8 = 5;
const DHCP_RETRY_AFTER_FAIL_MS: u64 = 30_000;

enum Dhcp {
    Idle,
    Discovering { xid: u32, since: u64, tries: u8 },
    Requesting { xid: u32, offer: Lease, since: u64, tries: u8 },
    Bound { since: u64 },
    Failed { since: u64 },
}

struct ArpEntry {
    ip: Ip,
    mac: Mac,
    learned: u64,
}

/// An IP packet waiting for its next hop's hardware address.
struct Pending {
    hop: Ip,
    dst: Ip,
    proto: u8,
    payload: Vec<u8>,
    last_request: u64,
    tries: u8,
}

struct Query {
    name: String,
    id: u16,
    port: u16,
    server: Ip,
    packet: Vec<u8>,
    sent: u64,
    tries: u8,
}

pub struct Stack {
    mac: Mac,
    lease: Option<Lease>,
    dhcp: Dhcp,
    arp: Vec<ArpEntry>,
    pending: Vec<Pending>,
    pings: Vec<(u16, u64)>,
    queries: Vec<Query>,
    events: Vec<Event>,
    ip_id: u16,
    rng: u32,
    next_port: u16,
    pub stats: Stats,
}

impl Stack {
    pub fn new(mac: Mac, seed: u32) -> Self {
        Self {
            mac,
            lease: None,
            dhcp: Dhcp::Idle,
            arp: Vec::new(),
            pending: Vec::new(),
            pings: Vec::new(),
            queries: Vec::new(),
            events: Vec::new(),
            ip_id: 1,
            rng: seed | 1,
            next_port: 49152,
            stats: Stats::default(),
        }
    }

    pub fn mac(&self) -> Mac {
        self.mac
    }

    pub fn lease(&self) -> Option<&Lease> {
        self.lease.as_ref()
    }

    /// Uses this configuration without DHCP (tests, static setups).
    pub fn configure(&mut self, lease: Lease) {
        self.lease = Some(lease);
        self.dhcp = Dhcp::Bound { since: 0 };
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.events)
    }

    fn random(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    // ---- Sending ----

    fn eth_ip(&mut self, dst_mac: Mac, src: Ip, dst: Ip, proto: u8, payload: &[u8]) -> Vec<u8> {
        let id = self.ip_id;
        self.ip_id = self.ip_id.wrapping_add(1);
        build_eth(dst_mac, self.mac, ETH_IPV4, &build_ipv4(src, dst, proto, id, 64, payload))
    }

    fn arp_lookup(&self, ip: Ip, now: u64) -> Option<Mac> {
        self.arp.iter().find(|e| e.ip == ip && now.saturating_sub(e.learned) < ARP_TTL_MS).map(|e| e.mac)
    }

    fn arp_request(&self, ip: Ip) -> Vec<u8> {
        let src = self.lease.as_ref().map_or(NO_IP, |l| l.ip);
        build_eth(BROADCAST_MAC, self.mac, ETH_ARP, &build_arp(&Arp { op: 1, sha: self.mac, spa: src, tha: [0; 6], tpa: ip }))
    }

    /// Sends an IP packet to `dst`: straight to the broadcast address, or via ARP to the
    /// destination (same network) or the router. Queues it if the hop is not known yet.
    pub fn send_ip(&mut self, dst: Ip, proto: u8, payload: Vec<u8>, now: u64, out: &mut Vec<Vec<u8>>) {
        let Some(lease) = self.lease.clone() else {
            self.stats.dropped += 1;
            return;
        };
        if dst == BROADCAST_IP {
            out.push(self.eth_ip(BROADCAST_MAC, lease.ip, dst, proto, &payload));
            return;
        }
        let hop = if same_subnet(dst, lease.ip, lease.mask) {
            dst
        } else if let Some(r) = lease.router {
            r
        } else {
            self.stats.dropped += 1;
            self.events.push(Event::Unreachable { ip: dst });
            return;
        };
        match self.arp_lookup(hop, now) {
            Some(mac) => out.push(self.eth_ip(mac, lease.ip, dst, proto, &payload)),
            None => {
                out.push(self.arp_request(hop));
                self.pending.push(Pending { hop, dst, proto, payload, last_request: now, tries: 1 });
            }
        }
    }

    fn send_udp(&mut self, dst: Ip, src_port: u16, dst_port: u16, payload: &[u8], now: u64, out: &mut Vec<Vec<u8>>) {
        let Some(src) = self.lease.as_ref().map(|l| l.ip) else { return };
        let udp = build_udp(src, dst, src_port, dst_port, payload);
        self.send_ip(dst, PROTO_UDP, udp, now, out);
    }

    /// DHCP goes out before we have an address: from 0.0.0.0 to the broadcast address.
    fn send_dhcp(&mut self, payload: &[u8]) -> Vec<u8> {
        let udp = build_udp(NO_IP, BROADCAST_IP, dhcp::CLIENT_PORT, dhcp::SERVER_PORT, payload);
        self.eth_ip(BROADCAST_MAC, NO_IP, BROADCAST_IP, PROTO_UDP, &udp)
    }

    // ---- Requests from the kernel ----

    /// Starts (or restarts) DHCP.
    pub fn start_dhcp(&mut self, now: u64) -> Vec<Vec<u8>> {
        let xid = self.random();
        self.dhcp = Dhcp::Discovering { xid, since: now, tries: 1 };
        let frame = self.send_dhcp(&dhcp::build_discover(self.mac, xid));
        alloc::vec![frame]
    }

    /// Sends an echo request.
    pub fn ping(&mut self, dst: Ip, seq: u16, now: u64) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let payload: [u8; 32] = core::array::from_fn(|i| b'a' + (i % 23) as u8);
        self.pings.push((seq, now));
        self.send_ip(dst, PROTO_ICMP, build_icmp_echo(ICMP_ECHO_REQUEST, PING_ID, seq, &payload), now, &mut out);
        out
    }

    /// Looks up `name`: a dotted address resolves to itself, anything else asks the DNS server.
    pub fn resolve(&mut self, name: &str, now: u64) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        if let Some(ip) = parse_ip(name) {
            self.events.push(Event::Resolved { name: String::from(name), result: Ok(ip) });
            return out;
        }
        let fail = |s: &mut Self, why: &'static str| s.events.push(Event::Resolved { name: String::from(name), result: Err(why) });
        let Some(server) = self.lease.as_ref().and_then(|l| l.dns) else {
            fail(self, "no DNS server (no network?)");
            return out;
        };
        let id = self.random() as u16;
        let Some(packet) = dns::build_query(id, name) else {
            fail(self, "not a valid name");
            return out;
        };
        let port = self.next_port;
        self.next_port = if self.next_port >= 65000 { 49152 } else { self.next_port + 1 };
        self.send_udp(server, port, dns::PORT, &packet, now, &mut out);
        self.queries.push(Query { name: String::from(name), id, port, server, packet, sent: now, tries: 1 });
        out
    }

    // ---- Receiving ----

    /// One received frame. Returns the frames to send in answer.
    pub fn handle_frame(&mut self, frame: &[u8], now: u64) -> Vec<Vec<u8>> {
        self.stats.frames_in += 1;
        let mut out = Vec::new();
        let Some(eth) = parse_eth(frame) else {
            self.stats.dropped += 1;
            return out;
        };
        if eth.dst != self.mac && eth.dst != BROADCAST_MAC {
            return out;
        }
        match eth.ethertype {
            ETH_ARP => self.on_arp(eth.payload, now, &mut out),
            ETH_IPV4 => self.on_ipv4(eth.payload, now, &mut out),
            _ => {}
        }
        out
    }

    fn on_arp(&mut self, payload: &[u8], now: u64, out: &mut Vec<Vec<u8>>) {
        let Some(a) = parse_arp(payload) else {
            self.stats.dropped += 1;
            return;
        };
        let Some(my_ip) = self.lease.as_ref().map(|l| l.ip) else { return };
        if a.spa == NO_IP || a.spa == BROADCAST_IP {
            return;
        }
        // Learn (or refresh) the sender, but only for hosts we may talk to.
        match self.arp.iter_mut().find(|e| e.ip == a.spa) {
            Some(e) => {
                e.mac = a.sha;
                e.learned = now;
            }
            None if a.tpa == my_ip || a.op == 2 => self.arp.push(ArpEntry { ip: a.spa, mac: a.sha, learned: now }),
            None => {}
        }
        if a.op == 1 && a.tpa == my_ip {
            let reply = Arp { op: 2, sha: self.mac, spa: my_ip, tha: a.sha, tpa: a.spa };
            out.push(build_eth(a.sha, self.mac, ETH_ARP, &build_arp(&reply)));
        }
        // Anything that was waiting for this address can go now.
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].hop == a.spa {
                let p = self.pending.remove(i);
                out.push(self.eth_ip(a.sha, my_ip, p.dst, p.proto, &p.payload));
            } else {
                i += 1;
            }
        }
    }

    fn on_ipv4(&mut self, payload: &[u8], now: u64, out: &mut Vec<Vec<u8>>) {
        let Some(ip) = parse_ipv4(payload) else {
            self.stats.dropped += 1;
            return;
        };
        let mine = self.lease.as_ref().map(|l| (l.ip, l.mask));
        // For us: our address, a broadcast, or (while DHCP runs) anything: the offer is addressed to our new IP.
        let for_us = match mine {
            Some((my, mask)) => ip.dst == my || ip.dst == BROADCAST_IP || (0..4).all(|i| ip.dst[i] == (my[i] & mask[i]) | !mask[i]),
            None => true,
        };
        if !for_us {
            return;
        }
        match ip.proto {
            PROTO_ICMP => self.on_icmp(&ip, now, out),
            PROTO_UDP => self.on_udp(&ip, now, out),
            _ => {}
        }
    }

    fn on_icmp(&mut self, ip: &Ipv4<'_>, now: u64, out: &mut Vec<Vec<u8>>) {
        let Some(icmp) = parse_icmp(ip.payload) else { return };
        match icmp.kind {
            ICMP_ECHO_REQUEST if self.lease.is_some() => {
                let reply = build_icmp_echo(ICMP_ECHO_REPLY, icmp.id, icmp.seq, icmp.payload);
                self.send_ip(ip.src, PROTO_ICMP, reply, now, out);
            }
            ICMP_ECHO_REPLY if icmp.id == PING_ID => {
                if let Some(i) = self.pings.iter().position(|p| p.0 == icmp.seq) {
                    let (_, sent) = self.pings.remove(i);
                    self.events.push(Event::PingReply { from: ip.src, seq: icmp.seq, ms: now.saturating_sub(sent), ttl: ip.ttl, bytes: icmp.payload.len() + 8 });
                }
            }
            _ => {}
        }
    }

    fn on_udp(&mut self, ip: &Ipv4<'_>, now: u64, out: &mut Vec<Vec<u8>>) {
        let Some(udp) = parse_udp(ip.src, ip.dst, ip.payload) else { return };
        if udp.dst_port == dhcp::CLIENT_PORT && udp.src_port == dhcp::SERVER_PORT {
            self.on_dhcp(udp.payload, now, out);
        } else if udp.src_port == dns::PORT
            && let Some(i) = self.queries.iter().position(|q| q.port == udp.dst_port && q.server == ip.src)
            && let Some(answer) = dns::parse_response(self.queries[i].id, udp.payload)
        {
            let q = self.queries.remove(i);
            let result = match answer {
                Answer::Addr(a) => Ok(a),
                Answer::NoSuchName => Err("no such name"),
                Answer::Failed(_) => Err("the DNS server could not answer"),
            };
            self.events.push(Event::Resolved { name: q.name, result });
        }
    }

    fn on_dhcp(&mut self, payload: &[u8], now: u64, out: &mut Vec<Vec<u8>>) {
        let mac = self.mac;
        match core::mem::replace(&mut self.dhcp, Dhcp::Idle) {
            Dhcp::Discovering { xid, since, tries } => match dhcp::parse(payload, xid, mac) {
                Some(Msg::Offer(offer)) => {
                    out.push(self.send_dhcp(&dhcp::build_request(mac, xid, offer.ip, offer.server)));
                    self.dhcp = Dhcp::Requesting { xid, offer, since: now, tries: 1 };
                }
                _ => self.dhcp = Dhcp::Discovering { xid, since, tries },
            },
            Dhcp::Requesting { xid, offer, since, tries } => match dhcp::parse(payload, xid, mac) {
                Some(Msg::Ack(lease)) => {
                    self.arp.clear();
                    self.lease = Some(lease.clone());
                    self.dhcp = Dhcp::Bound { since: now };
                    self.events.push(Event::Leased(lease));
                }
                Some(Msg::Nak) => {
                    out.extend(self.start_dhcp(now));
                }
                _ => self.dhcp = Dhcp::Requesting { xid, offer, since, tries },
            },
            other => self.dhcp = other,
        }
    }

    // ---- Timers ----

    /// Retransmissions and timeouts. Call every few milliseconds.
    pub fn poll(&mut self, now: u64) -> Vec<Vec<u8>> {
        let mut out = Vec::new();

        match core::mem::replace(&mut self.dhcp, Dhcp::Idle) {
            Dhcp::Discovering { xid, since, tries } if now.saturating_sub(since) >= 2_000 * tries as u64 => {
                if tries >= DHCP_TRIES {
                    self.events.push(Event::LeaseFailed);
                    self.dhcp = Dhcp::Failed { since: now };
                } else {
                    out.push(self.send_dhcp(&dhcp::build_discover(self.mac, xid)));
                    self.dhcp = Dhcp::Discovering { xid, since: now, tries: tries + 1 };
                }
            }
            Dhcp::Requesting { xid, offer, since, tries } if now.saturating_sub(since) >= 2_000 => {
                if tries >= 3 {
                    out.extend(self.start_dhcp(now));
                } else {
                    out.push(self.send_dhcp(&dhcp::build_request(self.mac, xid, offer.ip, offer.server)));
                    self.dhcp = Dhcp::Requesting { xid, offer, since: now, tries: tries + 1 };
                }
            }
            Dhcp::Bound { since } if self.lease.as_ref().is_some_and(|l| l.lease_secs != u32::MAX && now.saturating_sub(since) >= l.lease_secs as u64 * 750) => {
                out.extend(self.start_dhcp(now));
            }
            Dhcp::Failed { since } if now.saturating_sub(since) >= DHCP_RETRY_AFTER_FAIL_MS => {
                out.extend(self.start_dhcp(now));
            }
            other => self.dhcp = other,
        }

        // Packets still waiting for ARP.
        let mut i = 0;
        while i < self.pending.len() {
            if now.saturating_sub(self.pending[i].last_request) < ARP_RETRY_MS {
                i += 1;
            } else if self.pending[i].tries >= ARP_TRIES {
                let p = self.pending.remove(i);
                self.stats.dropped += 1;
                self.events.push(Event::Unreachable { ip: p.dst });
            } else {
                self.pending[i].tries += 1;
                self.pending[i].last_request = now;
                out.push(self.arp_request(self.pending[i].hop));
                i += 1;
            }
        }

        // DNS retries and timeouts.
        let mut i = 0;
        while i < self.queries.len() {
            if now.saturating_sub(self.queries[i].sent) < DNS_RETRY_MS {
                i += 1;
            } else if self.queries[i].tries >= DNS_TRIES {
                let q = self.queries.remove(i);
                self.events.push(Event::Resolved { name: q.name, result: Err("the DNS server did not answer") });
            } else {
                self.queries[i].tries += 1;
                self.queries[i].sent = now;
                let (server, port, packet) = (self.queries[i].server, self.queries[i].port, self.queries[i].packet.clone());
                self.send_udp(server, port, dns::PORT, &packet, now, &mut out);
                i += 1;
            }
        }

        self.pings.retain(|p| now.saturating_sub(p.1) < 10_000);
        self.stats.frames_out += out.len() as u64;
        out
    }

    /// Counts frames the kernel sends in answer to `handle_frame` / requests (for the statistics).
    pub fn count_sent(&mut self, n: usize) {
        self.stats.frames_out += n as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dhcp::tests::reply;

    const A_MAC: Mac = [0x52, 0x54, 0, 0, 0, 0xA1];
    const B_MAC: Mac = [0x52, 0x54, 0, 0, 0, 0xB2];

    fn lease(ip: Ip) -> Lease {
        Lease { ip, mask: [255, 255, 255, 0], router: Some([10, 0, 2, 2]), dns: Some([10, 0, 2, 3]), server: [10, 0, 2, 2], lease_secs: 3600 }
    }

    /// Delivers frames between two stacks until nothing more is sent. Returns how many frames crossed.
    fn wire(a: &mut Stack, b: &mut Stack, mut from_a: Vec<Vec<u8>>, now: u64) -> usize {
        let mut from_b: Vec<Vec<u8>> = Vec::new();
        let mut crossed = 0;
        for _ in 0..10 {
            if from_a.is_empty() && from_b.is_empty() {
                break;
            }
            let mut next_b = Vec::new();
            for f in from_a.drain(..) {
                crossed += 1;
                next_b.extend(b.handle_frame(&f, now));
            }
            let mut next_a = Vec::new();
            for f in from_b.drain(..) {
                crossed += 1;
                next_a.extend(a.handle_frame(&f, now));
            }
            from_a = next_a;
            from_b = next_b;
        }
        crossed
    }

    #[test]
    fn ping_between_two_stacks_through_arp() {
        let (mut a, mut b) = (Stack::new(A_MAC, 1), Stack::new(B_MAC, 2));
        a.configure(lease([10, 0, 2, 15]));
        b.configure(lease([10, 0, 2, 16]));
        let first = a.ping([10, 0, 2, 16], 1, 1000);
        assert_eq!(first.len(), 1, "an ARP request goes out first");
        assert_eq!(parse_eth(&first[0]).unwrap().ethertype, ETH_ARP);
        // ARP request, ARP reply, echo request, echo reply.
        assert_eq!(wire(&mut a, &mut b, first, 1004), 4);
        assert_eq!(a.take_events(), alloc::vec![Event::PingReply { from: [10, 0, 2, 16], seq: 1, ms: 4, ttl: 64, bytes: 40 }]);
        // The second ping needs no ARP.
        let second = a.ping([10, 0, 2, 16], 2, 2000);
        assert_eq!(parse_eth(&second[0]).unwrap().ethertype, ETH_IPV4);
        assert_eq!(wire(&mut a, &mut b, second, 2001), 2);
        assert_eq!(a.take_events().len(), 1);
    }

    #[test]
    fn off_link_traffic_goes_to_the_router() {
        let mut a = Stack::new(A_MAC, 1);
        a.configure(lease([10, 0, 2, 15]));
        let f = a.ping([8, 8, 8, 8], 1, 0);
        let arp = parse_arp(parse_eth(&f[0]).unwrap().payload).unwrap();
        assert_eq!(arp.tpa, [10, 0, 2, 2], "we ask for the router, not for 8.8.8.8");
        // The router answers; the queued echo goes to the router's MAC but 8.8.8.8's IP.
        let reply = build_eth(A_MAC, B_MAC, ETH_ARP, &build_arp(&Arp { op: 2, sha: B_MAC, spa: [10, 0, 2, 2], tha: A_MAC, tpa: [10, 0, 2, 15] }));
        let out = a.handle_frame(&reply, 5);
        let e = parse_eth(&out[0]).unwrap();
        assert_eq!(e.dst, B_MAC);
        assert_eq!(parse_ipv4(e.payload).unwrap().dst, [8, 8, 8, 8]);
    }

    #[test]
    fn unanswered_arp_gives_up_with_an_event() {
        let mut a = Stack::new(A_MAC, 1);
        a.configure(lease([10, 0, 2, 15]));
        a.ping([10, 0, 2, 99], 1, 0);
        assert_eq!(a.poll(500).len(), 0);
        assert_eq!(a.poll(1000).len(), 1, "second request");
        assert_eq!(a.poll(2000).len(), 1, "third request");
        assert!(a.take_events().is_empty());
        assert_eq!(a.poll(3000).len(), 0);
        assert_eq!(a.take_events(), alloc::vec![Event::Unreachable { ip: [10, 0, 2, 99] }]);
    }

    #[test]
    fn no_router_means_off_link_is_unreachable() {
        let mut a = Stack::new(A_MAC, 1);
        let mut l = lease([10, 0, 2, 15]);
        l.router = None;
        a.configure(l);
        assert!(a.ping([1, 1, 1, 1], 1, 0).is_empty());
        assert_eq!(a.take_events(), alloc::vec![Event::Unreachable { ip: [1, 1, 1, 1] }]);
        let mut unconfigured = Stack::new(A_MAC, 1);
        assert!(unconfigured.ping([10, 0, 2, 2], 1, 0).is_empty());
    }

    #[test]
    fn dhcp_discover_offer_request_ack() {
        let mut s = Stack::new(A_MAC, 42);
        let discover = s.start_dhcp(0);
        // From 0.0.0.0 to 255.255.255.255, UDP 68 -> 67, broadcast.
        let eth = parse_eth(&discover[0]).unwrap();
        assert_eq!(eth.dst, BROADCAST_MAC);
        let ip = parse_ipv4(eth.payload).unwrap();
        assert_eq!((ip.src, ip.dst), (NO_IP, BROADCAST_IP));
        let udp = parse_udp(ip.src, ip.dst, ip.payload).unwrap();
        assert_eq!((udp.src_port, udp.dst_port), (68, 67));
        let xid = u32::from_be_bytes(udp.payload[4..8].try_into().unwrap());

        // The server's frames: from the router's MAC, to our MAC, from 10.0.2.2 to the offered address.
        let server_frame = |payload: Vec<u8>, dst_ip: Ip| {
            let u = build_udp([10, 0, 2, 2], dst_ip, 67, 68, &payload);
            build_eth(A_MAC, B_MAC, ETH_IPV4, &build_ipv4([10, 0, 2, 2], dst_ip, PROTO_UDP, 1, 64, &u))
        };
        let request = s.handle_frame(&server_frame(reply(2, xid, A_MAC, [10, 0, 2, 15]), BROADCAST_IP), 100);
        assert_eq!(request.len(), 1, "an offer is answered with a request");
        let rip = parse_ipv4(parse_eth(&request[0]).unwrap().payload).unwrap();
        let rudp = parse_udp(rip.src, rip.dst, rip.payload).unwrap();
        assert_eq!(rudp.payload[240..243], [53, 1, 3]);
        assert!(s.lease().is_none(), "not bound until the ACK");
        // An offer meant for another client changes nothing.
        s.handle_frame(&server_frame(reply(5, xid.wrapping_add(1), A_MAC, [10, 0, 2, 99]), BROADCAST_IP), 110);
        assert!(s.lease().is_none());
        s.handle_frame(&server_frame(reply(5, xid, A_MAC, [10, 0, 2, 15]), [10, 0, 2, 15]), 120);
        let l = s.lease().unwrap();
        assert_eq!((l.ip, l.router, l.dns), ([10, 0, 2, 15], Some([10, 0, 2, 2]), Some([10, 0, 2, 3])));
        assert!(matches!(s.take_events()[..], [Event::Leased(_)]));
        // Bound: no more DHCP traffic from the timers.
        assert!(s.poll(10_000).is_empty());
    }

    #[test]
    fn dhcp_retries_then_fails_then_tries_again() {
        let mut s = Stack::new(A_MAC, 42);
        s.start_dhcp(0);
        let mut sent = 0;
        let mut t = 0;
        while s.take_events().is_empty() && t < 60_000 {
            t += 100;
            sent += s.poll(t).len();
        }
        assert_eq!(sent, 4, "the first discover plus four retransmissions: five in all");
        assert_eq!(t, 30_000, "2 + 4 + 6 + 8 + 10 s of back-off, then it gives up");
        assert!(s.poll(t + 10_000).is_empty(), "quiet while it waits to start over");
        assert_eq!(s.poll(t + 30_000).len(), 1, "starts again 30 s after giving up");
    }

    #[test]
    fn lease_renews_at_three_quarters() {
        let mut s = Stack::new(A_MAC, 1);
        s.configure(Lease { lease_secs: 100, ..lease([10, 0, 2, 15]) });
        assert!(s.poll(74_000).is_empty());
        assert_eq!(s.poll(75_000).len(), 1, "a new discover");
    }

    #[test]
    fn dns_resolves_retries_and_times_out() {
        let mut s = Stack::new(A_MAC, 7);
        s.configure(lease([10, 0, 2, 15]));
        // An address needs no lookup.
        s.resolve("1.2.3.4", 0);
        assert_eq!(s.take_events(), alloc::vec![Event::Resolved { name: "1.2.3.4".into(), result: Ok([1, 2, 3, 4]) }]);
        // The server (10.0.2.3) is on-link: ARP first.
        let f = s.resolve("example.com", 0);
        assert_eq!(parse_arp(parse_eth(&f[0]).unwrap().payload).unwrap().tpa, [10, 0, 2, 3]);
        let arp_reply = build_eth(A_MAC, B_MAC, ETH_ARP, &build_arp(&Arp { op: 2, sha: B_MAC, spa: [10, 0, 2, 3], tha: A_MAC, tpa: [10, 0, 2, 15] }));
        let out = s.handle_frame(&arp_reply, 5);
        let ip = parse_ipv4(parse_eth(&out[0]).unwrap().payload).unwrap();
        let udp = parse_udp(ip.src, ip.dst, ip.payload).unwrap();
        assert_eq!((ip.dst, udp.dst_port), ([10, 0, 2, 3], 53));
        let id = u16::from_be_bytes([udp.payload[0], udp.payload[1]]);
        // A response to a different query id is ignored; the right one resolves it.
        let respond = |id: u16, rcode: u16, addr| {
            let u = build_udp([10, 0, 2, 3], [10, 0, 2, 15], 53, udp.src_port, &dns::tests::response(id, rcode, addr));
            build_eth(A_MAC, B_MAC, ETH_IPV4, &build_ipv4([10, 0, 2, 3], [10, 0, 2, 15], PROTO_UDP, 1, 64, &u))
        };
        s.handle_frame(&respond(id.wrapping_add(1), 0, Some([9, 9, 9, 9])), 10);
        assert!(s.take_events().is_empty());
        s.handle_frame(&respond(id, 0, Some([93, 184, 216, 34])), 20);
        assert_eq!(s.take_events(), alloc::vec![Event::Resolved { name: "example.com".into(), result: Ok([93, 184, 216, 34]) }]);

        // NXDOMAIN is reported as such.
        let f = s.resolve("nope.invalid", 100);
        let ip = parse_ipv4(parse_eth(&f[0]).unwrap().payload).unwrap();
        let udp = parse_udp(ip.src, ip.dst, ip.payload).unwrap();
        let id = u16::from_be_bytes([udp.payload[0], udp.payload[1]]);
        let u = build_udp([10, 0, 2, 3], [10, 0, 2, 15], 53, udp.src_port, &dns::tests::response(id, 3, None));
        s.handle_frame(&build_eth(A_MAC, B_MAC, ETH_IPV4, &build_ipv4([10, 0, 2, 3], [10, 0, 2, 15], PROTO_UDP, 2, 64, &u)), 110);
        assert_eq!(s.take_events(), alloc::vec![Event::Resolved { name: "nope.invalid".into(), result: Err("no such name") }]);

        // Silence: two retries, then a timeout.
        s.resolve("silent.example", 1000);
        assert_eq!(s.poll(2500).len(), 1);
        assert_eq!(s.poll(4000).len(), 1);
        assert!(s.take_events().is_empty());
        s.poll(5500);
        assert_eq!(s.take_events(), alloc::vec![Event::Resolved { name: "silent.example".into(), result: Err("the DNS server did not answer") }]);
        // Bad names and no server fail at once.
        s.resolve("bad name", 6000);
        assert!(matches!(&s.take_events()[..], [Event::Resolved { result: Err("not a valid name"), .. }]));
        let mut none = Stack::new(A_MAC, 1);
        none.resolve("example.com", 0);
        assert!(matches!(&none.take_events()[..], [Event::Resolved { result: Err(_), .. }]));
    }

    #[test]
    fn answers_pings_and_ignores_other_hosts_traffic() {
        let (mut a, mut b) = (Stack::new(A_MAC, 1), Stack::new(B_MAC, 2));
        a.configure(lease([10, 0, 2, 15]));
        b.configure(lease([10, 0, 2, 16]));
        // Frames for another MAC are ignored; garbage is counted, not answered.
        let stray = build_eth([9; 6], B_MAC, ETH_IPV4, &[0; 30]);
        assert!(a.handle_frame(&stray, 0).is_empty());
        assert!(a.handle_frame(&[1, 2, 3], 0).is_empty());
        assert_eq!(a.stats.dropped, 1);
        // An ARP request for someone else is not answered.
        let who = build_eth(BROADCAST_MAC, B_MAC, ETH_ARP, &build_arp(&Arp { op: 1, sha: B_MAC, spa: [10, 0, 2, 16], tha: [0; 6], tpa: [10, 0, 2, 77] }));
        assert!(a.handle_frame(&who, 0).is_empty());
        let _ = &mut b;
    }
}
