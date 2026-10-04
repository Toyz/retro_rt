---
title: Starting a game on retro_rt
status: solid
crates: rrt, rrt-app, rrt-demo
covers: rrt::main, rrt_app::Game, rrt_app::headless, rrt_app::IntoGame
---

# Starting a game on retro_rt

The layout every port uses (hwtr, piney_apples): a Cargo workspace of the
game's own crates, the program crate depending on `rrt`.

## 1. The dependency

In the game's workspace `Cargo.toml`:

```toml
[workspace.dependencies]
rrt = { path = "../../repos/retro_rt/crates/rrt" }   # relative to the game's repo
```

and in the program crate `rrt.workspace = true`. Data crates (format
parsers) that need no window take `rrt` with `default-features = false` and
the features they use, or nothing at all.

## 2. The program

Copy `crates/rrt-demo/src/main.rs`. The shape:

```rust
use rrt::prelude::*;

struct MyGame { /* state */ }

impl Game for MyGame {
    fn init(&mut self, ctx: &mut Init<'_>) { /* pipelines, uploads */ }
    fn tick(&mut self, t: &mut Tick) { /* one frame at Config::hz, t.pad */ }
    fn draw(&mut self, d: &mut Draw<'_>) { /* d.present_picture / present_target */ }
}

#[rrt::main(title = "mygame", hz = rrt::app::rate::NTSC)]
fn main() -> Result<MyGame, String> {
    // parse arguments, find the disc, load what the first screen needs
    Ok(MyGame { /* ... */ })
}
```

## 3. Reading the pad

`t.pad` is the [pad convention](../conventions/pad.md)'s `Pad`. A PS1 game
uses `t.pad.buttons.bits()` as its pad word; a PS2 libpad game
`t.pad.buttons.libpad()`. The game's own pad logic (repeat delay, dead zone,
analog-to-D-pad) stays in the game's input crate and is fed from these.

## 4. Drawing

Either build a `Picture` and `d.present_picture(&pic, None)`, or make an
`rrt::gpu::Target` in `init` at the console's resolution, render into it with
`target.color_attachment(..)`, and `d.present_target(&target, None)`. The
presenter letterboxes at 4:3 with sharp bilinear scaling; change
`d.presenter.aspect` and `filter` at any time.

## 5. --shot

`rrt::app::headless(&mut game, &config, ticks, w, h)` runs `init`, `ticks`
ticks with the pad at rest, one `draw`, and returns the `Picture`;
`picture.to_png()` writes it. This is the draw path the window uses, so a
shot is a real frame.

## 6. Docs

Copy `.claude/skills/docs/SKILL.md` into the game's repo and adapt its
"This project" section; the game's own docs follow the same front matter
and the same `status` knob.
