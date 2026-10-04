//! CD-XA ADPCM: the PS1 CD drive's streamed audio (movie sound, speech,
//! streamed music), decoded from a sector and resampled to 44100 Hz the way
//! the drive does before the SPU mixes it in.
//!
//! A sector's audio data (after the 8-byte subheader of a Form 2 sector) is
//! 18 portions of 128 bytes:
//!
//! ```text
//! 0x00-0x0f  headers: byte 4 + n is block n's shift (bits 0-3, 13-15 act
//!            as 9) and filter (bits 4-5); bytes 0-3 and 12-15 are copies
//! 0x10-0x7f  28 words, word j holding sample j of every block: block n in
//!            bits 4n..4n+4 (signed 4-bit)
//! sample = (nibble << (12 - shift)) + ((s1 * F0 + s2 * F1 + 32) >> 6),
//!          clamped to i16; F0/F1 = (0, 0), (60, 0), (115, -52), (98, -55)
//! ```
//!
//! Mono plays blocks 0-7 in order; stereo takes the even blocks left and the
//! odd right. The subheader's coding byte ([`Coding`]) says which, and the
//! rate: 37800 Hz goes through seven 29-tap zigzag filters for every six
//! samples (7/6 of 37800 is 44100); 18900 Hz is each sample taken twice
//! first. The filters are psx-spx's ("CDROM XA Audio ADPCM Compression"),
//! which notes they give "nearly correct results" against the hardware;
//! each passes a held level at about 0.91 of itself. The history and the
//! filter ring carry from sector to sector of a stream ([`Decoder`]).
//!
//! Not here: 8-bit XA (no game known to these ports uses it; it decodes to
//! nothing), the drive's volume matrix (the game's `CdMix`) and choosing a
//! stream's sectors by file and channel - the game's or the disc reader's.

/// Bytes of audio in a sector: 18 portions of 128.
pub const SECTOR_AUDIO: usize = PORTIONS * 128;

const PORTIONS: usize = 18;
const POS: [i32; 4] = [0, 60, 115, 98];
const NEG: [i32; 4] = [0, 0, -52, -55];

/// The subheader's coding byte (its fourth byte).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Coding(pub u8);

impl Coding {
    /// Stereo (bits 0-1 = 1): even blocks left, odd right.
    pub fn stereo(self) -> bool {
        self.0 & 3 == 1
    }

    /// 18900 Hz (bits 2-3 = 1) rather than 37800.
    pub fn half_rate(self) -> bool {
        (self.0 >> 2) & 3 == 1
    }

    /// 8 bits a sample (bits 4-5 = 1) rather than 4.
    pub fn eight_bit(self) -> bool {
        (self.0 >> 4) & 3 == 1
    }
}

/// The zigzag filters: tap `i` (newest sample first) of output `j` of each
/// six inputs, in 0x8000ths.
#[rustfmt::skip]
const ZIGZAG: [[i32; 7]; 29] = [
    [0, 0, 0, 0, -0x0001, 0x0002, -0x0005],
    [0, 0, 0, -0x0001, 0x0003, -0x0008, 0x0011],
    [0, 0, -0x0001, 0x0003, -0x0008, 0x0010, -0x0023],
    [0, -0x0002, 0x0003, -0x0008, 0x0011, -0x0023, 0x0046],
    [0, 0, -0x0002, 0x0006, -0x0010, 0x002b, -0x0017],
    [-0x0002, 0x0003, -0x0005, 0x0005, 0x000a, 0x001a, -0x0044],
    [0x000a, -0x0013, 0x001f, -0x001b, 0x006b, -0x00eb, 0x015b],
    [-0x0022, 0x003c, -0x004a, 0x00a6, -0x016d, 0x027b, -0x0347],
    [0x0041, -0x004b, 0x00b3, -0x01a8, 0x0350, -0x0548, 0x080e],
    [-0x0054, 0x00a2, -0x0192, 0x0372, -0x0623, 0x0afa, -0x1249],
    [0x0034, -0x00e3, 0x02b1, -0x05bf, 0x0bcd, -0x16fa, 0x3c07],
    [0x0009, 0x0132, -0x039e, 0x09b8, -0x1780, 0x53e0, 0x53e0],
    [-0x010a, -0x0043, 0x04f8, -0x11b4, 0x6794, 0x3c07, -0x16fa],
    [0x0400, -0x0267, -0x05a6, 0x74bb, 0x234c, -0x1249, 0x0afa],
    [-0x0a78, 0x0c9d, 0x7939, 0x0c9d, -0x0a78, 0x080e, -0x0548],
    [0x234c, 0x74bb, -0x05a6, -0x0267, 0x0400, -0x0347, 0x027b],
    [0x6794, -0x11b4, 0x04f8, -0x0043, -0x010a, 0x015b, -0x00eb],
    [-0x1780, 0x09b8, -0x039e, 0x0132, 0x0009, -0x0044, 0x001a],
    [0x0bcd, -0x05bf, 0x02b1, -0x00e3, 0x0034, -0x0017, 0x002b],
    [-0x0623, 0x0372, -0x0192, 0x00a2, -0x0054, 0x0046, -0x0023],
    [0x0350, -0x01a8, 0x00b3, -0x004b, 0x0041, -0x0023, 0x0010],
    [-0x016d, 0x00a6, -0x004a, 0x003c, -0x0022, 0x0011, -0x0008],
    [0x006b, -0x001b, 0x001f, -0x0013, 0x000a, -0x0005, 0x0002],
    [0x000a, 0x0005, -0x0005, 0x0003, -0x0001, 0, 0],
    [-0x0010, 0x0006, -0x0002, 0, 0, 0, 0],
    [0x0011, -0x0008, 0x0003, -0x0002, 0x0001, 0, 0],
    [-0x0008, 0x0003, -0x0001, 0, 0, 0, 0],
    [0x0003, -0x0001, 0, 0, 0, 0, 0],
    [-0x0001, 0, 0, 0, 0, 0, 0],
];

