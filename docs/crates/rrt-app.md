---
title: rrt-app, the engine loop
status: solid
crates: rrt-app
covers: rrt_app::Game, rrt_app::Game::init, rrt_app::Game::tick, rrt_app::Game::draw, rrt_app::Game::event, rrt_app::Game::exit, rrt_app::Init, rrt_app::Tick, rrt_app::Tick::pressed, rrt_app::Tick::exit, rrt_app::Tick::set_title, rrt_app::Tick::rumble, rrt_app::Tick::dt, rrt_app::Tick::time, rrt_app::Tick::log_command, rrt_app::Console, rrt_app::Console::event, rrt_app::Console::key, rrt_app::Console::run, rrt_app::Console::take_commands, rrt_app::Console::say, rrt_app::Console::describe, rrt_app::Console::overlay, rrt_app::ConsoleKey, rrt_app::Draw, rrt_app::Draw::encoder, rrt_app::Draw::present_picture, rrt_app::Draw::present_target, rrt_app::Draw::submit, rrt_app::run, rrt_app::headless, rrt_app::launch, rrt_app::IntoGame, rrt_app::AppArgs, rrt_app::AppArgs::apply, rrt_app::WithArgs, rrt_app::headless_shots, rrt_app::Config, rrt_app::Clock, rrt_app::Clock::due, rrt_app::rate, rrt_app::init_logging
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
| `Tick` | `pad`, `previous` (the pad latched this tick and last), `frame` (ticks before this one), `dt` (the fixed step, `1 / hz`), `pad_line`, `commands` (replayed console commands for this frame) | `exit()`, `set_title()`, `rumble(Motors)`, `pressed()`, `time()` (`dt * frame`), `log_command()` |
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

## Console

`Console` is a drop-down debug console. It only collects lines: the game
takes them each tick (`take_commands`), acts on its own state, and answers
with `say`; `help` and `clear` are answered by the console, and `describe`
adds a command to `help`. Wiring:

```rust
fn event(&mut self, e: &WindowEvent) -> bool { self.console.event(e) }   // F1 or backquote toggles
fn tick(&mut self, t: &mut Tick) {
    for line in std::mem::take(&mut t.commands) { self.console.run(&line); }  // replayed
    for line in self.console.take_commands() { t.log_command(&line); /* act, then say */ }
    if self.console.open { return; }                                        // keys are the console's
}
fn draw(&mut self, d: &mut Draw<'_>) {
    let overlay = self.console.overlay(d.width, d.height);
    d.present_picture(&self.picture, overlay.as_ref());
}
```

While open it takes every key (`event` returns true, so the pad does not
see them): typed characters, Enter (run), Backspace, Up/Down (history,
repeats kept once), Escape (close). The key handling is `Console::key` on a
`ConsoleKey`, which `event` maps winit's keys onto. `overlay` draws the
latest lines and the prompt with `rrt_image::font` on a translucent panel
over the top half, at `scale` (2 by default). `rrt-demo` carries one.

## Recording and replay

`Config::record(path)` writes an `rrt_input::PadLog` when the loop ends
(windowed or headless): every tick's pad, plus the commands replayed or
logged with `Tick::log_command`. `Config::replay(log)` plays one back: on a
frame the log has a pad for, that pad replaces the live one entirely (the
script's presses still add), and its commands arrive in `Tick::commands`.
Frames past the log's end take the live pad again.

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
- `headless_shots(&mut game, &config, ticks, w, h, every, each)` - the same,
  also drawing a frame every `every` ticks for `each`.
- `launch(config, make)` - logging at `config.log`, `make()` into a game
  (`IntoGame`: the game, `WithArgs(game, AppArgs)`, or a `Result` of either),
  the flags applied, then `run` - or with `--shot`, the PNGs written and the
  process done; an error is logged and exits 1. `#[rrt::main]` expands to
  it.

## The standard flags

`AppArgs` is every game's shared command line, flattened into its own
`#[derive(rrt::Args)]` struct and handed back as `WithArgs(game, args)`:

| flag | effect |
| --- | --- |
| `--shot PATH` | run headless, write the frame as PNG, exit |
| `--frames N` | ticks before the shot (default 1) |
| `--every N` | with `--shot`, also `PATH-N.png`, `PATH-2N.png`, ... |
| `--size WxH` | the shot's size (default: the window's) |
| `--press SCRIPT` | `FRAME:BUTTONS[:FRAMES],...`, repeatable (`Config::script`) |
| `--replay PATH` | play a pad log (`Config::replay`) |
| `--record PATH`, `--pad-log PATH` | write one (`Config::record`) |
| `--fullscreen`, `--no-vsync`, `--fps-cap N`, `--log FILTER` | the window and logging |

`AppArgs::apply(config)` sets them on a `Config` (reading and parsing the
replay, parsing the script) and says which flag was wrong. Tests:
`the_standard_flags_parse`, `applying_sets_the_config`.
- `init_logging(filter)` - tracing to stderr, `RUST_LOG` winning.

## Tests

Unit: `ticks_come_due_one_period_apart`, `falling_far_behind_drops_the_excess`,
`escape_quits_and_f11_toggles_by_default`, `both_keys_can_be_turned_off`,
`present_mode_follows_vsync`, `config_builders_set_their_fields`,
`into_game_takes_a_game_or_a_result`.
Headless, on the machine's adapter (`tests/headless.rs`):
`headless_runs_init_the_ticks_and_one_draw` (and `dt` is `1 / hz`),
`exit_stops_the_headless_ticks`, `draw_work_runs_in_call_order`,
`a_script_drives_the_headless_pad`, `a_recorded_session_replays_frame_for_frame`.
Console: `closed_it_takes_only_the_toggle`,
`typed_lines_go_to_the_game_and_help_and_clear_are_answered`,
`up_and_down_walk_the_history`, `the_scrollback_keeps_its_limit`,
`the_overlay_is_drawn_only_while_open`.
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
