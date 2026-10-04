//! `headless` runs a game's real init, tick and draw. Skipped, with a note,
//! on a machine with no GPU adapter.

use rrt_app::{Config, Draw, Game, Init, Tick, headless};
use rrt_gpu::{Aspect, Filter, Picture};

/// Counts its calls and draws a colour that depends on how many ticks ran.
#[derive(Default)]
struct Counter {
    inits: u32,
    ticks: u64,
    drew: bool,
}

impl Game for Counter {
    fn init(&mut self, ctx: &mut Init<'_>) {
        assert!(ctx.window.is_none(), "headless has no window");
        self.inits += 1;
    }

    fn tick(&mut self, t: &mut Tick) {
        assert_eq!(t.frame, self.ticks, "frames count from 0, one a tick");
        assert_eq!(t.dt, std::time::Duration::from_secs_f64(1.0 / 50.0), "the fixed step is 1 / hz");
        assert_eq!(t.time(), t.dt * t.frame as u32);
        assert!(t.pressed().is_empty(), "the pad is at rest");
        self.ticks += 1;
    }

    fn draw(&mut self, d: &mut Draw<'_>) {
        self.drew = true;
        let v = (self.ticks * 10) as u8;
        d.present_picture(&Picture::filled(2, 2, [v, 0, 0, 255]), None);
    }
}

#[test]
fn headless_runs_init_the_ticks_and_one_draw() {
    let mut game = Counter::default();
    let config = Config::default().hz(50.0).aspect(Aspect::Stretch).filter(Filter::Nearest);
    let picture = match headless(&mut game, &config, 5, 8, 6) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("no GPU adapter, skipping: {e}");
            return;
        }
    };
    assert_eq!((game.inits, game.ticks, game.drew), (1, 5, true));
    assert_eq!((picture.width, picture.height), (8, 6));
    assert_eq!(picture.get(4, 3), Some([50, 0, 0, 255]));
}

/// A game that asks to exit stops ticking there.
#[test]
fn exit_stops_the_headless_ticks() {
    struct Quits(u32);
    impl Game for Quits {
        fn tick(&mut self, t: &mut Tick) {
            self.0 += 1;
            if self.0 == 3 {
                t.exit();
            }
        }
        fn draw(&mut self, _: &mut Draw<'_>) {}
    }
    let mut game = Quits(0);
    if headless(&mut game, &Config::default(), 100, 4, 4).is_err() {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    assert_eq!(game.0, 3);
}

/// Work recorded in `Draw::encoder` runs in call order with the present
/// helpers: a clear after a present wins, a clear before it is covered.
#[test]
fn draw_work_runs_in_call_order() {
    use rrt_gpu::wgpu;

    struct Order {
        clear_last: bool,
    }

    fn clear_blue(d: &mut Draw<'_>) {
        let view = d.target;
        let blue = wgpu::Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
        let _pass = d.encoder().begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(blue), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    impl Game for Order {
        fn tick(&mut self, _: &mut Tick) {}
        fn draw(&mut self, d: &mut Draw<'_>) {
            let green = Picture::filled(2, 2, [0, 255, 0, 255]);
            if self.clear_last {
                d.present_picture(&green, None);
                clear_blue(d);
            } else {
                clear_blue(d);
                d.present_picture(&green, None);
            }
        }
    }

    let config = Config::default().aspect(Aspect::Stretch).filter(Filter::Nearest);
    for (clear_last, want) in [(true, [0, 0, 255, 255]), (false, [0, 255, 0, 255])] {
        let Ok(p) = headless(&mut Order { clear_last }, &config, 0, 4, 4) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        assert_eq!(p.get(1, 1), Some(want), "clear last: {clear_last}");
    }
}

/// `Config::script` drives the headless pad: presses on their frames, with
/// edges against the frame before.
#[test]
fn a_script_drives_the_headless_pad() {
    struct Record(Vec<(u64, Buttons, Buttons)>);
    impl Game for Record {
        fn tick(&mut self, t: &mut Tick) {
            if !t.pad.buttons.is_empty() {
                self.0.push((t.frame, t.pad.buttons, t.pressed()));
            }
        }
        fn draw(&mut self, _: &mut Draw<'_>) {}
    }
    use rrt_input::Buttons;
    let config = Config::default().script(rrt_input::Script::parse("2:x:2,3:start").unwrap());
    let mut game = Record(Vec::new());
    if headless(&mut game, &config, 6, 4, 4).is_err() {
        eprintln!("no GPU adapter, skipping");
        return;
    }
    assert_eq!(game.0, [(2, Buttons::CROSS, Buttons::CROSS), (3, Buttons::CROSS | Buttons::START, Buttons::START)]);
}
