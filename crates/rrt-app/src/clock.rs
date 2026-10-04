//! The fixed rate a game ticks at, kept against the wall clock.

use std::time::{Duration, Instant};

/// Tick rates, in ticks a second.
pub mod rate {
    /// NTSC's field rate, 60000/1001 (59.94 Hz).
    pub const NTSC: f64 = 60000.0 / 1001.0;
    /// PAL's field rate.
    pub const PAL: f64 = 50.0;
}

/// Counts the ticks due at a fixed rate. Each tick is due one period after
/// the last; when the program falls behind by more than `max_catch_up`
/// ticks, the rest are dropped rather than run in a burst.
#[derive(Clone, Debug)]
pub struct Clock {
    period: Duration,
    max_catch_up: u32,
    next: Instant,
}

impl Clock {
    /// A clock at `hz` ticks a second, starting now.
    ///
    /// # Panics
    ///
    /// When `hz` is not positive and finite.
    pub fn new(hz: f64, max_catch_up: u32) -> Clock {
        assert!(hz > 0.0 && hz.is_finite(), "tick rate {hz}");
        Clock { period: Duration::from_secs_f64(1.0 / hz), max_catch_up: max_catch_up.max(1), next: Instant::now() }
    }

    /// The time one tick takes.
    pub fn period(&self) -> Duration {
        self.period
    }

    /// The next tick is due at `now`: after a pause, a load, a window
    /// opening.
    pub fn reset(&mut self, now: Instant) {
        self.next = now;
    }

    /// How many ticks are due at `now`, at most `max_catch_up`; the clock
    /// moves past them. Behind by more, it drops the excess and is due again
    /// from `now`.
    pub fn due(&mut self, now: Instant) -> u32 {
        let mut n = 0;
        while self.next <= now && n < self.max_catch_up {
            self.next += self.period;
            n += 1;
        }
        if self.next < now {
            self.next = now;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_come_due_one_period_apart() {
        let mut c = Clock::new(50.0, 4);
        let t0 = Instant::now();
        c.reset(t0);
        assert_eq!(c.due(t0), 1, "the first is due at once");
        assert_eq!(c.due(t0 + Duration::from_millis(10)), 0);
        assert_eq!(c.due(t0 + Duration::from_millis(20)), 1);
        assert_eq!(c.due(t0 + Duration::from_millis(60)), 2);
    }

    #[test]
    fn falling_far_behind_drops_the_excess() {
        let mut c = Clock::new(50.0, 4);
        let t0 = Instant::now();
        c.reset(t0);
        let late = t0 + Duration::from_secs(1);
        assert_eq!(c.due(late), 4);
        assert_eq!(c.due(late), 1, "due again from the late moment, not 46 behind");
    }
}
