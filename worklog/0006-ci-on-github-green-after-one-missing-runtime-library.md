---
number: 6
title: CI on GitHub: green, after one missing runtime library
date: 2026-10-03
area: build, test
files: .github/workflows/ci.yml
---

# 6. CI on GitHub: green, after one missing runtime library

The repository is public at github.com/Toyz/retro_rt, with the worklog and reference published to toyz.github.io/retro_rt by `worklog.yml` (GitHub Pages, Actions source). This answers the CI question [[2]] and [[3]] left open: lavapipe under Xvfb on `ubuntu-latest` runs the GPU and window tests.

The first run failed in one place. `tests/window.rs` panicked inside winit's X11 backend, in `xkbcommon-dl` (`x11.rs:59`): winit loads `libxkbcommon-x11` and the X11 client libraries with `dlopen` at run time, so nothing at build time notices their absence, and Xvfb alone does not bring them. Installing `libxkbcommon-x11-0 libx11-xcb1 libxcursor1 libxrandr2 libxi6` fixed it; the second run passed every step in 4 min 50 s, the window test reporting 60 ticks, 503 draws, 520 ms.

The same failure would hit a player's Linux machine without those libraries, as a panic from winit rather than an `Err` from `run`. That is winit's behaviour and outside the runtime's control; a port's README should list them.

Test output is captured by the harness, so the CI log does not show whether the GPU tests ran or took their skip path. They did run: `tests/headless.rs` took 3.00 s on the runner, where the skip path returns at once.

**Still unknown:** Gamepad reading, rumble and the F11 toggle remain checked by hand with rrt-demo only (see [[3]]). The audio device test cannot run on a runner, which has no sound card.
