//! `Vec`s lent out and taken back, so per-frame lists stop allocating once
//! the pool is warm. [`Pool`] for one thread; [`SyncPool`], shared by
//! cloning, for several (a decoder thread filling buffers the game thread
//! consumes, an audio callback).

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// A store of empty `Vec`s that keep their capacity. [`Pool::take`] lends
/// one; [`Pool::give`] takes it back, cleared, for the next taker.
///
/// Two limits bound what it holds, both set at construction or any time
/// after: [`Pool::max_spare`] Vecs at most, and none bigger than
/// [`Pool::max_capacity`], so one huge frame does not pin its memory for
/// good.
#[derive(Clone, Debug)]
pub struct Pool<T> {
    free: Vec<Vec<T>>,
    /// Spare Vecs kept at most; one given back past this is dropped. 64 by
    /// default.
    pub max_spare: usize,
    /// The most capacity a kept Vec may have; one given back bigger is
    /// shrunk to it. None (the default) for no limit.
    pub max_capacity: Option<usize>,
    /// Capacity a newly made Vec starts with.
    pub capacity: usize,
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Pool::new(0)
    }
}

impl<T> Pool<T> {
    /// A pool whose new Vecs start with room for `capacity` items, keeping
    /// at most 64 spares, of any size.
    pub fn new(capacity: usize) -> Pool<T> {
        Pool { free: Vec::new(), max_spare: 64, max_capacity: None, capacity }
    }

    /// Sets [`Pool::max_spare`], dropping spares past it now.
    pub fn with_max_spare(mut self, n: usize) -> Pool<T> {
        self.max_spare = n;
        self.trim(n);
        self
    }

    /// Sets [`Pool::max_capacity`], shrinking spares past it now.
    pub fn with_max_capacity(mut self, n: usize) -> Pool<T> {
        self.max_capacity = Some(n);
        for v in &mut self.free {
            v.shrink_to(n);
        }
        self
    }

    /// Lends an empty Vec: a spare one if there is one (with the capacity
    /// it had), else a new one with [`Pool::capacity`].
    pub fn take(&mut self) -> Vec<T> {
        self.free.pop().unwrap_or_else(|| Vec::with_capacity(self.capacity))
    }

    /// Takes `v` back, cleared and shrunk to [`Pool::max_capacity`], for a
    /// later [`Pool::take`]. Dropped when the pool already holds
    /// [`Pool::max_spare`].
    pub fn give(&mut self, mut v: Vec<T>) {
        if self.free.len() >= self.max_spare {
            return;
        }
        v.clear();
        if let Some(max) = self.max_capacity {
            v.shrink_to(max);
        }
        self.free.push(v);
    }

    /// Drops spares until at most `n` are left: after a level loads, say.
    pub fn trim(&mut self, n: usize) {
        self.free.truncate(n);
    }

    /// Spare Vecs held now.
    pub fn spare(&self) -> usize {
        self.free.len()
    }
}

/// [`Pool`] shared between threads: clone it to hand it to another thread;
/// every clone draws on the same spares.
///
/// One mutex guards the spare list and is held only to push or pop a `Vec`,
/// never while a caller uses one, so a taker on an audio thread waits at
/// most for another's push or pop. [`SyncPool::take`] returns a [`Pooled`]
/// guard that goes back to the pool when dropped, wherever that happens.
pub struct SyncPool<T> {
    shared: Arc<Shared<T>>,
}

struct Shared<T> {
    free: Mutex<Vec<Vec<T>>>,
    capacity: usize,
    max_spare: AtomicUsize,
    /// `usize::MAX` for no limit.
    max_capacity: AtomicUsize,
}

impl<T> Clone for SyncPool<T> {
    fn clone(&self) -> Self {
        SyncPool { shared: self.shared.clone() }
    }
}

impl<T> Default for SyncPool<T> {
    fn default() -> Self {
        SyncPool::new(0)
    }
}

impl<T> std::fmt::Debug for SyncPool<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncPool")
            .field("spare", &self.spare())
            .field("max_spare", &self.max_spare())
            .field("max_capacity", &self.max_capacity())
            .field("capacity", &self.shared.capacity)
            .finish()
    }
}

impl<T> SyncPool<T> {
    /// A pool whose new Vecs start with room for `capacity` items, keeping
    /// at most 64 spares, of any size.
    pub fn new(capacity: usize) -> SyncPool<T> {
        SyncPool {
            shared: Arc::new(Shared {
                free: Mutex::new(Vec::new()),
                capacity,
                max_spare: AtomicUsize::new(64),
                max_capacity: AtomicUsize::new(usize::MAX),
            }),
        }
    }

