//! Streams: a client to one server, a server to many clients.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::frame::{Decoder, Framing, TooLarge, encode};
use crate::{Closed, PeerId};

/// One connection's socket and buffers.
struct Conn {
    stream: TcpStream,
    framing: Framing,
    out: Vec<u8>,
    decoder: Decoder,
    scratch: Box<[u8]>,
}

impl Conn {
    fn new(stream: TcpStream, framing: Framing) -> io::Result<Conn> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Conn { stream, framing, out: Vec::new(), decoder: Decoder::new(framing), scratch: vec![0; 16384].into() })
    }

    fn send(&mut self, msg: &[u8]) {
        encode(self.framing, msg, &mut self.out);
    }

    /// Writes what is queued and reads what is ready; the messages that
    /// arrived go to `messages`. Err when the connection is over.
    fn pump(&mut self, messages: &mut Vec<Vec<u8>>) -> Result<(), Closed> {
        let io_err = |e: io::Error| Closed::Error(e.to_string());
        while !self.out.is_empty() {
            match self.stream.write(&self.out) {
                Ok(0) => return Err(Closed::Peer),
                Ok(n) => {
                    self.out.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(io_err(e)),
            }
        }
        let mut closed = None;
        loop {
            match self.stream.read(&mut self.scratch) {
                Ok(0) => {
                    closed = Some(Closed::Peer);
                    break;
                }
                Ok(n) => self.decoder.push(&self.scratch[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    closed = Some(io_err(e));
                    break;
                }
            }
        }
        // Messages that arrived before a close are still delivered.
        loop {
            match self.decoder.next_message() {
                Ok(Some(m)) => messages.push(m),
                Ok(None) => break,
                Err(TooLarge(n)) => return Err(Closed::TooLarge(n)),
            }
        }
        closed.map_or(Ok(()), Err)
    }
}

/// What a [`TcpClient`] poll found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientEvent {
    /// A message from the server.
    Message(Vec<u8>),
    /// The connection is over; later polls return nothing.
    Closed(Closed),
}

/// A connection to a server.
pub struct TcpClient {
    conn: Option<Conn>,
    peer: SocketAddr,
}

impl TcpClient {
    /// Connects to `addr`, waiting up to `timeout` for each address it
    /// resolves to. This blocks: call it from a menu or a loading screen,
    /// not mid-game.
    pub fn connect(addr: impl ToSocketAddrs, framing: Framing, timeout: Duration) -> io::Result<TcpClient> {
        let mut last = io::Error::new(io::ErrorKind::InvalidInput, "no address to connect to");
        for a in addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&a, timeout) {
                Ok(s) => return TcpClient::from_stream(s, framing),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// A client over a stream already connected.
    pub fn from_stream(stream: TcpStream, framing: Framing) -> io::Result<TcpClient> {
        let peer = stream.peer_addr()?;
        Ok(TcpClient { conn: Some(Conn::new(stream, framing)?), peer })
    }

    /// The server's address.
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    /// Still connected, as of the last poll.
    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// Queues `msg`; it goes out on the next poll. Nothing once closed.
    pub fn send(&mut self, msg: &[u8]) {
        if let Some(c) = &mut self.conn {
            c.send(msg);
        }
    }

    /// Sends what is queued and returns what arrived.
    pub fn poll(&mut self) -> Vec<ClientEvent> {
        let Some(c) = &mut self.conn else { return Vec::new() };
        let mut messages = Vec::new();
        let result = c.pump(&mut messages);
        let mut events: Vec<ClientEvent> = messages.into_iter().map(ClientEvent::Message).collect();
        if let Err(why) = result {
            self.conn = None;
            events.push(ClientEvent::Closed(why));
        }
        events
    }

    /// Closes the connection, sending what is queued first if the socket
    /// takes it at once.
    pub fn close(&mut self) {
        if let Some(mut c) = self.conn.take() {
            let _ = c.pump(&mut Vec::new());
            let _ = c.stream.shutdown(Shutdown::Both);
        }
    }
}

/// What a [`TcpServer`] poll found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerEvent {
    /// A client connected.
    Joined(PeerId, SocketAddr),
    /// A message from a client.
    Message(PeerId, Vec<u8>),
    /// A client's connection is over.
    Left(PeerId, Closed),
}

/// A listening socket and its clients.
pub struct TcpServer {
    listener: TcpListener,
    framing: Framing,
    peers: BTreeMap<PeerId, (Conn, SocketAddr)>,
    next: u32,
    kicked: Vec<PeerId>,
    /// Clients past this many are refused (accepted and closed at once).
    pub max_peers: usize,
}

