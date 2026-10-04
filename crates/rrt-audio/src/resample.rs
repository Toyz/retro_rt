//! Linear resampling from a [`Source`]'s rate to a device's.

use crate::Source;

/// Pulls stereo frames from a [`Source`] at its rate and hands them out at
/// another, interpolating linearly. At equal rates the frames pass through
/// untouched.
#[derive(Clone, Debug)]
pub struct Resampler {
    from: u32,
    to: u32,
    /// Position between `prev` and `next`, in units of 1/`to` of a source
    /// frame.
    phase: u64,
    prev: (i16, i16),
    next: (i16, i16),
    scratch: Vec<i16>,
}

impl Resampler {
    /// From `from` frames a second to `to`.
    pub fn new(from: u32, to: u32) -> Resampler {
        let (from, to) = (from.max(1), to.max(1));
        Resampler { from, to, phase: u64::from(to), prev: (0, 0), next: (0, 0), scratch: Vec::new() }
    }

    /// `frames` output frames, each handed to `put(index, left, right)` as
    /// -1.0..1.0.
    pub fn fill(&mut self, source: &mut impl Source, frames: usize, mut put: impl FnMut(usize, f32, f32)) {
        const SCALE: f32 = 1.0 / 32768.0;
        if self.from == self.to {
            self.scratch.resize(2 * frames, 0);
            source.render(&mut self.scratch);
            for i in 0..frames {
                put(i, f32::from(self.scratch[2 * i]) * SCALE, f32::from(self.scratch[2 * i + 1]) * SCALE);
            }
            return;
        }
        let mut one = [0i16; 2];
        for i in 0..frames {
            while self.phase >= u64::from(self.to) {
                self.phase -= u64::from(self.to);
                self.prev = self.next;
                source.render(&mut one);
                self.next = (one[0], one[1]);
            }
            let t = self.phase as f32 / self.to as f32;
            let l = f32::from(self.prev.0) + (f32::from(self.next.0) - f32::from(self.prev.0)) * t;
            let r = f32::from(self.prev.1) + (f32::from(self.next.1) - f32::from(self.prev.1)) * t;
            put(i, l * SCALE, r * SCALE);
            self.phase += u64::from(self.from);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source counting up by 1000 a frame, both channels.
    struct Ramp(i16);

    impl Source for Ramp {
        fn rate(&self) -> u32 {
            100
        }
        fn render(&mut self, out: &mut [i16]) {
            for f in out.as_chunks_mut::<2>().0 {
                self.0 += 1000;
                f.fill(self.0);
            }
        }
    }

    fn run(from: u32, to: u32, frames: usize) -> Vec<f32> {
        let mut rs = Resampler::new(from, to);
        let mut src = Ramp(0);
        let mut out = vec![0.0; frames];
        rs.fill(&mut src, frames, |i, l, _| out[i] = l * 32768.0);
        out
    }

    #[test]
    fn equal_rates_pass_frames_through() {
        assert_eq!(run(100, 100, 3), [1000.0, 2000.0, 3000.0]);
    }

    #[test]
    fn doubling_the_rate_puts_a_midpoint_between_frames() {
        let out = run(100, 200, 5);
        // The first source frame is reached after the start-up step from 0.
        assert_eq!(out, [0.0, 500.0, 1000.0, 1500.0, 2000.0]);
    }
}
