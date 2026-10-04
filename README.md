# retro_rt

The shared runtime for Rust ports of retro games. Each port (hwtr, piney_apples
and the ones after) rewrites one game; this workspace holds what they all need
and kept copying: the window and the loop, the pad, wgpu plumbing, sound out,
disc images, networking, PNG. Nothing here encodes one console's hardware or
one game's behaviour - that stays in the game.

## Crates

| crate | what |
| --- | --- |
| `crates/rrt` | the facade a game depends on: every part as a module, behind features, with the shared `wgpu`, `winit`, `glam`, `tracing` |
| `crates/rrt-app` | the engine loop: implement `Game`, `run` opens the window and ticks it at a fixed rate; `headless` for `--shot` |
| `crates/rrt-macros` | `#[rrt::main(title = ..., hz = ...)]` |
| `crates/rrt-input` | the host's pad: gilrs gamepads and the keyboard as a PlayStation-style pad, rumble |
| `crates/rrt-gpu` | wgpu: headless or windowed device, offscreen targets with readback, MSAA and depth, a per-frame growable buffer, a 4:3 sharp presenter |
| `crates/rrt-audio` | a game's mixer played through cpal, resampled to the device |
| `crates/rrt-disc` | CUE/BIN, ISO images, ISO 9660 with CD-XA |
| `crates/rrt-net` | non-blocking UDP, TCP client/server, and reliable UDP (unreliable, sequenced, reliable, reliable-ordered channels), polled once a tick |
| `crates/rrt-image` | `Picture` and a PNG writer |
| `crates/rrt-kit` | buffers recycled across frames, a generational arena, a ring, Q12 fixed point, a byte reader, RGB555, BCD |
| `crates/rrt-docs` | keeps `docs/` in step with the code |
| `crates/rrt-demo` | the template game: a pad tester |
| `docs/` | the [reference](docs/README.md) |

## A game in thirty lines

```rust
use rrt::prelude::*;

struct Game1 { picture: Picture }

impl Game for Game1 {
    fn tick(&mut self, t: &mut Tick) {
        if t.pressed().contains(Buttons::START) {
            t.exit();
        }
    }

    fn draw(&mut self, d: &mut Draw<'_>) {
        d.present_picture(&self.picture, None);
    }
}

#[rrt::main(title = "game1", hz = rrt::app::rate::NTSC)]
fn main() -> Game1 {
    Game1 { picture: Picture::filled(320, 240, [0, 0, 64, 255]) }
}
```

See [starting a game](docs/guides/new-game.md).

## Using it

```sh
cargo run -p rrt-demo                          # the pad tester, windowed
cargo run -p rrt-demo -- --shot work/demo.png  # one frame, headless
cargo test --workspace
cargo run -q -p rrt-docs -- check              # docs agree with the code
```

A game depends on it by path:

```toml
rrt = { path = "../../repos/retro_rt/crates/rrt" }
```

## License

MIT or Apache-2.0, at your option.
