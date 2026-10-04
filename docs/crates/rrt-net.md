---
title: rrt-net, networking for a fixed-tick loop
status: solid
crates: rrt-net
covers: rrt_net::Rudp, rrt_net::Rudp::bind, rrt_net::Rudp::connect, rrt_net::Rudp::send, rrt_net::Rudp::broadcast, rrt_net::Rudp::disconnect, rrt_net::Rudp::poll, rrt_net::Rudp::poll_at, rrt_net::Rudp::stats, rrt_net::RudpConfig, rrt_net::RudpConfig::max_message, rrt_net::RudpEvent, rrt_net::Delivery, rrt_net::Connection, rrt_net::Connection::send, rrt_net::Connection::receive, rrt_net::Connection::transmit, rrt_net::Connection::rto, rrt_net::SendError, rrt_net::Stats, rrt_net::RudpConfig::limit, rrt_net::reliable::PROTOCOL, rrt_net::reliable::WINDOW, rrt_net::reliable::MAX_CHANNELS, rrt_net::Udp, rrt_net::Udp::bind_reusable, rrt_net::Udp::broadcast_to, rrt_net::Udp::set_multicast_interface, rrt_net::Udp::bind, rrt_net::Udp::send_to, rrt_net::Udp::broadcast, rrt_net::Udp::join_multicast, rrt_net::Udp::poll, rrt_net::Datagram, rrt_net::TcpClient, rrt_net::TcpClient::connect, rrt_net::TcpClient::send, rrt_net::TcpClient::poll, rrt_net::TcpClient::close, rrt_net::TcpServer, rrt_net::TcpServer::bind, rrt_net::TcpServer::poll, rrt_net::TcpServer::send, rrt_net::TcpServer::broadcast, rrt_net::TcpServer::broadcast_except, rrt_net::TcpServer::kick, rrt_net::ClientEvent, rrt_net::ServerEvent, rrt_net::Closed, rrt_net::PeerId, rrt_net::Framing, rrt_net::Decoder, rrt_net::encode
---

# rrt-net

Non-blocking sockets pumped by a `poll` the game calls from `Game::tick`:
plain UDP, TCP client and server, and reliable UDP with per-channel
delivery.
No threads, no async runtime, std only. A poll does the I/O that is ready and
returns what arrived; sends are queued and go out on the next poll.

```rust
fn tick(&mut self, t: &mut Tick) {
    for e in self.server.poll() {
        match e {
            ServerEvent::Joined(id, addr) => { /* ... */ }
            ServerEvent::Message(id, bytes) => self.server.broadcast_except(id, &bytes),
            ServerEvent::Left(id, why) => { /* ... */ }
        }
    }
}
```

## Reliable UDP

`Rudp` is an endpoint: one UDP socket, any number of peers, each a
connection after a handshake. Messages go on channels, and each channel's
`Delivery` decides what the far side sees:

| `Delivery` | arrives | resent | for |
| --- | --- | --- | --- |
| `Unreliable` | whatever gets through, as it gets through, perhaps twice | no | fire and forget |
| `Sequenced` | only messages newer than the newest delivered | no | state overwritten every tick: positions, the pad |
| `Reliable` | each message exactly once, any order | until acked | events whose order does not matter |
| `ReliableOrdered` | each message exactly once, in order; a gap holds back what follows | until acked | chat, game events, lockstep input |

`RudpConfig::default()`: channels 0 `ReliableOrdered`, 1 `Unreliable`,
2 `Sequenced`, 3 `Reliable`; `mtu` 1200 bytes; `timeout` 5 s; `keepalive`
250 ms; `connect_retry` 100 ms; `accept` true; `max_peers` unlimited;
`max_reliable_message` 1 MiB; `bandwidth` 4 MiB/s. Both ends must use the
same channels, at most `MAX_CHANNELS` (128). `RudpConfig::limit(channel)` is
the longest message a channel takes: `max_reliable_message` on a reliable
channel, `max_message()` (one packet: 1173 bytes at the default MTU) on the
others. Longer is `SendError::TooLarge`.

