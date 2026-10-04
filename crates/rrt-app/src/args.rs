//! The command-line flags every game on the loop takes: screenshots, pad
//! scripts, replay and recording, the window. A game flattens them into its
//! own arguments and hands them back with [`WithArgs`]; [`crate::launch`]
//! applies them.

use std::path::PathBuf;

use rrt_kit::cli::{Args, Error, Kind, Matches, Size, Spec};

use crate::Config;

/// The standard flags, as `#[arg(flatten)] app: rrt::app::AppArgs` in a
/// game's `#[derive(rrt::Args)]` struct.
///
/// | flag | effect |
/// | --- | --- |
/// | `--shot PATH` | run headless, write the frame as PNG, exit |
/// | `--frames N` | ticks before the shot (default 1) |
/// | `--every N` | with `--shot`, also a picture every N ticks (`PATH-FRAME.png`) |
/// | `--size WxH` | the shot's size (default: the window's) |
/// | `--press SCRIPT` | scripted presses, `FRAME:BUTTONS[:FRAMES],...`; repeatable |
/// | `--replay PATH` | play a recorded pad log |
/// | `--record PATH` (or `--pad-log`) | write a pad log when the loop ends |
/// | `--fullscreen` | open borderless fullscreen |
/// | `--no-vsync` | do not wait for the display |
/// | `--fps-cap N` | frames shown a second at most |
/// | `--log FILTER` | the log filter (`RUST_LOG` wins) |
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppArgs {
    /// `--shot`: where to write a headless frame.
    pub shot: Option<PathBuf>,
    /// `--frames`: ticks before the shot.
    pub frames: u64,
    /// `--every`: a shot every this many ticks too.
    pub every: Option<u64>,
    /// `--size`: the shot's size.
    pub size: Option<Size>,
    /// `--press`: scripted presses, each in `rrt_input::Script`'s form.
    pub press: Vec<String>,
    /// `--replay`: a pad log to play.
    pub replay: Option<PathBuf>,
    /// `--record`: where to write a pad log.
    pub record: Option<PathBuf>,
    /// `--fullscreen`.
    pub fullscreen: bool,
    /// `--no-vsync`.
    pub no_vsync: bool,
    /// `--fps-cap`.
    pub fps_cap: Option<u32>,
    /// `--log`.
    pub log: Option<String>,
}

impl Args for AppArgs {
    fn specs() -> Vec<Spec> {
        vec![
            Spec {
                value_name: "PATH",
                help: "Run headless, write the frame as PNG, and exit",
                ..Spec::new("shot", Kind::Value)
            },
            Spec {
                value_name: "N",
                help: "Ticks to run before the shot",
                default: Some("1"),
                ..Spec::new("frames", Kind::Value)
            },
            Spec {
                value_name: "N",
                help: "With --shot, also a picture every N ticks (PATH-FRAME.png)",
                ..Spec::new("every", Kind::Value)
            },
            Spec {
                value_name: "WxH",
                help: "The shot's size [default: the window's]",
                ..Spec::new("size", Kind::Value)
            },
            Spec {
                value_name: "SCRIPT",
                help: "Scripted presses: FRAME:BUTTONS[:FRAMES],...",
                ..Spec::new("press", Kind::Repeated)
            },
            Spec { value_name: "PATH", help: "Play a recorded pad log", ..Spec::new("replay", Kind::Value) },
            Spec {
                aliases: &["pad-log"],
                value_name: "PATH",
                help: "Write a pad log when the game ends",
                ..Spec::new("record", Kind::Value)
            },
            Spec { help: "Open borderless fullscreen", ..Spec::new("fullscreen", Kind::Flag) },
            Spec { help: "Do not wait for the display's refresh", ..Spec::new("no-vsync", Kind::Flag) },
            Spec { value_name: "N", help: "Frames shown a second at most", ..Spec::new("fps-cap", Kind::Value) },
            Spec { value_name: "FILTER", help: "Log filter (RUST_LOG wins)", ..Spec::new("log", Kind::Value) },
        ]
    }