    /// Sets the spare limit ([`SyncPool::set_max_spare`]) as it is made.
    pub fn with_max_spare(self, n: usize) -> SyncPool<T> {
        self.set_max_spare(n);
        self
    }

    /// Sets the capacity limit ([`SyncPool::set_max_capacity`]) as it is made.
    pub fn with_max_capacity(self, n: usize) -> SyncPool<T> {
        self.set_max_capacity(Some(n));
        self
    }

    /// The spare list. A panic elsewhere cannot leave it half-changed (it
    /// is only pushed and popped), so a poisoned lock is used as it is.
    fn free(&self) -> MutexGuard<'_, Vec<Vec<T>>> {
        self.shared.free.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Spare Vecs kept at most, for every clone; spares past it are dropped
    /// now. 64 by default.
    pub fn set_max_spare(&self, n: usize) {
        self.shared.max_spare.store(n, Ordering::Relaxed);
        self.trim(n);
    }

    /// The spare limit.
    pub fn max_spare(&self) -> usize {
        self.shared.max_spare.load(Ordering::Relaxed)
    }

    /// The most capacity a kept Vec may have, for every clone; None for no
    /// limit (the default). Spares past it are shrunk now.
    pub fn set_max_capacity(&self, n: Option<usize>) {
        let n = n.unwrap_or(usize::MAX);
        self.shared.max_capacity.store(n, Ordering::Relaxed);
        for v in self.free().iter_mut() {
            v.shrink_to(n);
        }
    }

    /// The capacity limit.
    pub fn max_capacity(&self) -> Option<usize> {
        Some(self.shared.max_capacity.load(Ordering::Relaxed)).filter(|n| *n != usize::MAX)
    }

    /// Lends an empty Vec in a guard that gives it back when dropped.
    pub fn take(&self) -> Pooled<T> {
        Pooled { vec: Some(self.take_vec()), pool: self.clone() }
    }

    /// Lends an empty Vec to give back by hand with [`SyncPool::give`]: a
    /// spare one if there is one (with the capacity it had), else a new one.
    pub fn take_vec(&self) -> Vec<T> {
        let spare = self.free().pop();
        spare.unwrap_or_else(|| Vec::with_capacity(self.shared.capacity))
    }

    /// Takes `v` back, cleared and shrunk to the capacity limit; dropped
    /// when the pool already holds the spare limit. The clearing and
    /// shrinking happen before the lock is taken.
    pub fn give(&self, mut v: Vec<T>) {
        if self.free().len() >= self.max_spare() {
            return;
        }
        v.clear();
        v.shrink_to(self.shared.max_capacity.load(Ordering::Relaxed));
        let mut free = self.free();
        if free.len() < self.max_spare() {
            free.push(v);
        }
    }

    /// Drops spares until at most `n` are left.
    pub fn trim(&self, n: usize) {
        let dropped: Vec<Vec<T>> = {
            let mut free = self.free();
            let keep = n.min(free.len());
            free.drain(keep..).collect()
        };
        drop(dropped);
    }

    /// Spare Vecs held now.
    pub fn spare(&self) -> usize {
        self.free().len()
    }
}

/// A `Vec` lent by a [`SyncPool`], given back when dropped. Derefs to the
/// `Vec`; [`Pooled::into_inner`] keeps it instead.
pub struct Pooled<T> {
    vec: Option<Vec<T>>,
    pool: SyncPool<T>,
}

impl<T> Pooled<T> {
    /// The Vec, kept: it does not go back to the pool.
    pub fn into_inner(mut self) -> Vec<T> {
        self.vec.take().expect("a Pooled holds its Vec until dropped")
    }
}

impl<T> Deref for Pooled<T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Vec<T> {
        self.vec.as_ref().expect("a Pooled holds its Vec until dropped")
    }
}

impl<T> DerefMut for Pooled<T> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        self.vec.as_mut().expect("a Pooled holds its Vec until dropped")
    }
}

