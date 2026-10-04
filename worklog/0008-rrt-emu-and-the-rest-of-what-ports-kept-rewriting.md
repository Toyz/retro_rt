---
number: 8
title: rrt-emu, and the rest of what ports kept rewriting
date: 2026-10-03
area: design, build, test, input, app, audio
files: crates/rrt-emu, crates/rrt-kit/src/compress, crates/rrt-kit/src/angle.rs, crates/rrt-kit/src/cli.rs, crates/rrt-macros/src/args.rs, crates/rrt-app/src/console.rs, crates/rrt-app/src/args.rs, crates/rrt-input/src/padlog.rs, crates/rrt-image/src/font.rs
---

# 8. rrt-emu, and the rest of what ports kept rewriting

`rrt-emu` is the oracle tier: the originals run for comparison. Its `Cpu` trait was designed against the consoles that might come later, not just the two at hand: a CPU declares its byte order, its calling convention (`Abi`) and whether a call leaves its return address in a register (MIPS, ARM, SH, PowerPC) or on the stack (68000, x86, 6502, Z80); registers read as `u128`; `step` takes `&mut dyn Bus`, so the trait is object safe. `Machine` (calls, hooks, chained system call handlers, shadow checks, `call_recorded`) is tested on a made-up CPU in both return conventions, so none of it leans on MIPS.

The PS1 (R3000A, GTE, BIOS A0/B0/C0 tables, from hwtr) and the PS2 (EE with its float rules and VU0 macro mode, kernel stubs by symbol name, from piney_apples) were ported by two helper agents working in parallel on separate directories against the core traits. Their reviews found five real core faults, all fixed:

- hooks fired at an address reached as a delay slot (`Cpu::in_delay_slot` now stops that);
- the fix's first form, `self.hooks.remove(&pc).filter(..)`, removed the hook from the table even when it did not fire, losing it for every later call - the EE agent found this when its `a_hook_does_not_fire_in_a_delay_slot` failed; the hook is now taken out only when it runs;
- `psx::machine` could not keep its BIOS table reachable (`Machine::ext`, a typed map);
- one system call handler only (handlers now chain, `Continue` passes a call on);
- the oracle reported a byte written through KSEG0 and KUSEG as two changes (the journal records canonical addresses).

My own: a shadow check on the very function `call` was asked to run never closed, because the run loop tested for the return sentinel before closing checks; the toy CPU's `a_shadow_check_compares_each_call_at_entry_and_return` caught it. hwtr's harness only checks functions called from inside a running game, which hides that case.

The generic C library HLE first returned -1/0/1 from comparisons; newlib (what PS2 games link, and what piney matched) returns the first differing bytes' difference, so it does now. hwtr's BIOS `strcmp` keeps -1/0/1, which is what hwtr verified that table with.

## Also landed

- `spu`: PS-ADPCM as hwtr and piney both decoded it (identically, by different expressions), the loop point, and piney's 512-entry Gaussian table with its row-sum test. The first interpolation test asked a constant to come back within 40 of itself; the rows sum to 255/256 and four truncations lose up to 4 more, so 10000 reads 9957-9961.
- `rrt-kit::compress`: configurable LZSS (Okumura's `LZSS.C`, Nintendo LZ10) and PackBits / Nintendo RL, with encoders; Apple's TN1023 PackBits example decodes.
- `rrt-kit::angle`: `Angle<TURN>` (4096 and 65536 a turn), table sine as Q12, `atan2` within one unit everywhere, exact `isqrt`. The tables are generated once and embedded, since `f64::sin` may round differently by platform.
- `rrt-image::font`: X11 Misc Fixed 6x13, whose copyright property was read from the `.pcf` before use: "Public domain font. Share and enjoy."
- `rrt-input::PadLog`, and in `rrt-app` the `Console`, `Config::replay`/`record`, and `AppArgs`: the flags every port re-implemented (`--shot`, `--frames`, `--every`, `--press`, `--replay`, `--record`/`--pad-log`, ...), applied by `launch`, which now writes the shots itself.
- `rrt-kit::cli` and `#[derive(rrt::Args)]`, so a port's binaries stop hand-rolling `while let Some(a) = args.next()`.

## A false check, caught

A feature-matrix loop reported one error for every feature. It came from zsh, which does not word-split `${f:+--features $f}`: cargo received `--features net` as one unknown argument. The loop had never checked anything, including an earlier run I had reported as clean. Run correctly, every feature builds clean; CI now carries the matrix, written so bash and zsh read it the same.

**Still unknown:** The EE stops on lwl/lwr/swl/swr/sdl/sdr, mfsa/mtsa/qfsrv and most MMI (Stop::Reserved): piney's interpreter never had them and there is no reference here to test them against. Neither CPU port has yet run a game's own function through rrt-emu; the tests are hand-assembled code and the reference projects' unit tests.