```rust
let mut net = Rudp::bind("0.0.0.0:7777", RudpConfig::default())?;   // server
let server = client.connect("192.168.1.10:7777")?;                  // client: Joined later
client.send(server, 0, b"hello")?;                                  // queued until up
for e in net.poll() {                                               // once a tick
    match e {
        RudpEvent::Joined(peer, addr) => {}
        RudpEvent::Message { peer, channel, data } => {}
        RudpEvent::Left(peer, why) => {}   // Peer, Local, Timeout, Refused
    }
}
```

### The protocol

All little-endian; every packet starts with `PROTOCOL` (`RRT1`) and a kind
byte, and anything else on the port is ignored.

```
connect     u32 protocol, u8 0, u64 client salt, zero padding to 32 bytes
accept      u32 protocol, u8 1, u64 client salt, u64 server salt
deny        u32 protocol, u8 2, u64 client salt
data        u32 protocol, u8 3, u64 session (client salt ^ server salt)
            u16 seq, u16 ack, u32 ack bits, u8 count
            message[count]: u8 channel (bit 7: fragment), u16 id, u16 len,
                            [u16 index, u16 fragments: fragments only], [u8; len]
disconnect  u32 protocol, u8 4, u64 session
```

The client repeats `connect` every `connect_retry` until `accept` or `deny`;
a server that sees a repeat from a connected client sends its `accept` again
(the first was lost), and a new salt from a known address replaces that peer
(it restarted). `connect` is padded past the size of `accept` so the endpoint
cannot amplify a spoofed flood. Data packets carry the session, so a stale or
stray packet changes nothing.

Every data packet acknowledges the newest packet received and, in its bits,
the 32 before it. A reliable message stays queued until a packet carrying it
is acknowledged, and is sent again when `Connection::rto()` passes: twice the
smoothed round trip, clamped to 30 ms - 1 s (100 ms assumed before the first
measurement). At most `WINDOW` (1024) reliable ids a channel are in flight,
counted from the oldest unacknowledged id, so everything sent lands in the
receiver's window; later messages wait. Message ids and packet numbers wrap
at 65536. An idle connection sends a bare packet every `keepalive`; one
heard nothing from for `timeout` is gone.

A reliable message longer than one packet is cut into fragments of
`max_message() - 4` bytes, each a reliable message with consecutive ids and
its index and count. An ordered channel joins them as they come off in
order; an unordered one collects them by the first fragment's id and
delivers the message when every fragment is in. Either way the receiver sees
the message whole, once. A fragment on an unreliable or sequenced channel
makes the packet malformed.

With `bandwidth` set, each connection spends at most that many bytes a second
on messages, with a burst of a tenth of a second's worth (never less than one
packet). A reliable message that does not fit waits for the next transmit,
oldest first; an unreliable or sequenced one is dropped, since it would be
stale, and counted in `Stats::dropped`. Acks and keepalives always go. This
is pacing at a set rate, not adaptive congestion control: a game's traffic is
small and steady, and the rate is the ceiling a large reliable message is
spread under.

`Connection` is the protocol with no I/O - `send`, `receive(bytes, now)`,
`transmit(now)` - for tests and for running it over another datagram
transport. `Rudp::poll_at(now)` is `poll` with the time given. `Stats` gives
the round trip, packets sent and received, messages resent and acked, and
what is in flight.

