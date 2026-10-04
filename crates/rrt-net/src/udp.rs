//! Datagrams.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs, UdpSocket};

/// The largest UDP payload over IPv4.
pub const MAX_DATAGRAM: usize = 65507;

/// One datagram received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Datagram {
    /// Who sent it.
    pub from: SocketAddr,
    /// Its payload.
    pub data: Vec<u8>,
}

/// A non-blocking UDP socket, read with [`Udp::poll`] once a tick.
pub struct Udp {
    socket: UdpSocket,
    buf: Box<[u8]>,
}

impl Udp {
    /// Binds `addr`: `0.0.0.0:0` for any port, `0.0.0.0:PORT` to be found
    /// on a known port.
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Udp> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Udp { socket, buf: vec![0; 65536].into_boxed_slice() })
    }

    /// Binds `addr` with `SO_REUSEADDR`, so several sockets - two copies of a
    /// game on one machine - can share a discovery port and each receive its
    /// broadcasts and multicasts.
    pub fn bind_reusable(addr: SocketAddr) -> io::Result<Udp> {
        use socket2::{Domain, Protocol, Socket, Type};
        let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;
        socket.bind(&addr.into())?;
        socket.set_nonblocking(true)?;
        Ok(Udp { socket: socket.into(), buf: vec![0; 65536].into_boxed_slice() })
    }

    /// The address bound, with the port the system chose for port 0.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// The socket, for options this type does not wrap (TTL, IPv6
    /// multicast).
    pub fn socket(&self) -> &UdpSocket {
        &self.socket
    }

    /// Sends `msg` to `to`. Ok(false) when the send buffer is full and the
    /// datagram was dropped, as UDP may drop it anyway.
    pub fn send_to(&self, msg: &[u8], to: impl ToSocketAddrs) -> io::Result<bool> {
        match self.socket.send_to(msg, to) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Allows sending to broadcast addresses.
    pub fn set_broadcast(&self, on: bool) -> io::Result<()> {
        self.socket.set_broadcast(on)
    }

    /// Sends `msg` to every host on the local network at `port`
    /// (255.255.255.255, the limited broadcast). Turns broadcast on first.
    /// Whether this host's own sockets hear it depends on its firewall.
    pub fn broadcast(&self, msg: &[u8], port: u16) -> io::Result<bool> {
        self.broadcast_to(msg, Ipv4Addr::BROADCAST, port)
    }

    /// Sends `msg` to a broadcast `address` at `port`: a subnet's
    /// (192.168.1.255) or the limited one. Turns broadcast on first.
    pub fn broadcast_to(&self, msg: &[u8], address: Ipv4Addr, port: u16) -> io::Result<bool> {
        self.set_broadcast(true)?;
        self.send_to(msg, SocketAddrV4::new(address, port))
    }

    /// Joins the IPv4 multicast `group` on `interface` (`0.0.0.0` for the
    /// default).
    pub fn join_multicast(&self, group: Ipv4Addr, interface: Ipv4Addr) -> io::Result<()> {
        self.socket.join_multicast_v4(&group, &interface)
    }

    /// Leaves an IPv4 multicast group.
    pub fn leave_multicast(&self, group: Ipv4Addr, interface: Ipv4Addr) -> io::Result<()> {
        self.socket.leave_multicast_v4(&group, &interface)
    }

    /// The interface multicast sends go out of, by its address; the system
    /// picks by route until this is set.
    pub fn set_multicast_interface(&self, interface: Ipv4Addr) -> io::Result<()> {
        socket2::SockRef::from(&self.socket).set_multicast_if_v4(&interface)
    }

    /// Whether this host receives its own multicast sends.
    pub fn set_multicast_loop(&self, on: bool) -> io::Result<()> {
        self.socket.set_multicast_loop_v4(on)
    }

    /// The next datagram waiting, or None.
    pub fn recv(&mut self) -> io::Result<Option<Datagram>> {
        loop {
            match self.socket.recv_from(&mut self.buf) {
                Ok((n, from)) => return Ok(Some(Datagram { from, data: self.buf[..n].to_vec() })),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                // An ICMP port-unreachable from an earlier send, reported on
                // this read (Windows); not about this datagram.
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Every datagram waiting.
    pub fn poll(&mut self) -> io::Result<Vec<Datagram>> {
        let mut out = Vec::new();
        while let Some(d) = self.recv()? {
            out.push(d);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_datagram_crosses_loopback() {
        let mut a = Udp::bind("127.0.0.1:0").unwrap();
        let b = Udp::bind("127.0.0.1:0").unwrap();
        assert!(b.send_to(b"ping", a.local_addr().unwrap()).unwrap());
        let mut got = Vec::new();
        for _ in 0..200 {
            got.extend(a.poll().unwrap());
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got, [Datagram { from: b.local_addr().unwrap(), data: b"ping".to_vec() }]);
    }

    /// Polls `rx` for up to a second for a datagram carrying `want`.
    fn hears(rx: &mut Udp, want: &[u8]) -> bool {
        for _ in 0..200 {
            if rx.poll().unwrap().iter().any(|d| d.data == want) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }

    /// A directed broadcast on loopback reaches a listener on the port.
    #[test]
    fn a_broadcast_reaches_a_listener() {
        let mut rx = Udp::bind("0.0.0.0:0").unwrap();
        let port = rx.local_addr().unwrap().port();
        let tx = Udp::bind("0.0.0.0:0").unwrap();
        assert!(tx.broadcast_to(b"anyone", Ipv4Addr::new(127, 255, 255, 255), port).unwrap());
        assert!(hears(&mut rx, b"anyone"));
    }

    /// Two sockets sharing one port by `bind_reusable`, both members of a
    /// group on loopback: one multicast reaches both.
    #[test]
    fn a_multicast_reaches_every_member_sharing_a_port() {
        let group = Ipv4Addr::new(239, 255, 42, 99);
        let lo = Ipv4Addr::LOCALHOST;
        let mut a = Udp::bind_reusable("0.0.0.0:0".parse().unwrap()).unwrap();
        let port = a.local_addr().unwrap().port();
        let mut b = Udp::bind_reusable(SocketAddr::from(([0, 0, 0, 0], port))).unwrap();
        for s in [&a, &b] {
            s.join_multicast(group, lo).unwrap();
        }
        let tx = Udp::bind("127.0.0.1:0").unwrap();
        tx.set_multicast_interface(lo).unwrap();
        tx.set_multicast_loop(true).unwrap();
        assert!(tx.send_to(b"members", (group, port)).unwrap());
        assert!(hears(&mut a, b"members"));
        assert!(hears(&mut b, b"members"));
        a.leave_multicast(group, lo).unwrap();
    }
}
