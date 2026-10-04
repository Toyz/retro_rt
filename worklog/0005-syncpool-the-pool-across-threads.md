---
number: 5
title: SyncPool: the pool across threads
date: 2026-10-03
area: design, test
files: crates/rrt-kit/src/pool.rs
---

# 5. SyncPool: the pool across threads

`rrt_kit::SyncPool<T>` is `Pool` for several threads: a cloneable handle on one shared spare list, behind one `Mutex`, with the same two limits (`max_spare`, `max_capacity`) held in atomics so any clone reads and sets them.

The lock is held only to push or pop a `Vec`. `give` clears and shrinks the `Vec` before taking it, and checks the spare limit both before (to skip the work when full) and under the lock (to stay exact). That keeps the critical section to a pointer move, which matters for the case that motivated it, an audio callback taking buffers a decoder thread fills.

`take()` returns `Pooled<T>`, a guard that derefs to the `Vec` and gives it back on drop, so a buffer handed across a channel returns itself from whichever thread drops it; `into_inner()` opts out. `a_guard_dropped_while_its_thread_panics_still_goes_back` shows the guard is returned during unwinding. A poisoned lock is used as it is, since the spare list is only ever pushed and popped.

Tests: `a_sync_pool_and_its_guards_cross_threads` (Send + Sync), `a_dropped_guard_goes_back_and_is_lent_again`, `eight_threads_share_one_pool_within_its_limits` (8 threads x 1000 rounds of take, take_vec, give; the spare count never exceeds the limit), `limits_set_through_one_clone_hold_for_all`. Rejected: a lock-free stack - nothing measured shows the mutex costs anything here, and a correct lock-free free list needs ABA protection this crate would have to carry for no proven gain.

**Still unknown:** How long a SyncPool push or pop holds its lock under contention has not been measured; the docs call it inferred.
