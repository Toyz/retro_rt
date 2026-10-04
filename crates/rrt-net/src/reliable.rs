//! Connections over UDP with reliable, ordered and sequenced messages.
//!
//! [`Rudp`] is an endpoint: one UDP socket, any number of peers, each a
//! [`Connection`] after a handshake. A message goes on a channel, and the
//! channel's [`Delivery`] decides what the far side sees: everything in order,
//! everything in any order, only the newest, or whatever arrives. Like the
//! rest of the crate it is pumped by [`Rudp::poll`] once a tick.
//!
//! [`Connection`] is the protocol with no I/O: bytes in with
//! [`Connection::receive`], bytes out with [`Connection::transmit`], time
//! passed in. The tests drive it over a simulated link; a game can run it over
//! any datagram transport.
//!
//! # Wire format
//!
//! All little-endian. Every packet starts with [`PROTOCOL`] and a kind byte;
//! anything else on the port is ignored.
//!
//! ```text
//! connect     u32 protocol, u8 0, u64 client salt, zero padding to 32 bytes
//! accept      u32 protocol, u8 1, u64 client salt, u64 server salt
//! deny        u32 protocol, u8 2, u64 client salt
//! data        u32 protocol, u8 3, u64 session (client salt ^ server salt)
//!             u16 seq          this packet's number
//!             u16 ack          the newest packet number received
//!             u32 ack bits     bit n: packet ack - 1 - n received
//!             u8  count
//!             message[count]
//!               u8  channel    bit 7 set: a fragment, and two more fields follow len
//!               u16 id         per channel: reliable message id, or sequence
//!               u16 len
//!               u16 index      fragments only: this one's place, from 0
//!               u16 fragments  fragments only: how many the message was cut into
//!               [u8; len]
//! disconnect  u32 protocol, u8 4, u64 session
//! ```
//!
//! The connect request is padded to be larger than the accept it draws, so
//! the endpoint cannot be used to amplify a spoofed flood.
//!
//! # Reliability
//!
//! Every data packet acknowledges the newest 33 packets received (`ack` and
//! its bits). A reliable message stays queued until a packet that carried it
//! is acknowledged, and is sent again each time [`Connection::rto`] passes
//! without that. The receiver drops what it has already delivered, so a
//! message resent after its ack was lost arrives once. At most [`WINDOW`]
//! reliable messages a channel are in flight; later ones wait their turn.
//!
//! A reliable message longer than one packet holds is cut into fragments,
//! each a reliable message of its own with consecutive ids, and put back
//! together on the far side before it is delivered. Unreliable and sequenced
//! messages must fit one packet.
//!
//! # Pacing
//!
//! With [`RudpConfig::bandwidth`] set, a connection spends at most that many
//! bytes a second on messages, with a burst of a tenth of a second's worth.
//! A reliable message that does not fit the budget waits for the next
//! transmit; an unreliable one is dropped (counted in [`Stats::dropped`]),
//! since by then it would be stale. Acks and keepalives always go.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::udp::Udp;
use crate::{Closed, PeerId};

/// The first four bytes of every packet: `RRT1`.
pub const PROTOCOL: u32 = u32::from_le_bytes(*b"RRT1");

/// Reliable messages a channel may have in flight; also how far ahead of the
/// next expected id a receiver accepts one.
pub const WINDOW: u16 = 1024;

const CONNECT: u8 = 0;
const ACCEPT: u8 = 1;
const DENY: u8 = 2;
const DATA: u8 = 3;
const DISCONNECT: u8 = 4;

/// Bytes a connect request is padded to.
const CONNECT_LEN: usize = 32;
/// Bytes before the first message of a data packet.
const DATA_HEADER: usize = 4 + 1 + 8 + 2 + 2 + 4 + 1;
/// Bytes before each message's payload.
const MESSAGE_HEADER: usize = 1 + 2 + 2;
/// Bytes a fragment adds to its message header: index and count.
const FRAGMENT_HEADER: usize = 2 + 2;
/// The channel byte's fragment flag; the low 7 bits are the channel.
const FRAGMENT: u8 = 0x80;
/// Channels a configuration may have: the channel byte's low 7 bits.
pub const MAX_CHANNELS: usize = 128;
/// Packet numbers an ack covers: `ack` and its 32 bits.
const ACK_SPAN: u16 = 33;

/// What the far side of a channel sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Delivery {
    /// Whatever arrives, in the order it arrives, perhaps twice. Never
    /// resent.
    Unreliable,
    /// Only a message newer than the newest delivered: late ones are
    /// dropped. Never resent. For state that is overwritten every tick
    /// (positions, the pad).
    Sequenced,
    /// Every message exactly once, in any order. Resent until acknowledged.
    Reliable,
    /// Every message exactly once, in the order sent. Resent until
    /// acknowledged; a gap holds back what follows it.
    ReliableOrdered,
}

impl Delivery {
    fn reliable(self) -> bool {
        matches!(self, Delivery::Reliable | Delivery::ReliableOrdered)
    }
}

/// How an endpoint and its connections behave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RudpConfig {
    /// The channels, by index. Both ends must agree. The default is
    /// channel 0 [`Delivery::ReliableOrdered`], 1 [`Delivery::Unreliable`],
    /// 2 [`Delivery::Sequenced`], 3 [`Delivery::Reliable`]. At most
    /// [`MAX_CHANNELS`]; sends to a channel past that are refused.
    pub channels: Vec<Delivery>,
    /// The largest packet sent, in bytes, UDP payload. 1200 stays under
    /// every real path's MTU.
    pub mtu: usize,
    /// A peer heard nothing from for this long is gone
    /// ([`Closed::Timeout`]); so is a connect with no answer.
    pub timeout: Duration,
    /// An otherwise idle connection sends a packet this often, so the far
    /// side does not time it out and its acks keep flowing.
    pub keepalive: Duration,
    /// How often a connect request is repeated until answered.
    pub connect_retry: Duration,
    /// Accept connect requests: a server. A client-only endpoint sets
    /// false and answers requests with a deny.
    pub accept: bool,
    /// Peers past this many are denied ([`Closed::Refused`] on their side).
    pub max_peers: usize,
    /// The longest reliable message, in bytes, cut into fragments as it
    /// needs. 1 MiB by default.
    pub max_reliable_message: usize,
    /// Bytes a second each connection may spend on messages; None for no
    /// limit. 4 MiB/s by default: far above what a game's state needs, low
    /// enough that a large reliable message does not flood the socket's
    /// buffer in one tick.
    pub bandwidth: Option<u64>,
}

impl Default for RudpConfig {
    fn default() -> RudpConfig {
        RudpConfig {
            channels: vec![Delivery::ReliableOrdered, Delivery::Unreliable, Delivery::Sequenced, Delivery::Reliable],
            mtu: 1200,
            timeout: Duration::from_secs(5),
            keepalive: Duration::from_millis(250),
            connect_retry: Duration::from_millis(100),
            accept: true,
            max_peers: usize::MAX,
            max_reliable_message: 1 << 20,
            bandwidth: Some(4 << 20),
        }
    }
}

impl RudpConfig {
    /// The largest message one packet holds: the [`RudpConfig::mtu`] less
    /// the headers. The limit for unreliable and sequenced channels; a
    /// longer reliable message is fragmented.
    pub fn max_message(&self) -> usize {
        self.mtu.saturating_sub(DATA_HEADER + MESSAGE_HEADER).min(usize::from(u16::MAX))
    }