impl TcpServer {
    /// Listens on `addr` (`0.0.0.0:PORT`; port 0 for any).
    pub fn bind(addr: impl ToSocketAddrs, framing: Framing) -> io::Result<TcpServer> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(TcpServer { listener, framing, peers: BTreeMap::new(), next: 1, kicked: Vec::new(), max_peers: usize::MAX })
    }

    /// The address listened on, with the port the system chose for port 0.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// The connected clients and their addresses.
    pub fn peers(&self) -> impl Iterator<Item = (PeerId, SocketAddr)> + '_ {
        self.peers.iter().map(|(id, (_, addr))| (*id, *addr))
    }

    /// Queues `msg` for `peer`. False when there is no such client.
    pub fn send(&mut self, peer: PeerId, msg: &[u8]) -> bool {
        self.peers.get_mut(&peer).map(|(c, _)| c.send(msg)).is_some()
    }

    /// Queues `msg` for every client.
    pub fn broadcast(&mut self, msg: &[u8]) {
        for (c, _) in self.peers.values_mut() {
            c.send(msg);
        }
    }

    /// Queues `msg` for every client but `except` (relaying one client's
    /// input to the rest).
    pub fn broadcast_except(&mut self, except: PeerId, msg: &[u8]) {
        for (id, (c, _)) in &mut self.peers {
            if *id != except {
                c.send(msg);
            }
        }
    }

    /// Disconnects `peer`; the next poll reports it as [`Closed::Local`].
    pub fn kick(&mut self, peer: PeerId) {
        if let Some((c, _)) = self.peers.get_mut(&peer) {
            c.out.clear();
            let _ = c.stream.shutdown(Shutdown::Both);
            self.kicked.push(peer);
        }
    }

    /// Accepts who is waiting, sends what is queued, and returns what
    /// happened.
    pub fn poll(&mut self) -> Vec<ServerEvent> {
        let mut events = Vec::new();
        for id in std::mem::take(&mut self.kicked) {
            if self.peers.remove(&id).is_some() {
                events.push(ServerEvent::Left(id, Closed::Local));
            }
        }
        loop {
            match self.listener.accept() {
                Ok((stream, addr)) => {
                    if self.peers.len() >= self.max_peers {
                        let _ = stream.shutdown(Shutdown::Both);
                        continue;
                    }
                    // One that cannot be made non-blocking is dropped, which
                    // closes it.
                    if let Ok(c) = Conn::new(stream, self.framing) {
                        let id = PeerId(self.next);
                        self.next += 1;
                        self.peers.insert(id, (c, addr));
                        events.push(ServerEvent::Joined(id, addr));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let mut gone = Vec::new();
        for (id, (c, _)) in &mut self.peers {
            let mut messages = Vec::new();
            let result = c.pump(&mut messages);
            events.extend(messages.into_iter().map(|m| ServerEvent::Message(*id, m)));
            if let Err(why) = result {
                gone.push((*id, why));
            }
        }
        for (id, why) in gone {
            self.peers.remove(&id);
            events.push(ServerEvent::Left(id, why));
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Polls both ends until `done` says so, or panics after a second.
    fn until(
        server: &mut TcpServer,
        client: &mut TcpClient,
        mut done: impl FnMut(&[ServerEvent], &[ClientEvent]) -> bool,
    ) {
        let (mut s, mut c) = (Vec::new(), Vec::new());
        for _ in 0..200 {
            s.extend(server.poll());
            c.extend(client.poll());
            if done(&s, &c) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out: server saw {s:?}, client saw {c:?}");
    }

    #[test]
    fn a_client_joins_talks_and_leaves() {
        let mut server = TcpServer::bind("127.0.0.1:0", Framing::DEFAULT).unwrap();
        let addr = server.local_addr().unwrap();
        let mut client = TcpClient::connect(addr, Framing::DEFAULT, Duration::from_secs(1)).unwrap();
        client.send(b"hello");
        client.send(b"again");
        let mut peer = None;
        until(&mut server, &mut client, |s, _| {
            if let Some(ServerEvent::Joined(id, _)) = s.first() {
                peer = Some(*id);
            }
            s.iter().filter(|e| matches!(e, ServerEvent::Message(..))).count() == 2
        });
        let peer = peer.unwrap();
        assert_eq!(peer, PeerId(1));
        server.send(peer, b"welcome");
        until(&mut server, &mut client, |_, c| c.contains(&ClientEvent::Message(b"welcome".to_vec())));
        client.close();
        until(&mut server, &mut client, |s, _| {
            s.iter().any(|e| matches!(e, ServerEvent::Left(p, Closed::Peer) if *p == peer))
        });
        assert_eq!(server.peers().count(), 0);
    }

    #[test]
    fn a_kicked_client_sees_the_close() {
        let mut server = TcpServer::bind("127.0.0.1:0", Framing::Raw).unwrap();
        let addr = server.local_addr().unwrap();
        let mut client = TcpClient::connect(addr, Framing::Raw, Duration::from_secs(1)).unwrap();
        until(&mut server, &mut client, |s, _| !s.is_empty());
        server.kick(PeerId(1));
        until(&mut server, &mut client, |s, c| {
            s.contains(&ServerEvent::Left(PeerId(1), Closed::Local)) && matches!(c.last(), Some(ClientEvent::Closed(_)))
        });
        assert!(!client.is_connected());
    }
}
