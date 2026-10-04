//! How [`crate::run`] sets up: the window, the rate, the pad, the picture.
//! Every field has a builder method of the same name, which is what
//! `#[rrt::main(name = value, ...)]` calls.

use rrt_gpu::{Aspect, Filter};
use std::path::PathBuf;

use rrt_input::{KeyMap, PadLog, Script};
use winit::keyboard::KeyCode;

use crate::clock::rate;

/// The loop's settings. `Config::default()` is a 960 x 720 window ticking
/// at NTSC's 59.94 Hz, vsync on, Escape quits, F11 toggles fullscreen, 4:3
/// sharp bilinear, no script, no replay, no recording.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The window's title until the game sets one.
    pub title: String,
    /// The window's first inner width, logical pixels.
    pub width: u32,
    /// The window's first inner height, logical pixels.
    pub height: u32,
    /// Ticks a second ([`rate::NTSC`], [`rate::PAL`], or the game's own).
    pub hz: f64,
    /// Ticks run back to back at most, when behind, before the rest are
    /// dropped.
    pub max_catch_up: u32,
    /// Wait for the display's refresh.
    pub vsync: bool,
    /// Frames shown a second at most; None for as many as come.
    pub fps_cap: Option<u32>,
    /// Escape closes the window (unless [`crate::Game::event`] takes it).
    pub escape_quits: bool,
    /// The log filter when `RUST_LOG` is unset.
    pub log: String,
    /// The keyboard's layout as a pad.
    pub keymap: KeyMap,
    /// The shape the presenter shows the picture at.
    pub aspect: Aspect,
    /// How the presenter scales it.
    pub filter: Filter,
    /// Fill [`crate::Tick::pad_line`] with the gamepad's raw values every
    /// tick, for a pad log.
    pub pad_log: bool,
    /// Open the window borderless fullscreen on the current monitor.
    pub fullscreen: bool,
    /// The key that toggles borderless fullscreen; None for none. F11 by
    /// default. [`crate::Game::event`] can take it first.
    pub fullscreen_key: Option<KeyCode>,
    /// Presses held on given frames, added to the live pad by
    /// [`crate::run`] and the whole pad in [`crate::headless`]: replays,
    /// attract modes, scripted tests.
    pub script: Option<Script>,
    /// A recorded session to play back: on each frame the log has a pad
    /// for, that pad replaces the live one entirely, and the commands it ran
    /// arrive in [`crate::Tick::commands`]. Frames past its end take the
    /// live pad again.
    pub replay: Option<PadLog>,
    /// Where to write a [`PadLog`] of the session (every tick's pad, and the
    /// commands replayed or logged with [`crate::Tick::log_command`]) when
    /// the loop ends.
    pub record: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            title: "retro_rt".into(),
            width: 960,
            height: 720,
            hz: rate::NTSC,
            max_catch_up: 4,
            vsync: true,
            fps_cap: None,
            escape_quits: true,
            log: "info".into(),
            keymap: KeyMap::default(),
            aspect: Aspect::default(),
            filter: Filter::default(),
            pad_log: false,
            fullscreen: false,
            fullscreen_key: Some(KeyCode::F11),
            script: None,
            replay: None,
            record: None,
        }
    }
}

impl Config {
    /// Sets [`Config::title`].
    pub fn title(mut self, title: impl Into<String>) -> Config {
        self.title = title.into();
        self
    }

    /// Sets [`Config::width`] and [`Config::height`].
    pub fn size(mut self, (width, height): (u32, u32)) -> Config {
        (self.width, self.height) = (width, height);
        self
    }

    /// Sets [`Config::width`].
    pub fn width(mut self, width: u32) -> Config {
        self.width = width;
        self
    }

    /// Sets [`Config::height`].
    pub fn height(mut self, height: u32) -> Config {
        self.height = height;
        self
    }

    /// Sets [`Config::hz`].
    pub fn hz(mut self, hz: f64) -> Config {
        self.hz = hz;
        self
    }

    /// Sets [`Config::max_catch_up`].
    pub fn max_catch_up(mut self, ticks: u32) -> Config {
        self.max_catch_up = ticks;
        self
    }

    /// Sets [`Config::vsync`].
    pub fn vsync(mut self, on: bool) -> Config {
        self.vsync = on;
        self
    }

    /// Sets [`Config::fps_cap`]; 0 is no cap.
    pub fn fps_cap(mut self, fps: u32) -> Config {
        self.fps_cap = (fps > 0).then_some(fps);
        self
    }

    /// Sets [`Config::escape_quits`].
    pub fn escape_quits(mut self, on: bool) -> Config {
        self.escape_quits = on;
        self
    }

    /// Sets [`Config::log`].
    pub fn log(mut self, filter: impl Into<String>) -> Config {
        self.log = filter.into();
        self
    }

    /// Sets [`Config::keymap`].
    pub fn keymap(mut self, map: KeyMap) -> Config {
        self.keymap = map;
        self
    }

    /// Sets [`Config::aspect`].
    pub fn aspect(mut self, aspect: Aspect) -> Config {
        self.aspect = aspect;
        self
    }

    /// Sets [`Config::filter`].
    pub fn filter(mut self, filter: Filter) -> Config {
        self.filter = filter;
        self
    }

    /// Sets [`Config::fullscreen`].
    pub fn fullscreen(mut self, on: bool) -> Config {
        self.fullscreen = on;
        self
    }

    /// Sets [`Config::fullscreen_key`].
    pub fn fullscreen_key(mut self, key: Option<KeyCode>) -> Config {
        self.fullscreen_key = key;
        self
    }

    /// Sets [`Config::script`].
    pub fn script(mut self, script: Script) -> Config {
        self.script = Some(script);
        self
    }

    /// Sets [`Config::replay`].
    pub fn replay(mut self, log: PadLog) -> Config {
        self.replay = Some(log);
        self
    }

    /// Sets [`Config::record`].
    pub fn record(mut self, path: impl Into<PathBuf>) -> Config {
        self.record = Some(path.into());
        self
    }

    /// Sets [`Config::pad_log`].
    pub fn pad_log(mut self, on: bool) -> Config {
        self.pad_log = on;
        self
    }
}
