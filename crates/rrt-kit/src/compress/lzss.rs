//! LZSS: literals and back-references in groups of eight behind a flag byte.

use super::Error;

/// Which flag bit governs the first item after a flag byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagOrder {
    /// Bit 0 first (Okumura).
    Lsb,
    /// Bit 7 first (Nintendo).
    Msb,
}

/// How a back-reference's two bytes pack a position and a length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    /// Okumura: `b0` is the ring position's low 8 bits, `b1` its high 4 bits
    /// (top nibble) and the length less `min_match` (low nibble). The
    /// position is absolute in the ring.
    Okumura,
    /// Nintendo LZ10: `b0` is the length less `min_match` (top nibble) and the
    /// distance's high 4 bits, `b1` its low 8; the distance back is the field
    /// plus 1.
    Nintendo,
}

/// An LZSS variant. Decoding keeps a ring of `window` bytes filled with
/// `fill`, writing from `start`; each flag bit says whether the next item is
/// a literal byte or a two-byte back-reference of `min_match..=max_match`
/// bytes, copied byte by byte (so a reference may overlap what it writes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lzss {
    /// Ring size in bytes: 4096 for every known 12-bit-position variant.
    pub window: usize,
    /// What the ring holds before anything is written.
    pub fill: u8,
    /// Where the first byte is written in the ring.
    pub start: usize,
    /// The shortest reference.
    pub min_match: usize,
    /// The longest reference: `min_match + 15` for a 4-bit length.
    pub max_match: usize,
    /// Which flag bit comes first.
    pub flags: FlagOrder,
    /// The flag bit's value that means "literal".
    pub literal_bit: bool,
    /// How a reference is packed.
    pub token: Token,
}

impl Lzss {
    /// Okumura's `LZSS.C` (1989): window 4096, fill 0x20 (space), start
    /// 4078 (`N - F`), lengths 3-18, flags LSB first with 1 for a literal,
    /// absolute ring positions. Many games kept the layout and changed the
    /// fill to 0 or the start to 0.
    pub const OKUMURA: Lzss = Lzss {
        window: 4096,
        fill: b' ',
        start: 4096 - 18,
        min_match: 3,
        max_match: 18,
        flags: FlagOrder::Lsb,
        literal_bit: true,
        token: Token::Okumura,
    };

    /// Nintendo's LZ10 body: window 4096, lengths 3-18, flags MSB first with
    /// 0 for a literal, distances back of 1-4096. [`lz10_decode`] reads its
    /// 4-byte header.
    pub const LZ10: Lzss = Lzss {
        window: 4096,
        fill: 0,
        start: 0,
        min_match: 3,
        max_match: 18,
        flags: FlagOrder::Msb,
        literal_bit: false,
        token: Token::Nintendo,
    };

    fn is_literal(&self, flags: u8, k: u32) -> bool {
        let bit = match self.flags {
            FlagOrder::Lsb => (flags >> k) & 1,
            FlagOrder::Msb => (flags >> (7 - k)) & 1,
        };
        (bit == 1) == self.literal_bit
    }

    /// Decodes `data`: until it runs out, or until `size` bytes are out when
    /// given (a trailing partial group is then ignored).
    pub fn decode(&self, data: &[u8], size: Option<usize>) -> Result<Vec<u8>, Error> {
        let n = self.window;
        let mut ring = vec![self.fill; n];
        let mut r = self.start % n;
        let mut out = Vec::with_capacity(size.unwrap_or(data.len() * 2));
        let mut at = 0;
        let done = |out: &Vec<u8>| size.is_some_and(|s| out.len() >= s);
        while at < data.len() && !done(&out) {
            let flags = data[at];
            at += 1;
            for k in 0..8 {
                if done(&out) || at >= data.len() {
                    break;
                }
                if self.is_literal(flags, k) {
                    let b = data[at];
                    at += 1;
                    out.push(b);
                    ring[r] = b;
                    r = (r + 1) % n;
                    continue;
                }
                let pair = data.get(at..at + 2).ok_or(Error { at, what: "a reference cut short" })?;
                let (b0, b1) = (usize::from(pair[0]), usize::from(pair[1]));
                at += 2;
                let (src, len) = match self.token {
                    Token::Okumura => (b0 | ((b1 & 0xf0) << 4), (b1 & 0x0f) + self.min_match),
                    Token::Nintendo => {
                        let dist = (((b0 & 0x0f) << 8) | b1) + 1;
                        if dist > out.len() && self.fill_unused() {
                            return Err(Error { at: at - 2, what: "a reference before the start of the output" });
                        }
                        ((r + n - dist) % n, (b0 >> 4) + self.min_match)
                    }
                };
                for k in 0..len {
                    if done(&out) {
                        break;
                    }
                    let b = ring[(src + k) % n];
                    out.push(b);
                    ring[r] = b;
                    r = (r + 1) % n;
                }
            }
        }
        if let Some(s) = size
            && out.len() < s
        {
            return Err(Error { at, what: "the data ends before the size it declares" });
        }
        Ok(out)
    }

