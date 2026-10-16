//! Sony VAB sound banks: the `.VH` header and where each sample sits in the
//! `.VB` body (the samples are PS-ADPCM, [`crate::adpcm::spu`]).
//!
//! ```text
//! VH
//!   char[4]  "pBAV"
//!   u32      version
//!   u32      id
//!   u32      size              VH + VB bytes
//!   u16      reserved
//!   u16      programs          programs with tone blocks
//!   u16      tones             total over all programs
//!   u16      vags              samples in the VB
//!   u8       master volume, master pan, attr1, attr2
//!   u32      reserved
//!   program[128]   16 bytes each
//!   tone[16 * programs]  32 bytes each, 16 slots a program in use
//!   u16      vag_size[256]     entry 0 unused; entry n = bytes of sample n / 8
//! VB
//!   the samples back to back, 16-byte ADPCM frames
//! ```
//!
//! Only the first `programs` programs with tones have tone blocks. Some
//! banks list more past the count (Rugrats' CIRCUS, LIGHT, RACE and six
//! others list programs up to 14 with one tone each and nothing behind
//! them); those are dropped.

/// A program (an instrument): how many tones it has, its volume and pan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// Tones in use.
    pub tones: u8,
    /// Volume, 0-127.
    pub volume: u8,
    /// Priority.
    pub priority: u8,
    /// Mode.
    pub mode: u8,
    /// Pan, 0-127 with 64 the centre.
    pub pan: u8,
    /// Attributes.
    pub attr: u16,
}

/// A tone: one sample over a range of notes, with its tuning and envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tone {
    /// Priority.
    pub priority: u8,
    /// Mode (4 is reverb on).
    pub mode: u8,
    /// Volume, 0-127.
    pub volume: u8,
    /// Pan, 0-127 with 64 the centre.
    pub pan: u8,
    /// The note at which the sample plays at its recorded rate.
    pub center: u8,
    /// Fine tuning of `center`, in 1/128 semitones.
    pub shift: u8,
    /// The lowest note it plays.
    pub min_note: u8,
    /// The highest note it plays.
    pub max_note: u8,
    /// Vibrato width.
    pub vibrato_width: u8,
    /// Vibrato time.
    pub vibrato_time: u8,
    /// Portamento width.
    pub portamento_width: u8,
    /// Portamento time.
    pub portamento_time: u8,
    /// Pitch bend range down, in semitones.
    pub pitch_bend_min: u8,
    /// Pitch bend range up, in semitones.
    pub pitch_bend_max: u8,
    /// The SPU's ADSR registers.
    pub adsr1: u16,
    /// See [`Tone::adsr1`].
    pub adsr2: u16,
    /// The program it belongs to.
    pub program: u16,
    /// 1-based sample number in the VB.
    pub vag: u16,
}

