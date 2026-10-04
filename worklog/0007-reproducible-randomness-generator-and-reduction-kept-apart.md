---
number: 7
title: Reproducible randomness: generator and reduction, kept apart
date: 2026-10-03
area: design, test
files: crates/rrt-kit/src/rng/mod.rs, crates/rrt-kit/src/rng/generators.rs, crates/rrt-kit/src/rng/wrappers.rs, crates/rrt-kit/src/rng/tests.rs
---

# 7. Reproducible randomness: generator and reduction, kept apart

`rrt_kit::rng` gives ports one way to reproduce a game's randomness. The design rests on one observation: a game's random numbers are two separate things, and a port has to match both. The **generator** is the state and its step. The **reduction** is the expression that turned a raw value into a roll, and `rand() % 6` and `(rand() * 6) >> 15` give different rolls from the same raw value. A library that hid the reduction behind a `gen_range` would make exact ports impossible, so the reductions games wrote are methods named for their expressions - `modulo`, `scaled`, `top_bits` - and the unbiased helpers (`below`, `range`, `chance`, `pick`, `shuffle`) are documented as for randomness a port adds, since rejection sampling may draw more values than the original did.

`Generator` is four methods: `next`, `bits`, `state`, `set_state`. `bits` lets reductions like `scaled` work for any output width (15 for C's `rand`, 8 for a table, 32 for xorshift). The state is an associated type so a save state or a replay stores exactly what decides the sequence. Everything in `RngExt` comes free to any implementor, including through a trait object.

## Checked against published values

The constants are the risky part, so each common generator is tested against values published independently of this code: the C standard's example `rand` seeded with 1 gives 16838, 5758, 10113, 17515, 31051; MSVC's gives 41, 18467, 6334, 26500, 19169; PCG32's reference demo (`pcg32_srandom(42, 54)`) gives 0xa15c02b7, 0x7b47f409 and on; the textbook 16-bit Galois LFSR (0xACE1, taps 0xB400) steps to 0xE270 and has period 65535. All passed on the first run. newlib's 64-bit LCG is checked only against its own formula; no published sequence was at hand.

`Lcg::jump(n)` composes the affine step by squaring, for resyncing with an original in O(log n). Its test compares against plain stepping up to n = 123,456,789; that loop ran in 0.01 s even with one thread, which was checked rather than trusted: the stepping loop has no side effects, so the optimiser may shorten it, but whatever it emits computes the same state.

## Wrappers

`Counted` exists for lockstep work: comparing its call count with the original's at the same frame finds a draw the port makes once too often or not at all, the most common way a port's randomness drifts. `Forced` queues values without stepping the inner generator, so forcing a crit in a test does not shift every roll after it. `Recorded` keeps the last outputs in a `Ring` for an overlay or a failure message.

**Still unknown:** No generator here has yet been checked against a ported game's own draws (hwtr's or piney_apples' RNG); the published sequences check the generators, not any one game's use of them.
