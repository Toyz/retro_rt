//! The PS1's and PS2's SPU: the sample format and the interpolation every
//! game's sound goes through.
//!
//! **PS-ADPCM** (VAG data, VAB/VH banks, SPU2 sound data) is 16-byte frames
//! of 28 samples:
//!
//! ```text
//! byte 0     shift | filter << 4      shift 13-15 acts as 9; filter 0-4
//! byte 1     flags                    END 0x01, REPEAT 0x02, LOOP_START 0x04
//! bytes 2-15 28 signed 4-bit nibbles, the low nibble of each byte first
//! sample = (nibble << 12 >> shift) + ((s1 * F0 + s2 * F1 + 32) >> 6), clamped to i16
//! ```
//!
//! `s1` and `s2` are the last two samples and carry across frames
//! ([`History`]). A sample ends with the frame whose END bit is set, which is
//! played; with REPEAT too, playback jumps back to the last LOOP_START
//! frame, keeping its history. The format is the same on both consoles;
//! hwtr and piney_apples decoded it identically before this module.
//!
//! [`GAUSS`] and [`interpolate`] are the SPU's 4-point Gaussian
//! interpolation, which every voice is resampled through.
//!
//! Not here: the voice state machine, ADSR envelopes, reverb, the mixer -
//! each game's sound engine ports or drives those its own way. XA-ADPCM (the
//! PS1 CD's streamed audio) is [`super::xa`].

/// Bytes in a frame.
pub const FRAME: usize = 16;
/// Samples a frame decodes to.
pub const PER_FRAME: usize = 28;

/// Flag: the sample ends with this frame (which plays).
pub const END: u8 = 0x01;
/// Flag, with [`END`]: jump back to the loop start instead of stopping.
pub const REPEAT: u8 = 0x02;
/// Flag: the loop starts at this frame.
pub const LOOP_START: u8 = 0x04;

/// The prediction filters' coefficients, in 64ths: `(F0, F1)` for filters
/// 0-4. A filter number above 4 acts as 4.
pub const FILTERS: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

/// The filter history carried from sample to sample and frame to frame:
/// the last two outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// The last sample.
    pub s1: i32,
    /// The one before.
    pub s2: i32,
}

/// Decodes one 16-byte frame into `out`, carrying `hist`; returns how many
/// samples hit the 16-bit clamp.
///
/// # Panics
///
/// When `frame` is shorter than [`FRAME`].
pub fn decode_frame(frame: &[u8], hist: &mut History, out: &mut [i16; PER_FRAME]) -> usize {
    let head = frame[0];
    let shift = match u32::from(head & 15) {
        s if s > 12 => 9,
        s => s,
    };
    let (f0, f1) = FILTERS[usize::from(head >> 4).min(4)];
    let (mut s1, mut s2) = (hist.s1, hist.s2);
    let mut clipped = 0;
    for (i, slot) in out.iter_mut().enumerate() {
        let b = frame[2 + i / 2];
        let n = i32::from(if i & 1 == 0 { b & 15 } else { b >> 4 });
        let n = if n & 8 != 0 { n - 16 } else { n };
        let s = ((n << 12) >> shift) + ((s1 * f0 + s2 * f1 + 32) >> 6);
        let c = s.clamp(-32768, 32767);
        clipped += usize::from(c != s);
        *slot = c as i16;
        s2 = s1;
        s1 = c;
    }
    *hist = History { s1, s2 };
    clipped
}

/// A decoded sample: its PCM, and where a repeat jumps back to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    /// Every frame up to and including the END frame, as PCM.
    pub pcm: Vec<i16>,
    /// The first PCM index of the last LOOP_START frame at or before the
    /// end, when the END frame also has REPEAT: where playback continues.
    pub loop_start: Option<usize>,
    /// Samples that hit the 16-bit clamp.
    pub clipped: usize,
}

/// Decodes `data` frame by frame up to and including the first frame with
/// [`END`] (or to the data's end; a trailing partial frame is ignored).
pub fn decode_sample(data: &[u8]) -> Sample {
    let mut out = Sample { pcm: Vec::with_capacity(data.len() / FRAME * PER_FRAME), ..Sample::default() };
    let mut hist = History::default();
    let mut frame_out = [0i16; PER_FRAME];
    let mut last_loop = None;
    for frame in data.as_chunks::<FRAME>().0 {
        if frame[1] & LOOP_START != 0 {
            last_loop = Some(out.pcm.len());
        }
        out.clipped += decode_frame(frame, &mut hist, &mut frame_out);
        out.pcm.extend_from_slice(&frame_out);
        if frame[1] & END != 0 {
            if frame[1] & REPEAT != 0 {
                out.loop_start = last_loop;
            }
            break;
        }
    }
    out
}

/// [`decode_sample`]'s PCM alone: one pass of the sample.
pub fn decode(data: &[u8]) -> Vec<i16> {
    decode_sample(data).pcm
}

