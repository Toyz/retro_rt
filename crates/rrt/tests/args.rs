//! `#[derive(rrt::Args)]`, compiled and run as a game would use it.

use std::path::PathBuf;

use rrt::cli::{Args, Error, Kind, Size};

/// A test program.
///
/// This second paragraph is not in the about.
#[derive(Debug, PartialEq, rrt::Args)]
struct Cli {
    /// The disc image.
    #[arg(positional)]
    disc: PathBuf,
    /// Extra files.
    #[arg(positional)]
    more: Vec<String>,
    /// Start on this track.
    #[arg(short = 't', default = "DESERT1")]
    track: String,
    /// How many laps.
    laps: u32,
    /// Mute the sound.
    #[arg(short = 'm', alias = "silent")]
    mute: bool,
    /// The render scale.
    render_scale: Option<u32>,
    /// Cheats to apply.
    #[arg(long = "cheat", value_name = "CODE")]
    cheats: Vec<String>,
    /// The window size.
    window: Option<Size>,
    #[arg(flatten)]
    app: rrt::app::AppArgs,
}

fn parse(s: &str) -> Result<Cli, Error> {
    Cli::parse_from(s.split_whitespace().map(String::from))
}

#[test]
fn every_field_shape_parses() {
    let c = parse(
        "game.cue a b -t VOLCANO1 --laps 3 -m --render-scale=2 --cheat X --cheat Y --window 640x480 --shot s.png",
    )
    .unwrap();
    assert_eq!(c.disc, PathBuf::from("game.cue"));
    assert_eq!(c.more, ["a", "b"]);
    assert_eq!((c.track.as_str(), c.laps, c.mute), ("VOLCANO1", 3, true));
    assert_eq!((c.render_scale, c.window), (Some(2), Some(Size(640, 480))));
    assert_eq!(c.cheats, ["X", "Y"]);
    assert_eq!(c.app.shot, Some(PathBuf::from("s.png")), "a flattened struct's flags");
    let d = parse("game.cue --laps 1 --silent").unwrap();
    assert_eq!((d.track.as_str(), d.mute, d.more.len()), ("DESERT1", true, 0), "the default and the alias");
    assert_eq!(d.app.frames, 1);
}

#[test]
fn missing_and_wrong_arguments_are_errors() {
    assert_eq!(parse("game.cue").unwrap_err(), Error::Missing("--laps".into()));
    assert_eq!(parse("--laps 1").unwrap_err(), Error::Missing("DISC".into()));
    assert!(matches!(parse("g --laps many"), Err(Error::Invalid { .. })));
    assert!(matches!(parse("g --laps 1 --mute=yes"), Err(Error::Invalid { .. })));
}

#[test]
fn the_specs_and_usage_come_from_the_struct() {
    assert_eq!(Cli::about(), "A test program.");
    let specs = Cli::specs();
    let laps = specs.iter().find(|s| s.name == "laps").unwrap();
    assert!(laps.required && laps.kind == Kind::Value && laps.value_name == "N");
    assert_eq!(laps.help, "How many laps.");
    let track = specs.iter().find(|s| s.name == "track").unwrap();
    assert_eq!((track.short, track.default), (Some('t'), Some("DESERT1")));
    let cheat = specs.iter().find(|s| s.name == "cheat").unwrap();
    assert_eq!((cheat.kind.clone(), cheat.value_name), (Kind::Repeated, "CODE"));
    assert!(specs.iter().any(|s| s.name == "fps-cap"), "AppArgs flattened in");
    let usage = Cli::usage("game");
    assert!(usage.starts_with("A test program.\n\nusage: game [OPTIONS] DISC [MORE]...\n"), "{usage}");
    assert!(usage.contains("-m, --mute, --silent") && usage.contains("--render-scale N"));
}