impl<T> Drop for Pooled<T> {
    fn drop(&mut self) {
        if let Some(v) = self.vec.take() {
            self.pool.give(v);
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Pooled<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vec_given_back_is_lent_again_with_its_capacity() {
        let mut pool: Pool<u32> = Pool::new(4);
        let mut v = pool.take();
        assert!(v.capacity() >= 4);
        v.extend(0..100);
        let (cap, ptr) = (v.capacity(), v.as_ptr());
        pool.give(v);
        assert_eq!(pool.spare(), 1);
        let again = pool.take();
        assert!(again.is_empty(), "cleared");
        assert_eq!((again.capacity(), again.as_ptr()), (cap, ptr), "the same allocation");
    }

    #[test]
    fn past_max_spare_a_given_vec_is_dropped() {
        let mut pool: Pool<u8> = Pool::new(0).with_max_spare(1);
        pool.give(Vec::new());
        pool.give(Vec::new());
        assert_eq!(pool.spare(), 1);
    }

    #[test]
    fn a_vec_past_max_capacity_is_shrunk_when_given_back() {
        let mut pool: Pool<u8> = Pool::new(0).with_max_capacity(64);
        let mut v = pool.take();
        v.extend(std::iter::repeat_n(0, 10_000));
        pool.give(v);
        let back = pool.take();
        assert!(back.capacity() <= 64 && back.is_empty(), "capacity {}", back.capacity());
        let mut small = Vec::with_capacity(32);
        small.push(1u8);
        pool.give(small);
        assert_eq!(pool.take().capacity(), 32, "small ones keep what they have");
    }

    #[test]
    fn limits_can_be_lowered_later() {
        let mut pool: Pool<u16> = Pool::new(0);
        for _ in 0..5 {
            pool.give(Vec::with_capacity(1000));
        }
        pool.trim(2);
        assert_eq!(pool.spare(), 2);
        let mut pool = pool.with_max_capacity(10).with_max_spare(1);
        assert_eq!(pool.spare(), 1);
        assert!(pool.take().capacity() <= 10);
        pool.max_spare = 3;
        pool.give(Vec::new());
        pool.give(Vec::new());
        assert_eq!(pool.spare(), 2, "the field can be set directly too");
    }

    #[test]
    fn a_sync_pool_and_its_guards_cross_threads() {
        fn shareable<T: Send + Sync>() {}
        shareable::<SyncPool<u8>>();
        shareable::<Pooled<u8>>();
    }

    #[test]
    fn a_dropped_guard_goes_back_and_is_lent_again() {
        let pool: SyncPool<u32> = SyncPool::new(4);
        let ptr = {
            let mut v = pool.take();
            v.extend(0..100);
            v.as_ptr()
        };
        assert_eq!(pool.spare(), 1);
        let again = pool.take();
        assert!(again.is_empty());
        assert_eq!(again.as_ptr(), ptr, "the same allocation");
        let kept = again.into_inner();
        assert_eq!((pool.spare(), kept.as_ptr()), (0, ptr), "into_inner keeps it out of the pool");
    }

    #[test]
    fn eight_threads_share_one_pool_within_its_limits() {
        let pool: SyncPool<u64> = SyncPool::new(16).with_max_spare(4);
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let pool = pool.clone();
                std::thread::spawn(move || {
                    for i in 0..1000u64 {
                        let mut a = pool.take();
                        let mut b = pool.take_vec();
                        a.push(t * 1000 + i);
                        b.extend([1, 2, 3]);
                        assert_eq!(a.len(), 1, "every lent Vec is empty");
                        assert!(pool.spare() <= 4);
                        pool.give(b);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(pool.spare() <= 4 && pool.spare() > 0);
        assert!(pool.take().capacity() >= 3, "spares keep their capacity");
    }

    #[test]
    fn limits_set_through_one_clone_hold_for_all() {
        let pool: SyncPool<u8> = SyncPool::new(0);
        let other = pool.clone();
        for _ in 0..5 {
            pool.give(Vec::with_capacity(1000));
        }
        other.set_max_spare(2);
        other.set_max_capacity(Some(10));
        assert_eq!((pool.spare(), pool.max_spare(), pool.max_capacity()), (2, 2, Some(10)));
        assert!(pool.take_vec().capacity() <= 10, "spares already held were shrunk");
        pool.give(Vec::with_capacity(5000));
        assert!(other.take_vec().capacity() <= 10);
        other.set_max_capacity(None);
        assert_eq!(pool.max_capacity(), None);
        pool.trim(0);
        assert_eq!(other.spare(), 0);
    }

    #[test]
    fn a_guard_dropped_while_its_thread_panics_still_goes_back() {
        let pool: SyncPool<u8> = SyncPool::new(0);
        let p = pool.clone();
        let r = std::thread::spawn(move || {
            let mut v = p.take();
            v.push(1);
            panic!("the worker failed");
        })
        .join();
        assert!(r.is_err());
        assert_eq!(pool.spare(), 1, "unwinding dropped the guard, which gave it back");
        assert!(pool.take().is_empty());
    }
}
