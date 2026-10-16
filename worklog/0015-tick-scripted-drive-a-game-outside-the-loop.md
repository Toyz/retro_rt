---
number: 15
title: Tick::scripted: drive a Game outside the loop
date: 2026-10-15
area: app, port, test
files: crates/rrt-app/src/lib.rs, docs/crates/rrt-app.md
took: 6m
---

# 15. Tick::scripted: drive a Game outside the loop

Rugrats wanted to test its boot (the title, the menus, a level, the exits) by calling its own Game::tick with scripted pads. A Tick could only be made by the loop (Tick::new was private), and `headless` needs a GPU because it draws.

- **Tick::scripted(pad, previous, frame, dt)** is a tick made outside the loop. The loop's requests (exit, title, rumble) start unset.
- **Tick::exiting** reads back whether the game called exit().
- **Test** a_scripted_tick_drives_a_game:
  - Four scripted ticks with Cross held, held, released, pressed.
  - The game counts two presses, so a held button counts once.
  - It ends on the fourth, and the caller sees that through exiting().
- **rrt-app.md:** the Tick row and the test list.

**Still unknown:**
- nothing
