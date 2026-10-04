---
title: Moving an existing port onto retro_rt
status: draft
crates: all
covers: rrt_input::Input, rrt_gpu::Presenter, rrt_disc::Image, rrt_app::run
---

# Moving an existing port onto retro_rt

What in hwtr and piney_apples has a retro_rt equivalent, and what stays. Not
yet done for either; this page maps the work.

## What moves

| in the game today | retro_rt |
| --- | --- |
| `hwtr-input` (gilrs, keyboard bits, rumble) | `rrt::input::Input`; `Pad.buttons.bits()` is hwtr's `u16` word |
| piney-game `input.rs` (gilrs, `Keyboard`, `square_stick`, hat, `Rumble`) | `rrt::input::Input`; `Raw.buttons` from `Pad.buttons.libpad()`, sticks from `Pad.left/right` |
| hwtr `main.rs` / piney-game `main.rs` window, surface, `ApplicationHandler`, frame pacing | `rrt::app::run` and `Game`; `FRAME` / `VBLANK_HZ` become `Config::hz` |
| hwtr `present.rs`, piney-gs `Presenter` (4:3 letterbox, sharp bilinear, sRGB, overlay) | `rrt::gpu::Presenter` (piney's CRTC offset and deflicker merge stay in piney-gs) |
| hwtr `Scene::shot`, piney-gs `Gs::headless` / `read_back` | `rrt::gpu::Gpu::headless`, `Target::read_back`, or `rrt::app::headless` |
| hwtr-render `projection`, `plain_format`, depth texture | `rrt::gpu::projection`, `plain_format`, `DepthBuffer` / `TargetOptions::depth` |
| `hwtr-data::png`, piney-gs `png.rs`, piney-viewer `png.rs` | `rrt::image::png::encode` |
| piney-audio `output.rs` | `rrt::audio::Output` with piney's `Engine` as the `Source` |
| hwtr-render `VertexStaging` | `rrt::kit::Staging` with a `Pack` impl for `Vtx` |
| hwtr-render `Renderer::set_moving`'s power-of-two buffer | `rrt::gpu::GrowBuffer` |
| the formats' `le32` / `u16::from_le_bytes(b[at..])` helpers | `rrt::kit::Reader` |
| `/ 4096.0` on 20.12 positions and 4.12 matrices | `rrt::kit::Q12` where the arithmetic must match the GTE's truncation |
| 15-bit VRAM colour to RGBA (`tim.rs`, shaders' `* 8`) | `rrt::kit::Rgb555` (`rgb8_shifted` is the GPU's `<< 3`) |
| `hwtr-disc` | `rrt::disc::Image` (`Disc::open` becomes `Image::open`, `find_cue` `Image::find`) |

## What stays in the game

The console's hardware (PS1 VRAM texturing, the GS), the game's pad
reading (`ccPad::Read`, the actuator queue), its sound engine, its formats,
its CPU interpreter, its tools. These encode one machine or one game.

## Gaps

Not attempted. Each move should be its own change, checked by the game's
existing tests and `--shot` images before and after.
