//! Colour as the consoles of the era stored it.
//!
//! Each format is a newtype over its raw word ([`Rgb555`] wraps a `u16`) and
//! implements [`Pixel`]: to and from RGBA8 with straight alpha, channels
//! expanded to the full 0-255 range by bit replication ([`expand`]), reduced
//! by truncation, so every representable colour round-trips. Which systems
//! use which:
//!
//! | type | layout, high bit to low | systems |
//! | --- | --- | --- |
//! | [`Rgb555`] | `T BBBBB GGGGG RRRRR` | PS1 VRAM, PS2 PSMCT16, SNES, GBA, GBC, Saturn RGB, PSP 5551 |
//! | [`Argb1555`] | `A RRRRR GGGGG BBBBB` | Dreamcast, PC (A1R5G5B5) |
//! | [`Rgb565`] | `RRRRR GGGGGG BBBBB` | Dreamcast, Xbox, GameCube, PC |
//! | [`Bgr565`] | `BBBBB GGGGGG RRRRR` | PSP 5650 |
//! | [`Argb4444`] | `AAAA RRRR GGGG BBBB` | Dreamcast, PC |
//! | [`Abgr4444`] | `AAAA BBBB GGGG RRRR` | PSP 4444 |
//! | [`Rgba5551`] | `RRRRR GGGGG BBBBB A` | N64 RGBA16 |
//! | [`Rgb5a3`] | `1 RRRRR GGGGG BBBBB` or `0 AAA RRRR GGGG BBBB` | GameCube, Wii |
//! | [`Ia16`] | `IIIIIIII AAAAAAAA` | N64 IA16 |
//! | [`Ia8`] | `IIII AAAA` | N64 IA8 |
//! | [`Md333`] | `0000 BBB0 GGG0 RRR0` | Mega Drive / Genesis CRAM |
//! | [`Sms222`] | `00 BB GG RR` | Master System |
//! | [`Rgb332`] | `RRR GGG BB` | MSX2 screen 8, 8-bit PC modes |
//! | [`PsmCt32`] | bytes R G B A, A 0x80 = 1.0 | PS2 PSMCT32 |
//!
//! Also: [`ycbcr_to_rgb`] for decoded video (PS1 MDEC, PS2 IPU and PSS
//! movies), [`indexed4`] and [`indexed8`] for paletted textures, and
//! [`ps2_clut_index`] for the PS2's 8-bit CLUT order.

/// A pixel format: to and from RGBA8, straight alpha.
pub trait Pixel: Copy {
    /// The colour as R, G, B, A, each 0-255, channels expanded to the full
    /// range.
    fn to_rgba8(self) -> [u8; 4];
    /// The nearest colour this format holds to `rgba`: channels truncated
    /// to the format's bits.
    fn from_rgba8(rgba: [u8; 4]) -> Self;
}

/// A `bits`-wide channel value expanded to 8 bits by repeating its bits:
/// 0 stays 0 and the maximum becomes 255 (5 bits: 31 is 255, 16 is 132).
pub const fn expand(value: u32, bits: u32) -> u8 {
    let v = value & ((1 << bits) - 1);
    let mut out = 0u32;
    let mut have = 0;
    while have < 8 {
        out = (out << bits) | v;
        have += bits;
    }
    (out >> (have - 8)) as u8
}

/// An 8-bit channel reduced to `bits` by truncation: the inverse of
/// [`expand`] for every value `expand` gives.
pub const fn reduce(value: u8, bits: u32) -> u32 {
    (value >> (8 - bits)) as u32
}

/// Pixels converted to RGBA8, end to end: an image's bytes.
pub fn to_rgba8<P: Pixel>(pixels: &[P]) -> Vec<u8> {
    pixels.iter().flat_map(|p| p.to_rgba8()).collect()
}

/// Three channels at bit offsets in a word, expanded.
const fn rgb_at(w: u32, (rs, gs, bs): (u32, u32, u32), (rb, gb, bb): (u32, u32, u32)) -> [u8; 3] {
    [expand(w >> rs, rb), expand(w >> gs, gb), expand(w >> bs, bb)]
}