    /// The longest message `channel` takes: [`RudpConfig::max_reliable_message`]
    /// on a reliable channel, [`RudpConfig::max_message`] on the others. None
    /// for a channel that does not exist.
    pub fn limit(&self, channel: u8) -> Option<usize> {
        let d = self.channels.get(usize::from(channel)).filter(|_| usize::from(channel) < MAX_CHANNELS)?;
        Some(if d.reliable() { self.max_reliable_message } else { self.max_message() })
    }
}

/// Why a message was not queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendError {
    /// No such peer, or it has gone.
    NoPeer,
    /// The channel is not in [`RudpConfig::channels`].
    NoChannel(u8),
    /// Longer than the channel takes ([`RudpConfig::limit`]).
    TooLarge {
        /// The message's length.
        len: usize,
        /// The most a message may be.
        max: usize,
    },
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::NoPeer => write!(f, "no such peer"),
            SendError::NoChannel(c) => write!(f, "no channel {c}"),
            SendError::TooLarge { len, max } => write!(f, "a {len}-byte message; the channel takes at most {max}"),
        }
    }
}

impl std::error::Error for SendError {}

/// What a connection has done, for a network overlay or a test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// The smoothed round trip, from packets acknowledged.
    pub rtt: Duration,
    /// Packets sent.
    pub packets_sent: u64,
    /// Data packets received for this connection.
    pub packets_received: u64,
    /// Reliable messages sent again after going unacknowledged.
    pub resent: u64,
    /// Reliable messages acknowledged.
    pub acked: u64,
    /// Reliable messages (fragments counted one each) queued and not yet
    /// acknowledged, every channel.
    pub in_flight: usize,
    /// Unreliable and sequenced messages dropped for want of
    /// [`RudpConfig::bandwidth`].
    pub dropped: u64,
}

