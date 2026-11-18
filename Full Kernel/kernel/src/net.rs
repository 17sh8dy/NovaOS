//! Networking in the kernel: finds the network card, runs the stack
//! (`crates/net`) on a thread, and implements the shell commands `ip`,
//! `ping`, `dns` and `dhcp`.
//!
//! Only virtio-net (QEMU, most hypervisors) is supported so far; a real PC's
//! Realtek or Intel card needs its own driver, which plugs in where
//! [`Nic`] is used. Polled every 10 ms: a received frame waits at most one
//! timer tick, which is fine for DHCP, DNS and ping.

use crate::kmutex::KMutex;
use crate::pci::PciDevice;
use crate::virtio_net::{self, VirtioNet};
use crate::{interrupts, kprintln, sched};
use alloc::string::String;
use alloc::vec::Vec;
use nova_net::dhcp::Lease;
use nova_net::stack::{Event, Stack};
use nova_net::wire::{self, Ip};

/// The card the stack uses.
type Nic = VirtioNet;

struct Net {
    nic: Nic,
    stack: Stack,
    /// Events nobody has picked up yet (requests wait for theirs).
    events: Vec<Event>,
    name: String,
}

static NET: KMutex<Option<Net>> = KMutex::new(None);

fn now_ms() -> u64 {
    interrupts::ticks() * 1000 / interrupts::TICK_HZ as u64
}

/// Looks for a network card and sets it up. Returns whether there is one.
pub fn init(devices: &[PciDevice]) -> bool {
    let nics: Vec<&PciDevice> = devices.iter().filter(|d| d.class == 0x02).collect();
    if nics.is_empty() {
        kprintln!("net: no network controller found");
        return false;
    }
    for d in &nics {
        if !virtio_net::is_virtio_net(d) {
            kprintln!("net: {} {:04x}:{:04x}: no driver for this network card yet (only virtio-net)", d.address(), d.vendor, d.device_id);
            continue;
        }
        match VirtioNet::probe(d) {
            Ok(nic) => {
                let mac = nic.mac;
                let seed = (nova_x86::rdtsc() as u32) ^ u32::from_le_bytes([mac[2], mac[3], mac[4], mac[5]]);
                kprintln!("net: virtio-net {} up, MAC {}", d.address(), wire::mac_text(mac));
                *NET.lock() = Some(Net { nic, stack: Stack::new(mac, seed), events: Vec::new(), name: alloc::format!("virtio-net {}", d.address()) });
                return true;
            }
            Err(e) => kprintln!("net: virtio-net {}: {e}", d.address()),
        }
    }
    false
}

/// Sends frames out of the card.
fn transmit(net: &mut Net, frames: Vec<Vec<u8>>) {
    let n = frames.len();
    for f in frames {
        if !net.nic.send(&f) {
            net.stack.stats.dropped += 1;
        }
    }
    net.stack.count_sent(n);
}

/// Moves new events from the stack to the shared list, announcing the ones worth logging.
fn collect_events(net: &mut Net) {
    for e in net.stack.take_events() {
        match &e {
            Event::Leased(l) => kprintln!("net: DHCP lease: {}", lease_text(l)),
            Event::LeaseFailed => kprintln!("net: DHCP got no answer (will try again)"),
            _ => {}
        }
        net.events.push(e);
    }
    if net.events.len() > 64 {
        net.events.drain(..32);
    }
}

fn lease_text(l: &Lease) -> String {
    let prefix: u32 = l.mask.iter().map(|b| b.count_ones()).sum();
    alloc::format!(
        "{}/{} router {} dns {}",
        wire::ip_text(l.ip),
        prefix,
        l.router.map_or(String::from("none"), wire::ip_text),
        l.dns.map_or(String::from("none"), wire::ip_text)
    )
}

/// Starts the thread that runs the network: DHCP first, then receive and timers.
pub fn start_service() {
    if NET.lock().is_none() {
        return;
    }
    let spawned = sched::spawn("net", || {
        {
            let mut g = NET.lock();
            if let Some(net) = g.as_mut() {
                let frames = net.stack.start_dhcp(now_ms());
                transmit(net, frames);
                kprintln!("net: asking for an address (DHCP)");
            }
        }
        loop {
            {
                let mut g = NET.lock();
                if let Some(net) = g.as_mut() {
                    let now = now_ms();
                    while let Some(frame) = net.nic.recv() {
                        let out = net.stack.handle_frame(&frame, now);
                        transmit(net, out);
                    }
                    let out = net.stack.poll(now);
                    transmit(net, out);
                    collect_events(net);
                }
            }
            sched::sleep_ms(10);
        }
    });
    if spawned.is_none() {
        kprintln!("net: could not start the network thread");
    }
}

/// Lines for the System window and the `ip` command.
pub fn describe() -> Vec<String> {
    let g = NET.lock();
    let Some(net) = g.as_ref() else { return alloc::vec![String::from("no network card (only virtio-net is supported so far)")] };
    let mut v = alloc::vec![alloc::format!("{}, MAC {}", net.name, wire::mac_text(net.stack.mac()))];
    match net.stack.lease() {
        Some(l) => {
            v.push(alloc::format!("address {}", lease_text(l)));
        }
        None => v.push(String::from("no address yet (DHCP running or no server)")),
    }
    let s = net.stack.stats;
    v.push(alloc::format!("{} frames in, {} out, {} dropped", s.frames_in, s.frames_out, s.dropped));
    v
}