/// RGBA8 reduced into a word: channel shifts and widths.
const fn pack_rgb([r, g, b, _]: [u8; 4], (rs, gs, bs): (u32, u32, u32), (rb, gb, bb): (u32, u32, u32)) -> u32 {
    (reduce(r, rb) << rs) | (reduce(g, gb) << gs) | (reduce(b, bb) << bs)
}

// 16-bit, red low -------------------------------------------------------------

/// `T BBBBB GGGGG RRRRR`: red in bits 0-4, green 5-9, blue 10-14, and bit 15
/// the PS1's semi-transparency flag (STP), the PS2's alpha bit, the Saturn's
/// RGB-mode bit; unused on the SNES and GBA. As a [`Pixel`] it follows the
/// PS1 texture rule: the word 0x0000 is transparent and every other word,
/// black with STP included, is opaque.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb555(pub u16);

impl Rgb555 {
    /// From 5-bit channels and the top bit.
    pub const fn new(r: u8, g: u8, b: u8, top: bool) -> Rgb555 {
        Rgb555((r as u16 & 31) | ((g as u16 & 31) << 5) | ((b as u16 & 31) << 10) | ((top as u16) << 15))
    }

    /// The 5-bit channels: red, green, blue.
    pub const fn channels(self) -> [u8; 3] {
        [(self.0 & 31) as u8, ((self.0 >> 5) & 31) as u8, ((self.0 >> 10) & 31) as u8]
    }

    /// Bit 15: STP on the PS1, alpha on the PS2.
    pub const fn top(self) -> bool {
        self.0 & 0x8000 != 0
    }

    /// The PS1 texture rule: the value 0x0000 is transparent (not drawn);
    /// every other value, black with STP included, is drawn.
    pub const fn is_transparent(self) -> bool {
        self.0 == 0
    }

    /// RGB8 with each channel shifted left 3 (`c << 3`, 31 is 248): what the
    /// PS1 GPU itself does with a 15-bit texel before shading.
    pub const fn rgb8_shifted(self) -> [u8; 3] {
        let [r, g, b] = self.channels();
        [r << 3, g << 3, b << 3]
    }

    /// RGB8 over the full range (`c << 3 | c >> 2`, 31 is 255): for showing
    /// a texture or a screen as an image.
    pub const fn rgb8(self) -> [u8; 3] {
        rgb_at(self.0 as u32, (0, 5, 10), (5, 5, 5))
    }

    /// [`Rgb555::rgb8`] with alpha: 0 for a transparent texel
    /// ([`Rgb555::is_transparent`]), else 255.
    pub const fn rgba8(self) -> [u8; 4] {
        let [r, g, b] = self.rgb8();
        [r, g, b, if self.is_transparent() { 0 } else { 255 }]
    }

    /// The nearest 15-bit colour to RGB8 (each channel `>> 3`), top bit clear.
    pub const fn from_rgb8([r, g, b]: [u8; 3]) -> Rgb555 {
        Rgb555::new(r >> 3, g >> 3, b >> 3, false)
    }
}

impl Pixel for Rgb555 {
    fn to_rgba8(self) -> [u8; 4] {
        self.rgba8()
    }

    /// Alpha 0 is the transparent word 0x0000. An opaque colour that would
    /// reduce to 0x0000 (black) gets STP set, 0x8000, so it stays drawn;
    /// otherwise STP is clear. RGBA8 has no place for a texel's STP, so it
    /// survives the round trip only on black: keep the raw words when STP
    /// matters.
    fn from_rgba8(rgba: [u8; 4]) -> Rgb555 {
        if rgba[3] == 0 {
            return Rgb555(0);
        }
        let c = Rgb555::from_rgb8([rgba[0], rgba[1], rgba[2]]);
        if c.0 == 0 { Rgb555(0x8000) } else { c }
    }
}

/// `BBBBB GGGGGG RRRRR`: the PSP's 5650. Opaque.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bgr565(pub u16);

