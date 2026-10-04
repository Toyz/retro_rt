---
title: rrt-audio, sound out
status: solid
crates: rrt-audio
covers: rrt_audio::Source, rrt_audio::Source::rate, rrt_audio::Source::render, rrt_audio::Output, rrt_audio::Output::open, rrt_audio::mix_into, rrt_audio::Resampler, rrt_audio::Resampler::new, rrt_audio::Resampler::fill
---

# rrt-audio

A game's mixer implements `Source`: interleaved stereo `i16` at its own rate
(44100 for a PS1 SPU, 48000 for a PS2's). `Output::open(Arc<Mutex<S>>)` plays
it on the default device through cpal.

```rust
pub trait Source: Send + 'static {
    fn rate(&self) -> u32;
    fn render(&mut self, out: &mut [i16]);   // out.len() / 2 frames, L R L R
}
```

## How it plays

The device is opened at the source's rate when it offers it (a config with 2
or more channels, the default sample format, the rate in range), else at its
default. cpal's callback locks the source and calls `mix_into`, which is the
whole of the callback's work and public for testing: it renders through a
`Resampler`, converts to the device's sample format (f32, i16, i32, u16, f64;
u16 is offset binary, 32768 silence), puts left and right in the first two
channels, gives a mono device `(l + r) / 2`, and leaves extra channels and a
trailing partial frame silent. A poisoned lock gives silence.

`Resampler` interpolates linearly between source frames when the rates
differ; at equal rates frames pass through untouched. Linear is the choice:
the consoles' own sound chips interpolate no better (the PS1 SPU's 4-tap
Gaussian is the source's job), and the difference is below what the
originals' 22-44 kHz samples carry.

No device, or one that will not open, is an `Err` the game logs and carries
on from in silence. Dropping the `Output` stops the sound.

## Sharing the source

The game holds the same `Arc<Mutex<S>>` and changes it between ticks; the
device's thread renders from it. The lock is held for one callback's render,
so keep `render` cheap and do not hold the lock across a tick.

## Tests

`equal_rates_pass_frames_through`, `doubling_the_rate_puts_a_midpoint_between_frames`,
`stereo_goes_to_the_first_two_channels_and_the_rest_are_silent`,
`a_mono_device_gets_the_mean`, `integer_sample_formats_are_converted`,
`a_partial_frame_at_the_end_is_left_silent`, and on the machine's default
device `the_default_device_pulls_from_the_source` (passes here; skipped with
a note where there is no device, as in CI).

## Not here

Any console's sound chip, sequencer or ADPCM decoder: those are the game's
`Source`.

## Gaps

Nothing known.
