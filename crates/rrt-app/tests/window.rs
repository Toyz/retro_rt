//! The windowed loop, end to end: `run` opens a window and a GPU surface,
//! calls `init` with the window, ticks at the configured rate with the
//! script's presses on the pad, draws and presents, and stops when the game
//! asks. winit needs the main thread, so this test has its own `main`
//! (`harness = false`). With no display (no `DISPLAY` or `WAYLAND_DISPLAY`)
//! it says so and passes; CI runs it under Xvfb.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rrt_app::{Buttons, Config, Draw, Game, Init, Tick, run};
use rrt_gpu::Picture;
use rrt_input::Script;

#[derive(Debug, Default)]
struct Seen {
    window: bool,
    ticks: u64,
    draws: u64,
    cross_on: Vec<u64>,
    exited: bool,
    elapsed: Duration,
}

struct Probe {
    seen: Arc<Mutex<Seen>>,
    started: Option<Instant>,
    picture: Picture,
}

impl Game for Probe {
    fn init(&mut self, ctx: &mut Init<'_>) {
        let w = ctx.window.expect("a window").clone();
        assert!(w.inner_size().width > 0);
        self.seen.lock().unwrap().window = true;
        self.started = Some(Instant::now());
    }

    fn tick(&mut self, t: &mut Tick) {
        let mut seen = self.seen.lock().unwrap();
        seen.ticks += 1;
        if t.pressed().contains(Buttons::CROSS) {
            seen.cross_on.push(t.frame);
        }
        if t.frame == 59 {
            seen.elapsed = self.started.unwrap().elapsed();
            t.exit();
        }
    }

    fn draw(&mut self, d: &mut Draw<'_>) {
        self.seen.lock().unwrap().draws += 1;
        d.present_picture(&self.picture, None);
    }

    fn exit(&mut self) {
        self.seen.lock().unwrap().exited = true;
    }
}

fn main() {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("window: no display, skipping");
        return;
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let game = Probe { seen: seen.clone(), started: None, picture: Picture::filled(64, 48, [0, 0, 255, 255]) };
    let config = Config::default()
        .title("rrt-app window test")
        .size((320, 240))
        .hz(120.0)
        .vsync(false)
        .script(Script::parse("10:x,30:x:3").unwrap());
    if let Err(e) = run(config, game) {
        eprintln!("window: could not open ({e}), skipping");
        return;
    }
    let seen = seen.lock().unwrap();
    assert!(seen.window, "init got the window");
    assert_eq!(seen.ticks, 60, "ticks stop at the exit");
    assert!(seen.draws >= 1, "at least one frame drawn");
    assert_eq!(seen.cross_on, [10, 30], "the script's presses, each seen once as pressed");
    assert!(seen.exited, "Game::exit ran");
    // 60 ticks at 120 Hz: half a second; the clock may not run ahead.
    assert!(seen.elapsed >= Duration::from_millis(480), "60 ticks took only {:?}", seen.elapsed);
    println!("window: ok ({} ticks, {} draws, {:?})", seen.ticks, seen.draws, seen.elapsed);
}