impl Pixel for Bgr565 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b] = rgb_at(self.0 as u32, (0, 5, 11), (5, 6, 5));
        [r, g, b, 255]
    }
    fn from_rgba8(c: [u8; 4]) -> Bgr565 {
        Bgr565(pack_rgb(c, (0, 5, 11), (5, 6, 5)) as u16)
    }
}

/// `AAAA BBBB GGGG RRRR`: the PSP's 4444.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Abgr4444(pub u16);

impl Pixel for Abgr4444 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b] = rgb_at(self.0 as u32, (0, 4, 8), (4, 4, 4));
        [r, g, b, expand(self.0 as u32 >> 12, 4)]
    }
    fn from_rgba8(c: [u8; 4]) -> Abgr4444 {
        Abgr4444((pack_rgb(c, (0, 4, 8), (4, 4, 4)) | (reduce(c[3], 4) << 12)) as u16)
    }
}

// 16-bit, red high ------------------------------------------------------------

/// `A RRRRR GGGGG BBBBB`: the Dreamcast's ARGB1555, DirectX's A1R5G5B5.
/// Alpha is all or nothing; from RGBA8, alpha 128 and up is opaque.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Argb1555(pub u16);

impl Pixel for Argb1555 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b] = rgb_at(self.0 as u32, (10, 5, 0), (5, 5, 5));
        [r, g, b, if self.0 & 0x8000 != 0 { 255 } else { 0 }]
    }
    fn from_rgba8(c: [u8; 4]) -> Argb1555 {
        Argb1555((pack_rgb(c, (10, 5, 0), (5, 5, 5)) | (reduce(c[3], 1) << 15)) as u16)
    }
}

/// `RRRRR GGGGGG BBBBB`: 16-bit colour with green's extra bit, on the
/// Dreamcast, Xbox, GameCube and PC. Opaque.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb565(pub u16);

impl Pixel for Rgb565 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b] = rgb_at(self.0 as u32, (11, 5, 0), (5, 6, 5));
        [r, g, b, 255]
    }
    fn from_rgba8(c: [u8; 4]) -> Rgb565 {
        Rgb565(pack_rgb(c, (11, 5, 0), (5, 6, 5)) as u16)
    }
}

/// `AAAA RRRR GGGG BBBB`: the Dreamcast's ARGB4444, DirectX's A4R4G4B4.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Argb4444(pub u16);

impl Pixel for Argb4444 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b] = rgb_at(self.0 as u32, (8, 4, 0), (4, 4, 4));
        [r, g, b, expand(self.0 as u32 >> 12, 4)]
    }
    fn from_rgba8(c: [u8; 4]) -> Argb4444 {
        Argb4444((pack_rgb(c, (8, 4, 0), (4, 4, 4)) | (reduce(c[3], 4) << 12)) as u16)
    }
}

/// `RRRRR GGGGG BBBBB A`: the N64's RGBA16, alpha in bit 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgba5551(pub u16);

impl Pixel for Rgba5551 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b] = rgb_at(self.0 as u32, (11, 6, 1), (5, 5, 5));
        [r, g, b, if self.0 & 1 != 0 { 255 } else { 0 }]
    }
    fn from_rgba8(c: [u8; 4]) -> Rgba5551 {
        Rgba5551((pack_rgb(c, (11, 6, 1), (5, 5, 5)) | reduce(c[3], 1)) as u16)
    }
}

/// The GameCube's and Wii's RGB5A3: with bit 15 set, `1 RRRRR GGGGG BBBBB`
/// and opaque; clear, `0 AAA RRRR GGGG BBBB` with 3 bits of alpha. From
/// RGBA8, alpha 255 picks the opaque form, anything less the other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb5a3(pub u16);

