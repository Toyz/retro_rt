//! Networking for a fixed-tick loop.
//!
//! Every socket here is non-blocking and is pumped by a `poll` the game calls
//! once a tick, from the tick: nothing runs on another thread and nothing
//! needs an async runtime. A poll does the I/O that is ready and returns
//! what arrived as events; sends are queued and go out on the next poll.
//!
//! - [`Udp`]: datagrams to one peer, a broadcast address or a multicast
//!   group (LAN discovery, system link, lockstep input).
//! - [`TcpClient`] and [`TcpServer`]: streams, cut into messages by a
//!   [`Framing`]: [`Framing::LengthPrefixed`] for the port's own protocol,
//!   [`Framing::Raw`] for speaking an original game's protocol byte for
//!   byte.
//!
//! - [`Rudp`]: connections over UDP with per-channel [`Delivery`]:
//!   unreliable, sequenced, reliable unordered, reliable ordered. The
//!   protocol itself is [`Connection`], which does no I/O and can run over
//!   any datagram transport.
//!
//! What is not here: encryption, NAT traversal, adaptive congestion control
//! (sends are paced at a configured rate instead). See
//! `docs/crates/rrt-net.md`.

pub mod frame;
pub mod reliable;
pub mod tcp;
pub mod udp;

pub use frame::{Decoder, Framing, TooLarge, encode};
pub use reliable::{Connection, Delivery, Rudp, RudpConfig, RudpEvent, SendError, Stats};
pub use tcp::{ClientEvent, ServerEvent, TcpClient, TcpServer};
pub use udp::{Datagram, Udp};

/// Why a connection ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Closed {
    /// The peer closed it.
    Peer,
    /// This side closed it ([`TcpClient::close`], [`TcpServer::kick`], [`Rudp::disconnect`]).
    Local,
    /// A length prefix over the framing's limit.
    TooLarge(u32),
    /// A socket error, as text.
    Error(String),
    /// Nothing heard from the peer for the configured timeout
    /// ([`RudpConfig::timeout`]).
    Timeout,
    /// The server turned the connection down: full, or not accepting.
    Refused,
}

/// A peer of a server or endpoint, numbered from 1 in the order it joined.
/// Numbers are not reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId(pub u32);