/// The SPU's 4-point Gaussian interpolation table (psx-spx, "4-Point
/// Gaussian Interpolation"): a four-term window; the four entries
/// `[i], [0xff - i], [0x100 + i], [0x1ff - i]` sum to 0x7f7f-0x7f81 for every
/// `i`. Carried over from piney_apples' SPU, unchanged.
#[rustfmt::skip]
pub static GAUSS: [i16; 512] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 3, 3,
    3, 4, 4, 5, 5, 6, 7, 7, 8, 9, 9, 10, 11, 12, 13, 14,
    15, 16, 17, 18, 19, 21, 22, 24, 25, 27, 28, 30, 32, 33, 35, 37,
    39, 41, 44, 46, 48, 51, 53, 56, 58, 61, 64, 67, 70, 73, 77, 80,
    84, 87, 91, 95, 99, 103, 107, 111, 116, 120, 125, 130, 135, 140, 145, 150,
    156, 161, 167, 173, 179, 186, 192, 199, 205, 212, 219, 227, 234, 242, 250, 257,
    266, 274, 283, 291, 300, 309, 319, 328, 338, 348, 358, 369, 379, 390, 401, 412,
    424, 436, 448, 460, 473, 485, 498, 512, 525, 539, 553, 567, 582, 597, 612, 627,
    643, 659, 675, 692, 708, 726, 743, 761, 779, 797, 816, 835, 854, 874, 894, 914,
    935, 956, 977, 999, 1020, 1043, 1066, 1089, 1112, 1136, 1160, 1184, 1209, 1234, 1260, 1286,
    1312, 1339, 1366, 1394, 1422, 1450, 1479, 1508, 1537, 1567, 1598, 1628, 1660, 1691, 1723, 1756,
    1789, 1822, 1856, 1890, 1924, 1959, 1995, 2031, 2067, 2104, 2141, 2179, 2217, 2256, 2295, 2334,
    2374, 2415, 2456, 2497, 2539, 2582, 2624, 2668, 2712, 2756, 2801, 2846, 2892, 2938, 2985, 3032,
    3079, 3128, 3176, 3225, 3275, 3325, 3376, 3427, 3479, 3531, 3584, 3637, 3691, 3745, 3799, 3855,
    3910, 3967, 4023, 4081, 4138, 4197, 4255, 4315, 4374, 4435, 4495, 4557, 4619, 4681, 4744, 4807,
    4871, 4935, 5000, 5065, 5131, 5197, 5264, 5332, 5399, 5468, 5536, 5606, 5676, 5746, 5817, 5888,
    5959, 6032, 6104, 6177, 6251, 6325, 6400, 6475, 6550, 6626, 6702, 6779, 6856, 6934, 7012, 7091,
    7170, 7249, 7329, 7409, 7490, 7571, 7653, 7735, 7817, 7900, 7983, 8066, 8150, 8234, 8319, 8404,
    8489, 8575, 8661, 8748, 8834, 8922, 9009, 9097, 9185, 9273, 9362, 9451, 9541, 9630, 9720, 9811,
    9901, 9992, 10083, 10174, 10266, 10358, 10450, 10542, 10635, 10727, 10820, 10913, 11007, 11100, 11194, 11288,
    11382, 11476, 11571, 11665, 11760, 11855, 11950, 12045, 12140, 12236, 12331, 12427, 12522, 12618, 12714, 12809,
    12905, 13001, 13097, 13193, 13289, 13385, 13481, 13577, 13673, 13769, 13865, 13961, 14056, 14152, 14248, 14343,
    14439, 14534, 14630, 14725, 14820, 14915, 15010, 15104, 15199, 15293, 15387, 15481, 15575, 15669, 15762, 15855,
    15948, 16041, 16133, 16226, 16317, 16409, 16500, 16592, 16682, 16773, 16863, 16953, 17042, 17131, 17220, 17308,
    17396, 17484, 17571, 17658, 17744, 17830, 17916, 18001, 18086, 18170, 18254, 18337, 18420, 18502, 18584, 18665,
    18746, 18826, 18905, 18985, 19063, 19141, 19219, 19295, 19372, 19447, 19522, 19597, 19671, 19744, 19816, 19888,
    19959, 20030, 20100, 20169, 20238, 20306, 20373, 20439, 20505, 20570, 20634, 20698, 20760, 20822, 20884, 20944,
    21004, 21063, 21121, 21178, 21235, 21290, 21345, 21399, 21452, 21505, 21556, 21607, 21657, 21706, 21754, 21801,
    21848, 21893, 21938, 21982, 22025, 22066, 22107, 22148, 22187, 22225, 22262, 22299, 22334, 22369, 22402, 22435,
    22467, 22498, 22527, 22556, 22584, 22611, 22637, 22662, 22686, 22709, 22731, 22752, 22772, 22791, 22809, 22826,
    22842, 22857, 22872, 22885, 22897, 22908, 22918, 22927, 22935, 22942, 22948, 22953, 22957, 22960, 22962, 22963,
];