    fn build(m: &mut Matches) -> Result<AppArgs, Error> {
        Ok(AppArgs {
            shot: m.value("shot")?,
            frames: m.value_or("frames", "1")?,
            every: m.value("every")?,
            size: m.value("size")?,
            press: m.values("press")?,
            replay: m.value("replay")?,
            record: m.value("record")?,
            fullscreen: m.flag("fullscreen"),
            no_vsync: m.flag("no-vsync"),
            fps_cap: m.value("fps-cap")?,
            log: m.value("log")?,
        })
    }
}

impl AppArgs {
    /// `config` with these flags applied: the script parsed, the replay
    /// read, the window settings set. An error says which flag was wrong.
    pub fn apply(&self, mut config: Config) -> Result<Config, String> {
        if !self.press.is_empty() {
            let script = rrt_input::Script::parse(&self.press.join(",")).map_err(|e| format!("--press: {e}"))?;
            config = config.script(script);
        }
        if let Some(path) = &self.replay {
            let text = std::fs::read_to_string(path).map_err(|e| format!("--replay {}: {e}", path.display()))?;
            let log = rrt_input::PadLog::parse(&text).map_err(|e| format!("--replay {}: {e}", path.display()))?;
            config = config.replay(log);
        }
        if let Some(path) = &self.record {
            config = config.record(path);
        }
        if self.fullscreen {
            config = config.fullscreen(true);
        }
        if self.no_vsync {
            config = config.vsync(false);
        }
        if let Some(fps) = self.fps_cap {
            config = config.fps_cap(fps);
        }
        if let Some(log) = &self.log {
            config = config.log(log.clone());
        }
        Ok(config)
    }
}

/// A game and the standard flags it parsed, from a `#[rrt::main]` function:
/// [`crate::launch`] applies the flags before running the game.
pub struct WithArgs<G>(pub G, pub AppArgs);

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<AppArgs, Error> {
        AppArgs::parse_from(s.split_whitespace().map(String::from))
    }

    #[test]
    fn the_standard_flags_parse() {
        let a = parse("--shot a.png --frames 30 --every 10 --size 320x240 --press 1:x --press 5:start:3 --pad-log p.txt --fullscreen --no-vsync --fps-cap 144 --log debug").unwrap();
        assert_eq!(a.shot, Some(PathBuf::from("a.png")));
        assert_eq!((a.frames, a.every, a.size), (30, Some(10), Some(Size(320, 240))));
        assert_eq!(a.press, ["1:x", "5:start:3"]);
        assert_eq!(a.record, Some(PathBuf::from("p.txt")), "--pad-log is --record");
        assert!(a.fullscreen && a.no_vsync);
        assert_eq!((a.fps_cap, a.log.as_deref()), (Some(144), Some("debug")));
        assert_eq!(parse("").unwrap(), AppArgs { frames: 1, ..AppArgs::default() });
        assert!(matches!(parse("--fps-cap fast"), Err(Error::Invalid { .. })));
    }

    #[test]
    fn applying_sets_the_config() {
        let a = parse("--press 1:x,2:o --no-vsync --fps-cap 30 --fullscreen --record r.txt --log warn").unwrap();
        let c = a.apply(Config::default()).unwrap();
        assert_eq!(c.script.unwrap().buttons_at(2), rrt_input::Buttons::CIRCLE);
        assert!(!c.vsync && c.fullscreen);
        assert_eq!((c.fps_cap, c.log.as_str()), (Some(30), "warn"));
        assert_eq!(c.record, Some(PathBuf::from("r.txt")));
        assert!(parse("--press nonsense").unwrap().apply(Config::default()).unwrap_err().contains("--press"));
        let missing = parse("--replay /nonexistent/log.txt").unwrap().apply(Config::default()).unwrap_err();
        assert!(missing.contains("--replay"));
    }
}
