---
number: 18
title: libsnd: pause and replay
date: 2026-10-15
area: audio
files: crates/rrt-kit/src/libsnd/synth.rs docs/crates/rrt-kit.md
took: 10m
---

# 18. libsnd: pause and replay

Rugrats pauses its song around some in-game cinematics and fades it back
in afterwards. A cinematic whose table byte +7 has bit 32 calls
0x8006ec40 (SsSeqPause) at its start, and 0x8006ed30 (SsSeqReplay) then
0x8006ef64 (SsSeqSetVol 0, SsSeqSetCrescendo over 60 ticks) at its end.
The software libsnd had no pause, so `Spu` gains three calls:

- `pause(slot)`: the slot's song stops sequencing and its sounding notes
  stop;
- `replay(slot)`: it goes on from the event it stopped at;
- `paused(slot)`: whether it is paused.

A paused slot's fade still steps, so set_volume(0) plus replay plus fade
is the game's fade-in. The test `a_song_pauses_and_replays` checks the
silence and that the sequence position holds and then moves on.

**Still unknown:**
- whether the real SsSeqPause keys its voices off with their release or cuts them; this cuts them, as `stop` does
