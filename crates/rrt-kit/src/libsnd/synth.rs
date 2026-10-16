//! The software SPU: voices, the ADSR envelope, libsnd's SEQ sequencer in
//! song slots, effects keyed on, and the mix.

use std::ops::Range;
use std::sync::Arc;

use super::sep::Song;
use super::vab::{Tone, Vab};
use crate::adpcm::spu;

/// The output rate: the SPU's 44100 Hz, stereo.
pub const RATE: u32 = 44100;

/// The SPU's voices.
pub const VOICES: usize = 24;

/// The main volume libsnd sets (`SsSetMVol` 127: 0x3fff of 0x7fff).
pub const MAIN_VOLUME: i32 = 0x3fff;

/// A bank: its header and its samples decoded.
pub struct Bank {
    /// The header.
    pub vab: Vab,
    samples: Vec<Arc<spu::Sample>>,
}

impl Bank {
    /// The bank of header `vh` and body `vb`.
    pub fn new(vh: &[u8], vb: &[u8]) -> Result<Bank, String> {
        let vab = Vab::parse(vh)?;
        let samples = (0..vab.vag_sizes.len())
            .map(|n| Arc::new(vab.vag(vb, n).map(spu::decode_sample).unwrap_or_default()))
            .collect();
        Ok(Bank { vab, samples })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Attack,
    Decay,
    Sustain,
    Release,
    Off,
}

/// The SPU's envelope (psx-spx "SPU ADSR Generator").
#[derive(Clone, Copy, Debug)]
struct Envelope {
    adsr1: u16,
    adsr2: u16,
    phase: Phase,
    level: i32,
    wait: u32,
}

impl Envelope {
    fn new(adsr1: u16, adsr2: u16) -> Envelope {
        Envelope { adsr1, adsr2, phase: Phase::Attack, level: 0, wait: 0 }
    }

    /// (exponential, decreasing, shift, step) of the phase.
    fn rate(&self) -> (bool, bool, u32, i32) {
        let (a1, a2) = (self.adsr1 as u32, self.adsr2 as u32);
        match self.phase {
            Phase::Attack => (a1 & 0x8000 != 0, false, a1 >> 10 & 31, 7 - (a1 >> 8 & 3) as i32),
            Phase::Decay => (true, true, a1 >> 4 & 15, -8),
            Phase::Sustain => {
                let down = a2 & 0x4000 != 0;
                let step = (a2 >> 6 & 3) as i32;
                (a2 & 0x8000 != 0, down, a2 >> 8 & 31, if down { -8 + step } else { 7 - step })
            }
            Phase::Release => (a2 & 0x20 != 0, true, a2 & 31, -8),
            Phase::Off => (false, true, 0, 0),
        }
    }

    fn step(&mut self) {
        if self.phase == Phase::Off {
            return;
        }
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        let (exp, down, shift, step) = self.rate();
        let mut cycles = 1u32 << shift.saturating_sub(11);
        let mut step = step << 11u32.saturating_sub(shift);
        if exp && !down && self.level > 0x6000 {
            cycles *= 4;
        }
        if exp && down {
            step = (step * self.level) >> 15;
        }
        self.wait = cycles - 1;
        self.level = (self.level + step).clamp(0, 0x7fff);
        match self.phase {
            Phase::Attack if self.level >= 0x7fff => self.phase = Phase::Decay,
            Phase::Decay if self.level <= ((self.adsr1 as i32 & 15) + 1) * 0x800 => self.phase = Phase::Sustain,
            Phase::Release if self.level == 0 => self.phase = Phase::Off,
            _ => {}
        }
    }
}

#[derive(Clone)]
struct Voice {
    sample: Arc<spu::Sample>,
    /// Position in samples, 12 bits of fraction.
    pos: u64,
    pitch: u32,
    env: Envelope,
    vol: (i32, i32),
    /// The song slot, channel and note playing, for note-offs and the
    /// slot's volume; None for an effect.
    key: Option<(usize, u8, u8)>,
    started: u64,
}

impl Voice {
    fn sample_at(&self, i: i64) -> i16 {
        let pcm = &self.sample.pcm;
        let n = pcm.len() as i64;
        if i < 0 || n == 0 {
            return 0;
        }
        if i < n {
            return pcm[i as usize];
        }
        match self.sample.loop_start {
            Some(l) if (l as i64) < n => pcm[(l as i64 + (i - n) % (n - l as i64)) as usize],
            _ => 0,
        }
    }

