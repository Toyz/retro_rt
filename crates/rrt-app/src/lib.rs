//! The engine loop.
//!
//! A game implements [`Game`]: [`Game::init`] once the GPU exists,
//! [`Game::tick`] at the console's fixed rate with that frame's pad,
//! [`Game::draw`] once a displayed frame. [`run`] opens the window and the
//! GPU, feeds the keyboard and the gamepads to the pad
//! ([`rrt_input::Input`]), keeps the fixed rate with a [`Clock`] (catching up
//! at most [`Config::max_catch_up`] ticks), and presents. [`headless`] runs
//! the same game with no window and returns the picture, for `--shot` and
//! tests. [`launch`] is what `#[rrt::main]` expands to: logging, the game,
//! then [`run`].
//!
//! The loop owns no game state: what the game is, how it reads the pad, what
//! a tick does, is the game's. See `docs/crates/rrt-app.md`.

pub mod clock;
pub mod config;

pub use clock::{Clock, rate};
pub use config::Config;
pub use rrt_gpu::{Gpu, Picture, Presenter, Target};
pub use rrt_input::{Buttons, Input, Motors, Pad};
pub use tracing;
pub use winit;

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rrt_gpu::wgpu;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

/// A game the loop runs. Only [`Game::tick`] and [`Game::draw`] are required.
pub trait Game: 'static {
    /// Once, when the GPU exists and before the first tick: make pipelines
    /// and upload what does not change. With a window this runs when the
    /// window opens; headless, before anything else.
    fn init(&mut self, ctx: &mut Init<'_>) {
        let _ = ctx;
    }

    /// One frame of the game at [`Config::hz`], with that frame's pad.
    fn tick(&mut self, ctx: &mut Tick);

    /// Draws the game's latest state. Hand the picture to the presenter
    /// ([`Draw::present_picture`], [`Draw::present_target`]) or submit
    /// command buffers of your own ([`Draw::submit`]). Called once for each
    /// frame shown, whether 0, 1 or several ticks ran before it.
    fn draw(&mut self, ctx: &mut Draw<'_>);

    /// A window event, before the loop's own handling. Return true to take
    /// it: a taken key does not reach the pad, a taken Escape does not quit.
    /// For a console, a text box, mouse look.
    fn event(&mut self, event: &WindowEvent) -> bool {
        let _ = event;
        false
    }

    /// The window is closing or [`Tick::exit`] was asked: save what needs
    /// saving.
    fn exit(&mut self) {}
}

/// What [`Game::init`] gets.
pub struct Init<'a> {
    /// The GPU.
    pub gpu: &'a Gpu,
    /// The format [`Draw::target`] is in: the surface's (usually sRGB), or
    /// [`Target::FORMAT`] headless.
    pub format: wgpu::TextureFormat,
    /// The format [`Draw::plain`] is in: `format` without sRGB, for drawing
    /// display-encoded colours straight to the screen.
    pub plain_format: wgpu::TextureFormat,
    /// The window; None headless. Clone the `Arc` to keep it: for mouse
    /// capture, the cursor, a fullscreen toggle of the game's own.
    pub window: Option<&'a Arc<Window>>,
}

/// What [`Game::tick`] gets: the pad, and requests back to the loop.
pub struct Tick {
    /// This frame's pad.
    pub pad: Pad,
    /// The last frame's pad, for edges: `pad.pressed(&previous)`.
    pub previous: Pad,
    /// Ticks run before this one.
    pub frame: u64,
    /// The fixed step: one tick's share of time, `1 / Config::hz`. The same
    /// every tick, however late the tick runs, so a game's physics stays
    /// deterministic. Game time is `dt * frame`.
    pub dt: Duration,
    /// What the gamepad's describe line read this tick: empty unless
    /// [`Config::pad_log`] is on.
    pub pad_line: String,
    exit: bool,
    title: Option<String>,
    motors: Option<Motors>,
}

