//! Nova's network stack, as pure logic that is tested on the host.
//!
//! * [`wire`]: Ethernet, ARP, IPv4, ICMP echo, UDP and the Internet checksum.
//! * [`dhcp`], [`dns`]: the client messages.
//! * [`stack`]: the state machine that ties them together (DHCP lease, ARP cache,
//!   ping, name lookups, retransmission timers).
//!
//! Hardware is the kernel's business: a driver hands received frames to
//! [`stack::Stack::handle_frame`] and transmits what comes back.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod dhcp;
pub mod dns;
pub mod stack;
pub mod wire;
