---
title: rrt-demo, the template game
status: solid
crates: rrt-demo
covers: rrt_demo::Demo, rrt_demo::Tone, rrt_demo::main
---

# rrt-demo

The smallest whole game on retro_rt, and what a new one is copied from. A pad
tester: a 320 x 240 `Picture` with a box per button lit while held, the
sticks as dots, L2/R2's analog depth as bars. Cross plays a 440 Hz tone
through `rrt::audio`, Circle runs the motors, Select quits, the title names
the buttons pressed.

```sh
cargo run -p rrt-demo                                      # windowed
cargo run -p rrt-demo -- --help                            # every flag
cargo run -p rrt-demo -- --shot work/demo.png              # one frame, no window
cargo run -p rrt-demo -- --shot work/d.png --frames 20 --every 10 --size 320x240 --press 0:x:20
```

Its command line is `#[derive(rrt::Args)]` with `--tone HZ` of its own and
`AppArgs` flattened in; F1 opens its console (`help`, `frame`,
`rumble on|off`).

It touches every part a game uses: `#[rrt::main]`, `Game::tick` with
`Tick::pressed`, `rumble`, `set_title` and `exit`, `Draw::present_picture`,
`rrt::audio::Source`, `rrt::app::headless`.

## Gaps

Nothing known.
