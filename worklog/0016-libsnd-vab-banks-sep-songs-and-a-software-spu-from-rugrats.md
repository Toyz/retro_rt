---
number: 16
title: libsnd: VAB banks, SEP songs and a software SPU, from rugrats
date: 2026-10-15
area: audio, port, bug
files: crates/rrt-kit/src/libsnd/mod.rs, crates/rrt-kit/src/libsnd/vab.rs, crates/rrt-kit/src/libsnd/sep.rs, crates/rrt-kit/src/libsnd/synth.rs, docs/crates/rrt-kit.md
took: 25m
---

# 16. libsnd: VAB banks, SEP songs and a software SPU, from rugrats

Rugrats' sound engine was Sony's libsnd rewritten in the port: VAB banks, SEP songs, the SEQ sequencer and a software SPU. None of it is Rugrats' own, so it moves to rrt-kit as `libsnd`, as `adpcm::xa::Player` did (entry 13). The port keeps what is the game's: its song sets (which files), its effect table, effects on voices 17-23, and the room switch (0x8006efcc), whose song ids and slots it tracks over Spu's slots.

- **vab:** the .VH header (Program, Tone, the sample sizes), Vab::vag, tones_of, program_volume.
  - Only the header's `programs` have tone blocks. Nine Rugrats banks list programs up to 14 past the count, one tone each and nothing behind them. Reading those went past the VH's end and crashed eight of its levels (rugrats worklog 72); they are dropped here.
- **sep:** SEP packs.
- **Bank::new(vh, vb)** decodes the samples with adpcm::spu; the port's own copy of the ADPCM decoder is gone.
- **Spu::new(slots, effect_voices):**
  - 24 voices, the ADSR generator, Gaussian interpolation.
  - The sequencer per slot: running status, tempo, CC 7/10/11, program change, bend, NRPN 20/30 loops.
  - Slot volume applied at mix (set_volume), with fade(slot, volume, frames, then_stop) for SsSeqSetCrescendo / Decrescendo.
  - key_on for SsUtKeyOn effects; render mixes over the buffer at the main volume.
- **Tests** on banks and songs built in the test:
  - programs_and_tones_line_up, programs_past_the_count_are_dropped, short_or_foreign_headers_are_errors;
  - two_songs_back_to_back;
  - a_song_sounds_and_stops, slots_fade_and_stop, effects_key_on.
- **Checked in rugrats:** the menu's song 0 renders as before (peak 15581, RMS 2026), every bank on its disc parses, and its 50 tests pass.

**Still unknown:**
- Reverb, vibrato and portamento are not modelled; libsnd's exact volume and pitch tables are not compared against the hardware.
