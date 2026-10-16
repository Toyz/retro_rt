---
number: 17
title: libsnd: reserved voices, and where fade ticks come from
date: 2026-10-15
area: audio, port
files: crates/rrt-kit/src/libsnd/synth.rs, docs/crates/rrt-kit.md
took: 15m
---

# 17. libsnd: reserved voices, and where fade ticks come from

Two details of libsnd as Rugrats sets it up, read from its sound init (0x8006e7e8) and its VSync callback.

- **SsSetReservedVoice(14)** (0x800ad588: 1-24 into the voice count every allocator loop bounds): the sequencer's notes take voices 0-13. Spu::set_reserved_voices(n) does the same. The default stays all voices below the effects'.
- **SsSetTickMode(0x1000)** (0x800a8f58), SS_NOTICK: libsnd installs no tick of its own. Rugrats' VSync callback (0x800192b4) calls SsSeqCalledTbyT (0x800a48b8) every vblank.
  - That runs each open SEQ's crescendo and decrescendo (flags 16/32, fn 0x800a4b28: delta x ticks run / ticks, linear).
  - So libsnd ticks are 60 a second on NTSC, and a fade over 30 is half a second. Spu::fade takes output frames; rrt-kit.md says where the ticks come from.
- **Test:** notes_keep_to_the_reserved_voices.

**Still unknown:**
- nothing