impl Pixel for Rgb5a3 {
    fn to_rgba8(self) -> [u8; 4] {
        let w = self.0 as u32;
        if w & 0x8000 != 0 {
            let [r, g, b] = rgb_at(w, (10, 5, 0), (5, 5, 5));
            [r, g, b, 255]
        } else {
            let [r, g, b] = rgb_at(w, (8, 4, 0), (4, 4, 4));
            [r, g, b, expand(w >> 12, 3)]
        }
    }
    fn from_rgba8(c: [u8; 4]) -> Rgb5a3 {
        if c[3] == 255 {
            Rgb5a3((0x8000 | pack_rgb(c, (10, 5, 0), (5, 5, 5))) as u16)
        } else {
            Rgb5a3((pack_rgb(c, (8, 4, 0), (4, 4, 4)) | (reduce(c[3], 3) << 12)) as u16)
        }
    }
}

// Intensity -------------------------------------------------------------------

/// The N64's IA16: intensity in the high byte, alpha in the low. Grey: R, G
/// and B are the intensity. From RGBA8, intensity is the mean of R, G, B.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Ia16(pub u16);

/// The mean of R, G and B, for the intensity formats.
const fn grey([r, g, b, _]: [u8; 4]) -> u8 {
    ((r as u32 + g as u32 + b as u32) / 3) as u8
}

impl Pixel for Ia16 {
    fn to_rgba8(self) -> [u8; 4] {
        let i = (self.0 >> 8) as u8;
        [i, i, i, self.0 as u8]
    }
    fn from_rgba8(c: [u8; 4]) -> Ia16 {
        Ia16((grey(c) as u16) << 8 | c[3] as u16)
    }
}

/// The N64's IA8: intensity in the high nibble, alpha in the low.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Ia8(pub u8);

impl Pixel for Ia8 {
    fn to_rgba8(self) -> [u8; 4] {
        let i = expand(self.0 as u32 >> 4, 4);
        [i, i, i, expand(self.0 as u32, 4)]
    }
    fn from_rgba8(c: [u8; 4]) -> Ia8 {
        Ia8(((reduce(grey(c), 4) << 4) | reduce(c[3], 4)) as u8)
    }
}

// Sega and 8-bit --------------------------------------------------------------

/// The Mega Drive's (Genesis') CRAM word: `0000 BBB0 GGG0 RRR0`, 3 bits a
/// channel. Expanded linearly; the console's DAC is not linear (its
/// shadow/highlight steps and analogue curve differ), so this is the
/// colour as data, not as a television showed it. Opaque.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Md333(pub u16);

impl Pixel for Md333 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b] = rgb_at(self.0 as u32, (1, 5, 9), (3, 3, 3));
        [r, g, b, 255]
    }
    fn from_rgba8(c: [u8; 4]) -> Md333 {
        Md333(pack_rgb(c, (1, 5, 9), (3, 3, 3)) as u16)
    }
}

/// The Master System's CRAM byte: `00 BB GG RR`, 2 bits a channel. Opaque.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Sms222(pub u8);

impl Pixel for Sms222 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b] = rgb_at(self.0 as u32, (0, 2, 4), (2, 2, 2));
        [r, g, b, 255]
    }
    fn from_rgba8(c: [u8; 4]) -> Sms222 {
        Sms222(pack_rgb(c, (0, 2, 4), (2, 2, 2)) as u8)
    }
}

/// `RRR GGG BB`: MSX2's screen 8 and 8-bit PC direct colour. Opaque.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb332(pub u8);

impl Pixel for Rgb332 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b] = rgb_at(self.0 as u32, (5, 2, 0), (3, 3, 2));
        [r, g, b, 255]
    }
    fn from_rgba8(c: [u8; 4]) -> Rgb332 {
        Rgb332(pack_rgb(c, (5, 2, 0), (3, 3, 2)) as u8)
    }
}

// 32-bit ----------------------------------------------------------------------

/// The PS2's PSMCT32: bytes R, G, B, A in memory, with alpha 0x80 meaning
/// 1.0 (the GS's scale; 0x81-0xff are over 1.0, used by blending). As a
/// [`Pixel`], alpha doubles and saturates at 255; from RGBA8 it halves,
/// rounding up, so 255 is 0x80.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PsmCt32(pub [u8; 4]);

