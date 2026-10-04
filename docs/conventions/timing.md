---
title: Timing: fixed ticks, frames shown when they come
status: solid
crates: rrt-app
covers: rrt_app::Clock, rrt_app::rate, rrt_app::Config
---

# Timing

Game logic runs at a fixed rate, `Config::hz`, in `Game::tick`; drawing
happens once per frame the display shows, in `Game::draw`. The two are
decoupled: a 144 Hz monitor draws the same tick several times, a slow frame
runs several ticks before one draw.

- `rate::NTSC` is 60000/1001 (59.94...), `rate::PAL` 50. A game whose logic
  runs every other field (30 fps) counts fields itself in `tick` (piney_apples'
  `frame_rate` divisor) rather than setting `hz` to 29.97, so field-timed
  things (vblank counters, rumble timers) keep their unit.
- At most `Config::max_catch_up` ticks (default 4) run before a draw; past
  that the excess is dropped and the clock restarts from now. A load or a
  dragged window does not fast-forward the game.
- `Tick::frame` counts ticks run since start, the unit for scripted presses
  and replay logs.
- `Config::vsync` waits for the display; `Config::fps_cap` caps frames shown
  without vsync. Neither changes the tick rate.

Tests: `ticks_come_due_one_period_apart`, `falling_far_behind_drops_the_excess`.
