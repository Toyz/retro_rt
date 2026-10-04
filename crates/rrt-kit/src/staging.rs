//! A byte buffer packed again each frame and never shrunk: vertex data for a
//! GPU upload, a packet, a save block.

/// A value with a fixed little-endian byte layout: what [`Staging::pack`]
/// writes. Implemented for the integer and float primitives and arrays of
/// them; a vertex type implements it field by field, in the order its
/// pipeline declares them.
pub trait Pack {
    /// Bytes one value packs to.
    const SIZE: usize;
    /// Appends the value's bytes to `out`, exactly [`Pack::SIZE`] of them.
    fn pack(&self, out: &mut Vec<u8>);
}

macro_rules! pack_le {
    ($($t:ty),*) => {$(
        impl Pack for $t {
            const SIZE: usize = std::mem::size_of::<$t>();
            fn pack(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
        }
    )*};
}

pack_le!(u8, i8, u16, i16, u32, i32, u64, i64, f32, f64);

impl<T: Pack, const N: usize> Pack for [T; N] {
    const SIZE: usize = T::SIZE * N;
    fn pack(&self, out: &mut Vec<u8>) {
        for v in self {
            v.pack(out);
        }
    }
}

/// A byte buffer reused from frame to frame: each [`Staging::pack`] clears
/// it and writes again, reallocating only when the values outgrow every
/// earlier frame.
#[derive(Clone, Debug, Default)]
pub struct Staging {
    buf: Vec<u8>,
}

impl Staging {
    /// An empty buffer.
    pub fn new() -> Staging {
        Staging::default()
    }

    /// `values` packed end to end, in the buffer kept from last time.
    pub fn pack<T: Pack>(&mut self, values: &[T]) -> &[u8] {
        self.buf.clear();
        self.buf.reserve(values.len() * T::SIZE);
        for v in values {
            v.pack(&mut self.buf);
        }
        &self.buf
    }

    /// The buffer cleared, to write into by hand ([`Pack::pack`] or
    /// `extend_from_slice`); it keeps its capacity.
    pub fn begin(&mut self) -> &mut Vec<u8> {
        self.buf.clear();
        &mut self.buf
    }

    /// What was last packed or written.
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Bytes the buffer holds without reallocating.
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vertex as hwtr's renderer packs it: position, colour, uv, mode.
    struct Vtx {
        pos: [f32; 3],
        colour: u32,
        uv: u32,
        mode: u32,
    }

    impl Pack for Vtx {
        const SIZE: usize = 24;
        fn pack(&self, out: &mut Vec<u8>) {
            self.pos.pack(out);
            self.colour.pack(out);
            self.uv.pack(out);
            self.mode.pack(out);
        }
    }

    #[test]
    fn values_pack_little_endian_in_field_order() {
        let mut s = Staging::new();
        let v = Vtx { pos: [1.0, 0.0, -2.0], colour: 0x00a0_b0c0, uv: 0x0201, mode: 7 };
        let bytes = s.pack(&[v]);
        assert_eq!(bytes.len(), Vtx::SIZE);
        assert_eq!(&bytes[..4], 1.0f32.to_le_bytes());
        assert_eq!(&bytes[12..16], [0xc0, 0xb0, 0xa0, 0x00]);
        assert_eq!(&bytes[20..], 7u32.to_le_bytes());
    }

    #[test]
    fn the_buffer_is_reused_and_never_shrinks() {
        let mut s = Staging::new();
        s.pack(&[0u32; 1000]);
        let cap = s.capacity();
        let first = s.bytes().as_ptr();
        assert_eq!(s.pack(&[1u16, 2]), [1, 0, 2, 0]);
        assert_eq!(s.capacity(), cap, "a smaller frame keeps the allocation");
        assert_eq!(s.bytes().as_ptr(), first, "and the same memory");
        s.begin().extend_from_slice(b"raw");
        assert_eq!(s.bytes(), b"raw");
    }
}