impl Pixel for PsmCt32 {
    fn to_rgba8(self) -> [u8; 4] {
        let [r, g, b, a] = self.0;
        [r, g, b, (a as u16 * 2).min(255) as u8]
    }
    fn from_rgba8([r, g, b, a]: [u8; 4]) -> PsmCt32 {
        PsmCt32([r, g, b, (a as u16).div_ceil(2) as u8])
    }
}

// Video and palettes ----------------------------------------------------------

/// Which YCbCr range a decoder produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    /// Y, Cb and Cr over 0-255 (JPEG / JFIF).
    Full,
    /// BT.601 studio range: Y 16-235, Cb and Cr 16-240 (MPEG-1 and -2, so
    /// the PS2's PSS movies and IPU output).
    Limited,
}

/// One YCbCr sample as RGB8, by BT.601's coefficients in `range`, each
/// channel rounded and clamped. The PS1 MDEC and the PS2 IPU leave this
/// step to the program, so a port does it here.
pub fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8, range: Range) -> [u8; 3] {
    let (cb, cr) = (f32::from(cb) - 128.0, f32::from(cr) - 128.0);
    let (y, kr, kgb, kgr, kb) = match range {
        Range::Full => (f32::from(y), 1.402, 0.344_136, 0.714_136, 1.772),
        Range::Limited => (1.164_383 * (f32::from(y) - 16.0), 1.596_027, 0.391_762, 0.812_968, 2.017_232),
    };
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    [c(y + kr * cr), c(y - kgb * cb - kgr * cr), c(y + kb * cb)]
}

/// Which nibble of a 4-bit indexed texture's byte is the first pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nibbles {
    /// The low nibble first: PS1, PS2, PSP, GBA, Saturn.
    LowFirst,
    /// The high nibble first: N64, GameCube, Wii, Mega Drive tiles.
    HighFirst,
}

/// A 4-bit indexed image's pixels looked up in `palette`: two a byte, in
/// `order`. An index past the palette's end is transparent black.
pub fn indexed4<P: Pixel>(data: &[u8], palette: &[P], order: Nibbles) -> Vec<[u8; 4]> {
    let look = |i: u8| palette.get(usize::from(i)).map_or([0; 4], |p| p.to_rgba8());
    data.iter()
        .flat_map(|b| {
            let (lo, hi) = (b & 15, b >> 4);
            match order {
                Nibbles::LowFirst => [look(lo), look(hi)],
                Nibbles::HighFirst => [look(hi), look(lo)],
            }
        })
        .collect()
}

/// An 8-bit indexed image's pixels looked up in `palette`. An index past the
/// palette's end is transparent black.
pub fn indexed8<P: Pixel>(data: &[u8], palette: &[P]) -> Vec<[u8; 4]> {
    data.iter().map(|&i| palette.get(usize::from(i)).map_or([0; 4], |p| p.to_rgba8())).collect()
}