/// The address as text, for the System window ("not connected" if there is none).
pub fn address_text() -> String {
    let g = NET.lock();
    match g.as_ref() {
        None => String::from("no network card"),
        Some(net) => net.stack.lease().map_or(String::from("not connected"), |l| wire::ip_text(l.ip)),
    }
}

/// Waits (up to `timeout_ms`) for an event `pick` likes; other events stay.
fn wait_event<T>(timeout_ms: u64, mut pick: impl FnMut(&Event) -> Option<T>) -> Option<T> {
    let end = now_ms() + timeout_ms;
    loop {
        {
            let mut g = NET.lock();
            let net = g.as_mut()?;
            if let Some(i) = net.events.iter().position(|e| pick(e).is_some()) {
                let e = net.events.remove(i);
                return pick(&e);
            }
        }
        if now_ms() >= end {
            return None;
        }
        sched::sleep_ms(10);
    }
}

fn with_net<T>(f: impl FnOnce(&mut Net, u64) -> T) -> Option<T> {
    let mut g = NET.lock();
    let net = g.as_mut()?;
    Some(f(net, now_ms()))
}

/// Resolves a name or dotted address (up to 8 s).
fn resolve(host: &str) -> Result<Ip, String> {
    let name = String::from(host);
    with_net(|net, now| {
        let frames = net.stack.resolve(&name, now);
        transmit(net, frames);
        collect_events(net);
    })
    .ok_or_else(|| String::from("no network card"))?;
    let want = name.clone();
    match wait_event(9_000, |e| match e {
        Event::Resolved { name, result } if *name == want => Some(*result),
        _ => None,
    }) {
        Some(Ok(ip)) => Ok(ip),
        Some(Err(why)) => Err(alloc::format!("cannot resolve {host}: {why}")),
        None => Err(alloc::format!("cannot resolve {host}: no answer")),
    }
}

/// The shell commands. `out(text, is_error)` prints a line.
pub fn command(cmd: &str, args: &str, out: &mut dyn FnMut(&str, bool)) {
    match cmd {
        "ip" | "net" | "ifconfig" => {
            for l in describe() {
                out(&l, false);
            }
        }
        "dhcp" => {
            if with_net(|net, now| {
                let frames = net.stack.start_dhcp(now);
                transmit(net, frames);
            })
            .is_none()
            {
                out("no network card", true);
                return;
            }
            out("asking for an address...", false);
            match wait_event(35_000, |e| match e {
                Event::Leased(l) => Some(Some(l.clone())),
                Event::LeaseFailed => Some(None),
                _ => None,
            }) {
                Some(Some(l)) => out(&alloc::format!("got {}", lease_text(&l)), false),
                Some(None) => out("no DHCP server answered", true),
                None => out("still waiting (see 'ip')", true),
            }
        }
        "dns" | "nslookup" => {
            let host = args.split_whitespace().next().unwrap_or("");
            if host.is_empty() {
                out("usage: dns NAME", true);
                return;
            }
            match resolve(host) {
                Ok(ip) => out(&alloc::format!("{host} is {}", wire::ip_text(ip)), false),
                Err(e) => out(&e, true),
            }
        }
        "ping" => ping(args, out),
        _ => out("unknown network command", true),
    }
}

fn ping(args: &str, out: &mut dyn FnMut(&str, bool)) {
    let host = args.split_whitespace().next().unwrap_or("");
    if host.is_empty() {
        out("usage: ping HOST (a name or an address)", true);
        return;
    }
    let ip = match resolve(host) {
        Ok(ip) => ip,
        Err(e) => {
            out(&e, true);
            return;
        }
    };
    out(&alloc::format!("pinging {host} ({}) with 32 bytes of data", wire::ip_text(ip)), false);
    let (mut got, mut total_ms) = (0u32, 0u64);
    for seq in 1..=4u16 {
        let sent = now_ms();
        if with_net(|net, now| {
            let frames = net.stack.ping(ip, seq, now);
            transmit(net, frames);
        })
        .is_none()
        {
            out("no network card", true);
            return;
        }
        let r = wait_event(2_000, |e| match e {
            Event::PingReply { seq: s, from, ms, ttl, bytes } if *s == seq => Some(Some((*from, *ms, *ttl, *bytes))),
            Event::Unreachable { ip: u } if *u == ip => Some(None),
            _ => None,
        });
        match r {
            Some(Some((from, ms, ttl, bytes))) => {
                got += 1;
                total_ms += ms;
                out(&alloc::format!("reply from {}: bytes={bytes} time={ms}ms ttl={ttl} seq={seq}", wire::ip_text(from)), false);
            }
            Some(None) => {
                out(&alloc::format!("{host}: host unreachable (nobody answered ARP)"), true);
                break;
            }
            None => out(&alloc::format!("request {seq} timed out"), true),
        }
        // One ping per second.
        let spent = now_ms().saturating_sub(sent);
        if seq < 4 && spent < 1000 {
            sched::sleep_ms(1000 - spent);
        }
    }
    let avg = if got > 0 { total_ms / got as u64 } else { 0 };
    out(&alloc::format!("{got} of 4 replies{}", if got > 0 { alloc::format!(", average {avg} ms") } else { String::new() }), got == 0);
}