    /// Nintendo's format may not reach into the ring's fill: there is none.
    fn fill_unused(&self) -> bool {
        self.token == Token::Nintendo
    }

    /// Encodes `data` so [`Lzss::decode`] with these settings gives it back:
    /// greedy longest matches found through a hash of three-byte prefixes,
    /// within the window, never into the initial fill.
    ///
    /// # Panics
    ///
    /// When `max_match - min_match` does not fit the token's 4-bit length, or
    /// the window is not 4096 (both tokens carry 12-bit positions).
    pub fn encode(&self, data: &[u8]) -> Vec<u8> {
        assert!(self.max_match - self.min_match <= 15 && self.window == 4096, "a 12-bit position, 4-bit length layout");
        let n = self.window;
        let mut out = Vec::with_capacity(data.len() + data.len() / 8 + 1);
        let mut head = std::collections::HashMap::<[u8; 3], usize>::new();
        let mut prev = vec![usize::MAX; data.len()];
        let mut i = 0;
        let mut group: Vec<u8> = Vec::new();
        let mut flags = 0u8;
        let mut count = 0u32;
        let set_literal = |flags: &mut u8, k: u32, literal: bool| {
            if literal == self.literal_bit {
                *flags |= match self.flags {
                    FlagOrder::Lsb => 1 << k,
                    FlagOrder::Msb => 1 << (7 - k),
                };
            }
        };
        let insert = |head: &mut std::collections::HashMap<[u8; 3], usize>, prev: &mut Vec<usize>, j: usize| {
            if j + 3 <= data.len() {
                let key = [data[j], data[j + 1], data[j + 2]];
                prev[j] = head.insert(key, j).unwrap_or(usize::MAX);
            }
        };
        while i < data.len() {
            let (mut best, mut best_at) = (0, 0);
            if i + self.min_match <= data.len() && i + 3 <= data.len() {
                let mut cand = head.get(&[data[i], data[i + 1], data[i + 2]]).copied().unwrap_or(usize::MAX);
                let mut tries = 0;
                while cand != usize::MAX && i - cand < n && tries < 512 {
                    let limit = self.max_match.min(data.len() - i);
                    let len = (0..limit).take_while(|&k| data[cand + k] == data[i + k]).count();
                    if len > best {
                        (best, best_at) = (len, cand);
                        if len == limit {
                            break;
                        }
                    }
                    cand = prev[cand];
                    tries += 1;
                }
            }
            if best >= self.min_match {
                set_literal(&mut flags, count, false);
                let field = best - self.min_match;
                match self.token {
                    Token::Okumura => {
                        let pos = (self.start + best_at) % n;
                        group.push(pos as u8);
                        group.push((((pos >> 8) as u8) << 4) | field as u8);
                    }
                    Token::Nintendo => {
                        let dist = i - best_at - 1;
                        group.push(((field as u8) << 4) | (dist >> 8) as u8);
                        group.push(dist as u8);
                    }
                }
                for j in i..i + best {
                    insert(&mut head, &mut prev, j);
                }
                i += best;
            } else {
                set_literal(&mut flags, count, true);
                group.push(data[i]);
                insert(&mut head, &mut prev, i);
                i += 1;
            }
            count += 1;
            if count == 8 {
                out.push(flags);
                out.append(&mut group);
                (flags, count) = (0, 0);
            }
        }
        if count > 0 {
            out.push(flags);
            out.append(&mut group);
        }
        out
    }
}

/// Decodes Nintendo LZ10 with its header: byte 0 is 0x10, bytes 1-3 the
/// decompressed size, little-endian.
pub fn lz10_decode(data: &[u8]) -> Result<Vec<u8>, Error> {
    let head = data.get(..4).ok_or(Error { at: 0, what: "no LZ10 header" })?;
    if head[0] != 0x10 {
        return Err(Error { at: 0, what: "not LZ10 (type byte is not 0x10)" });
    }
    let size = usize::from(head[1]) | usize::from(head[2]) << 8 | usize::from(head[3]) << 16;
    Lzss::LZ10.decode(&data[4..], Some(size)).map_err(|e| Error { at: e.at + 4, ..e })
}