impl Tick {
    fn new(pad: Pad, previous: Pad, frame: u64, dt: Duration) -> Tick {
        Tick { pad, previous, frame, dt, pad_line: String::new(), exit: false, title: None, motors: None }
    }

    /// Game time at the start of this tick: `dt * frame`.
    pub fn time(&self) -> Duration {
        self.dt.mul_f64(self.frame as f64)
    }

    /// The buttons pressed this frame.
    pub fn pressed(&self) -> Buttons {
        self.pad.pressed(&self.previous)
    }

    /// Ends the loop after this tick.
    pub fn exit(&mut self) {
        self.exit = true;
    }

    /// Sets the window's title.
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = Some(title.into());
    }

    /// Runs the pad's motors ([`Input::rumble`]); they keep running until set
    /// again.
    pub fn rumble(&mut self, motors: Motors) {
        self.motors = Some(motors);
    }
}

/// What [`Game::draw`] gets: where to draw and the presenter to draw with.
pub struct Draw<'a> {
    /// The GPU.
    pub gpu: &'a Gpu,
    /// The presenter: letterboxes a picture into [`Draw::target`].
    pub presenter: &'a mut Presenter,
    /// The frame to draw into, in [`Init::format`].
    pub target: &'a wgpu::TextureView,
    /// The same frame viewed in [`Init::plain_format`].
    pub plain: &'a wgpu::TextureView,
    /// The frame's size in pixels.
    pub width: u32,
    /// See [`Draw::width`].
    pub height: u32,
    commands: Vec<wgpu::CommandBuffer>,
    encoder: Option<wgpu::CommandEncoder>,
}

impl<'a> Draw<'a> {
    fn new(
        gpu: &'a Gpu,
        presenter: &'a mut Presenter,
        target: &'a wgpu::TextureView,
        plain: &'a wgpu::TextureView,
        (width, height): (u32, u32),
    ) -> Draw<'a> {
        Draw { gpu, presenter, target, plain, width, height, commands: Vec::new(), encoder: None }
    }

    /// A command encoder for this frame, made on first use. What is recorded
    /// in it runs in call order with [`Draw::submit`] and the present
    /// helpers: each of those first closes the open encoder, and the next
    /// call here opens a fresh one.
    pub fn encoder(&mut self) -> &mut wgpu::CommandEncoder {
        let device = &self.gpu.device;
        self.encoder.get_or_insert_with(|| device.create_command_encoder(&Default::default()))
    }

    /// Closes the open encoder, if any, into the queue of command buffers.
    fn flush(&mut self) {
        if let Some(e) = self.encoder.take() {
            self.commands.push(e.finish());
        }
    }

    /// Every command buffer of the frame, in call order.
    fn finish(mut self) -> Vec<wgpu::CommandBuffer> {
        self.flush();
        self.commands
    }

    /// Queues a command buffer; the loop submits them in order, then shows
    /// the frame.
    pub fn submit(&mut self, commands: wgpu::CommandBuffer) {
        self.flush();
        self.commands.push(commands);
    }

    /// Shows `picture` through the presenter, `overlay` over it at the
    /// window's resolution.
    pub fn present_picture(&mut self, picture: &Picture, overlay: Option<&Picture>) {
        self.flush();
        let c = self.presenter.present_picture(self.gpu, picture, self.target, self.width, self.height, overlay);
        self.commands.push(c);
    }

    /// Shows what a renderer drew into `target` through the presenter.
    pub fn present_target(&mut self, target: &Target, overlay: Option<&Picture>) {
        self.flush();
        let c = self.presenter.present_view(
            self.gpu,
            &target.view,
            (target.width, target.height),
            self.target,
            self.width,
            self.height,
            overlay,
        );
        self.commands.push(c);
    }
}

/// Why the loop could not start or stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<rrt_gpu::Error> for Error {
    fn from(e: rrt_gpu::Error) -> Error {
        Error(e.0)
    }
}

/// What a game's constructor may return: the game, or a `Result` whose
/// error the loop logs before exiting with status 1.
pub trait IntoGame {
    /// The game.
    type Game: Game;
    /// The game, or why there is none.
    fn into_game(self) -> Result<Self::Game, String>;
}

