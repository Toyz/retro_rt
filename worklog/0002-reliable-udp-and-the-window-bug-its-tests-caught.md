---
number: 2
title: Reliable UDP, and the window bug its tests caught
date: 2026-10-03
area: net, bug, test, build
files: crates/rrt-net/src/reliable.rs, crates/rrt-net/src/lib.rs, crates/rrt-gpu/tests/gpu.rs, crates/rrt-app/tests/headless.rs, .github/workflows/ci.yml, cairns.toml
---

# 2. Reliable UDP, and the window bug its tests caught

rrt-net now carries reliable UDP: `Rudp`, an endpoint with a salted connect handshake, and `Connection`, the protocol itself with no I/O. Each channel has a `Delivery`: `Unreliable`, `Sequenced` (newest wins, late ones dropped), `Reliable` (once, any order) or `ReliableOrdered` (once, in order). Packets carry `seq`, `ack` and 32 ack bits; a reliable message is resent each `rto` (twice the smoothed round trip, 30 ms to 1 s) until a packet that carried it is acknowledged. The wire format is on `docs/crates/rrt-net.md`.

Keeping `Connection` free of sockets and taking the time as an argument is what made it testable: the tests run two connections over a simulated link that drops, doubles and swaps packets, on a fixed pattern and on three xorshift seeds at 30% loss and 10% duplication, with no sleeps and the same result every run.

## The bug

`more_than_a_window_of_messages_waits_its_turn` (3072 messages through a 1024-message window) stalled with 1284 delivered. The sender chose what to send as the first `WINDOW` entries of its unacknowledged queue. But acks remove entries out of order: with id 260 still unacknowledged and 261..1283 acknowledged and gone, the first 1024 entries reached id 2306, which is more than 1024 past the receiver's next expected id (260). The receiver dropped those as out of window - and the packet carrying them was still acknowledged at the packet level, so the sender removed them as delivered. They were lost for good.

The fix counts the window by id from the oldest unacknowledged message (`o.id.wrapping_sub(oldest) < WINDOW`). Every id below the oldest has been acknowledged, so it was received, so the receiver's next expected id is at least the oldest, and everything sent lands inside its window. The out-of-window drop on the receiving side can now only be reached by a sender that is broken.

The general lesson for this protocol: acknowledgement is per packet, delivery is per message, and any receive path that discards a message without delivering it must be unreachable for a correct sender, because the ack will claim it anyway.

## Tests and CI

The GPU paths gained integration tests (`crates/rrt-gpu/tests/gpu.rs`, `crates/rrt-app/tests/headless.rs`) that run on whatever adapter exists and skip with a note where none does. `display_encoded_values_survive_an_srgb_target` confirms the colour convention on this machine: 0, 1, 10, 64, 128, 200 and 255 presented into an sRGB target come back within 1. The workspace has 56 tests; the full sequence (fmt, clippy and tests under `RUSTFLAGS=-D warnings`, strict rustdoc, `rrt-docs check`, `rrt-demo --shot`, `cairns check`) passes locally.

`.github/workflows/worklog.yml` is piney_apples' Pages workflow as it was. `ci.yml` is new: the sequence above on ubuntu with libudev, ALSA and Mesa's lavapipe for the GPU, the demo shot uploaded as an artifact, and `cairns check` in its own job. `cairns.toml` now describes the runtime and files entries under areas that fit it (`app`, `input`, `gpu`, `audio`, `disc`, `net`, `docs`, `port`, plus the usual `design`, `bug`, `perf`, `build`, `test`); `cairns init` regenerated the worklog skill's area table from it and kept the hand-written project section.

**Still unknown:** No fragmentation (a message must fit one packet, 1173 bytes at the default MTU), no congestion control, no path-MTU discovery. Not yet measured over a real lossy network, only the simulated link and loopback. Whether lavapipe on the GitHub runner passes the GPU tests is unconfirmed until the first CI run.
