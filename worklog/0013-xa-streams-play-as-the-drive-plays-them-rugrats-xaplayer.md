---
number: 13
title: XA streams play as the drive plays them: rugrats' XaPlayer moves to rrt-kit
date: 2026-10-15
area: audio, test, docs
files: crates/rrt-kit/src/adpcm/xa.rs, docs/crates/rrt-kit.md
---

# 13. XA streams play as the drive plays them: rugrats' XaPlayer moves to rrt-kit

Rugrats' opening played its sound about six times too long: 300 s for a
47 s scene. The XA decoder from entry 9 was right. The stream repeats each
audio sector to fill double speed's bandwidth, and the CD-ROM controller's
decoder takes a sector only when its output has drained to a few frames,
dropping the rest unread. The hardware has no flow control, and
DuckStation models the same rule. Rugrats found this (its worklog 22) and
wrote `movie::XaPlayer`; nothing in it is specific to the game, so it
moves here as `rrt_kit::adpcm::xa::Player`.

The port keeps the rule as written:
- a sector is taken when the output holds no more than 10 frames
- a dropped sector leaves the ADPCM history as it was
- time before the first audio sector is silence

What changes:
- **The drive speed is a parameter.** `DOUBLE_SPEED` (150 sectors a
  second, what games stream XA at) and `SINGLE_SPEED` (75); rugrats
  hardcoded double.
- **Counters.** `taken` and `dropped` count sectors, so a port can see the
  rule acting.
- **The coding byte** is the decoder's `Coding` type.

Four tests:
- **Repeated sectors.** A Rugrats-like stream, four sectors in five audio
  and repeated, keeps pace with the disc rather than playing every copy.
- **A movie stream.** One sector in eight of 37800 Hz stereo (exactly the
  rate double speed consumes) never drops.
- **History.** A dropped sector leaves the history alone: the output
  equals a plain decoder's fed only the taken sectors.
- **Timing.** Silence comes before the first sector, and single speed is
  half as fast.

Entry 9's question about the zigzag tables against a hardware recording
still stands; this entry answers why the opening sounded wrong, which was
not the decoder.

**Still unknown:** whether the drive's low-water mark is exactly 10 frames on hardware (it is DuckStation's rule, and rugrats' opening sounds right with it)
