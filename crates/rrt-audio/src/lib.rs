//! Sound out.
//!
//! A game's mixer - an SPU, a sequencer, a tone - implements [`Source`]:
//! interleaved stereo `i16` at its own [`Source::rate`]. [`Output::open`]
//! plays it on the default device: cpal's callback locks the source, renders
//! what the device asks for, and resamples linearly ([`Resampler`]) to the
//! device's rate and channel count. With no device (CI, no sound server) the
//! open fails and the game carries on silent.
//!
//! The source is shared with the game as `Arc<Mutex<S>>`: the game changes
//! it between ticks (key on a voice, start a stream), the device's thread
//! renders from it. Keep the work done under the lock small.

pub mod resample;

pub use resample::Resampler;

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

/// Something that makes sound: interleaved stereo frames at a fixed rate.
pub trait Source: Send + 'static {
    /// Frames a second: 44100 for the PS1's SPU, 48000 for the PS2's.
    fn rate(&self) -> u32;

    /// Fills `out` with `out.len() / 2` stereo frames, left then right.
    fn render(&mut self, out: &mut [i16]);
}

/// A [`Source`] playing on the default output device. Dropping it stops the
/// sound.
pub struct Output {
    _stream: cpal::Stream,
    /// The device's rate, frames a second.
    pub rate: u32,
    /// The device's channels: 1 gets left and right mixed, 2 or more get left
    /// and right in the first two.
    pub channels: u16,
}

impl Output {
    /// Plays `source` on the default device, at the source's rate if the
    /// device offers it, else at the device's own. `Err` when there is no
    /// device or it will not open; the caller carries on silent.
    pub fn open<S: Source>(source: Arc<Mutex<S>>) -> Result<Output, String> {
        let want = source.lock().map_err(|_| "the source's lock is poisoned")?.rate();
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no output device")?;
        let default = device.default_output_config().map_err(|e| e.to_string())?;
        let mut chosen = default;
        if default.sample_rate() != want
            && let Ok(configs) = device.supported_output_configs()
        {
            for c in configs {
                if c.channels() >= 2
                    && c.sample_format() == default.sample_format()
                    && c.min_sample_rate() <= want
                    && want <= c.max_sample_rate()
                {
                    chosen = c.with_sample_rate(want);
                    break;
                }
            }
        }
        let format = chosen.sample_format();
        let config: StreamConfig = chosen.into();
        let (rate, channels) = (config.sample_rate, config.channels);
        let stream = match format {
            SampleFormat::F32 => build::<f32, S>(&device, config, source, want),
            SampleFormat::I16 => build::<i16, S>(&device, config, source, want),
            SampleFormat::I32 => build::<i32, S>(&device, config, source, want),
            SampleFormat::U16 => build::<u16, S>(&device, config, source, want),
            SampleFormat::F64 => build::<f64, S>(&device, config, source, want),
            other => Err(format!("unsupported sample format {other}")),
        }?;
        tracing::info!("audio: {rate} Hz, {channels} channels, source at {want} Hz");
        Ok(Output { _stream: stream, rate, channels })
    }
}

/// Fills a device buffer of `channels`-channel frames from `source` through
/// `rs`: left and right in the first two channels, their mean on a mono
/// device, silence in any beyond two. This is all of the device callback
/// but the lock.
pub fn mix_into<T>(data: &mut [T], channels: usize, rs: &mut Resampler, source: &mut impl Source)
where
    T: SizedSample + FromSample<f32>,
{
    let channels = channels.max(1);
    data.fill(T::from_sample(0.0f32));
    let frames = data.len() / channels;
    rs.fill(source, frames, |i, l, r| {
        let frame = &mut data[i * channels..(i + 1) * channels];
        if channels == 1 {
            frame[0] = T::from_sample((l + r) * 0.5);
        } else {
            frame[0] = T::from_sample(l);
            frame[1] = T::from_sample(r);
        }
    });
}

fn build<T, S>(
    device: &cpal::Device,
    config: StreamConfig,
    source: Arc<Mutex<S>>,
    source_rate: u32,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
    S: Source,
{
    let channels = usize::from(config.channels).max(1);
    let mut rs = Resampler::new(source_rate, config.sample_rate);
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                if let Ok(mut src) = source.lock() {
                    mix_into(data, channels, &mut rs, &mut *src);
                } else {
                    data.fill(T::from_sample(0.0f32));
                }
            },
            |e| tracing::warn!("audio: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Left at +0.5 full scale, right at -0.25, every frame; counts frames.
    struct Steady(usize);

    impl Source for Steady {
        fn rate(&self) -> u32 {
            48000
        }
        fn render(&mut self, out: &mut [i16]) {
            for f in out.as_chunks_mut::<2>().0 {
                *f = [16384, -8192];
                self.0 += 1;
            }
        }
    }

    #[test]
    fn stereo_goes_to_the_first_two_channels_and_the_rest_are_silent() {
        let mut data = [1.0f32; 12];
        mix_into(&mut data, 6, &mut Resampler::new(48000, 48000), &mut Steady(0));
        assert_eq!(data, [0.5, -0.25, 0.0, 0.0, 0.0, 0.0, 0.5, -0.25, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_mono_device_gets_the_mean() {
        let mut data = [0f32; 3];
        let mut src = Steady(0);
        mix_into(&mut data, 1, &mut Resampler::new(48000, 48000), &mut src);
        assert_eq!(data, [0.125; 3]);
        assert_eq!(src.0, 3, "one source frame a device frame");
    }

    #[test]
    fn integer_sample_formats_are_converted() {
        let mut i16s = [0i16; 2];
        mix_into(&mut i16s, 2, &mut Resampler::new(48000, 48000), &mut Steady(0));
        assert_eq!(i16s, [16384, -8192]);
        let mut u16s = [0u16; 2];
        mix_into(&mut u16s, 2, &mut Resampler::new(48000, 48000), &mut Steady(0));
        assert_eq!(u16s, [49152, 24576], "u16 is offset binary: 32768 is silence");
    }

    #[test]
    fn a_partial_frame_at_the_end_is_left_silent() {
        let mut data = [9i16; 5];
        mix_into(&mut data, 2, &mut Resampler::new(48000, 48000), &mut Steady(0));
        assert_eq!(data, [16384, -8192, 16384, -8192, 0]);
    }

    /// Counts the frames rendered, shared with the test.
    struct Counting(Arc<Mutex<usize>>);

    impl Source for Counting {
        fn rate(&self) -> u32 {
            44100
        }
        fn render(&mut self, out: &mut [i16]) {
            out.fill(0);
            *self.0.lock().unwrap() += out.len() / 2;
        }
    }

    /// On the machine's default device: the stream opens and the device
    /// pulls frames from the source within a second. Skipped, with a note,
    /// where there is no device (CI).
    #[test]
    fn the_default_device_pulls_from_the_source() {
        let count = Arc::new(Mutex::new(0));
        let source = Arc::new(Mutex::new(Counting(count.clone())));
        let output = match Output::open(source) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("no audio device, skipping: {e}");
                return;
            }
        };
        assert!(output.rate > 0 && output.channels > 0);
        for _ in 0..100 {
            if *count.lock().unwrap() > 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the device took no frames in a second");
    }
}