/// A bank's header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vab {
    /// The bank id.
    pub id: u32,
    /// The format version.
    pub version: u32,
    /// Master volume, 0-127.
    pub master_volume: u8,
    /// Master pan.
    pub master_pan: u8,
    /// The programs in use, by program number.
    pub programs: Vec<(usize, Program)>,
    /// Tones per program, in program order.
    pub tones: Vec<Tone>,
    /// Byte size of each sample in the VB, 1-based numbering at index 0.
    pub vag_sizes: Vec<u32>,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Vab {
    /// Reads a `.VH`.
    pub fn parse(vh: &[u8]) -> Result<Vab, String> {
        if vh.len() < 32 || &vh[..4] != b"pBAV" {
            return Err("no pBAV magic".into());
        }
        let programs_used = u16_at(vh, 18) as usize;
        let tone_count = u16_at(vh, 20) as usize;
        let vags = u16_at(vh, 22) as usize;
        let tones_at = 32 + 128 * 16;
        let sizes_at = tones_at + programs_used * 16 * 32;
        if vh.len() < sizes_at + 512 {
            return Err(format!("VH is {} bytes, needs {}", vh.len(), sizes_at + 512));
        }
        let mut programs: Vec<(usize, Program)> = (0..128)
            .map(|p| (p, &vh[32 + p * 16..48 + p * 16]))
            .filter(|(_, r)| r[0] != 0)
            .map(|(p, r)| {
                (p, Program { tones: r[0], volume: r[1], priority: r[2], mode: r[3], pan: r[4], attr: u16_at(r, 6) })
            })
            .collect();
        programs.truncate(programs_used);
        let mut tones = Vec::new();
        for (slot, (_, prog)) in programs.iter().enumerate() {
            for t in 0..prog.tones as usize {
                let at = tones_at + slot * 16 * 32 + t * 32;
                let r = &vh[at..at + 32];
                tones.push(Tone {
                    priority: r[0],
                    mode: r[1],
                    volume: r[2],
                    pan: r[3],
                    center: r[4],
                    shift: r[5],
                    min_note: r[6],
                    max_note: r[7],
                    vibrato_width: r[8],
                    vibrato_time: r[9],
                    portamento_width: r[10],
                    portamento_time: r[11],
                    pitch_bend_min: r[12],
                    pitch_bend_max: r[13],
                    adsr1: u16_at(r, 16),
                    adsr2: u16_at(r, 18),
                    program: u16_at(r, 20),
                    vag: u16_at(r, 22),
                });
            }
        }
        if tones.len() != tone_count {
            return Err(format!("header says {tone_count} tones, programs hold {}", tones.len()));
        }
        let vag_sizes = (0..=vags.min(255)).map(|i| u16_at(vh, sizes_at + 2 * i) as u32 * 8).collect();
        Ok(Vab {
            id: u32_at(vh, 8),
            version: u32_at(vh, 4),
            master_volume: vh[24],
            master_pan: vh[25],
            programs,
            tones,
            vag_sizes,
        })
    }

    /// The bytes of sample `n` (1-based) within the VB.
    pub fn vag<'a>(&self, vb: &'a [u8], n: usize) -> Option<&'a [u8]> {
        let start: u32 = self.vag_sizes.get(1..n)?.iter().sum();
        let len = *self.vag_sizes.get(n)?;
        vb.get(start as usize..(start + len) as usize)
    }

    /// Program `p`'s tones.
    pub fn tones_of(&self, p: u8) -> &[Tone] {
        let mut at = 0;
        for (n, prog) in &self.programs {
            if *n == p as usize {
                return self.tones.get(at..at + prog.tones as usize).unwrap_or(&[]);
            }
            at += prog.tones as usize;
        }
        &[]
    }

    /// Program `p`'s volume and pan; (127, 64) for one not in the bank.
    pub fn program_volume(&self, p: u8) -> (u8, u8) {
        self.programs.iter().find(|(n, _)| *n == p as usize).map_or((127, 64), |(_, g)| (g.volume, g.pan))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A VH with `used` programs counted and `listed` listed, one tone each.
    fn vh(used: u16, listed: usize) -> Vec<u8> {
        let mut b = vec![0u8; 32 + 128 * 16 + used as usize * 16 * 32 + 512];
        b[..4].copy_from_slice(b"pBAV");
        b[18..20].copy_from_slice(&used.to_le_bytes());
        b[20..22].copy_from_slice(&used.to_le_bytes());
        b[22..24].copy_from_slice(&1u16.to_le_bytes());
        b[24] = 127;
        for p in 0..listed {
            b[32 + p * 16] = 1;
            b[33 + p * 16] = 100;
        }
        for slot in 0..used as usize {
            let at = 32 + 128 * 16 + slot * 16 * 32;
            b[at + 4] = 60;
            b[at + 7] = 127;
            b[at + 22] = 1;
        }
        let sizes = 32 + 128 * 16 + used as usize * 16 * 32;
        b[sizes + 2..sizes + 4].copy_from_slice(&2u16.to_le_bytes());
        b
    }

    #[test]
    fn programs_and_tones_line_up() {
        let v = Vab::parse(&vh(2, 2)).unwrap();
        assert_eq!(v.programs.len(), 2);
        assert_eq!(v.tones_of(1).len(), 1);
        assert_eq!(v.tones_of(5), &[]);
        assert_eq!(v.program_volume(0), (100, 0));
        assert_eq!(v.program_volume(9), (127, 64));
        assert_eq!(v.vag(&[0u8; 32], 1).map(<[u8]>::len), Some(16));
    }

    /// Programs listed past the header's count have no tone blocks: they
    /// are dropped, not read past the VH's end.
    #[test]
    fn programs_past_the_count_are_dropped() {
        let v = Vab::parse(&vh(2, 15)).unwrap();
        assert_eq!((v.programs.len(), v.tones.len()), (2, 2));
    }

    #[test]
    fn short_or_foreign_headers_are_errors() {
        assert!(Vab::parse(b"pBAV").is_err());
        assert!(Vab::parse(&[0u8; 4000]).is_err());
        let mut b = vh(2, 2);
        b.truncate(b.len() - 600);
        assert!(Vab::parse(&b).is_err());
    }
}