impl<G: Game> IntoGame for G {
    type Game = G;
    fn into_game(self) -> Result<G, String> {
        Ok(self)
    }
}

impl<G: Game, E: fmt::Display> IntoGame for Result<G, E> {
    type Game = G;
    fn into_game(self) -> Result<G, String> {
        self.map_err(|e| e.to_string())
    }
}

/// Starts logging at `filter` (`RUST_LOG` wins when set): `info`,
/// `my_game=debug,wgpu=warn`. A second call does nothing.
pub fn init_logging(filter: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// A whole program: logging at [`Config::log`], the game from `make`, then
/// [`run`]. A failure is logged and the process exits with status 1. This
/// is what `#[rrt::main]` expands to.
pub fn launch<R: IntoGame>(config: Config, make: impl FnOnce() -> R) {
    init_logging(&config.log);
    let result = make().into_game().and_then(|game| run(config, game).map_err(|e| e.0));
    if let Err(e) = result {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}

/// Runs `game` in a window until it closes: the pad read, [`Game::tick`] at
/// the fixed rate, [`Game::draw`], present.
pub fn run<G: Game>(config: Config, game: G) -> Result<(), Error> {
    let event_loop = EventLoop::new().map_err(|e| Error(e.to_string()))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut input = Input::new();
    input.keyboard.map = config.keymap.clone();
    for line in input.gamepads() {
        tracing::info!("{line}");
    }
    let mut app = App {
        clock: Clock::new(config.hz, config.max_catch_up),
        config,
        game,
        input,
        display: event_loop.owned_display_handle(),
        win: None,
        pad: Pad::default(),
        frame: 0,
        presented: Instant::now(),
        error: None,
        exiting: false,
    };
    event_loop.run_app(&mut app).map_err(|e| Error(e.to_string()))?;
    app.error.map_or(Ok(()), Err)
}

/// Runs `game` with no window: [`Game::init`], `ticks` ticks with the pad
/// driven by [`Config::script`] (at rest without one), one [`Game::draw`] into
/// a `width` x `height` [`Target`], and that frame read back. For `--shot`,
/// golden-image tests and CI.
pub fn headless<G: Game>(game: &mut G, config: &Config, ticks: u64, width: u32, height: u32) -> Result<Picture, Error> {
    let gpu = Gpu::headless()?;
    let format = Target::FORMAT;
    game.init(&mut Init { gpu: &gpu, format, plain_format: format, window: None });
    let dt = Duration::from_secs_f64(1.0 / config.hz);
    let mut previous = Pad::default();
    for frame in 0..ticks {
        let buttons = config.script.as_ref().map_or(Buttons::NONE, |s| s.buttons_at(frame));
        let pad = Pad { buttons, ..Pad::default() };
        let mut t = Tick::new(pad, previous, frame, dt);
        previous = pad;
        game.tick(&mut t);
        if t.exit {
            break;
        }
    }
    let target = Target::new(&gpu.device, width, height);
    let mut presenter = Presenter::new(&gpu.device, format);
    presenter.aspect = config.aspect;
    presenter.filter = config.filter;
    let mut draw = Draw::new(&gpu, &mut presenter, &target.view, &target.view, (target.width, target.height));
    game.draw(&mut draw);
    gpu.queue.submit(draw.finish());
    Ok(target.read_back(&gpu))
}

/// The window and everything drawn to it, once open.
struct Shown {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    presenter: Presenter,
}

struct App<G: Game> {
    config: Config,
    game: G,
    input: Input,
    clock: Clock,
    display: winit::event_loop::OwnedDisplayHandle,
    win: Option<Shown>,
    pad: Pad,
    frame: u64,
    presented: Instant,
    error: Option<Error>,
    exiting: bool,
}

impl<G: Game> App<G> {
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<Shown, Error> {
        let attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(winit::dpi::LogicalSize::new(self.config.width, self.config.height))
            .with_fullscreen(self.config.fullscreen.then_some(Fullscreen::Borderless(None)));
        let window = Arc::new(event_loop.create_window(attrs).map_err(|e| Error(e.to_string()))?);
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(self.display.clone())));
        let surface = instance.create_surface(window.clone()).map_err(|e| Error(e.to_string()))?;
        let gpu = Gpu::for_surface(instance, &surface)?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| Error("the window's surface is not supported by this adapter".into()))?;
        let caps = surface.get_capabilities(&gpu.adapter);
        if let Some(f) = caps.formats.iter().find(|f| f.is_srgb()) {
            config.format = *f;
        }
        config.present_mode = present_mode(self.config.vsync);
        config.view_formats = vec![config.format.remove_srgb_suffix()];
        surface.configure(&gpu.device, &config);
        let mut presenter = Presenter::new(&gpu.device, config.format);
        presenter.aspect = self.config.aspect;
        presenter.filter = self.config.filter;
        Ok(Shown { window, surface, config, gpu, presenter })
    }

    /// The ticks due since the last frame shown.
    fn tick(&mut self) {
        let steps = self.clock.due(Instant::now());
        for _ in 0..steps {
            let line = if self.config.pad_log { self.input.describe() } else { String::new() };
            let mut pad = self.input.read();
            if let Some(script) = &self.config.script {
                pad.buttons |= script.buttons_at(self.frame);
            }
            let mut t = Tick::new(pad, self.pad, self.frame, self.clock.period());
            t.pad_line = line;
            self.game.tick(&mut t);
            self.pad = pad;
            self.frame += 1;
            if let Some(m) = t.motors {
                self.input.rumble(m);
            }
            if let (Some(title), Some(w)) = (&t.title, &self.win) {
                w.window.set_title(title);
            }
            if t.exit {
                self.exiting = true;
                break;
            }
        }
    }

    fn draw(&mut self) {
        let Some(w) = &mut self.win else { return };
        let frame = match w.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                w.surface.configure(&w.gpu.device, &w.config);
                return;
            }
            _ => return,
        };
        let target = frame.texture.create_view(&Default::default());
        let plain = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(w.config.format.remove_srgb_suffix()),
            ..Default::default()
        });
        let size = (w.config.width, w.config.height);
        let mut draw = Draw::new(&w.gpu, &mut w.presenter, &target, &plain, size);
        self.game.draw(&mut draw);
        w.gpu.queue.submit(draw.finish());
        w.window.pre_present_notify();
        w.gpu.queue.present(frame);
    }
}

