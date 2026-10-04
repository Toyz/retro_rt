//! Generators wrapped to watch or steer them.

use std::collections::VecDeque;

use super::Generator;
use crate::Ring;

/// A generator that counts its calls. Comparing the count with the
/// original's at the same frame finds where a port draws one value too many
/// or too few.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counted<G> {
    /// The generator.
    pub inner: G,
    /// Calls to [`Generator::next`] since made or last reset.
    pub calls: u64,
}

impl<G> Counted<G> {
    /// `inner`, counting from 0.
    pub fn new(inner: G) -> Counted<G> {
        Counted { inner, calls: 0 }
    }
}

impl<G: Generator> Generator for Counted<G> {
    type State = G::State;

    fn next(&mut self) -> u32 {
        self.calls += 1;
        self.inner.next()
    }

    fn bits(&self) -> u32 {
        self.inner.bits()
    }

    fn state(&self) -> G::State {
        self.inner.state()
    }

    fn set_state(&mut self, state: G::State) {
        self.inner.set_state(state);
    }
}

/// A generator that keeps its last outputs, oldest first, for a debug
/// overlay or a failing test's message.
#[derive(Clone, Debug)]
pub struct Recorded<G> {
    /// The generator.
    pub inner: G,
    /// The last outputs, at most the capacity given.
    pub history: Ring<u32>,
}

impl<G> Recorded<G> {
    /// `inner`, remembering its last `keep` outputs.
    ///
    /// # Panics
    ///
    /// When `keep` is 0.
    pub fn new(inner: G, keep: usize) -> Recorded<G> {
        Recorded { inner, history: Ring::new(keep) }
    }
}

impl<G: Generator> Generator for Recorded<G> {
    type State = G::State;

    fn next(&mut self) -> u32 {
        let v = self.inner.next();
        self.history.push(v);
        v
    }

    fn bits(&self) -> u32 {
        self.inner.bits()
    }

    fn state(&self) -> G::State {
        self.inner.state()
    }

    fn set_state(&mut self, state: G::State) {
        self.inner.set_state(state);
    }
}

/// A generator that gives queued values first: to force the roll a test or
/// a debug menu wants (the crit, the rare drop). A forced value does not
/// step the inner generator, so the sequence after it is the one the game
/// would have continued with.
#[derive(Clone, Debug)]
pub struct Forced<G> {
    /// The generator.
    pub inner: G,
    queue: VecDeque<u32>,
}

impl<G> Forced<G> {
    /// `inner`, with nothing queued.
    pub fn new(inner: G) -> Forced<G> {
        Forced { inner, queue: VecDeque::new() }
    }

    /// Queues `values` to come out next, in order.
    pub fn force(&mut self, values: impl IntoIterator<Item = u32>) {
        self.queue.extend(values);
    }

    /// Values still queued.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
}

impl<G: Generator> Generator for Forced<G> {
    type State = G::State;

    fn next(&mut self) -> u32 {
        self.queue.pop_front().unwrap_or_else(|| self.inner.next())
    }

    fn bits(&self) -> u32 {
        self.inner.bits()
    }

    /// The inner generator's state; queued values are not part of it.
    fn state(&self) -> G::State {
        self.inner.state()
    }

    fn set_state(&mut self, state: G::State) {
        self.inner.set_state(state);
    }
}