    /// One output frame.
    fn render(&mut self) -> (i32, i32) {
        let i = (self.pos >> 12) as i64;
        let s = spu::interpolate(
            [self.sample_at(i - 3), self.sample_at(i - 2), self.sample_at(i - 1), self.sample_at(i)],
            (self.pos >> 4 & 255) as u8,
        );
        self.env.step();
        let v = (s * self.env.level) >> 15;
        self.pos += self.pitch.min(0x3fff) as u64;
        let n = self.sample.pcm.len() as u64;
        if self.sample.loop_start.is_none() && self.pos >> 12 >= n + 3 {
            self.env.phase = Phase::Off;
        }
        ((v * self.vol.0) >> 15, (v * self.vol.1) >> 15)
    }
}

#[derive(Clone, Copy, Debug)]
struct Channel {
    program: u8,
    volume: u8,
    pan: u8,
    expression: u8,
    bend: i32,
    nrpn: u8,
}

impl Default for Channel {
    fn default() -> Channel {
        Channel { program: 0, volume: 127, pan: 64, expression: 127, bend: 0, nrpn: 0 }
    }
}

/// A song playing (libsnd's SEQ interpreter).
struct Player {
    song: Song,
    pos: usize,
    status: u8,
    channels: [Channel; 16],
    /// Ticks until the next event, in 1/RATE ticks.
    wait: f64,
    tempo: u32,
    loop_at: Option<usize>,
    loop_count: Option<u8>,
    loops: u32,
    done: bool,
    /// The slot's volume (0-127) and a fade under way: the volume it goes
    /// to, the change a frame, and whether the slot stops there.
    volume: f32,
    fade: Option<(f32, f32, bool)>,
}

impl Player {
    fn new(song: &Song, loops: u8, volume: f32) -> Player {
        let mut p = Player {
            song: song.clone(),
            pos: 0,
            status: 0,
            channels: [Channel::default(); 16],
            wait: 0.0,
            tempo: song.tempo.max(1),
            loop_at: None,
            loop_count: (loops > 0).then_some(loops),
            loops: 0,
            done: false,
            volume,
            fade: None,
        };
        p.wait = p.delta() as f64;
        p
    }

    fn delta(&mut self) -> u32 {
        let mut v = 0u32;
        while let Some(&b) = self.song.events.get(self.pos) {
            self.pos += 1;
            v = v << 7 | (b & 127) as u32;
            if b & 128 == 0 {
                break;
            }
        }
        v
    }
}

/// libsnd's volume from note velocity, tone, program and channel volumes
/// and the pans combined (`SsUtKeyOn` and the SEQ note-on), as SPU voice
/// volumes (0-0x3fff).
fn volumes(parts: &[i32], pans: &[i32]) -> (i32, i32) {
    let v = parts.iter().fold(0x3fff, |v, &p| v * p / 127);
    // Pans 0-127, 64 the centre; each pulls the sound to one side.
    let pan = pans.iter().fold(64i32, |c, &p| (c + p - 64).clamp(0, 127));
    let l = v * (127 - pan).min(64) / 64;
    let r = v * pan.min(64) / 64;
    (l, r)
}

/// The SPU pitch for `note` on a tone sampled at its centre note: 0x1000
/// plays at 44100 Hz.
fn pitch(tone: &Tone, note: u8, bend: i32) -> u32 {
    let semis = note as f64 - tone.center as f64
        + tone.shift as f64 / 128.0
        + bend as f64 / 8192.0 * tone.pitch_bend_max.max(1) as f64;
    (4096.0 * 2f64.powf(semis / 12.0)).round().clamp(0.0, 0x3fff as f64) as u32
}

/// The software SPU and libsnd's sequencer: song slots playing through the
/// music bank, effects keyed on through the effects bank on their own
/// voices.
pub struct Spu {
    voices: Vec<Option<Voice>>,
    music: Option<Arc<Bank>>,
    effects: Option<Arc<Bank>>,
    songs: Vec<Option<Player>>,
    effect_voices: Range<usize>,
    next_effect: usize,
    clock: u64,
}

impl Spu {
    /// An SPU with `slots` song slots (libsnd's SEQ access numbers) whose
    /// effects take `effect_voices` in turn; the songs take the voices
    /// below them.
    pub fn new(slots: usize, effect_voices: Range<usize>) -> Spu {
        let effect_voices = effect_voices.start.min(VOICES)..effect_voices.end.min(VOICES);
        Spu {
            voices: vec![None; VOICES],
            music: None,
            effects: None,
            songs: (0..slots).map(|_| None).collect(),
            next_effect: effect_voices.start,
            effect_voices,
            clock: 0,
        }
    }

