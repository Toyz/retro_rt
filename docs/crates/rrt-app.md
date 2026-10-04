---
title: rrt-app, the engine loop
status: solid
crates: rrt-app
covers: rrt_app::Game, rrt_app::Game::init, rrt_app::Game::tick, rrt_app::Game::draw, rrt_app::Game::event, rrt_app::Game::exit, rrt_app::Init, rrt_app::Tick, rrt_app::Tick::pressed, rrt_app::Tick::exit, rrt_app::Tick::set_title, rrt_app::Tick::rumble, rrt_app::Tick::dt, rrt_app::Tick::time, rrt_app::Draw, rrt_app::Draw::encoder, rrt_app::Draw::present_picture, rrt_app::Draw::present_target, rrt_app::Draw::submit, rrt_app::run, rrt_app::headless, rrt_app::launch, rrt_app::IntoGame, rrt_app::Config, rrt_app::Clock, rrt_app::Clock::due, rrt_app::rate, rrt_app::init_logging
---

# rrt-app

The loop every game runs in. A game implements `Game`; `run` owns the window,
the wgpu surface, the pad and the clock, and calls the game. `headless` runs
the same game with no window and returns the frame, so `--shot` and golden
tests exercise the real draw path.

## The trait

```rust
pub trait Game: 'static {
    fn init(&mut self, ctx: &mut Init<'_>) {}       // once, GPU ready
    fn tick(&mut self, ctx: &mut Tick);              // Config::hz times a second
    fn draw(&mut self, ctx: &mut Draw<'_>);          // once per frame shown
    fn event(&mut self, event: &WindowEvent) -> bool { false }  // true = taken
    fn exit(&mut self) {}                            // closing
}
```

| context | holds | asks of the loop |
| --- | --- | --- |
| `Init` | `gpu`, `format` (surface, usually sRGB), `plain_format` (same without sRGB), `window` (`&Arc<Window>`, clone to keep; None headless) | - |
| `Tick` | `pad`, `previous` (the pad latched this tick and last), `frame` (ticks before this one), `dt` (the fixed step, `1 / hz`), `pad_line` | `exit()`, `set_title()`, `rumble(Motors)`, `pressed()`, `time()` (`dt * frame`) |
| `Draw` | `gpu`, `presenter`, `target` (frame view, `format`), `plain` (same frame, `plain_format`), `width`, `height` | `encoder()`, `present_picture()`, `present_target()`, `submit()` |

A game that builds a CPU picture calls `present_picture`; one with its own
renderer draws into an `rrt_gpu::Target` and calls `present_target`; one that
draws straight to the window records into `Draw::encoder()` (made on first
use) against `target` or `plain`, or submits command buffers of its own.
Everything runs in call order: `submit` and the present helpers close the
open encoder first, and the next `encoder()` opens a fresh one. Then the
frame is shown.

`dt` is the same every tick however late the tick runs, so a game's physics
stays deterministic; there is no variable time step and no interpolation
between ticks (a retro port draws the latest tick, as the console did).

## Order of a frame

```
RedrawRequested
  Clock::due(now) -> n (at most Config::max_catch_up)
  n times:  Input::read, OR Config::script's presses -> Tick { pad, previous } -> Game::tick
            rumble / title / exit requests applied
  Game::draw -> submit -> present
about_to_wait: next redraw now, or at Config::fps_cap's time
```

Window events go to `Game::event` first; a taken event never reaches the
loop. Otherwise: close, and Escape with `escape_quits`, end the loop after
`Game::exit`; `fullscreen_key` (F11) going down toggles borderless
fullscreen; keys go to the pad's keyboard; losing focus releases every key;
a resize reconfigures the surface.

## Config

`Config::default()`: 960 x 720 window, `hz` = `rate::NTSC` (60000/1001),
`max_catch_up` 4, vsync on, no fps cap, Escape quits, log filter `info`,
the default `KeyMap`, `Aspect::TV`, `Filter::SharpBilinear`, `pad_log` off,
windowed, F11 toggles fullscreen, no script. Every field has a builder method
of the same name, which `#[rrt::main]` calls. `rate::PAL` is 50.

## Clock

`Clock::due(now)` returns the ticks due since the last call, each one period
after the last. Behind by more than `max_catch_up`, the excess is dropped and
the clock is due again from `now`, so a stall (a load, a dragged window)
resumes at speed instead of fast-forwarding.

## Entry points

- `run(config, game)` - the window loop, until it closes.
- `headless(&mut game, &config, ticks, w, h)` - `init`, `ticks` ticks with
  the pad driven by `config.script` (at rest without one), one `draw` into a
  `w` x `h` `Target` in `Target::FORMAT`, read back as a `Picture`.
  `rrt-demo --shot` uses it.
- `launch(config, make)` - logging at `config.log`, `make()` into a game
  (`IntoGame`: the game or a `Result` of it), `run`; an error is logged and
  exits 1. `#[rrt::main]` expands to it.
- `init_logging(filter)` - tracing to stderr, `RUST_LOG` winning.

## Tests

Unit: `ticks_come_due_one_period_apart`, `falling_far_behind_drops_the_excess`,
`escape_quits_and_f11_toggles_by_default`, `both_keys_can_be_turned_off`,
`present_mode_follows_vsync`, `config_builders_set_their_fields`,
`into_game_takes_a_game_or_a_result`.
Headless, on the machine's adapter (`tests/headless.rs`):
`headless_runs_init_the_ticks_and_one_draw` (and `dt` is `1 / hz`),
`exit_stops_the_headless_ticks`, `draw_work_runs_in_call_order`,
`a_script_drives_the_headless_pad`.
Windowed (`tests/window.rs`, its own `main` because winit needs the main
thread; CI runs it under Xvfb): `run` opens a window and gives it to `init`,
ticks 60 times at 120 Hz in no less than half a second, sees the script's
presses as pressed on frames 10 and 30, draws, stops on `Tick::exit` and runs
`Game::exit`. With no display it says so and passes.

## Not here

Several windows; mouse capture and cursor control (a game does them through
`Init::window`, which it may keep); audio in `headless` (open an
`rrt_audio::Output` where sound is wanted).

## Gaps

Nothing known.
