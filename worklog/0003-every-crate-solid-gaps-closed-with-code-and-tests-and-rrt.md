---
number: 3
title: Every crate solid: gaps closed with code and tests, and rrt-kit
date: 2026-10-03
area: net, disc, input, audio, gpu, app, test, docs
files: crates/rrt-net/src/reliable.rs, crates/rrt-net/src/udp.rs, crates/rrt-disc/src/iso.rs, crates/rrt-input/src/gamepad.rs, crates/rrt-input/src/script.rs, crates/rrt-audio/src/lib.rs, crates/rrt-app/src/lib.rs, crates/rrt-app/tests/window.rs, crates/rrt-gpu/src/buffer.rs, crates/rrt-kit/src/lib.rs, .claude/skills/docs/SKILL.md
---

# 3. Every crate solid: gaps closed with code and tests, and rrt-kit

Every crate page is now `solid`, and each gap was closed with code and a test rather than relabelled. The workspace has 117 tests plus a windowed-loop test with its own `main`; the full sequence (fmt, clippy and tests under `RUSTFLAGS=-D warnings`, the tests under Xvfb, strict rustdoc, `rrt-docs check`, `rrt-demo --shot`, `cairns check`) passes.

## What closed each gap

- **net**: reliable messages longer than a packet are fragmented (channel byte bit 7, then a u16 index and count) and reassembled whole, in place on an ordered channel, by first id on an unordered one; `a_message_of_more_fragments_than_the_window_gets_through` sends 1124 fragments through a 1024 window. Sends are paced by `RudpConfig::bandwidth` (4 MiB/s default, a tenth of a second's burst): `pacing_spreads_a_large_send_over_time` holds every transmit to its budget and sees 600 kB at 120 kB/s arrive in 4.5-6 s. Adaptive congestion control is a deliberate omission, recorded under Not here.
- **net, broadcast and multicast**: the first tests sent a limited broadcast and a default-interface multicast and heard nothing back on this machine. A Python probe showed why: the host's firewall on `enp12s0` drops them inbound, while a directed broadcast to 127.255.255.255 and a multicast pinned to 127.0.0.1 loop back. The tests now use loopback, which needed `Udp::set_multicast_interface` (socket2), and `Udp::bind_reusable` (SO_REUSEADDR) came with it, so two copies of a game share a discovery port: `a_multicast_reaches_every_member_sharing_a_port`.
- **disc**: one synthetic disc written both cooked and as raw Mode 2 sectors with a CUE sheet exercises the raw path; multi-extent files (records flagged 0x80, "not final") are joined into one `DirEntry` with `extents` and a `u64` size.
- **input**: the mapping moved out of the gilrs calls into `compose(keyboard, Option<&GamepadState>, shape)`, a pure function, and the active-pad rule into `next_active`; both are tested with fake pads. `Script` parses the ports' `--press` form.
- **audio**: the device callback's work is `mix_into` (formats, mono mean, silent extra channels and partial frames), tested; `the_default_device_pulls_from_the_source` runs on this machine's device and skips where there is none.
- **gpu**: `sharp_bilinear_blends_only_the_seam` checks the shader's arithmetic worked out by hand - two texels over five pixels give red, red, an even mix, green, green - and plain bilinear blends three.
- **app**: `Config::script` drives the pad in `run` and `headless`; `Config::fullscreen` and F11; `Init::window` is the `Arc<Window>` a game can keep. Escape and F11 handling is `key_action`, tested. `tests/window.rs` runs the real loop: 60 ticks at 120 Hz in 492 ms (no less than half a second), the script's presses seen on frames 10 and 30, then `Tick::exit` and `Game::exit`. It ran on this machine's display and under Xvfb.

The pattern in every case was the same: shrink the part that needs a device to a few lines of glue and test the rest in-process. The docs skill now says so, and says that a page with a gap is `partial` while a deliberate omission goes under Not here.

## rrt-kit and GrowBuffer

hwtr had just grown its own `VertexStaging` (a byte buffer packed each frame, reallocating only when it grows) and `Renderer::set_moving`'s power-of-two GPU buffer. They are now `rrt_kit::Staging` with a `Pack` trait and `rrt_gpu::GrowBuffer`. `rrt-kit` also carries what every port rewrites: `Pool`, a generational `Slab`, an allocation-free `Ring`, `Q12` fixed point whose multiply truncates toward minus infinity like the GTE (`multiply_truncates_toward_minus_infinity`), a byte `Reader`, `Rgb555` with both 8-bit expansions (`c << 3` as the PS1 GPU does it, `c << 3 | c >> 2` for images), and BCD.

Two test failures on the way were the tests' arithmetic, not the code's, and are worth recording so nobody "fixes" the code toward them: a round stick at 45 degrees reads y 37, not 38 (128 - 0.7071 x 128 = 37.49), and `"SLUS_009\0junk"` is 13 bytes.

**Still unknown:** Gamepad reading and rumble through gilrs, and the F11 toggle on a real window manager, are checked by hand with rrt-demo, not by a test. Whether lavapipe under Xvfb on the GitHub runner passes the GPU and window tests is unconfirmed until the first CI run.