    /// The banks songs and effects play through.
    pub fn set_banks(&mut self, music: Arc<Bank>, effects: Arc<Bank>) {
        self.music = Some(music);
        self.effects = Some(effects);
    }

    /// Plays `song` in `slot` at `volume` (0-127): `loops` 0 forever. What
    /// the slot played stops.
    pub fn play(&mut self, slot: usize, song: &Song, loops: u8, volume: i32) {
        self.stop(slot);
        if let Some(s) = self.songs.get_mut(slot) {
            *s = Some(Player::new(song, loops, volume as f32));
        }
    }

    /// Stops `slot`'s song and its notes.
    pub fn stop(&mut self, slot: usize) {
        if let Some(s) = self.songs.get_mut(slot) {
            *s = None;
        }
        for v in self.voices.iter_mut() {
            if v.as_ref().is_some_and(|v| v.key.is_some_and(|k| k.0 == slot)) {
                *v = None;
            }
        }
    }

    /// Stops every song.
    pub fn stop_all(&mut self) {
        for slot in 0..self.songs.len() {
            self.stop(slot);
        }
    }

    /// Whether `slot` has a song that has not ended.
    pub fn playing(&self, slot: usize) -> bool {
        self.songs.get(slot).is_some_and(|s| s.as_ref().is_some_and(|p| !p.done))
    }

    /// `slot`'s volume now, if it has a song.
    pub fn volume(&self, slot: usize) -> Option<f32> {
        self.songs.get(slot)?.as_ref().map(|p| p.volume)
    }

    /// Sets `slot`'s volume at once (`SsSeqSetVol`), notes already
    /// sounding included; a fade under way stops.
    pub fn set_volume(&mut self, slot: usize, volume: i32) {
        if let Some(Some(p)) = self.songs.get_mut(slot) {
            p.volume = volume as f32;
            p.fade = None;
        }
    }

    /// Fades `slot` to `volume` over `frames` output frames
    /// (`SsSeqSetCrescendo` / `SsSeqSetDecrescendo`); with `then_stop` the
    /// slot stops when it gets there.
    pub fn fade(&mut self, slot: usize, volume: i32, frames: u32, then_stop: bool) {
        if let Some(Some(p)) = self.songs.get_mut(slot) {
            let to = volume as f32;
            p.fade = Some((to, (to - p.volume) / frames.max(1) as f32, then_stop));
        }
    }

