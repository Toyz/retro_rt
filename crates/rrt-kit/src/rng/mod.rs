//! Random numbers the way games made them, so a port can make the same
//! ones.
//!
//! A game's randomness has two parts, and a port must match both bit for
//! bit:
//!
//! 1. the **generator**: the state and how it steps - an LCG, an LFSR, a
//!    table, an xorshift. That is a [`Generator`]: [`Generator::next`] steps
//!    and returns a raw value [`Generator::bits`] wide, and
//!    [`Generator::state`] / [`Generator::set_state`] save and restore it
//!    for save states, replays and lockstep checks.
//! 2. the **reduction**: how a raw value became a die roll. `rand() % 6` and
//!    `(rand() * 6) >> 15` give different rolls from the same value, so
//!    [`RngExt`] names each: [`RngExt::modulo`], [`RngExt::scaled`],
//!    [`RngExt::top_bits`]. Port the game's own expression with these; use
//!    the unbiased [`RngExt::below`], [`RngExt::range`] and the rest only for
//!    randomness the port adds.
//!
//! Generators here: [`Lcg`] (with [`Lcg::ANSI_C`] and [`Lcg::MSVC`]),
//! [`Lcg64`] (with [`Lcg64::NEWLIB`]), [`Xorshift32`], [`Lfsr`],
//! [`TableRng`], and [`Pcg32`] for a port's own randomness. A game with its
//! own scheme implements [`Generator`]; [`RngExt`] then comes with it.
//! Wrappers that are generators themselves: [`Counted`] (how many calls -
//! where a port drifts from the original), [`Recorded`] (the last outputs),
//! [`Forced`] (values to come out next, for tests and debugging).

mod generators;
mod wrappers;

pub use generators::{Lcg, Lcg64, Lfsr, Pcg32, TableRng, Xorshift32};
pub use wrappers::{Counted, Forced, Recorded};

use std::fmt::Debug;

/// A random number generator as a game keeps one.
pub trait Generator {
    /// Everything that determines the sequence from here on: what a save
    /// state or a replay stores.
    type State: Copy + Eq + Debug;

    /// Steps the state and returns its output, in the low [`Generator::bits`]
    /// bits.
    fn next(&mut self) -> u32;

    /// How many low bits of [`Generator::next`]'s output are random: 15 for
    /// C's `rand`, 8 for a byte table, 32 for xorshift.
    fn bits(&self) -> u32;

    /// The state now.
    fn state(&self) -> Self::State;

    /// Puts the state back: the sequence continues from where
    /// [`Generator::state`] was read.
    fn set_state(&mut self, state: Self::State);
}

/// What every [`Generator`] can do with its values: the reductions games
/// used, named for what they compute, and unbiased helpers for new code.
pub trait RngExt: Generator {
    /// `next() % n`: the most common reduction in game code. Biased toward
    /// small results when `n` does not divide 2^bits; keep it when porting
    /// code that did it.
    ///
    /// # Panics
    ///
    /// When `n` is 0.
    fn modulo(&mut self, n: u32) -> u32 {
        self.next() % n
    }

    /// `(next() * n) >> bits`: the other common reduction, which uses the
    /// value's high bits (better on an LCG, whose low bits cycle).
    fn scaled(&mut self, n: u32) -> u32 {
        let bits = self.bits();
        ((u64::from(self.next()) * u64::from(n)) >> bits) as u32
    }

    /// The top `k` of the output's bits: `next() >> (bits - k)`.
    ///
    /// # Panics
    ///
    /// When `k` is more than [`Generator::bits`].
    fn top_bits(&mut self, k: u32) -> u32 {
        let bits = self.bits();
        assert!(k <= bits, "{k} bits asked of a {bits}-bit generator");
        if k == 0 { 0 } else { self.next() >> (bits - k) }
    }

    /// A uniform value below `n`, without modulo bias: values from the top
    /// partial run are drawn again. Draws a varying number of values, so do
    /// not use it where the original drew exactly one.
    ///
    /// # Panics
    ///
    /// When `n` is 0 or does not fit [`Generator::bits`].
    fn below(&mut self, n: u32) -> u32 {
        let bits = self.bits();
        assert!(n > 0, "below(0)");
        let span = 1u64 << bits;
        assert!(u64::from(n) <= span, "below({n}) from a {bits}-bit generator");
        let limit = span - span % u64::from(n);
        loop {
            let v = u64::from(self.next());
            if v < limit {
                return (v % u64::from(n)) as u32;
            }
        }
    }

    /// A uniform value in `lo..=hi`, unbiased ([`RngExt::below`]).
    ///
    /// # Panics
    ///
    /// When `lo > hi`, or the span does not fit the generator.
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        assert!(lo <= hi, "range({lo}, {hi})");
        let span = (i64::from(hi) - i64::from(lo) + 1) as u32;
        (i64::from(lo) + i64::from(self.below(span))) as i32
    }

    /// True `num` times in `den`, unbiased.
    ///
    /// # Panics
    ///
    /// When `den` is 0.
    fn chance(&mut self, num: u32, den: u32) -> bool {
        self.below(den) < num
    }

    /// One element of `items`, unbiased; None for an empty slice.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() { None } else { items.get(self.below(items.len() as u32) as usize) }
    }

    /// Shuffles `items` in place, Fisher-Yates with [`RngExt::below`]. To
    /// port a game's shuffle, write its own loop with its own reduction.
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }

    /// A float in `0.0..1.0`: the output over 2^bits.
    fn unit_f32(&mut self) -> f32 {
        let bits = self.bits();
        (f64::from(self.next()) / (1u64 << bits) as f64) as f32
    }

    /// Steps `n` times, discarding the outputs: to follow an original that
    /// drew values the port does not use.
    fn skip(&mut self, n: u64) {
        for _ in 0..n {
            self.next();
        }
    }
}

impl<G: Generator + ?Sized> RngExt for G {}

#[cfg(test)]
mod tests;
