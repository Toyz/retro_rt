---
number: 9
title: The PlayStation's ADPCM formats move to rrt-kit, and CD-XA joins them
date: 2026-10-04
area: audio, design, test, docs
files: crates/rrt-kit/src/adpcm/mod.rs, crates/rrt-kit/src/adpcm/spu.rs, crates/rrt-kit/src/adpcm/xa.rs, crates/rrt-emu/src/spu.rs, docs/crates/rrt-kit.md, docs/crates/rrt-emu.md
---

# 9. The PlayStation's ADPCM formats move to rrt-kit, and CD-XA joins them

PS-ADPCM and the SPU's Gaussian interpolation lived in rrt-emu, whose rule is
that nothing in it runs at play time. A port's sound engine needs both at play
time: forbidden (Yu-Gi-Oh! Forbidden Memories) plays its VAB banks and sound
effects through its own driver and mixer, and was about to grow a second copy
of the decoder and the Gaussian table rather than ship rrt-emu's CPUs in the
game binary. The format is not an emulator's: it is data every PS1 and PS2
port decodes. So it moves to `rrt_kit::adpcm::spu`, unchanged, with its seven
tests; `rrt_emu::spu` re-exports it so the oracle's users see no change.

CD-XA ADPCM is new, in `rrt_kit::adpcm::xa`: the drive's streamed audio, which
forbidden needs for its intro movie and its speech (`MASTER.XA`). A `Decoder`
per stream takes a sector's 0x900 bytes with the subheader's coding byte and
appends 44100 Hz stereo, resampled by the drive's seven 29-tap zigzag filters
from psx-spx. Each filter's taps sum to 0x73e5-0x741d of 0x8000, so a held
level comes out at about 0.91 of itself, the same in every phase; the tests
check that gain, the sample counts (4032 mono samples a sector become 4704),
stereo routing (even blocks left, odd right), the history carrying across
sectors, and that 8-bit sectors decode to nothing. Range 0 is the loudest
(the nibble shifted 12 left): the first draft of the tests assumed the
opposite and saw silence.

The rrt-kit page is now `partial`: the zigzag tables are psx-spx's, which it
calls nearly correct, and nothing here has compared them with the hardware.

**Still unknown:** XA's zigzag tables are psx-spx's and have not been compared with a recording of the hardware; 8-bit XA is not decoded