    /// Keys on tone `tone` of program `program` in the effects bank at
    /// `note`, on the next effect voice, at volumes (left, right) 0-127
    /// (`SsUtKeyOn`'s voll and volr).
    pub fn key_on(&mut self, program: u8, tone: u8, note: u8, (left, right): (i32, i32)) {
        let Some(bank) = self.effects.clone() else { return };
        let Some(t) = bank.vab.tones_of(program).get(tone as usize).cloned() else { return };
        if self.effect_voices.is_empty() {
            return;
        }
        let (pv, pp) = bank.vab.program_volume(program);
        let (l, r) = volumes(&[t.volume as i32, pv as i32, bank.vab.master_volume as i32], &[t.pan as i32, pp as i32]);
        let vol = (l * left / 127, r * right / 127);
        let k = self.next_effect;
        self.next_effect = if k + 1 >= self.effect_voices.end { self.effect_voices.start } else { k + 1 };
        self.voices[k] = self.voice(&bank, &t, note, 0, vol, None);
    }

    fn voice(
        &self,
        bank: &Bank,
        t: &Tone,
        note: u8,
        bend: i32,
        vol: (i32, i32),
        key: Option<(usize, u8, u8)>,
    ) -> Option<Voice> {
        let sample = bank.samples.get(t.vag as usize)?.clone();
        Some(Voice {
            sample,
            pos: 0,
            pitch: pitch(t, note, bend),
            env: Envelope::new(t.adsr1, t.adsr2),
            vol,
            key,
            started: self.clock,
        })
    }

    fn note_on(&mut self, slot: usize, c: Channel, ch: u8, note: u8, vel: u8) {
        let Some(bank) = self.music.clone() else { return };
        let (pv, pp) = bank.vab.program_volume(c.program);
        let tones: Vec<Tone> = bank
            .vab
            .tones_of(c.program)
            .iter()
            .filter(|t| (t.min_note..=t.max_note).contains(&note))
            .cloned()
            .collect();
        let below = self.effect_voices.start;
        for t in tones {
            let vol = volumes(
                &[
                    vel as i32,
                    t.volume as i32,
                    pv as i32,
                    c.volume as i32,
                    c.expression as i32,
                    bank.vab.master_volume as i32,
                ],
                &[t.pan as i32, pp as i32, c.pan as i32],
            );
            let v = self.voice(&bank, &t, note, c.bend, vol, Some((slot, ch, note)));
            // A free voice below the effects', else the oldest.
            let k = (0..below)
                .find(|&k| self.voices[k].as_ref().is_none_or(|v| v.env.phase == Phase::Off))
                .or_else(|| (0..below).min_by_key(|&k| self.voices[k].as_ref().map_or(0, |v| v.started)));
            if let Some(k) = k {
                self.voices[k] = v;
            }
        }
    }

    fn note_off(&mut self, slot: usize, ch: u8, note: u8) {
        for v in self.voices.iter_mut().flatten() {
            if v.key == Some((slot, ch, note)) && v.env.phase != Phase::Off {
                v.env.phase = Phase::Release;
                v.env.wait = 0;
                v.key = Some((slot, ch, 255));
            }
        }
    }