impl<G: Game> ApplicationHandler for App<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.win.is_some() {
            return;
        }
        match self.open(event_loop) {
            Ok(w) => {
                let plain_format = w.config.format.remove_srgb_suffix();
                self.game.init(&mut Init {
                    gpu: &w.gpu,
                    format: w.config.format,
                    plain_format,
                    window: Some(&w.window),
                });
                w.window.request_redraw();
                self.win = Some(w);
                self.clock.reset(Instant::now());
            }
            Err(e) => {
                self.error = Some(Error(format!("no window: {e}")));
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.game.event(&event) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => self.exiting = true,
            WindowEvent::Resized(size) => {
                if let Some(w) = &mut self.win
                    && size.width > 0
                    && size.height > 0
                {
                    w.config.width = size.width;
                    w.config.height = size.height;
                    w.surface.configure(&w.gpu.device, &w.config);
                }
            }
            WindowEvent::Focused(false) => self.input.keyboard.release_all(),
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                match key_action(&self.config, event.physical_key, pressed, event.repeat) {
                    KeyAction::Quit => self.exiting = true,
                    KeyAction::ToggleFullscreen => {
                        if let Some(w) = &self.win {
                            let on = w.window.fullscreen().is_none();
                            w.window.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
                        }
                    }
                    KeyAction::Pad => {}
                }
                self.input.keyboard.key(event.physical_key, pressed);
            }
            WindowEvent::RedrawRequested => {
                self.tick();
                self.draw();
                self.presented = Instant::now();
            }
            _ => {}
        }
        if self.exiting {
            self.game.exit();
            event_loop.exit();
        }
    }

    /// The next frame: at once, or when [`Config::fps_cap`]'s time comes.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(cap) = self.config.fps_cap.filter(|c| *c > 0) {
            let next = self.presented + Duration::from_secs_f64(1.0 / f64::from(cap));
            if Instant::now() < next {
                event_loop.set_control_flow(ControlFlow::WaitUntil(next));
                return;
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
        if let Some(w) = &self.win {
            w.window.request_redraw();
        }
    }
}