/// Encodes as Nintendo LZ10 with its header.
///
/// # Panics
///
/// When `data` is 16 MiB or longer (the header's 24-bit size).
pub fn lz10_encode(data: &[u8]) -> Vec<u8> {
    assert!(data.len() < 1 << 24, "LZ10 holds under 16 MiB");
    let n = data.len() as u32;
    let mut out = vec![0x10, n as u8, (n >> 8) as u8, (n >> 16) as u8];
    out.extend(Lzss::LZ10.encode(data));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worked from `LZSS.C`: literals a, b, c land at ring 4078-4080; a
    /// reference to 4078 of length 6 (field 3) copies over its own output.
    /// Flags 0b0111: three literals, then a reference.
    #[test]
    fn okumura_references_absolute_ring_positions_and_overlap() {
        let data = [0x07, b'a', b'b', b'c', 0xee, 0xf3];
        assert_eq!(Lzss::OKUMURA.decode(&data, None).unwrap(), b"abcabcabc");
    }

    /// Before anything is written the ring is spaces: a reference to 0
    /// copies them, the trick Okumura's format is known for.
    #[test]
    fn okumura_can_copy_the_initial_fill() {
        assert_eq!(Lzss::OKUMURA.decode(&[0x00, 0x00, 0x02], None).unwrap(), b"     ");
        let zero_fill = Lzss { fill: 0, ..Lzss::OKUMURA };
        assert_eq!(zero_fill.decode(&[0x00, 0x00, 0x00], None).unwrap(), [0, 0, 0]);
    }

    /// Worked from GBATEK's LZ77 description: 10 'a's are a literal and a
    /// reference of 9 at distance 1 (fields 6 and 0); flags 0b0100_0000.
    #[test]
    fn lz10_decodes_its_header_and_overlapping_runs() {
        let data = [0x10, 0x0a, 0x00, 0x00, 0x40, b'a', 0x60, 0x00];
        assert_eq!(lz10_decode(&data).unwrap(), b"aaaaaaaaaa");
        assert_eq!(lz10_encode(b"aaaaaaaaaa"), data);
    }

    #[test]
    fn corrupt_data_is_an_error_not_a_panic() {
        let ends = lz10_decode(&[0x10, 0x0a, 0, 0, 0x40, b'a']).unwrap_err();
        assert_eq!(ends.what, "the data ends before the size it declares");
        let cut = lz10_decode(&[0x10, 0x0a, 0, 0, 0x40, b'a', 0x60]).unwrap_err();
        assert_eq!((cut.what, cut.at), ("a reference cut short", 6), "one byte of a two-byte reference");
        assert!(lz10_decode(&[0x11, 0, 0, 0]).is_err());
        assert!(lz10_decode(&[0x10, 0x05, 0, 0, 0x00, b'a']).is_err(), "shorter than declared");
        assert!(lz10_decode(&[0x10, 0x04, 0, 0, 0x80, 0x00, 0x05]).is_err(), "reaches before the start");
        assert!(Lzss::OKUMURA.decode(&[0x00, 0x00], None).is_err());
    }

    /// Text, binary, runs and noise round-trip through both layouts, and
    /// repetitive data shrinks.
    #[test]
    fn encode_then_decode_gives_the_data_back() {
        let mut noise = Vec::new();
        let mut x = 0x1234_5678u32;
        for _ in 0..5000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            noise.push(x as u8);
        }
        let text = b"the quick brown fox jumps over the lazy dog. the quick brown fox jumps again.".repeat(40);
        let runs = [vec![0u8; 3000], vec![7u8; 17], b"ab".repeat(900)].concat();
        for data in [&b""[..], b"x", b"abcabcabc", &text, &runs, &noise] {
            for codec in [Lzss::OKUMURA, Lzss::LZ10, Lzss { fill: 0, start: 0, ..Lzss::OKUMURA }] {
                let packed = codec.encode(data);
                assert_eq!(codec.decode(&packed, Some(data.len())).unwrap(), data, "{codec:?}");
            }
            assert_eq!(lz10_decode(&lz10_encode(data)).unwrap(), data);
        }
        assert!(Lzss::OKUMURA.encode(&text).len() < text.len() / 4);
        assert!(Lzss::OKUMURA.encode(&runs).len() < runs.len() / 6);
    }
}
