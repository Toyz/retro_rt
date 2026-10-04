//! Cutting a byte stream into messages.

/// How a TCP stream is cut into messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// Each message is a `u32` little-endian byte count, then that many
    /// bytes. A count above `max` closes the connection: a peer that sends
    /// one is broken or hostile.
    LengthPrefixed {
        /// The largest message accepted, in bytes.
        max: u32,
    },
    /// No framing: each message is whatever bytes one read returned, and a
    /// send is written as it is. For a protocol that frames itself, such as
    /// an original game's.
    Raw,
}

impl Framing {
    /// Length-prefixed with a 16 MiB limit.
    pub const DEFAULT: Framing = Framing::LengthPrefixed { max: 16 << 20 };
}

impl Default for Framing {
    fn default() -> Self {
        Framing::DEFAULT
    }
}

/// Appends `msg` to `out` as `framing` puts it on the wire.
///
/// # Panics
///
/// When a length-prefixed message is longer than `u32::MAX` bytes.
pub fn encode(framing: Framing, msg: &[u8], out: &mut Vec<u8>) {
    if let Framing::LengthPrefixed { .. } = framing {
        let len = u32::try_from(msg.len()).expect("message longer than u32::MAX bytes");
        out.extend_from_slice(&len.to_le_bytes());
    }
    out.extend_from_slice(msg);
}

/// Bytes in, whole messages out.
#[derive(Clone, Debug)]
pub struct Decoder {
    framing: Framing,
    buf: Vec<u8>,
}

/// A length prefix above [`Framing::LengthPrefixed`]'s `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLarge(pub u32);

impl Decoder {
    /// A decoder for `framing`, empty.
    pub fn new(framing: Framing) -> Decoder {
        Decoder { framing, buf: Vec::new() }
    }

    /// Bytes as they came off the stream.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message, if one is buffered. Raw framing gives
    /// everything buffered as one message.
    pub fn next_message(&mut self) -> Result<Option<Vec<u8>>, TooLarge> {
        match self.framing {
            Framing::Raw => Ok((!self.buf.is_empty()).then(|| std::mem::take(&mut self.buf))),
            Framing::LengthPrefixed { max } => {
                let Some(head) = self.buf.get(..4) else { return Ok(None) };
                let len = u32::from_le_bytes(head.try_into().unwrap());
                if len > max {
                    return Err(TooLarge(len));
                }
                let end = 4 + len as usize;
                if self.buf.len() < end {
                    return Ok(None);
                }
                let msg = self.buf[4..end].to_vec();
                self.buf.drain(..end);
                Ok(Some(msg))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_survive_being_split_and_joined() {
        let mut wire = Vec::new();
        encode(Framing::DEFAULT, b"hello", &mut wire);
        encode(Framing::DEFAULT, b"", &mut wire);
        encode(Framing::DEFAULT, b"world", &mut wire);
        let mut d = Decoder::new(Framing::DEFAULT);
        let mut got = Vec::new();
        for b in &wire {
            d.push(std::slice::from_ref(b));
            while let Some(m) = d.next_message().unwrap() {
                got.push(m);
            }
        }
        assert_eq!(got, [b"hello".to_vec(), Vec::new(), b"world".to_vec()]);
    }

    #[test]
    fn an_oversized_prefix_is_refused() {
        let mut d = Decoder::new(Framing::LengthPrefixed { max: 4 });
        d.push(&5u32.to_le_bytes());
        assert_eq!(d.next_message(), Err(TooLarge(5)));
    }

    #[test]
    fn raw_gives_what_is_buffered() {
        let mut d = Decoder::new(Framing::Raw);
        assert_eq!(d.next_message(), Ok(None));
        d.push(b"ab");
        d.push(b"c");
        assert_eq!(d.next_message(), Ok(Some(b"abc".to_vec())));
    }
}