    /// Runs `slot`'s events due now.
    fn sequence(&mut self, slot: usize) {
        let Some(mut p) = self.songs[slot].take() else { return };
        // Ticks a frame: resolution per quarter, tempo microseconds a quarter.
        p.wait -= p.song.resolution as f64 * 1e6 / p.tempo as f64 / RATE as f64;
        while p.wait <= 0.0 && !p.done {
            let Some(&b) = p.song.events.get(p.pos) else {
                p.done = true;
                break;
            };
            let status = if b & 0x80 != 0 {
                p.pos += 1;
                if b < 0xf0 {
                    p.status = b;
                }
                b
            } else {
                p.status
            };
            let arg = |p: &mut Player| {
                let v = p.song.events.get(p.pos).copied().unwrap_or(0);
                p.pos += 1;
                v
            };
            let ch = status & 15;
            match status & 0xf0 {
                0x90 => {
                    let (n, v) = (arg(&mut p), arg(&mut p));
                    if v == 0 {
                        self.note_off(slot, ch, n);
                    } else {
                        self.note_on(slot, p.channels[ch as usize], ch, n, v);
                    }
                }
                0x80 => {
                    let n = arg(&mut p);
                    arg(&mut p);
                    self.note_off(slot, ch, n);
                }
                0xb0 => {
                    let (c, v) = (arg(&mut p), arg(&mut p));
                    let chan = &mut p.channels[ch as usize];
                    match c {
                        7 => chan.volume = v,
                        10 => chan.pan = v,
                        11 => chan.expression = v,
                        99 => chan.nrpn = v,
                        // libsnd's loop marks: NRPN 20 a loop start (data
                        // the count), 30 its end.
                        6 if chan.nrpn == 20 => {
                            p.loop_at = Some(p.pos);
                            p.loop_count = p.loop_count.or((v != 127 && v != 0).then_some(v));
                        }
                        6 if chan.nrpn == 30 => {
                            if let Some(at) = p.loop_at {
                                p.loops += 1;
                                if p.loop_count.is_none_or(|n| p.loops < n as u32) {
                                    p.pos = at;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                0xc0 => p.channels[ch as usize].program = arg(&mut p),
                0xd0 => {
                    arg(&mut p);
                }
                0xa0 => {
                    arg(&mut p);
                    arg(&mut p);
                }
                0xe0 => {
                    let (lo, hi) = (arg(&mut p) as i32, arg(&mut p) as i32);
                    p.channels[ch as usize].bend = (hi << 7 | lo) - 8192;
                }
                0xf0 if status == 0xff => {
                    let kind = arg(&mut p);
                    let len = arg(&mut p) as usize;
                    let at = p.pos;
                    p.pos += len;
                    match kind {
                        0x51 if len == 3 => {
                            let e = &p.song.events;
                            let byte = |k: usize| e.get(at + k).copied().unwrap_or(0) as u32;
                            p.tempo = (byte(0) << 16 | byte(1) << 8 | byte(2)).max(1);
                        }
                        0x2f => {
                            // The end: from the top again unless a count ran out.
                            p.loops += 1;
                            if p.loop_count.is_none_or(|n| p.loops < n as u32) {
                                p.pos = 0;
                                p.status = 0;
                            } else {
                                p.done = true;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            if !p.done {
                p.wait += p.delta() as f64;
            }
        }
        self.songs[slot] = Some(p);
    }

    /// One frame of the fades; a fade with `then_stop` stops its slot.
    fn step_fades(&mut self) {
        for slot in 0..self.songs.len() {
            let Some(p) = &mut self.songs[slot] else { continue };
            let Some((to, step, then_stop)) = p.fade else { continue };
            p.volume += step;
            if (step >= 0.0 && p.volume >= to) || (step < 0.0 && p.volume <= to) {
                p.volume = to;
                p.fade = None;
                if then_stop {
                    self.stop(slot);
                }
            }
        }
    }

    /// Mixes into `out` (stereo frames): the songs and effects over what
    /// is there, at the main volume.
    pub fn render(&mut self, out: &mut [i16]) {
        for frame in out.as_chunks_mut::<2>().0 {
            for slot in 0..self.songs.len() {
                self.sequence(slot);
            }
            self.step_fades();
            let (mut l, mut r) = (frame[0] as i32, frame[1] as i32);
            for v in self.voices.iter_mut() {
                let Some(voice) = v else { continue };
                let (mut a, mut b) = voice.render();
                // A song's notes at its slot's volume now (SsSeqSetVol).
                if let Some((slot, ..)) = voice.key {
                    let volume = self.songs.get(slot).and_then(|s| s.as_ref()).map_or(0, |p| p.volume as i32);
                    (a, b) = (a * volume / 127, b * volume / 127);
                }
                l += a;
                r += b;
                if voice.env.phase == Phase::Off {
                    *v = None;
                }
            }
            frame[0] = (frame[0] as i32 + (((l - frame[0] as i32) * MAIN_VOLUME) >> 15)).clamp(-32768, 32767) as i16;
            frame[1] = (frame[1] as i32 + (((r - frame[1] as i32) * MAIN_VOLUME) >> 15)).clamp(-32768, 32767) as i16;
            self.clock += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-program bank: a square-ish looping sample, held sustain.
    fn bank() -> Arc<Bank> {
        let mut vh = vec![0u8; 32 + 128 * 16 + 16 * 32 + 512];
        vh[..4].copy_from_slice(b"pBAV");
        vh[18] = 1;
        vh[20] = 1;
        vh[22] = 1;
        vh[24] = 127;
        vh[32] = 1;
        vh[33] = 127;
        vh[36] = 64;
        let t = 32 + 128 * 16;
        vh[t + 2] = 127;
        vh[t + 3] = 64;
        vh[t + 4] = 60;
        vh[t + 7] = 127;
        // ADSR: fast attack, no decay, full sustain held.
        vh[t + 16..t + 18].copy_from_slice(&0x000fu16.to_le_bytes());
        vh[t + 18..t + 20].copy_from_slice(&0x0000u16.to_le_bytes());
        vh[t + 22] = 1;
        let sizes = t + 16 * 32;
        vh[sizes + 2..sizes + 4].copy_from_slice(&4u16.to_le_bytes());
        // Two frames: loud nibbles, LOOP_START on the first, END|REPEAT.
        let mut vb = vec![0x77u8; 32];
        vb[0] = 0;
        vb[1] = 4;
        vb[16] = 0;
        vb[17] = 3;
        Arc::new(Bank::new(&vh, &vb).unwrap())
    }

    /// A song holding note 60 for a long time.
    fn held() -> Song {
        Song {
            id: 0,
            resolution: 480,
            tempo: 500_000,
            events: vec![0, 0x90, 60, 127, 0x83, 0x60, 0x80, 60, 0, 0, 0xff, 0x2f, 0],
        }
    }

    fn peak(spu: &mut Spu, frames: usize) -> i32 {
        let mut out = vec![0i16; 2 * frames];
        spu.render(&mut out);
        out.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0)
    }

    #[test]
    fn a_song_sounds_and_stops() {
        let mut spu = Spu::new(2, 17..24);
        spu.set_banks(bank(), bank());
        spu.play(0, &held(), 1, 127);
        assert!(spu.playing(0));
        assert!(peak(&mut spu, 4410) > 1000, "the note sounds");
        spu.stop(0);
        assert!(!spu.playing(0));
        assert_eq!(peak(&mut spu, 100), 0);
    }

    /// Two slots crossfade: one down to 0 and stopped, the other up.
    #[test]
    fn slots_fade_and_stop() {
        let mut spu = Spu::new(2, 17..24);
        spu.set_banks(bank(), bank());
        spu.play(0, &held(), 0, 90);
        spu.play(1, &held(), 0, 0);
        spu.fade(0, 0, 1000, true);
        spu.fade(1, 90, 1000, false);
        peak(&mut spu, 500);
        let (a, b) = (spu.volume(0).unwrap(), spu.volume(1).unwrap());
        assert!((a - 45.0).abs() < 1.0 && (b - 45.0).abs() < 1.0, "{a} {b}");
        peak(&mut spu, 600);
        assert_eq!((spu.playing(0), spu.volume(1)), (false, Some(90.0)));
        spu.set_volume(1, 30);
        assert_eq!(spu.volume(1), Some(30.0));
    }

    /// Effects take their voices in turn and sound at their volumes.
    #[test]
    fn effects_key_on() {
        let mut spu = Spu::new(1, 22..24);
        spu.set_banks(bank(), bank());
        spu.key_on(0, 0, 60, (127, 0));
        let mut out = vec![0i16; 2 * 2000];
        spu.render(&mut out);
        let (l, r) = out.as_chunks::<2>().0.iter().fold((0, 0), |(l, r), f| (l.max(f[0].abs()), r.max(f[1].abs())));
        assert!(l > 1000 && r == 0, "left only: {l} {r}");
        spu.key_on(9, 0, 60, (127, 127));
        spu.key_on(0, 5, 60, (127, 127));
    }
}