/// `a` is later than `b` in sequence space, where numbers wrap at 65536
/// and the later of two is the one less than half the space ahead.
fn newer(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

/// A reliable message waiting for its ack.
#[derive(Clone, Debug)]
struct Outgoing {
    id: u16,
    data: Vec<u8>,
    /// Index and count, when this is one fragment of a longer message.
    frag: Option<(u16, u16)>,
    last_sent: Option<Instant>,
}

/// A reliable message received and not yet delivered: held for order, or
/// marked as seen.
#[derive(Clone, Debug)]
struct Piece {
    frag: Option<(u16, u16)>,
    data: Vec<u8>,
}

/// An unordered channel's fragmented message being put back together.
#[derive(Clone, Debug)]
struct Assembly {
    parts: Vec<Option<Vec<u8>>>,
    have: usize,
}

/// One channel's state, both directions.
#[derive(Clone, Debug)]
struct Channel {
    delivery: Delivery,
    /// The id the next message sent gets.
    next_id: u16,
    /// Reliable messages not yet acknowledged, oldest first.
    reliable: VecDeque<Outgoing>,
    /// Unreliable and sequenced messages for the next transmit.
    unreliable: Vec<(u16, Vec<u8>)>,
    /// Reliable: the oldest id not yet delivered.
    next_expected: u16,
    /// Reliable: ids at or past `next_expected` already received, with the
    /// payload when ordered delivery is holding it back.
    received: HashMap<u16, Piece>,
    /// Ordered: the fragments of the message being delivered, so far.
    partial: Vec<u8>,
    /// Unordered: fragmented messages being put back together, by the id
    /// of their first fragment.
    assembling: HashMap<u16, Assembly>,
    /// Sequenced: the newest id delivered.
    newest: Option<u16>,
}

impl Channel {
    fn new(delivery: Delivery) -> Channel {
        Channel {
            delivery,
            next_id: 0,
            reliable: VecDeque::new(),
            unreliable: Vec::new(),
            next_expected: 0,
            received: HashMap::new(),
            partial: Vec::new(),
            assembling: HashMap::new(),
            newest: None,
        }
    }

    /// A message that arrived on this channel; what it delivers goes to `out`.
    fn arrive(&mut self, channel: u8, id: u16, frag: Option<(u16, u16)>, data: &[u8], out: &mut Vec<(u8, Vec<u8>)>) {
        match self.delivery {
            Delivery::Unreliable => out.push((channel, data.to_vec())),
            Delivery::Sequenced => {
                if self.newest.is_none_or(|n| newer(id, n)) {
                    self.newest = Some(id);
                    out.push((channel, data.to_vec()));
                }
            }
            Delivery::Reliable | Delivery::ReliableOrdered => {
                let ahead = id.wrapping_sub(self.next_expected);
                // Behind the window is already delivered; past it the sender
                // would never send.
                if ahead >= WINDOW || self.received.contains_key(&id) {
                    return;
                }
                if self.delivery == Delivery::Reliable {
                    self.received.insert(id, Piece { frag: None, data: Vec::new() });
                    while self.received.remove(&self.next_expected).is_some() {
                        self.next_expected = self.next_expected.wrapping_add(1);
                    }
                    match frag {
                        None => out.push((channel, data.to_vec())),
                        Some((index, count)) => self.assemble(channel, id, index, count, data, out),
                    }
                } else {
                    self.received.insert(id, Piece { frag, data: data.to_vec() });
                    while let Some(p) = self.received.remove(&self.next_expected) {
                        self.next_expected = self.next_expected.wrapping_add(1);
                        match p.frag {
                            None => out.push((channel, p.data)),
                            Some((index, count)) => {
                                if index == 0 {
                                    self.partial.clear();
                                }
                                self.partial.extend_from_slice(&p.data);
                                if index + 1 == count {
                                    out.push((channel, std::mem::take(&mut self.partial)));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// An unordered fragment: kept until its message is whole, then the
    /// message delivered.
    fn assemble(&mut self, channel: u8, id: u16, index: u16, count: u16, data: &[u8], out: &mut Vec<(u8, Vec<u8>)>) {
        let first = id.wrapping_sub(index);
        let a =
            self.assembling.entry(first).or_insert_with(|| Assembly { parts: vec![None; usize::from(count)], have: 0 });
        let Some(slot) = a.parts.get_mut(usize::from(index)) else { return };
        if slot.is_none() {
            *slot = Some(data.to_vec());
            a.have += 1;
        }
        if a.have == a.parts.len() {
            let a = self.assembling.remove(&first).unwrap();
            out.push((channel, a.parts.into_iter().flatten().flatten().collect()));
        }
    }
}

/// A message chosen for this transmit, before it is packed.
struct Queued {
    channel: u8,
    id: u16,
    frag: Option<(u16, u16)>,
    data: Vec<u8>,
    reliable: bool,
}

/// A packet sent and not yet acknowledged or given up on.
#[derive(Clone, Debug)]
struct SentPacket {
    at: Instant,
    /// The reliable messages it carried: channel, id.
    reliable: Vec<(u8, u16)>,
}

/// One connection's protocol, after the handshake: no sockets, the time
/// passed in. [`Rudp`] runs one per peer.
#[derive(Clone, Debug)]
pub struct Connection {
    session: u64,
    channels: Vec<Channel>,
    mtu: usize,
    max_message: usize,
    max_reliable: usize,
    keepalive: Duration,
    bandwidth: Option<u64>,
    /// Bytes the pacing budget holds now, and when it was last topped up.
    tokens: f64,
    refilled: Option<Instant>,
    /// The number the next packet sent gets.
    local_seq: u16,
    /// The newest packet number received, and which of the 32 before it.
    remote_seq: u16,
    ack_bits: u32,
    received_any: bool,
    /// A packet with reliable messages arrived since the last transmit.
    ack_pending: bool,
    sent: BTreeMap<u16, SentPacket>,
    srtt: Option<Duration>,
    last_sent: Option<Instant>,
    last_received: Instant,
    stats: Stats,
}

/// A round trip assumed before one is measured.
const INITIAL_RTT: Duration = Duration::from_millis(100);

impl Connection {
    /// A connection for `session` (both ends must use the same), with
    /// `config`'s channels, packet size and keepalive, last heard from at
    /// `now`.
    pub fn new(session: u64, config: &RudpConfig, now: Instant) -> Connection {
        Connection {
            session,
            channels: config.channels.iter().map(|d| Channel::new(*d)).collect(),
            mtu: config.mtu,
            max_message: config.max_message(),
            max_reliable: config.max_reliable_message,
            keepalive: config.keepalive,
            bandwidth: config.bandwidth,
            tokens: config.bandwidth.map_or(0.0, |rate| burst(rate, config.mtu)),
            refilled: None,
            local_seq: 0,
            remote_seq: 0,
            ack_bits: 0,
            received_any: false,
            ack_pending: false,
            sent: BTreeMap::new(),
            srtt: None,
            last_sent: None,
            last_received: now,
            stats: Stats::default(),
        }
    }

    /// The session number data packets carry.
    pub fn session(&self) -> u64 {
        self.session
    }

    /// What the connection has done.
    pub fn stats(&self) -> Stats {
        let in_flight = self.channels.iter().map(|c| c.reliable.len()).sum();
        Stats { rtt: self.srtt.unwrap_or(INITIAL_RTT), in_flight, ..self.stats }
    }

    /// How long an unacknowledged reliable message waits before it is sent
    /// again: twice the smoothed round trip, between 30 ms and 1 s.
    pub fn rto(&self) -> Duration {
        (self.srtt.unwrap_or(INITIAL_RTT) * 2).clamp(Duration::from_millis(30), Duration::from_secs(1))
    }

    /// How long since a packet for this connection arrived.
    pub fn idle(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_received)
    }

    /// Queues `data` on `channel`; it goes out on the next transmit. A
    /// reliable message longer than one packet holds is cut into fragments.
    pub fn send(&mut self, channel: u8, data: &[u8]) -> Result<(), SendError> {
        let (single, fragment) = (self.max_message, self.max_message - FRAGMENT_HEADER);
        let c = self
            .channels
            .get_mut(usize::from(channel))
            .filter(|_| usize::from(channel) < MAX_CHANNELS)
            .ok_or(SendError::NoChannel(channel))?;
        let max = if c.delivery.reliable() { self.max_reliable } else { single };
        if data.len() > max {
            return Err(SendError::TooLarge { len: data.len(), max });
        }
        let mut id = || {
            let id = c.next_id;
            c.next_id = c.next_id.wrapping_add(1);
            id
        };
        if !c.delivery.reliable() {
            let id = id();
            c.unreliable.push((id, data.to_vec()));
        } else if data.len() <= single {
            let id = id();
            c.reliable.push_back(Outgoing { id, data: data.to_vec(), frag: None, last_sent: None });
        } else {
            let chunks: Vec<&[u8]> = data.chunks(fragment).collect();
            let count = chunks.len() as u16;
            for (index, chunk) in chunks.into_iter().enumerate() {
                let id = id();
                let frag = Some((index as u16, count));
                c.reliable.push_back(Outgoing { id, data: chunk.to_vec(), frag, last_sent: None });
            }
        }
        Ok(())
    }

    /// A data packet from the peer: its acks applied, its messages delivered
    /// as each channel's [`Delivery`] allows, as (channel, payload). A packet
    /// that is malformed, for another session, or not data delivers nothing
    /// and changes nothing.
    pub fn receive(&mut self, packet: &[u8], now: Instant) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        let Some(p) = parse_data(packet) else { return out };
        if p.session != self.session {
            return out;
        }
        // Check every message before touching any state.
        let mut messages = Vec::with_capacity(usize::from(p.count));
        let mut at = DATA_HEADER;
        for _ in 0..p.count {
            let Some(head) = packet.get(at..at + MESSAGE_HEADER) else { return out };
            let channel = head[0] & !FRAGMENT;
            let id = u16::from_le_bytes([head[1], head[2]]);
            let len = usize::from(u16::from_le_bytes([head[3], head[4]]));
            at += MESSAGE_HEADER;
            let frag = if head[0] & FRAGMENT != 0 {
                let Some(f) = packet.get(at..at + FRAGMENT_HEADER) else { return out };
                at += FRAGMENT_HEADER;
                let (index, count) = (u16::from_le_bytes([f[0], f[1]]), u16::from_le_bytes([f[2], f[3]]));
                if index >= count {
                    return out;
                }
                Some((index, count))
            } else {
                None
            };
            let Some(data) = packet.get(at..at + len) else { return out };
            at += len;
            // Only reliable channels carry fragments.
            match self.channels.get(usize::from(channel)) {
                Some(c) if frag.is_none() || c.delivery.reliable() => {}
                _ => return out,
            }
            messages.push((channel, id, frag, data));
        }
        self.last_received = now;
        self.stats.packets_received += 1;
        self.note_received(p.seq);
        self.apply_acks(p.ack, p.ack_bits, now);
        for (channel, id, frag, data) in messages {
            let c = &mut self.channels[usize::from(channel)];
            if c.delivery.reliable() {
                self.ack_pending = true;
            }
            c.arrive(channel, id, frag, data, &mut out);
        }
        out
    }

    /// Packet `seq` arrived: fold it into `remote_seq` and the ack bits.
    fn note_received(&mut self, seq: u16) {
        if !self.received_any {
            self.received_any = true;
            self.remote_seq = seq;
            self.ack_bits = 0;
        } else if newer(seq, self.remote_seq) {
            let shift = u32::from(seq.wrapping_sub(self.remote_seq));
            // The old newest becomes bit shift - 1.
            self.ack_bits =
                if shift > 32 { 0 } else { ((u64::from(self.ack_bits) << shift) | (1 << (shift - 1))) as u32 };
            self.remote_seq = seq;
        } else {
            let back = u32::from(self.remote_seq.wrapping_sub(seq));
            if (1..=32).contains(&back) {
                self.ack_bits |= 1 << (back - 1);
            }
        }
    }

    /// The peer has packets `ack` and those its bits name: what they carried
    /// is delivered.
    fn apply_acks(&mut self, ack: u16, bits: u32, now: Instant) {
        let acked = |seq: u16| {
            let back = ack.wrapping_sub(seq);
            back == 0 || (1..=32).contains(&back) && bits & (1 << (back - 1)) != 0
        };
        let seqs: Vec<u16> = self.sent.keys().copied().filter(|s| acked(*s)).collect();
        for seq in seqs {
            let p = self.sent.remove(&seq).unwrap();
            let sample = now.saturating_duration_since(p.at);
            self.srtt = Some(match self.srtt {
                None => sample,
                Some(s) => s * 7 / 8 + sample / 8,
            });
            for (channel, id) in p.reliable {
                let c = &mut self.channels[usize::from(channel)];
                if let Some(i) = c.reliable.iter().position(|o| o.id == id) {
                    c.reliable.remove(i);
                    self.stats.acked += 1;
                }
            }
        }
        // Packets too far behind the newest ack can never be acknowledged;
        // their messages are resent on their own timers.
        self.sent.retain(|seq, _| !newer(ack, *seq) || ack.wrapping_sub(*seq) < ACK_SPAN);
    }

    /// The packets to send now: reliable messages due (new, or unacknowledged
    /// for [`Connection::rto`]), then unreliable ones, as many packets as
    /// they take; or one bare packet when only an ack or a keepalive is due;
    /// or none.
    pub fn transmit(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let rto = self.rto();
        if let Some(rate) = self.bandwidth {
            let elapsed = self.refilled.map_or(Duration::ZERO, |t| now.saturating_duration_since(t));
            self.tokens = (self.tokens + rate as f64 * elapsed.as_secs_f64()).min(burst(rate, self.mtu));
            self.refilled = Some(now);
        }
        // What a message costs the pacing budget: its bytes on the wire.
        let cost = |frag: &Option<(u16, u16)>, data: &[u8]| {
            (MESSAGE_HEADER + if frag.is_some() { FRAGMENT_HEADER } else { 0 } + data.len()) as f64
        };
        let paced = self.bandwidth.is_some();
        let mut queued: Vec<Queued> = Vec::new();
        for (ch, c) in self.channels.iter_mut().enumerate() {
            // The window is by id from the oldest unacknowledged message, not
            // by queue position: acks remove messages out of order, and every
            // id below the oldest is delivered, so the receiver's next
            // expected id is at least the oldest and everything sent lands
            // inside its window. A message it dropped as out of window would
            // still be acknowledged by its packet, and lost.
            let oldest = c.reliable.front().map_or(0, |o| o.id);
            for o in c.reliable.iter_mut().take_while(|o| o.id.wrapping_sub(oldest) < WINDOW) {
                let due = o.last_sent.is_none_or(|t| now.saturating_duration_since(t) >= rto);
                if !due {
                    continue;
                }
                let c = cost(&o.frag, &o.data);
                if paced && self.tokens < c {
                    // Out of budget: this and the rest wait for the next
                    // transmit, oldest first.
                    break;
                }
                self.tokens -= c;
                if o.last_sent.is_some() {
                    self.stats.resent += 1;
                }
                o.last_sent = Some(now);
                queued.push(Queued { channel: ch as u8, id: o.id, frag: o.frag, data: o.data.clone(), reliable: true });
            }
        }
        for (ch, c) in self.channels.iter_mut().enumerate() {
            for (id, d) in c.unreliable.drain(..) {
                let c = cost(&None, &d);
                if paced && self.tokens < c {
                    self.stats.dropped += 1;
                    continue;
                }
                self.tokens -= c;
                queued.push(Queued { channel: ch as u8, id, frag: None, data: d, reliable: false });
            }
        }
        let keepalive_due = self.last_sent.is_none_or(|t| now.saturating_duration_since(t) >= self.keepalive);
        if queued.is_empty() && !self.ack_pending && !keepalive_due {
            return Vec::new();
        }
        let mut packets = Vec::new();
        let mut queued = queued.into_iter().peekable();
        loop {
            let mut packet = self.header();
            let mut reliable = Vec::new();
            let mut count = 0u8;
            while let Some(q) = queued.peek() {
                let size = MESSAGE_HEADER + if q.frag.is_some() { FRAGMENT_HEADER } else { 0 } + q.data.len();
                if count == u8::MAX || packet.len() + size > self.mtu {
                    break;
                }
                let q = queued.next().unwrap();
                packet.push(if q.frag.is_some() { q.channel | FRAGMENT } else { q.channel });
                packet.extend_from_slice(&q.id.to_le_bytes());
                packet.extend_from_slice(&(q.data.len() as u16).to_le_bytes());
                if let Some((index, count)) = q.frag {
                    packet.extend_from_slice(&index.to_le_bytes());
                    packet.extend_from_slice(&count.to_le_bytes());
                }
                packet.extend_from_slice(&q.data);
                if q.reliable {
                    reliable.push((q.channel, q.id));
                }
                count += 1;
            }
            packet[DATA_HEADER - 1] = count;
            self.sent.insert(self.local_seq, SentPacket { at: now, reliable });
            self.local_seq = self.local_seq.wrapping_add(1);
            self.stats.packets_sent += 1;
            packets.push(packet);
            if queued.peek().is_none() {
                break;
            }
        }
        self.ack_pending = false;
        self.last_sent = Some(now);
        packets
    }

    /// A data packet's header for the next packet number, count 0.
    fn header(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(self.mtu);
        p.extend_from_slice(&PROTOCOL.to_le_bytes());
        p.push(DATA);
        p.extend_from_slice(&self.session.to_le_bytes());
        p.extend_from_slice(&self.local_seq.to_le_bytes());
        p.extend_from_slice(&self.remote_seq.to_le_bytes());
        p.extend_from_slice(&self.ack_bits.to_le_bytes());
        p.push(0);
        p
    }
}

/// The pacing budget's ceiling: a tenth of a second at `rate`, and never
/// less than one packet, so a packet always fits a full budget.
fn burst(rate: u64, mtu: usize) -> f64 {
    (rate as f64 / 10.0).max(mtu as f64)
}

/// A data packet's header fields.
struct DataHeader {
    session: u64,
    seq: u16,
    ack: u16,
    ack_bits: u32,
    count: u8,
}

fn parse_data(p: &[u8]) -> Option<DataHeader> {
    if p.len() < DATA_HEADER || kind(p)? != DATA {
        return None;
    }
    Some(DataHeader {
        session: u64::from_le_bytes(p[5..13].try_into().ok()?),
        seq: u16::from_le_bytes([p[13], p[14]]),
        ack: u16::from_le_bytes([p[15], p[16]]),
        ack_bits: u32::from_le_bytes(p[17..21].try_into().ok()?),
        count: p[21],
    })
}

/// The kind byte of a packet of this protocol; None for anything else.
fn kind(p: &[u8]) -> Option<u8> {
    (p.len() >= 5 && p[..4] == PROTOCOL.to_le_bytes()).then(|| p[4])
}

/// The u64 at bytes 5..13, the salt or session every control packet carries.
fn word(p: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(p.get(5..13)?.try_into().ok()?))
}

fn control(kind: u8, words: &[u64], pad_to: usize) -> Vec<u8> {
    let mut p = PROTOCOL.to_le_bytes().to_vec();
    p.push(kind);
    for w in words {
        p.extend_from_slice(&w.to_le_bytes());
    }
    p.resize(p.len().max(pad_to), 0);
    p
}

/// A random 64-bit salt, from the standard library's per-process hash keys
/// and the clock.
fn salt() -> u64 {
    let mut h = RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    h.finish() | 1
}

/// What a [`Rudp`] poll found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RudpEvent {
    /// A connection is up: one this side asked for with [`Rudp::connect`],
    /// or one a peer asked for and this side accepted.
    Joined(PeerId, SocketAddr),
    /// A message from a peer.
    Message {
        /// Who sent it.
        peer: PeerId,
        /// The channel it came on.
        channel: u8,
        /// The payload.
        data: Vec<u8>,
    },
    /// A connection is over, or a connect failed.
    Left(PeerId, Closed),
}

/// A peer's side of the handshake.
enum State {
    /// This side asked; waiting for the accept.
    Connecting { started: Instant, last_try: Option<Instant>, pending: Vec<(u8, Vec<u8>)> },
    /// Up.
    Connected(Box<Connection>),
}

struct Peer {
    addr: SocketAddr,
    /// The client's salt: ours when we connected, theirs when they did.
    client_salt: u64,
    /// The server's salt, once known.
    server_salt: u64,
    state: State,
}

/// A UDP endpoint carrying [`Connection`]s: a server, a client, or both.
pub struct Rudp {
    udp: Udp,
    /// The settings every connection is made with.
    pub config: RudpConfig,
    peers: BTreeMap<PeerId, Peer>,
    by_addr: HashMap<SocketAddr, PeerId>,
    next: u32,
    left: Vec<(PeerId, Closed)>,
}

impl Rudp {
    /// Binds `addr` (`0.0.0.0:PORT` for a server, `0.0.0.0:0` for a client).
    pub fn bind(addr: impl ToSocketAddrs, config: RudpConfig) -> io::Result<Rudp> {
        Ok(Rudp {
            udp: Udp::bind(addr)?,
            config,
            peers: BTreeMap::new(),
            by_addr: HashMap::new(),
            next: 1,
            left: Vec::new(),
        })
    }

    /// The address bound.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.udp.local_addr()
    }

    /// Starts connecting to `addr`. The peer's id is returned at once;
    /// [`RudpEvent::Joined`] or [`RudpEvent::Left`] follows from a later
    /// poll. Messages sent before then wait for the connection.
    pub fn connect(&mut self, addr: impl ToSocketAddrs) -> io::Result<PeerId> {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address to connect to"))?;
        let state = State::Connecting { started: Instant::now(), last_try: None, pending: Vec::new() };
        Ok(self.add(Peer { addr, client_salt: salt(), server_salt: 0, state }))
    }

    fn add(&mut self, peer: Peer) -> PeerId {
        let id = PeerId(self.next);
        self.next += 1;
        self.by_addr.insert(peer.addr, id);
        self.peers.insert(id, peer);
        id
    }

    fn remove(&mut self, id: PeerId) -> Option<Peer> {
        let p = self.peers.remove(&id)?;
        if self.by_addr.get(&p.addr) == Some(&id) {
            self.by_addr.remove(&p.addr);
        }
        Some(p)
    }

    /// The peers connected or connecting, and their addresses.
    pub fn peers(&self) -> impl Iterator<Item = (PeerId, SocketAddr)> + '_ {
        self.peers.iter().map(|(id, p)| (*id, p.addr))
    }

    /// Whether `peer` has finished the handshake.
    pub fn is_connected(&self, peer: PeerId) -> bool {
        self.peers.get(&peer).is_some_and(|p| matches!(p.state, State::Connected(_)))
    }

    /// A connected peer's [`Stats`].
    pub fn stats(&self, peer: PeerId) -> Option<Stats> {
        match &self.peers.get(&peer)?.state {
            State::Connected(c) => Some(c.stats()),
            State::Connecting { .. } => None,
        }
    }

    /// Queues `data` for `peer` on `channel`. Sent on the next poll, or once
    /// the connection is up.
    pub fn send(&mut self, peer: PeerId, channel: u8, data: &[u8]) -> Result<(), SendError> {
        let max = self.config.limit(channel).ok_or(SendError::NoChannel(channel))?;
        if data.len() > max {
            return Err(SendError::TooLarge { len: data.len(), max });
        }
        match &mut self.peers.get_mut(&peer).ok_or(SendError::NoPeer)?.state {
            State::Connected(c) => c.send(channel, data),
            State::Connecting { pending, .. } => {
                pending.push((channel, data.to_vec()));
                Ok(())
            }
        }
    }

    /// Queues `data` for every peer on `channel`.
    pub fn broadcast(&mut self, channel: u8, data: &[u8]) -> Result<(), SendError> {
        let ids: Vec<PeerId> = self.peers.keys().copied().collect();
        for id in ids {
            self.send(id, channel, data)?;
        }
        Ok(())
    }

    /// Ends the connection to `peer`, telling it so; the next poll reports
    /// [`Closed::Local`].
    pub fn disconnect(&mut self, peer: PeerId) {
        let Some(p) = self.remove(peer) else { return };
        if let State::Connected(c) = &p.state {
            // Three times: it is not resent, and the far side otherwise waits
            // out the timeout.
            let bye = control(DISCONNECT, &[c.session()], 0);
            for _ in 0..3 {
                let _ = self.udp.send_to(&bye, p.addr);
            }
        }
        self.left.push((peer, Closed::Local));
    }

    /// Receives, answers handshakes, times out the silent, sends what is due,
    /// at the current time.
    pub fn poll(&mut self) -> Vec<RudpEvent> {
        self.poll_at(Instant::now())
    }

    /// [`Rudp::poll`] at `now`.
    pub fn poll_at(&mut self, now: Instant) -> Vec<RudpEvent> {
        let mut events: Vec<RudpEvent> = self.left.drain(..).map(|(id, why)| RudpEvent::Left(id, why)).collect();
        loop {
            match self.udp.recv() {
                Ok(Some(d)) => self.arrive(&d.data, d.from, now, &mut events),
                Ok(None) => break,
                Err(_) => break,
            }
        }
        let ids: Vec<PeerId> = self.peers.keys().copied().collect();
        for id in ids {
            let Some(p) = self.peers.get_mut(&id) else { continue };
            let addr = p.addr;
            match &mut p.state {
                State::Connecting { started, last_try, .. } => {
                    if now.saturating_duration_since(*started) >= self.config.timeout {
                        self.remove(id);
                        events.push(RudpEvent::Left(id, Closed::Timeout));
                    } else if last_try.is_none_or(|t| now.saturating_duration_since(t) >= self.config.connect_retry) {
                        *last_try = Some(now);
                        let _ = self.udp.send_to(&control(CONNECT, &[p.client_salt], CONNECT_LEN), addr);
                    }
                }
                State::Connected(c) => {
                    if c.idle(now) >= self.config.timeout {
                        self.remove(id);
                        events.push(RudpEvent::Left(id, Closed::Timeout));
                    } else {
                        for packet in c.transmit(now) {
                            let _ = self.udp.send_to(&packet, addr);
                        }
                    }
                }
            }
        }
        events
    }

    /// One datagram, routed by kind.
    fn arrive(&mut self, p: &[u8], from: SocketAddr, now: Instant, events: &mut Vec<RudpEvent>) {
        let Some(kind) = kind(p) else { return };
        let known = self.by_addr.get(&from).copied();
        match kind {
            CONNECT if p.len() >= CONNECT_LEN => {
                let Some(client_salt) = word(p) else { return };
                if let Some(id) = known {
                    let peer = &self.peers[&id];
                    if peer.client_salt == client_salt {
                        // Our accept was lost: send it again.
                        if matches!(peer.state, State::Connected(_)) {
                            let _ = self.udp.send_to(&control(ACCEPT, &[client_salt, peer.server_salt], 0), from);
                        }
                        return;
                    }
                    // A new salt from a known address: the peer restarted.
                    self.remove(id);
                    events.push(RudpEvent::Left(id, Closed::Peer));
                }
                if !self.config.accept || self.peers.len() >= self.config.max_peers {
                    let _ = self.udp.send_to(&control(DENY, &[client_salt], 0), from);
                    return;
                }
                let server_salt = salt();
                let conn = Connection::new(client_salt ^ server_salt, &self.config, now);
                let id =
                    self.add(Peer { addr: from, client_salt, server_salt, state: State::Connected(Box::new(conn)) });
                let _ = self.udp.send_to(&control(ACCEPT, &[client_salt, server_salt], 0), from);
                events.push(RudpEvent::Joined(id, from));
            }
            ACCEPT => {
                let (Some(id), Some(client_salt), Some(server_salt)) =
                    (known, word(p), p.get(13..21).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes))
                else {
                    return;
                };
                let peer = self.peers.get_mut(&id).unwrap();
                if peer.client_salt != client_salt {
                    return;
                }
                if let State::Connecting { pending, .. } = &mut peer.state {
                    let mut conn = Connection::new(client_salt ^ server_salt, &self.config, now);
                    for (channel, data) in pending.drain(..) {
                        // Checked when queued.
                        let _ = conn.send(channel, &data);
                    }
                    peer.server_salt = server_salt;
                    peer.state = State::Connected(Box::new(conn));
                    events.push(RudpEvent::Joined(id, from));
                }
            }
            DENY => {
                if let (Some(id), Some(salt)) = (known, word(p))
                    && self.peers[&id].client_salt == salt
                    && matches!(self.peers[&id].state, State::Connecting { .. })
                {
                    self.remove(id);
                    events.push(RudpEvent::Left(id, Closed::Refused));
                }
            }
            DATA => {
                let Some(id) = known else { return };
                if let State::Connected(c) = &mut self.peers.get_mut(&id).unwrap().state {
                    for (channel, data) in c.receive(p, now) {
                        events.push(RudpEvent::Message { peer: id, channel, data });
                    }
                }
            }
            DISCONNECT => {
                if let (Some(id), Some(session)) = (known, word(p))
                    && let State::Connected(c) = &self.peers[&id].state
                    && c.session() == session
                {
                    self.remove(id);
                    events.push(RudpEvent::Left(id, Closed::Peer));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(16);

    fn pair() -> (Connection, Connection, Instant) {
        let now = Instant::now();
        let config = RudpConfig::default();
        (Connection::new(7, &config, now), Connection::new(7, &config, now), now)
    }

    /// A link that loses, duplicates and reorders packets. With no seed, on
    /// a fixed pattern: of every 7 packets the 3rd is lost and the 5th
    /// arrives twice. With a seed, at random (xorshift, so the run repeats):
    /// 30% lost, 10% doubled. Either way each pair that gets through is
    /// swapped.
    #[derive(Default)]
    struct Link {
        count: u64,
        seed: Option<u64>,
    }

    impl Link {
        fn random(seed: u64) -> Link {
            Link { count: 0, seed: Some(seed) }
        }

        /// 0..100.
        fn roll(&mut self) -> u64 {
            let s = self.seed.as_mut().unwrap();
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            *s % 100
        }

        fn carry(&mut self, packets: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
            let mut out = Vec::new();
            for p in packets {
                self.count += 1;
                let (lose, double) = match self.seed {
                    None => (self.count % 7 == 3, self.count % 7 == 5),
                    Some(_) => {
                        let r = self.roll();
                        (r < 30, r >= 90)
                    }
                };
                if lose {
                    continue;
                }
                if double {
                    out.push(p.clone());
                }
                out.push(p);
            }
            for pair in out.chunks_mut(2) {
                pair.reverse();
            }
            out
        }
    }

    /// Ticks both ends over pattern-lossy links in both directions until
    /// `done`, or panics after `limit` ticks. Returns what B received.
    fn run(
        a: &mut Connection,
        b: &mut Connection,
        now: &mut Instant,
        limit: u32,
        done: impl FnMut(&[(u8, Vec<u8>)]) -> bool,
    ) -> Vec<(u8, Vec<u8>)> {
        run_over(a, b, now, limit, (Link::default(), Link::default()), done)
    }

    fn run_over(
        a: &mut Connection,
        b: &mut Connection,
        now: &mut Instant,
        limit: u32,
        (mut ab, mut ba): (Link, Link),
        mut done: impl FnMut(&[(u8, Vec<u8>)]) -> bool,
    ) -> Vec<(u8, Vec<u8>)> {
        let mut got = Vec::new();
        for _ in 0..limit {
            *now += TICK;
            for p in ab.carry(a.transmit(*now)) {
                got.extend(b.receive(&p, *now));
            }
            for p in ba.carry(b.transmit(*now)) {
                a.receive(&p, *now);
            }
            if done(&got) {
                return got;
            }
        }
        panic!("not done after {limit} ticks: {} received", got.len());
    }

    #[test]
    fn reliable_ordered_survives_loss_duplication_and_reordering() {
        let (mut a, mut b, mut now) = pair();
        let sent: Vec<Vec<u8>> = (0..300u32).map(|i| i.to_le_bytes().to_vec()).collect();
        for m in &sent {
            a.send(0, m).unwrap();
        }
        let got = run(&mut a, &mut b, &mut now, 2000, |g| g.len() == sent.len());
        assert_eq!(got.into_iter().map(|(_, d)| d).collect::<Vec<_>>(), sent);
        assert!(a.stats().resent > 0, "the lossy link forced resends");
        // Nothing left in flight once the acks get back over a clean link.
        for _ in 0..50 {
            now += TICK;
            for p in b.transmit(now) {
                a.receive(&p, now);
            }
            for p in a.transmit(now) {
                b.receive(&p, now);
            }
        }
        assert_eq!(a.stats().in_flight, 0);
        assert_eq!(a.stats().acked, 300);
    }

    #[test]
    fn heavy_random_loss_still_delivers_everything_in_order() {
        for seed in [1, 0x5eed, 0xdead_beef] {
            let (mut a, mut b, mut now) = pair();
            let sent: Vec<Vec<u8>> = (0..500u32).map(|i| i.to_le_bytes().to_vec()).collect();
            for m in &sent {
                a.send(0, m).unwrap();
                a.send(1, b"noise").unwrap();
            }
            let links = (Link::random(seed), Link::random(seed ^ 0xffff));
            let got = run_over(&mut a, &mut b, &mut now, 5000, links, |g| {
                g.iter().filter(|(c, _)| *c == 0).count() == sent.len()
            });
            let reliable: Vec<Vec<u8>> = got.into_iter().filter(|(c, _)| *c == 0).map(|(_, d)| d).collect();
            assert_eq!(reliable, sent, "seed {seed}");
        }
    }

    /// Bytes 0..n as a pattern that shows a misplaced fragment.
    fn pattern(n: usize, salt: u8) -> Vec<u8> {
        (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(salt)).collect()
    }

    #[test]
    fn a_large_ordered_message_is_fragmented_and_reassembled_in_place() {
        let (mut a, mut b, mut now) = pair();
        let big = pattern(100_000, 7);
        a.send(0, b"before").unwrap();
        a.send(0, &big).unwrap();
        a.send(0, b"after").unwrap();
        let got = run(&mut a, &mut b, &mut now, 5000, |g| g.len() == 3);
        assert_eq!(got, [(0, b"before".to_vec()), (0, big), (0, b"after".to_vec())]);
    }

    #[test]
    fn large_unordered_messages_reassemble_whole() {
        let (mut a, mut b, mut now) = pair();
        let (x, y) = (pattern(50_000, 1), pattern(30_001, 2));
        a.send(3, &x).unwrap();
        a.send(3, &y).unwrap();
        let links = (Link::random(42), Link::random(43));
        let mut got = run_over(&mut a, &mut b, &mut now, 5000, links, |g| g.len() == 2);
        got.sort_by_key(|(_, d)| d.len());
        assert_eq!(got, [(3, y), (3, x)]);
    }

    #[test]
    fn a_message_of_more_fragments_than_the_window_gets_through() {
        let (mut a, mut b, mut now) = pair();
        let config = RudpConfig::default();
        let fragment = config.max_message() - FRAGMENT_HEADER;
        let big = pattern(fragment * (usize::from(WINDOW) + 100), 9);
        a.max_reliable = big.len();
        a.send(0, &big).unwrap();
        let got = run(&mut a, &mut b, &mut now, 20_000, |g| !g.is_empty());
        assert_eq!(got, [(0, big)]);
    }

    #[test]
    fn limits_are_per_delivery() {
        let (mut a, _, _) = pair();
        let config = RudpConfig::default();
        let (single, reliable) = (config.max_message(), config.max_reliable_message);
        assert_eq!(config.limit(0), Some(reliable));
        assert_eq!(config.limit(1), Some(single));
        assert_eq!(config.limit(4), None);
        assert!(a.send(0, &vec![0; reliable]).is_ok());
        assert_eq!(a.send(0, &vec![0; reliable + 1]), Err(SendError::TooLarge { len: reliable + 1, max: reliable }));
        assert_eq!(a.send(1, &vec![0; single + 1]), Err(SendError::TooLarge { len: single + 1, max: single }));
    }

    #[test]
    fn a_fragment_on_an_unreliable_channel_is_malformed() {
        let (mut a, mut b, now) = pair();
        a.send(0, &pattern(3000, 0)).unwrap();
        let mut p = a.transmit(now).remove(0);
        assert_eq!(p[DATA_HEADER], FRAGMENT, "channel 0, fragment flag");
        p[DATA_HEADER] = 1 | FRAGMENT;
        assert!(b.receive(&p, now).is_empty());
        assert_eq!(b.stats().packets_received, 0);
    }

    /// 600 kB at 120 kB/s with a 12 kB burst: no transmit exceeds what the
    /// budget allows, and the whole arrives in about five seconds.
    #[test]
    fn pacing_spreads_a_large_send_over_time() {
        let config = RudpConfig { bandwidth: Some(120_000), ..RudpConfig::default() };
        let mut now = Instant::now();
        let (mut a, mut b) = (Connection::new(1, &config, now), Connection::new(1, &config, now));
        let start = now;
        let big = pattern(600_000, 3);
        a.send(0, &big).unwrap();
        let mut got = Vec::new();
        let mut budget = burst(120_000, config.mtu);
        let mut last = now;
        while got.is_empty() {
            now += TICK;
            budget = (budget + 120_000.0 * (now - last).as_secs_f64()).min(burst(120_000, config.mtu));
            last = now;
            let packets = a.transmit(now);
            let bytes: usize = packets.iter().map(|p| p.len() - DATA_HEADER).sum();
            assert!(bytes as f64 <= budget + 1.0, "{bytes} bytes sent with {budget} in the budget");
            budget -= bytes as f64;
            for p in packets {
                got.extend(b.receive(&p, now));
            }
            for p in b.transmit(now) {
                a.receive(&p, now);
            }
            assert!(now - start < Duration::from_secs(10), "stalled");
        }
        assert_eq!(got, [(0, big)]);
        let took = (now - start).as_secs_f64();
        assert!((4.5..6.0).contains(&took), "took {took} s");
    }

    #[test]
    fn unreliable_messages_over_the_budget_are_dropped_and_counted() {
        let config = RudpConfig { bandwidth: Some(12_000), mtu: 1200, ..RudpConfig::default() };
        let now = Instant::now();
        let mut a = Connection::new(1, &config, now);
        for _ in 0..10 {
            a.send(1, &[0; 1000]).unwrap();
        }
        let sent = a.transmit(now).len();
        assert_eq!(sent, 1, "the 1200-byte burst holds one message");
        assert_eq!(a.stats().dropped, 9);
    }

    #[test]
    fn reliable_unordered_delivers_each_message_exactly_once() {
        let (mut a, mut b, mut now) = pair();
        for i in 0..300u32 {
            a.send(3, &i.to_le_bytes()).unwrap();
        }
        let got = run(&mut a, &mut b, &mut now, 2000, |g| g.len() >= 300);
        let mut ids: Vec<u32> = got.iter().map(|(_, d)| u32::from_le_bytes(d[..4].try_into().unwrap())).collect();
        ids.sort();
        assert_eq!(ids, (0..300).collect::<Vec<_>>());
    }

    #[test]
    fn more_than_a_window_of_messages_waits_its_turn() {
        let (mut a, mut b, mut now) = pair();
        let n = u32::from(WINDOW) * 3;
        for i in 0..n {
            a.send(0, &i.to_le_bytes()).unwrap();
        }
        let got = run(&mut a, &mut b, &mut now, 5000, |g| g.len() == n as usize);
        assert!(got.iter().enumerate().all(|(i, (_, d))| d[..] == (i as u32).to_le_bytes()));
    }

    #[test]
    fn ids_and_packet_numbers_wrap_at_65536() {
        let (mut a, mut b, mut now) = pair();
        a.channels[0].next_id = 65530;
        b.channels[0].next_expected = 65530;
        a.local_seq = 65530;
        for i in 0..20u8 {
            a.send(0, &[i]).unwrap();
            now += TICK;
            for p in a.transmit(now) {
                b.receive(&p, now);
            }
            for p in b.transmit(now) {
                a.receive(&p, now);
            }
        }
        assert_eq!(b.channels[0].next_expected, 65530u16.wrapping_add(20));
        assert_eq!(a.stats().acked, 20);
    }

    #[test]
    fn sequenced_drops_what_arrives_after_a_newer_one() {
        let (mut a, mut b, now) = pair();
        a.send(2, b"old").unwrap();
        let first = a.transmit(now);
        a.send(2, b"new").unwrap();
        let second = a.transmit(now);
        assert_eq!(b.receive(&second[0], now), [(2, b"new".to_vec())]);
        assert_eq!(b.receive(&first[0], now), []);
    }

    #[test]
    fn unreliable_is_sent_once_and_never_again() {
        let (mut a, mut b, mut now) = pair();
        a.send(1, b"gone").unwrap();
        let lost = a.transmit(now);
        assert_eq!(lost.len(), 1);
        now += Duration::from_secs(1);
        for p in a.transmit(now) {
            assert!(b.receive(&p, now).is_empty(), "only a keepalive, no message");
        }
    }

    #[test]
    fn a_duplicated_packet_delivers_once() {
        let (mut a, mut b, now) = pair();
        a.send(0, b"x").unwrap();
        let p = a.transmit(now).remove(0);
        assert_eq!(b.receive(&p, now).len(), 1);
        assert_eq!(b.receive(&p, now).len(), 0);
    }

    #[test]
    fn another_sessions_packets_change_nothing() {
        let now = Instant::now();
        let config = RudpConfig::default();
        let (mut a, mut b) = (Connection::new(1, &config, now), Connection::new(2, &config, now));
        a.send(0, b"x").unwrap();
        let p = a.transmit(now).remove(0);
        assert!(b.receive(&p, now + TICK).is_empty());
        assert_eq!(b.stats().packets_received, 0);
        assert_eq!(b.idle(now + TICK), TICK, "a stray packet does not keep it alive");
    }

    #[test]
    fn truncated_packets_are_ignored() {
        let (mut a, mut b, now) = pair();
        a.send(0, b"hello").unwrap();
        let p = a.transmit(now).remove(0);
        for len in 0..p.len() {
            assert!(b.receive(&p[..len], now).is_empty());
        }
        assert_eq!(b.stats().packets_received, 0);
        assert_eq!(b.receive(&p, now), [(0, b"hello".to_vec())]);
    }

    #[test]
    fn oversized_unreliable_and_unknown_channel_sends_are_refused() {
        let (mut a, _, _) = pair();
        let max = RudpConfig::default().max_message();
        assert_eq!(max, 1173);
        assert!(a.send(1, &vec![0; max]).is_ok());
        assert_eq!(a.send(1, &vec![0; max + 1]), Err(SendError::TooLarge { len: max + 1, max }));
        assert_eq!(a.send(9, b"x"), Err(SendError::NoChannel(9)));
    }

    #[test]
    fn many_messages_split_across_packets_within_the_mtu() {
        let (mut a, _, now) = pair();
        for _ in 0..10 {
            a.send(0, &[0; 1000]).unwrap();
        }
        let packets = a.transmit(now);
        assert_eq!(packets.len(), 10);
        assert!(packets.iter().all(|p| p.len() <= 1200));
    }

    #[test]
    fn an_idle_connection_sends_only_keepalives() {
        let (mut a, _, mut now) = pair();
        assert_eq!(a.transmit(now).len(), 1, "the first transmit announces the connection");
        now += TICK;
        assert!(a.transmit(now).is_empty());
        now += RudpConfig::default().keepalive;
        assert_eq!(a.transmit(now).len(), 1);
    }

    #[test]
    fn ack_bits_cover_the_32_packets_before_the_newest() {
        let (_, mut b, _) = pair();
        for seq in [10u16, 12, 11, 42, 9] {
            b.note_received(seq);
        }
        assert_eq!(b.remote_seq, 42);
        // 12 is 30 back (bit 29), 11 bit 30, 10 bit 31; 9 is 33 back, too old.
        assert_eq!(b.ack_bits, (1 << 29) | (1 << 30) | (1 << 31));
    }

    #[test]
    fn newer_wraps() {
        assert!(newer(1, 0));
        assert!(newer(0, 65535));
        assert!(!newer(65535, 0));
        assert!(!newer(5, 5));
    }

    // Over real sockets, on loopback.

    fn poll_until(eps: &mut [&mut Rudp], mut done: impl FnMut(&[Vec<RudpEvent>]) -> bool) -> Vec<Vec<RudpEvent>> {
        let mut seen = vec![Vec::new(); eps.len()];
        for _ in 0..400 {
            for (i, e) in eps.iter_mut().enumerate() {
                seen[i].extend(e.poll());
            }
            if done(&seen) {
                return seen;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out: {seen:?}");
    }

    fn server(config: RudpConfig) -> (Rudp, SocketAddr) {
        let s = Rudp::bind("127.0.0.1:0", config).unwrap();
        let addr = s.local_addr().unwrap();
        (s, addr)
    }

    #[test]
    fn a_client_connects_and_messages_flow_both_ways() {
        let (mut s, addr) = server(RudpConfig::default());
        let mut c = Rudp::bind("127.0.0.1:0", RudpConfig { accept: false, ..RudpConfig::default() }).unwrap();
        let to_server = c.connect(addr).unwrap();
        c.send(to_server, 0, b"queued before the handshake").unwrap();
        let seen = poll_until(&mut [&mut s, &mut c], |seen| {
            seen[0].iter().any(|e| matches!(e, RudpEvent::Message { .. })) && !seen[1].is_empty()
        });
        let Some(RudpEvent::Joined(client, _)) = seen[0].first() else { panic!("{seen:?}") };
        assert_eq!(seen[1][0], RudpEvent::Joined(to_server, addr));
        assert!(seen[0].contains(&RudpEvent::Message {
            peer: *client,
            channel: 0,
            data: b"queued before the handshake".to_vec()
        }));
        s.send(*client, 0, b"welcome").unwrap();
        poll_until(&mut [&mut s, &mut c], |seen| {
            seen[1].contains(&RudpEvent::Message { peer: to_server, channel: 0, data: b"welcome".to_vec() })
        });
        assert!(s.stats(*client).unwrap().packets_received > 0);
    }

    #[test]
    fn a_full_server_refuses() {
        let (mut s, addr) = server(RudpConfig { max_peers: 0, ..RudpConfig::default() });
        let mut c = Rudp::bind("127.0.0.1:0", RudpConfig::default()).unwrap();
        let id = c.connect(addr).unwrap();
        poll_until(&mut [&mut s, &mut c], |seen| seen[1].contains(&RudpEvent::Left(id, Closed::Refused)));
        assert_eq!(c.peers().count(), 0);
    }

    #[test]
    fn a_disconnect_is_seen_by_the_other_side() {
        let (mut s, addr) = server(RudpConfig::default());
        let mut c = Rudp::bind("127.0.0.1:0", RudpConfig::default()).unwrap();
        let id = c.connect(addr).unwrap();
        let seen = poll_until(&mut [&mut s, &mut c], |seen| !seen[0].is_empty() && !seen[1].is_empty());
        let Some(RudpEvent::Joined(client, _)) = seen[0].first() else { panic!("{seen:?}") };
        c.disconnect(id);
        poll_until(&mut [&mut s, &mut c], |seen| {
            seen[0].contains(&RudpEvent::Left(*client, Closed::Peer))
                && seen[1].contains(&RudpEvent::Left(id, Closed::Local))
        });
    }

    #[test]
    fn connecting_to_nobody_times_out() {
        // A port bound and never polled: nothing answers.
        let silent = Udp::bind("127.0.0.1:0").unwrap();
        let config = RudpConfig { timeout: Duration::from_millis(150), ..RudpConfig::default() };
        let mut c = Rudp::bind("127.0.0.1:0", config).unwrap();
        let id = c.connect(silent.local_addr().unwrap()).unwrap();
        poll_until(&mut [&mut c], |seen| seen[0].contains(&RudpEvent::Left(id, Closed::Timeout)));
    }

    #[test]
    fn a_peer_that_goes_silent_times_out() {
        let config = RudpConfig { timeout: Duration::from_millis(200), ..RudpConfig::default() };
        let (mut s, addr) = server(config.clone());
        let mut c = Rudp::bind("127.0.0.1:0", config).unwrap();
        let id = c.connect(addr).unwrap();
        poll_until(&mut [&mut s, &mut c], |seen| !seen[0].is_empty() && !seen[1].is_empty());
        // The server stops polling, so it stops answering.
        poll_until(&mut [&mut c], |seen| seen[0].contains(&RudpEvent::Left(id, Closed::Timeout)));
    }
}