/// What a key does to the loop, beyond reaching the pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyAction {
    /// Escape with [`Config::escape_quits`], down or up.
    Quit,
    /// [`Config::fullscreen_key`] going down (not a repeat).
    ToggleFullscreen,
    /// Nothing; the pad sees it.
    Pad,
}

fn key_action(config: &Config, key: PhysicalKey, pressed: bool, repeat: bool) -> KeyAction {
    if config.escape_quits && key == PhysicalKey::Code(KeyCode::Escape) {
        KeyAction::Quit
    } else if pressed && !repeat && config.fullscreen_key.is_some_and(|k| key == PhysicalKey::Code(k)) {
        KeyAction::ToggleFullscreen
    } else {
        KeyAction::Pad
    }
}

/// The surface's present mode: the display's refresh waited for, or not.
fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_quits_and_f11_toggles_by_default() {
        let c = Config::default();
        let key = |k| PhysicalKey::Code(k);
        assert_eq!(key_action(&c, key(KeyCode::Escape), true, false), KeyAction::Quit);
        assert_eq!(key_action(&c, key(KeyCode::F11), true, false), KeyAction::ToggleFullscreen);
        assert_eq!(key_action(&c, key(KeyCode::F11), true, true), KeyAction::Pad, "a held key toggles once");
        assert_eq!(key_action(&c, key(KeyCode::F11), false, false), KeyAction::Pad, "the release does nothing");
        assert_eq!(key_action(&c, key(KeyCode::KeyZ), true, false), KeyAction::Pad);
    }

    #[test]
    fn both_keys_can_be_turned_off() {
        let c = Config::default().escape_quits(false).fullscreen_key(None);
        assert_eq!(key_action(&c, PhysicalKey::Code(KeyCode::Escape), true, false), KeyAction::Pad);
        assert_eq!(key_action(&c, PhysicalKey::Code(KeyCode::F11), true, false), KeyAction::Pad);
    }

    #[test]
    fn present_mode_follows_vsync() {
        assert_eq!(present_mode(true), wgpu::PresentMode::AutoVsync);
        assert_eq!(present_mode(false), wgpu::PresentMode::AutoNoVsync);
    }

    #[test]
    fn config_builders_set_their_fields() {
        let c = Config::default()
            .title("t")
            .size((320, 240))
            .hz(rate::PAL)
            .max_catch_up(2)
            .vsync(false)
            .fps_cap(144)
            .log("debug")
            .fullscreen(true)
            .pad_log(true)
            .script(rrt_input::Script::parse("5:x").unwrap());
        assert_eq!((c.title.as_str(), c.width, c.height, c.hz), ("t", 320, 240, 50.0));
        assert_eq!((c.max_catch_up, c.vsync, c.fps_cap, c.log.as_str()), (2, false, Some(144), "debug"));
        assert!(c.fullscreen && c.pad_log && c.script.is_some());
        assert_eq!(Config::default().fps_cap(0).fps_cap, None, "0 is no cap");
    }

    #[test]
    fn into_game_takes_a_game_or_a_result() {
        struct G;
        impl Game for G {
            fn tick(&mut self, _: &mut Tick) {}
            fn draw(&mut self, _: &mut Draw<'_>) {}
        }
        assert!(G.into_game().is_ok());
        assert!(Ok::<G, String>(G).into_game().is_ok());
        assert_eq!(Err::<G, &str>("no disc").into_game().err().as_deref(), Some("no disc"));
    }
}