/// One output sample interpolated from four consecutive decoded samples
/// `s[0..4]` (oldest first) at fractional position `i` (0-255, the pitch
/// counter's bits 4-11), as the SPU computes it: each product shifted right
/// 15 before the sum.
pub fn interpolate(s: [i16; 4], i: u8) -> i32 {
    let i = usize::from(i);
    let g = |k: usize| i32::from(GAUSS[k]);
    let b = |k: usize| i32::from(s[k]);
    ((g(0xff - i) * b(0)) >> 15) + ((g(0x1ff - i) * b(1)) >> 15) + ((g(0x100 + i) * b(2)) >> 15) + ((g(i) * b(3)) >> 15)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame: shift, filter, flags and 28 nibbles.
    fn frame(shift: u8, filter: u8, flags: u8, nibbles: &[i8; 28]) -> [u8; 16] {
        let mut f = [0u8; 16];
        f[0] = shift | filter << 4;
        f[1] = flags;
        for (i, n) in nibbles.iter().enumerate() {
            let v = (*n as u8) & 15;
            f[2 + i / 2] |= if i & 1 == 0 { v } else { v << 4 };
        }
        f
    }

    #[test]
    fn a_silent_end_frame_is_28_zeros() {
        assert_eq!(decode(&frame(0, 0, END, &[0; 28])), vec![0; 28]);
    }

    /// hwtr's nibble_scaling: shift 12 leaves the nibble's value; shift 0
    /// puts it in the top bits; the low nibble is first.
    #[test]
    fn the_shift_scales_each_nibble() {
        let mut f = [0u8; 16];
        f[0] = 12;
        f[1] = END;
        f[2] = 0x71;
        assert_eq!(&decode(&f)[..2], &[1, 7]);
        f[0] = 0;
        assert_eq!(decode(&f)[0], 4096);
        f[0] = 14;
        assert_eq!(decode(&f)[0], 1 << 3, "shift 13-15 acts as 9");
        let mut neg = [0i8; 28];
        neg[0] = -8;
        assert_eq!(decode(&frame(12, 0, END, &neg))[0], -8);
    }

    /// Filter 1 adds 60/64 of the last sample, and the history carries into
    /// the next frame.
    #[test]
    fn the_filter_history_carries_across_frames() {
        let mut first = [0i8; 28];
        first[27] = 7;
        let mut data = frame(0, 0, 0, &first).to_vec();
        data.extend(frame(12, 1, END, &[0; 28]));
        let pcm = decode(&data);
        assert_eq!(pcm[27], 7 << 12);
        assert_eq!(i32::from(pcm[28]), (28672 * 60 + 32) >> 6, "filter 1 on the carried s1");
    }

    #[test]
    fn samples_past_16_bits_clamp_and_are_counted() {
        let mut data = frame(0, 0, 0, &[7; 28]).to_vec();
        data.extend(frame(0, 4, END, &[7; 28]));
        let s = decode_sample(&data);
        assert!(s.pcm.contains(&i16::MAX));
        assert!(s.clipped > 0);
    }

    #[test]
    fn decoding_stops_at_the_end_frame_and_reports_the_loop() {
        let mut data = frame(12, 0, 0, &[1; 28]).to_vec();
        data.extend(frame(12, 0, LOOP_START, &[2; 28]));
        data.extend(frame(12, 0, END | REPEAT, &[3; 28]));
        data.extend(frame(12, 0, 0, &[4; 28]));
        let s = decode_sample(&data);
        assert_eq!(s.pcm.len(), 3 * PER_FRAME, "the END frame plays; nothing after it");
        assert_eq!(s.loop_start, Some(PER_FRAME));
        assert_eq!(decode_sample(&data[..32]).loop_start, None, "no END: no loop");
        let mut one_shot = data.clone();
        one_shot[33] = END;
        assert_eq!(decode_sample(&one_shot).loop_start, None, "END without REPEAT stops");
    }

    /// piney_apples' gauss_rows_sum_to_255_256ths.
    #[test]
    fn every_gauss_row_sums_to_255_256ths() {
        for i in 0..256 {
            let s: i32 = [i, 0xff - i, 0x100 + i, 0x1ff - i].iter().map(|&k| i32::from(GAUSS[k])).sum();
            assert!((0x7f7f..=0x7f81).contains(&s), "{i}: {s:x}");
        }
    }

    /// A constant comes back scaled by the row's 255/256 (0x7f7f-0x7f81 of
    /// 0x8000), less up to 4 for the four products each truncated by `>> 15`:
    /// 10000 reads 9957-9961, never more.
    #[test]
    fn interpolating_a_constant_gives_back_255_256ths_of_it() {
        for i in 0..=255u8 {
            let v = interpolate([10000; 4], i);
            assert!((9956..=9961).contains(&v), "{i}: {v}");
        }
        assert_eq!(interpolate([0; 4], 77), 0);
    }
}