Tests, on a simulated link: `reliable_ordered_survives_loss_duplication_and_reordering`,
`a_large_ordered_message_is_fragmented_and_reassembled_in_place`,
`large_unordered_messages_reassemble_whole`,
`a_message_of_more_fragments_than_the_window_gets_through`, `limits_are_per_delivery`,
`a_fragment_on_an_unreliable_channel_is_malformed`,
`pacing_spreads_a_large_send_over_time` (600 kB at 120 kB/s: no transmit over
budget, delivered in 4.5-6 s), `unreliable_messages_over_the_budget_are_dropped_and_counted`,
`heavy_random_loss_still_delivers_everything_in_order` (30% loss, 10%
duplication, reordering, three seeds), `reliable_unordered_delivers_each_message_exactly_once`,
`more_than_a_window_of_messages_waits_its_turn`, `ids_and_packet_numbers_wrap_at_65536`,
`sequenced_drops_what_arrives_after_a_newer_one`, `unreliable_is_sent_once_and_never_again`,
`a_duplicated_packet_delivers_once`, `another_sessions_packets_change_nothing`,
`truncated_packets_are_ignored`, `oversized_unreliable_and_unknown_channel_sends_are_refused`,
`ack_bits_cover_the_32_packets_before_the_newest`, `newer_wraps`,
`many_messages_split_across_packets_within_the_mtu`, `an_idle_connection_sends_only_keepalives`.
Over loopback sockets: `a_client_connects_and_messages_flow_both_ways`,
`a_full_server_refuses`, `a_disconnect_is_seen_by_the_other_side`,
`connecting_to_nobody_times_out`, `a_peer_that_goes_silent_times_out`.

## UDP

`Udp::bind(addr)`; `Udp::bind_reusable(addr)` sets `SO_REUSEADDR` so several
sockets (two copies of a game on one machine) share a discovery port and
each hears its broadcasts and multicasts. `send_to` returns `Ok(false)` when
the send buffer is full and the datagram is dropped (as UDP may drop it
anyway). `broadcast(msg, port)` sends to 255.255.255.255 and
`broadcast_to(msg, address, port)` to a subnet's broadcast address, both
with broadcast turned on; `join_multicast(group, interface)`,
`leave_multicast`, `set_multicast_loop` and `set_multicast_interface` for
groups. Whether this host's own sockets hear a limited broadcast, or a
multicast sent out of the default interface, depends on the host's firewall:
the tests use loopback, where they always do. `recv` gives the next `Datagram { from, data }`, `poll` all of
them. A Windows `ConnectionReset` on read (an ICMP reply to an earlier send)
is skipped. `socket()` exposes the `UdpSocket` for other options.

## TCP

`TcpClient::connect(addr, framing, timeout)` blocks while connecting: call
it from a menu, not mid-game. `TcpServer::bind(addr, framing)` listens;
`max_peers` refuses clients past a count. Peers are numbered `PeerId(1)` up
in join order and never reused. Every socket has `TCP_NODELAY` set.

| event | when |
| --- | --- |
| `ClientEvent::Message(bytes)` | a whole message arrived |
| `ClientEvent::Closed(why)` | over; later polls return nothing |
| `ServerEvent::Joined(id, addr)` | accepted |
| `ServerEvent::Message(id, bytes)` | a whole message arrived |
| `ServerEvent::Left(id, why)` | over |

`Closed` is `Peer`, `Local` (`close`, `kick`), `TooLarge(n)` or
`Error(text)`; reliable UDP adds `Timeout` and `Refused`. `PeerId` and
`Closed` are shared by TCP and reliable UDP. Messages that arrived before a close are delivered before the
close event.

## Framing

```
Framing::LengthPrefixed { max }   u32 little-endian byte count, then the bytes
Framing::Raw                      bytes as one read returned them; sends written as they are
```

`Framing::DEFAULT` is length-prefixed with a 16 MiB limit; a prefix over
`max` closes the connection. `Raw` is for speaking an original game's own
protocol, which frames itself. `Decoder` and `encode` are public for
framing over another transport. Tests: `messages_survive_being_split_and_joined`,
`an_oversized_prefix_is_refused`, `raw_gives_what_is_buffered`,
`a_client_joins_talks_and_leaves`, `a_kicked_client_sees_the_close`,
`a_datagram_crosses_loopback`, `a_broadcast_reaches_a_listener`,
`a_multicast_reaches_every_member_sharing_a_port`.

## Not here

Encryption or authentication (the session number stops strays, not an
attacker who can read the traffic), NAT traversal, path-MTU discovery (1200
bytes fits every real path), adaptive congestion control (see pacing above).

## Gaps

Nothing known.
