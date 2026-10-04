---
number: 1
title: retro_rt: the shared runtime, split out of hwtr and piney_apples
date: 2026-10-03
area: design, build
files: Cargo.toml, crates/*, docs/*, .claude/skills/docs/SKILL.md
---

# 1. retro_rt: the shared runtime, split out of hwtr and piney_apples

retro_rt holds what every port had grown its own copy of. Reading hwtr and piney_apples side by side, the duplicates were: gilrs pad reading with the same button table, active-pad choice and rumble (hwtr-input, piney-game input.rs); the winit ApplicationHandler with a fixed-step catch-up loop (both main.rs); a 4:3 letterbox presenter (hwtr present.rs, piney-gs Presenter); headless wgpu and texture readback (hwtr Scene::shot, piney-gs Gs::headless); three PNG writers (hwtr-data, piney-gs, piney-viewer); cpal output with a resampler (piney-audio output.rs); and CUE/ISO 9660 reading (hwtr-disc, piney-data iso.rs).

## What went in, and what stayed out

The runtime takes the host side only. The line is: code goes in when two games would otherwise carry a copy of it and it encodes neither one console's hardware nor one game's behaviour. So `ccPad::Read`, the actuator queue, PS1 VRAM texturing and the GS stay in their games; the host gamepad, the presenter and the loop come here.

The pad is fixed to the PlayStation SIO word (hwtr's order). piney's libpad `direct` turned out to be the same word byte-swapped (L2 bit 8 -> 0x0001, UP bit 4 -> 0x1000), so one `Buttons` type serves both, with `libpad()`/`from_libpad()`; test `libpad_order_is_the_byte_swapped_word`.

From each source the better behaviour was kept: piney's active-pad rule (a press or a push past 0.5 takes over; drift does not), hat fallback and `square_stick`; hwtr's `axis_byte` rounding; a held-key set instead of hwtr's bit toggling, which released a button shared by two keys when either came up (`a_button_stays_held_while_any_of_its_keys_is`). The presenter is piney's (convert to linear only for an sRGB target), which serves hwtr's case too, and now keeps its picture texture between frames instead of making one per frame.

## The engine shape

`Game` has `init` (GPU ready), `tick` (fixed `Config::hz`), `draw` (once per shown frame), `event`, `exit`. `#[rrt::main(k = v, ...)]` expands each pair to the `Config` builder method of that name, so the macro needs no list of settings and a misspelt one is a compile error. `headless` runs the same `init`/`tick`/`draw` into a `Target`, so `--shot` exercises the real draw path; `rrt-demo --shot` produced a correct 640x480 PNG through game -> Picture -> Presenter -> Target -> readback.

Rejected: an async runtime for rrt-net (the loop already ticks; polling non-blocking std sockets from `tick` needs no threads), and a game-side factory closure taking `&Gpu` (it would force argument parsing and `--shot` handling inside the window callback; `Game::init` instead).

## Docs

`rrt-docs check` holds the pages to the code: every crate has a page, every `covers` item is still declared, every crate has a `//!` header and the workspace lints. Its first run caught its own bug: `pub const fn` was not recognised as a declaration (test `declarations_are_found_by_their_last_segment` now covers it).

**Still unknown:** Neither hwtr nor piney_apples has been moved onto it yet (docs/guides/porting.md maps the work); the windowed loop, the presenter's scaling, gamepad reading and the audio device path have no automated tests; broadcast and multicast UDP are untested off loopback.
