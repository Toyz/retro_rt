//! rrt-demo: the smallest whole game on retro_rt, and the template a new one
//! starts from.
//!
//! A pad tester: a 320 x 240 picture with a box per button, lit while held,
//! and the two sticks as dots. Cross plays a tone, Circle runs the motors,
//! Select quits. F1 or backquote opens the console (`help`, `frame`,
//! `rumble on|off`).
//!
//! ```text
//! rrt-demo [--tone HZ] [the standard flags: --shot, --frames, --press, --replay, --record, ...]
//! ```
//!
//! Its arguments are `#[derive(rrt::Args)]` with the standard
//! `rrt::app::AppArgs` flattened in, so `--shot OUT.png` draws a frame with
//! no window and writes it, and `--help` lists everything.

use std::sync::{Arc, Mutex};

use rrt::audio::{Output, Source};
use rrt::prelude::*;

/// The picture's size: a PS1's common 320 x 240.
const W: u32 = 320;
const H: u32 = 240;

/// Where each button's box is, in the picture.
const BOXES: [(Buttons, u32, u32); 16] = [
    (Buttons::L2, 40, 20),
    (Buttons::L1, 40, 40),
    (Buttons::R2, 260, 20),
    (Buttons::R1, 260, 40),
    (Buttons::UP, 60, 80),
    (Buttons::LEFT, 40, 100),
    (Buttons::RIGHT, 80, 100),
    (Buttons::DOWN, 60, 120),
    (Buttons::TRIANGLE, 260, 80),
    (Buttons::SQUARE, 240, 100),
    (Buttons::CIRCLE, 280, 100),
    (Buttons::CROSS, 260, 120),
    (Buttons::SELECT, 130, 100),
    (Buttons::START, 170, 100),
    (Buttons::L3, 110, 180),
    (Buttons::R3, 190, 180),
];

/// A sine at `hz` while `on`.
struct Tone {
    on: bool,
    phase: f32,
    hz: f32,
}

impl Source for Tone {
    fn rate(&self) -> u32 {
        48000
    }

    fn render(&mut self, out: &mut [i16]) {
        for frame in out.as_chunks_mut::<2>().0 {
            let v = if self.on { (self.phase * std::f32::consts::TAU).sin() * 6000.0 } else { 0.0 };
            self.phase = (self.phase + self.hz / 48000.0).fract();
            frame.fill(v as i16);
        }
    }
}

struct Demo {
    pad: Pad,
    console: Console,
    frame: u64,
    /// The console's `rumble on` holds the motors on.
    rumble: bool,
    picture: Picture,
    tone: Arc<Mutex<Tone>>,
    /// Kept so the sound plays; None with no device.
    _audio: Option<Output>,
}

impl Demo {
    fn new(with_audio: bool, hz: f32) -> Demo {
        let tone = Arc::new(Mutex::new(Tone { on: false, phase: 0.0, hz }));
        let audio = with_audio
            .then(|| Output::open(tone.clone()))
            .and_then(|r| r.map_err(|e| rrt::tracing::warn!("no audio: {e}")).ok());
        let mut console = Console::new();
        console.describe("frame", "the tick count");
        console.describe("rumble", "on or off: hold the motors");
        console.say("rrt-demo console - type help");
        Demo {
            pad: Pad::default(),
            console,
            frame: 0,
            rumble: false,
            picture: Picture::filled(W, H, [0, 0, 0, 255]),
            tone,
            _audio: audio,
        }
    }

    fn paint(&mut self) {
        let p = &mut self.picture;
        p.fill_rect(0, 0, W, H, [16, 16, 32, 255]);
        for (b, x, y) in BOXES {
            let c = if self.pad.held(b) { [255, 208, 64, 255] } else { [64, 64, 96, 255] };
            p.fill_rect(x - 8, y - 6, 16, 12, c);
        }
        for (stick, cx) in [(self.pad.left, 110), (self.pad.right, 190)] {
            p.fill_rect(cx - 20, 140, 40, 40, [40, 40, 64, 255]);
            let (dx, dy) = stick.offset();
            let (x, y) = ((cx as i32 + dx * 18 / 128) as u32, (160 + dy * 18 / 128) as u32);
            p.fill_rect(x - 2, y - 2, 4, 4, [255, 255, 255, 255]);
        }
        // L2 and R2's analog depth as bars.
        for (depth, x) in [(self.pad.l2, 10), (self.pad.r2, 304)] {
            let h = u32::from(depth) * 40 / 255;
            p.fill_rect(x, 60 - h, 6, h, [96, 200, 96, 255]);
        }
    }
}

impl Demo {
    /// Answers one console command.
    fn command(&mut self, t: &mut Tick, line: &str) {
        t.log_command(line);
        let reply = match line.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["frame"] => format!("frame {}", t.frame),
            ["rumble", "on"] => {
                self.rumble = true;
                "rumble on".into()
            }
            ["rumble", "off"] => {
                self.rumble = false;
                "rumble off".into()
            }
            _ => format!("unknown command: {line} (try help)"),
        };
        self.console.say(reply);
    }
}

impl Game for Demo {
    fn event(&mut self, event: &rrt::winit::event::WindowEvent) -> bool {
        self.console.event(event)
    }

    fn tick(&mut self, t: &mut Tick) {
        self.frame = t.frame;
        for line in std::mem::take(&mut t.commands) {
            self.console.run(&line);
        }
        for line in self.console.take_commands() {
            self.command(t, &line);
        }
        if self.console.open {
            // The keyboard is the console's while it is open.
            return;
        }
        self.pad = t.pad;
        if t.pressed().contains(Buttons::SELECT) {
            t.exit();
        }
        if let Ok(mut tone) = self.tone.lock() {
            tone.on = t.pad.held(Buttons::CROSS);
        }
        let on = t.pad.held(Buttons::CIRCLE) || self.rumble;
        t.rumble(Motors { small: on, large: if on { 0xc0 } else { 0 } });
        if t.pressed() != Buttons::NONE {
            t.set_title(format!("rrt-demo - {}", t.pad.buttons.names().collect::<Vec<_>>().join(" ")));
        }
    }

    fn draw(&mut self, d: &mut Draw<'_>) {
        self.paint();
        let overlay = self.console.overlay(d.width, d.height);
        d.present_picture(&self.picture, overlay.as_ref());
    }
}

/// The smallest whole game on retro_rt: a pad tester.
#[derive(rrt::Args)]
struct Cli {
    /// The tone's pitch, in hertz.
    #[arg(default = "440")]
    tone: f32,
    #[arg(flatten)]
    app: AppArgs,
}

#[rrt::main(title = "rrt-demo", size = (960, 720))]
fn main() -> WithArgs<Demo> {
    let cli = Cli::parse_env(env!("CARGO_PKG_VERSION"));
    // A headless shot plays no sound.
    let demo = Demo::new(cli.app.shot.is_none(), cli.tone);
    WithArgs(demo, cli.app)
}