/// Where the PS2 keeps colour `i` of a 256-colour CLUT stored in CSM1, its
/// usual layout: bits 3 and 4 of the index swapped, because the GS lays the
/// CLUT out in 8 x 2 blocks. Its own inverse; apply it to a CLUT read from
/// memory before using it with [`indexed8`].
pub const fn ps2_clut_index(i: u8) -> u8 {
    (i & 0xe7) | ((i & 0x08) << 1) | ((i & 0x10) >> 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_replicates_bits_to_the_full_range() {
        assert_eq!((expand(31, 5), expand(16, 5), expand(0, 5)), (255, 132, 0));
        assert_eq!((expand(63, 6), expand(7, 3), expand(1, 1), expand(3, 2)), (255, 255, 255, 255));
        assert_eq!((expand(4, 3), expand(1, 2), expand(8, 4)), (146, 85, 136));
        for bits in 1..=8 {
            for v in 0..(1u32 << bits) {
                assert_eq!(reduce(expand(v, bits), bits), v, "{v} in {bits} bits");
            }
        }
    }

    /// Every word of a 16-bit format: to RGBA8 and back is the same word.
    fn round_trips<P: Pixel + PartialEq + std::fmt::Debug>(make: impl Fn(u16) -> P, words: impl Iterator<Item = u16>) {
        for w in words {
            let p = make(w);
            assert_eq!(P::from_rgba8(p.to_rgba8()), p, "{w:#06x}");
        }
    }

    #[test]
    fn every_word_of_every_16_bit_format_round_trips() {
        // STP has no place in RGBA8: it survives only on black, as 0x8000.
        let rgb555 = |w: &u16| (w & 0x8000 == 0 && *w != 0) || *w == 0x8000;
        round_trips(Rgb555, (1..=0xffff).filter(rgb555));
        round_trips(Argb1555, 0..=0xffff);
        round_trips(Rgb565, 0..=0xffff);
        round_trips(Bgr565, 0..=0xffff);
        round_trips(Argb4444, 0..=0xffff);
        round_trips(Abgr4444, 0..=0xffff);
        round_trips(Rgba5551, 0..=0xffff);
        round_trips(Rgb5a3, (0..=0xffff).filter(|w| w & 0x8000 != 0 || (w >> 12) & 7 != 7));
        round_trips(Ia16, 0..=0xffff);
        round_trips(Md333, (0..=0xffff).filter(|w| w & !0x0eee == 0));
    }

    #[test]
    fn every_byte_of_every_8_bit_format_round_trips() {
        for b in 0..=255u8 {
            assert_eq!(Ia8::from_rgba8(Ia8(b).to_rgba8()), Ia8(b));
            assert_eq!(Rgb332::from_rgba8(Rgb332(b).to_rgba8()), Rgb332(b));
            if b < 64 {
                assert_eq!(Sms222::from_rgba8(Sms222(b).to_rgba8()), Sms222(b));
            }
        }
    }

    /// Pure red in each format: where the format puts it.
    #[test]
    fn red_lands_where_each_format_keeps_it() {
        const RED: [u8; 4] = [255, 0, 0, 255];
        assert_eq!(Rgb555::from_rgba8(RED).0, 0x001f);
        assert_eq!(Argb1555::from_rgba8(RED).0, 0xfc00);
        assert_eq!(Rgb565::from_rgba8(RED).0, 0xf800);
        assert_eq!(Bgr565::from_rgba8(RED).0, 0x001f);
        assert_eq!(Argb4444::from_rgba8(RED).0, 0xff00);
        assert_eq!(Abgr4444::from_rgba8(RED).0, 0xf00f);
        assert_eq!(Rgba5551::from_rgba8(RED).0, 0xf801);
        assert_eq!(Rgb5a3::from_rgba8(RED).0, 0xfc00);
        assert_eq!(Md333::from_rgba8(RED).0, 0x000e);
        assert_eq!(Sms222::from_rgba8(RED).0, 0x03);
        assert_eq!(Rgb332::from_rgba8(RED).0, 0xe0);
        for (rgba, name) in [
            (Rgb555(0x001f).to_rgba8(), "rgb555"),
            (Argb1555(0xfc00).to_rgba8(), "argb1555"),
            (Rgb565(0xf800).to_rgba8(), "rgb565"),
            (Rgba5551(0xf801).to_rgba8(), "rgba5551"),
            (Rgb5a3(0xfc00).to_rgba8(), "rgb5a3"),
            (Md333(0x000e).to_rgba8(), "md333"),
        ] {
            assert_eq!(rgba, RED, "{name}");
        }
    }

    #[test]
    fn alpha_rules() {
        assert_eq!(Rgb555::from_rgba8([0, 0, 0, 255]).0, 0x8000, "opaque black keeps STP so it is drawn");
        assert_eq!(Rgb555::from_rgba8([9, 9, 9, 0]).0, 0, "transparent is the zero word");
        assert_eq!(Argb1555(0x7fff).to_rgba8()[3], 0);
        assert_eq!(Rgba5551(0xfffe).to_rgba8()[3], 0);
        assert_eq!(Rgb5a3(0x7fff).to_rgba8(), [255, 255, 255, 255], "3-bit alpha 7 is opaque");
        assert_eq!(Rgb5a3(0x0fff).to_rgba8()[3], 0);
        assert_eq!(Rgb5a3::from_rgba8([255, 255, 255, 128]).0 >> 12, 4, "translucent picks the 3444 form");
        assert_eq!(Ia16(0x80ff).to_rgba8(), [128, 128, 128, 255]);
        assert_eq!(Ia8(0xf0).to_rgba8(), [255, 255, 255, 0]);
    }

    #[test]
    fn rgb555_channels_top_bit_and_both_expansions() {
        let c = Rgb555(0x7c1f | 0x8000);
        assert_eq!(c.channels(), [31, 0, 31]);
        assert!(c.top() && !c.is_transparent());
        assert_eq!(Rgb555::new(31, 0, 31, true), c);
        let white = Rgb555(0x7fff);
        assert_eq!((white.rgb8(), white.rgb8_shifted()), ([255; 3], [248; 3]));
        assert_eq!(Rgb555::new(16, 1, 0, false).rgb8(), [132, 8, 0]);
        assert_eq!(Rgb555::from_rgb8([255, 128, 7]).channels(), [31, 16, 0]);
        assert_eq!(Rgb555(0).rgba8(), [0, 0, 0, 0]);
    }

    #[test]
    fn ps2_alpha_is_0x80_for_one() {
        assert_eq!(PsmCt32([1, 2, 3, 0x80]).to_rgba8(), [1, 2, 3, 255]);
        assert_eq!(PsmCt32([0, 0, 0, 0x40]).to_rgba8()[3], 128);
        assert_eq!(PsmCt32([0, 0, 0, 0xff]).to_rgba8()[3], 255, "over 1.0 saturates");
        assert_eq!(PsmCt32::from_rgba8([0, 0, 0, 255]).0[3], 0x80);
        assert_eq!(PsmCt32::from_rgba8([0, 0, 0, 0]).0[3], 0);
    }

    #[test]
    fn ycbcr_greys_stay_grey_and_ranges_differ() {
        assert_eq!(ycbcr_to_rgb(128, 128, 128, Range::Full), [128; 3]);
        assert_eq!(ycbcr_to_rgb(16, 128, 128, Range::Limited), [0; 3]);
        assert_eq!(ycbcr_to_rgb(235, 128, 128, Range::Limited), [255; 3]);
        assert_eq!(ycbcr_to_rgb(16, 128, 128, Range::Full), [16; 3]);
        // BT.601 red: Y 76, Cb 85, Cr 255 in full range.
        let [r, g, b] = ycbcr_to_rgb(76, 85, 255, Range::Full);
        assert!(r >= 253 && g <= 2 && b <= 1, "{:?}", [r, g, b]);
        assert_eq!(ycbcr_to_rgb(255, 255, 255, Range::Limited)[0], 255, "clamped");
    }

    #[test]
    fn indexed_images_look_up_their_palette() {
        let pal = [Rgb565(0), Rgb565(0xf800), Rgb565(0x07e0)];
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        assert_eq!(indexed4(&[0x21], &pal, Nibbles::LowFirst), [red, green]);
        assert_eq!(indexed4(&[0x21], &pal, Nibbles::HighFirst), [green, red]);
        assert_eq!(indexed8(&[2, 1, 9], &pal), [green, red, [0; 4]], "past the palette is transparent");
    }

    #[test]
    fn the_ps2_clut_swaps_index_bits_3_and_4() {
        assert_eq!((ps2_clut_index(8), ps2_clut_index(16), ps2_clut_index(24)), (16, 8, 24));
        assert_eq!((ps2_clut_index(0), ps2_clut_index(7), ps2_clut_index(0xe7)), (0, 7, 0xe7));
        for i in 0..=255 {
            assert_eq!(ps2_clut_index(ps2_clut_index(i)), i, "its own inverse");
        }
    }
}