/// One channel's filter history and resampling ring.
#[derive(Clone, Debug, Default)]
struct Channel {
    s1: i32,
    s2: i32,
    ring: [i16; 32],
    p: usize,
    six: u8,
}

impl Channel {
    /// One 37800 Hz sample in; every sixth, seven 44100 Hz samples out.
    fn push(&mut self, s: i16, out: &mut Vec<i16>) {
        self.ring[self.p & 31] = s;
        self.p += 1;
        self.six = if self.six == 0 { 5 } else { self.six - 1 };
        if self.six == 0 {
            for j in 0..7 {
                let mut sum = 0;
                for (i, taps) in ZIGZAG.iter().enumerate() {
                    sum += (i32::from(self.ring[self.p.wrapping_sub(i + 1) & 31]) * taps[j]) / 0x8000;
                }
                out.push(sum.clamp(-0x8000, 0x7fff) as i16);
            }
        }
    }
}

/// A stream's decoder: the filter history and the resampling rings, carried
/// from sector to sector.
#[derive(Clone, Debug, Default)]
pub struct Decoder {
    ch: [Channel; 2],
}

impl Decoder {
    /// A decoder for a new stream: no history.
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Decodes a sector's audio data (at least [`SECTOR_AUDIO`] bytes, the
    /// sector's data after its subheader) and appends it to `out` as stereo
    /// frames at 44100 Hz, left then right (mono on both). An 8-bit or short
    /// sector adds nothing.
    pub fn decode(&mut self, coding: Coding, data: &[u8], out: &mut Vec<(i16, i16)>) {
        if coding.eight_bit() || data.len() < SECTOR_AUDIO {
            return;
        }
        let stereo = coding.stereo();
        let repeat = if coding.half_rate() { 2 } else { 1 };
        let mut pcm: [Vec<i16>; 2] = [Vec::new(), Vec::new()];
        for portion in data[..SECTOR_AUDIO].as_chunks::<128>().0 {
            for block in 0..4 {
                for nibble in 0..2 {
                    let c = if stereo { nibble } else { 0 };
                    let head = portion[4 + block * 2 + nibble];
                    let range = match head & 15 {
                        r if r > 12 => 9,
                        r => r,
                    };
                    let shift = 12 - i32::from(range);
                    let filter = usize::from((head >> 4) & 3);
                    let (f0, f1) = (POS[filter], NEG[filter]);
                    let ch = &mut self.ch[c];
                    for j in 0..28 {
                        let b = portion[16 + block + j * 4] >> (nibble * 4);
                        let t = i32::from(((b << 4) as i8) >> 4);
                        let s = ((t << shift) + ((ch.s1 * f0 + ch.s2 * f1 + 32) >> 6)).clamp(-0x8000, 0x7fff);
                        ch.s2 = ch.s1;
                        ch.s1 = s;
                        for _ in 0..repeat {
                            ch.push(s as i16, &mut pcm[c]);
                        }
                    }
                }
            }
        }
        if stereo {
            out.extend(pcm[0].iter().zip(&pcm[1]).map(|(&l, &r)| (l, r)));
        } else {
            out.extend(pcm[0].iter().map(|&s| (s, s)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sector whose every block header is `head` and whose nibbles are
    /// `nib` in the blocks `blocks` selects, zero elsewhere.
    fn sector(head: u8, nib: u8, blocks: impl Fn(usize) -> bool) -> Vec<u8> {
        let mut d = vec![0u8; SECTOR_AUDIO];
        for p in d.as_chunks_mut::<128>().0 {
            p[..16].fill(head);
            for j in 0..28 {
                let mut word = 0u32;
                for b in 0..8 {
                    if blocks(b) {
                        word |= u32::from(nib & 15) << (4 * b);
                    }
                }
                p[16 + 4 * j..20 + 4 * j].copy_from_slice(&word.to_le_bytes());
            }
        }
        d
    }

    #[test]
    fn a_sector_makes_its_share_of_44100() {
        // 18 * 8 * 28 = 4032 samples at 37800 mono; 7 out for every 6 in.
        let mut xa = Decoder::new();
        let mut out = Vec::new();
        xa.decode(Coding(0), &[0; 0x914], &mut out);
        assert_eq!(out.len(), 4032 * 7 / 6);
        // Stereo halves it; 18900 doubles it back.
        let mut out = Vec::new();
        xa.decode(Coding(1), &[0; 0x914], &mut out);
        assert_eq!(out.len(), 2016 * 7 / 6);
        let mut out = Vec::new();
        xa.decode(Coding(5), &[0; 0x914], &mut out);
        assert_eq!(out.len(), 4032 * 7 / 6);
    }

    #[test]
    fn the_filters_pass_a_constant_at_the_same_gain() {
        // Each filter's taps sum to 0x73e5-0x741d of 0x8000.
        for j in 0..7 {
            let sum: i32 = ZIGZAG.iter().map(|t| t[j]).sum();
            assert!((0x73e5..=0x741d).contains(&sum), "filter {j}: {sum:#x}");
        }
    }

    #[test]
    fn stereo_puts_even_blocks_left_and_odd_right() {
        // Range 0 (the loudest), filter 0: nibble 1 is 1 << 12.
        let d = sector(0x00, 1, |b| b % 2 == 0);
        let mut out = Vec::new();
        Decoder::new().decode(Coding(1), &d, &mut out);
        assert!(out[100..].iter().all(|&(l, r)| l > 3000 && r == 0), "{:?}", &out[100..104]);
    }

    #[test]
    fn a_held_level_comes_out_at_the_filters_gain() {
        let d = sector(0x00, 1, |_| true);
        let mut out = Vec::new();
        Decoder::new().decode(Coding(0), &d, &mut out);
        // 4096 in, 0x73e5..0x741d / 0x8000 of it out, less the truncation
        // of each of 29 products.
        assert!(out[100..].iter().all(|&(l, r)| l == r && (3680..=3720).contains(&l)), "{:?}", &out[100..104]);
    }

    #[test]
    fn the_history_carries_into_the_next_sector() {
        // Filter 1 adds 60/64 of the last sample: zeros after a loud sector
        // decay rather than dropping to silence at once.
        let loud = sector(0x00, 7, |_| true);
        let quiet = sector(0x10, 0, |_| true);
        let mut xa = Decoder::new();
        let mut out = Vec::new();
        xa.decode(Coding(0), &loud, &mut out);
        let n = out.len();
        xa.decode(Coding(0), &quiet, &mut out);
        assert!(out[n + 40].0 > 1000, "{:?}", out[n + 40]);
        let mut fresh = Vec::new();
        Decoder::new().decode(Coding(0), &quiet, &mut fresh);
        assert!(fresh.iter().all(|&(l, _)| l == 0));
    }

    #[test]
    fn eight_bit_and_short_sectors_decode_to_nothing() {
        let mut out = Vec::new();
        Decoder::new().decode(Coding(0x10), &[0; 0x914], &mut out);
        Decoder::new().decode(Coding(0), &[0; 100], &mut out);
        assert!(out.is_empty());
    }
}
