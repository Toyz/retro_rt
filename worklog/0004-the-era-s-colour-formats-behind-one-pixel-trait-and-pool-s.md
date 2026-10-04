---
number: 4
title: The era's colour formats behind one Pixel trait, and Pool's limits
date: 2026-10-03
area: design, docs
files: crates/rrt-kit/src/color.rs, crates/rrt-kit/src/pool.rs
---

# 4. The era's colour formats behind one Pixel trait, and Pool's limits

`rrt_kit::color` now covers the formats of the PS1-to-Dreamcast era, each a newtype over its raw word implementing `Pixel` (`to_rgba8`, `from_rgba8`): `Rgb555`, `Argb1555`, `Rgb565`, `Bgr565`, `Argb4444`, `Abgr4444`, `Rgba5551`, `Rgb5a3`, `Ia16`, `Ia8`, `Md333`, `Sms222`, `Rgb332`, `PsmCt32`; plus BT.601 `ycbcr_to_rgb` in full and studio range, `indexed4`/`indexed8` with a nibble order, and `ps2_clut_index`.

Channels expand by bit replication and reduce by truncation. That pair was chosen because it makes every representable colour round-trip, which `every_word_of_every_16_bit_format_round_trips` checks over all 65536 words of each 16-bit format; rounding on the way back would not.

One format is lossy through RGBA8 and says so: `Rgb555`'s bit 15 (PS1 STP) has nowhere to go. As a `Pixel` it follows the PS1 texture rule - 0x0000 transparent, every other word opaque - and opaque black packs as 0x8000 so it stays drawn. A tool that must keep STP keeps the raw words.

SNES, GBA, GBC, Saturn and PSP 5551 share the PS1's layout (red in the low bits), so they are `Rgb555` and the module's table says so, rather than near-identical types under other names. `Md333` is expanded linearly although the Mega Drive's DAC is not linear; the page records that it is the colour as data. Left out on purpose: the NES palette (no canonical values), compressed formats (S3TC, CMPR, PVRTC) and texture swizzles other than the PS2 CLUT's, which belong to a game's decoder.

`Pool` had one limit, `max`, settable only as a field. It now has two, each a field and a builder: `max_spare` (how many spare Vecs) and `max_capacity` (a Vec given back larger is shrunk, so one huge frame does not hold its memory for good), and `trim(n)`. Tests: `a_vec_past_max_capacity_is_shrunk_when_given_back`, `limits_can_be_lowered_later`.

**Still unknown:** nothing
