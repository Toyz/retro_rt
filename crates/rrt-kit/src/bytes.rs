//! Reading file formats: a cursor over bytes with little- and big-endian
//! integers, and an error that says where the data ran out.

use std::fmt;

/// A read past the end: where it started and how many bytes it wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Eof {
    /// The offset the read started at.
    pub at: usize,
    /// Bytes it wanted.
    pub want: usize,
    /// Bytes there were in all.
    pub len: usize,
}

impl fmt::Display for Eof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} bytes wanted at {:#x}, past the end at {:#x}", self.want, self.at, self.len)
    }
}

impl std::error::Error for Eof {}

/// A cursor over `&[u8]`. Every read advances past what it read, or fails
/// with [`Eof`] and moves nothing.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

macro_rules! ints {
    ($($le:ident $be:ident $t:ty),*) => {$(
        #[doc = concat!("A little-endian `", stringify!($t), "`.")]
        pub fn $le(&mut self) -> Result<$t, Eof> {
            Ok(<$t>::from_le_bytes(self.array()?))
        }
        #[doc = concat!("A big-endian `", stringify!($t), "`.")]
        pub fn $be(&mut self) -> Result<$t, Eof> {
            Ok(<$t>::from_be_bytes(self.array()?))
        }
    )*};
}

impl<'a> Reader<'a> {
    /// A cursor at the start of `data`.
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }

    /// A cursor at `offset` into `data`; past the end is [`Eof`].
    pub fn at(data: &'a [u8], offset: usize) -> Result<Reader<'a>, Eof> {
        let mut r = Reader::new(data);
        r.seek(offset)?;
        Ok(r)
    }

    /// The offset of the next read.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Bytes left after the cursor.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Moves to `offset`; past the end is [`Eof`] (the end itself is fine).
    pub fn seek(&mut self, offset: usize) -> Result<(), Eof> {
        if offset > self.data.len() {
            return Err(Eof { at: offset, want: 0, len: self.data.len() });
        }
        self.pos = offset;
        Ok(())
    }

    /// Skips `n` bytes.
    pub fn skip(&mut self, n: usize) -> Result<(), Eof> {
        self.bytes(n).map(|_| ())
    }

    /// The next `n` bytes.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], Eof> {
        let eof = Eof { at: self.pos, want: n, len: self.data.len() };
        let end = self.pos.checked_add(n).ok_or(eof)?;
        let out = self.data.get(self.pos..end).ok_or(eof)?;
        self.pos = end;
        Ok(out)
    }

    /// The next `N` bytes as an array.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], Eof> {
        Ok(self.bytes(N)?.try_into().unwrap())
    }

    /// A byte.
    pub fn u8(&mut self) -> Result<u8, Eof> {
        Ok(self.array::<1>()?[0])
    }

    /// A signed byte.
    pub fn i8(&mut self) -> Result<i8, Eof> {
        Ok(self.u8()? as i8)
    }

    ints!(u16_le u16_be u16, i16_le i16_be i16, u32_le u32_be u32, i32_le i32_be i32, u64_le u64_be u64, f32_le f32_be f32);

    /// A fixed-width text field: up to the first NUL, trailing spaces
    /// trimmed, bytes that are not UTF-8 replaced.
    pub fn text(&mut self, n: usize) -> Result<String, Eof> {
        let b = self.bytes(n)?;
        let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
        Ok(String::from_utf8_lossy(&b[..end]).trim_end().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_read_in_either_order_and_advance() {
        let data = [0x01, 0x02, 0x03, 0x04, 0xff, 0xfe];
        let mut r = Reader::new(&data);
        assert_eq!(r.u16_le(), Ok(0x0201));
        assert_eq!(r.u16_be(), Ok(0x0304));
        assert_eq!(r.i16_le(), Ok(-257));
        assert_eq!(r.remaining(), 0);
        let mut r = Reader::at(&data, 0).unwrap();
        assert_eq!(r.u32_le(), Ok(0x0403_0201));
        assert_eq!(r.pos(), 4);
    }

    #[test]
    fn a_read_past_the_end_fails_and_moves_nothing() {
        let data = [1, 2, 3];
        let mut r = Reader::new(&data);
        r.skip(2).unwrap();
        assert_eq!(r.u32_le(), Err(Eof { at: 2, want: 4, len: 3 }));
        assert_eq!(r.pos(), 2);
        assert_eq!(r.u8(), Ok(3));
        assert!(Reader::at(&data, 4).is_err());
        assert!(Reader::at(&data, 3).is_ok(), "the end itself is a place");
        assert!(r.bytes(usize::MAX).is_err(), "no overflow");
    }

    #[test]
    fn text_fields_stop_at_nul_and_lose_trailing_spaces() {
        let mut r = Reader::new(b"SLUS_009\0junkCD001  ");
        assert_eq!(r.text(13).unwrap(), "SLUS_009", "the field is 13 bytes; the NUL ends the text");
        assert_eq!(r.text(7).unwrap(), "CD001");
    }
}
