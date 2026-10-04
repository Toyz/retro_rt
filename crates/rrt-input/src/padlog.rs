//! A record of every frame's pad, and the console commands run on the way:
//! for replaying a session exactly, attaching to a bug report, and lockstep
//! runs against an original.
//!
//! Text, one line a frame, so a log diffs, greps and edits by hand:
//!
//! ```text
//! # rrt pad log 1                      header lines start with '#'
//! FRAME BUTTONS LX LY RX RY L2 R2 A    decimal frame; the rest hex; A 1 analog, 0 digital
//! 120 4000 80 80 80 80 00 00 1
//! 120 > rumble on                      a console command run on frame 120
//! ```
//!
//! Where [`crate::Script`] is a few sparse presses, a log is the whole pad,
//! every frame, sticks and triggers included.

use std::collections::BTreeMap;

use crate::{Buttons, Pad, Stick};

/// The first line every log starts with.
pub const HEADER: &str = "# rrt pad log 1";

/// A session's pads and commands, by frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PadLog {
    /// Free-form header lines (without their `#`): the game, its version,
    /// the gamepads connected.
    pub notes: Vec<String>,
    pads: BTreeMap<u64, Pad>,
    commands: BTreeMap<u64, Vec<String>>,
}

/// A log line that did not parse: its number (from 1) and what was wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The line number, from 1.
    pub line: usize,
    /// What was wrong.
    pub what: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pad log line {}: {}", self.line, self.what)
    }
}

impl std::error::Error for ParseError {}

impl PadLog {
    /// An empty log.
    pub fn new() -> PadLog {
        PadLog::default()
    }

    /// Records `pad` for `frame`, replacing what was there.
    pub fn record(&mut self, frame: u64, pad: Pad) {
        self.pads.insert(frame, pad);
    }

    /// Records a console command run on `frame`, after any already there.
    pub fn record_command(&mut self, frame: u64, line: impl Into<String>) {
        self.commands.entry(frame).or_default().push(line.into());
    }

    /// The pad recorded for `frame`.
    pub fn pad(&self, frame: u64) -> Option<Pad> {
        self.pads.get(&frame).copied()
    }

    /// The commands recorded for `frame`, in order.
    pub fn commands(&self, frame: u64) -> &[String] {
        self.commands.get(&frame).map_or(&[], Vec::as_slice)
    }

    /// The first frame after the last thing recorded: when a replay is done.
    pub fn end(&self) -> u64 {
        let last = |m: Option<u64>| m.map_or(0, |f| f + 1);
        last(self.pads.keys().next_back().copied()).max(last(self.commands.keys().next_back().copied()))
    }

    /// Frames with a pad recorded.
    pub fn len(&self) -> usize {
        self.pads.len()
    }

    /// Nothing recorded.
    pub fn is_empty(&self) -> bool {
        self.pads.is_empty() && self.commands.is_empty()
    }

    /// The log as text: the header, the notes, then frame by frame, each
    /// frame's pad before its commands.
    pub fn to_text(&self) -> String {
        let mut s = String::from(HEADER);
        s.push('\n');
        for n in &self.notes {
            s.push_str(&format!("# {n}\n"));
        }
        let frames: std::collections::BTreeSet<u64> = self.pads.keys().chain(self.commands.keys()).copied().collect();
        for f in frames {
            if let Some(p) = self.pads.get(&f) {
                s.push_str(&format!(
                    "{f} {:04x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {}\n",
                    p.buttons.bits(),
                    p.left.x,
                    p.left.y,
                    p.right.x,
                    p.right.y,
                    p.l2,
                    p.r2,
                    u8::from(p.analog)
                ));
            }
            for c in self.commands(f) {
                s.push_str(&format!("{f} > {c}\n"));
            }
        }
        s
    }

    /// Reads a log's text. `#` lines after the first become notes; blank
    /// lines are skipped.
    pub fn parse(text: &str) -> Result<PadLog, ParseError> {
        let mut log = PadLog::new();
        for (i, line) in text.lines().enumerate() {
            let n = i + 1;
            let bad = |what: &str| ParseError { line: n, what: what.to_string() };
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            if let Some(note) = line.strip_prefix('#') {
                if n > 1 || line != HEADER {
                    log.notes.push(note.trim_start().to_string());
                }
                continue;
            }
            let (frame, rest) = line.split_once(' ').ok_or_else(|| bad("want FRAME then the pad or > COMMAND"))?;
            let frame: u64 = frame.parse().map_err(|_| bad("the frame is not a number"))?;
            if let Some(cmd) = rest.strip_prefix("> ") {
                log.record_command(frame, cmd);
                continue;
            }
            let f: Vec<&str> = rest.split_whitespace().collect();
            if f.len() != 8 {
                return Err(bad("want 8 fields after the frame: BUTTONS LX LY RX RY L2 R2 A"));
            }
            let hex8 = |s: &str| u8::from_str_radix(s, 16).map_err(|_| bad("a stick or trigger is not a hex byte"));
            let buttons = u16::from_str_radix(f[0], 16).map_err(|_| bad("the buttons are not a hex word"))?;
            let analog = match f[7] {
                "1" => true,
                "0" => false,
                _ => return Err(bad("A is 1 or 0")),
            };
            log.record(
                frame,
                Pad {
                    buttons: Buttons(buttons),
                    left: Stick { x: hex8(f[1])?, y: hex8(f[2])? },
                    right: Stick { x: hex8(f[3])?, y: hex8(f[4])? },
                    l2: hex8(f[5])?,
                    r2: hex8(f[6])?,
                    analog,
                },
            );
        }
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(buttons: Buttons, lx: u8) -> Pad {
        Pad { buttons, left: Stick { x: lx, y: 0x80 }, analog: true, l2: 0x40, ..Pad::default() }
    }

    #[test]
    fn a_log_round_trips_through_text() {
        let mut log = PadLog::new();
        log.notes.push("game hwtr 0.1".into());
        log.record(0, Pad::default());
        log.record(1, pad(Buttons::CROSS, 0xff));
        log.record_command(1, "rumble on");
        log.record_command(1, "frame");
        log.record_command(5, "quit");
        let text = log.to_text();
        assert!(text.starts_with("# rrt pad log 1\n# game hwtr 0.1\n0 0000 80 80 80 80 00 00 0\n"));
        assert!(text.contains("1 4000 ff 80 80 80 40 00 1\n1 > rumble on\n1 > frame\n5 > quit\n"));
        assert_eq!(PadLog::parse(&text).unwrap(), log);
        assert_eq!(log.end(), 6);
        assert_eq!(log.commands(1), ["rumble on", "frame"]);
        assert_eq!(log.commands(2), [] as [String; 0]);
    }

    #[test]
    fn bad_lines_say_which_and_why() {
        for (text, line, why) in [
            ("# rrt pad log 1\nx 0000 80 80 80 80 00 00 0", 2, "not a number"),
            ("3 0000 80 80", 1, "8 fields"),
            ("3 zzzz 80 80 80 80 00 00 0", 1, "hex word"),
            ("3 0000 80 80 80 gg 00 00 0", 1, "hex byte"),
            ("3 0000 80 80 80 80 00 00 7", 1, "1 or 0"),
            ("3", 1, "want FRAME"),
        ] {
            let e = PadLog::parse(text).unwrap_err();
            assert_eq!(e.line, line, "{text}");
            assert!(e.what.contains(why), "{text}: {e}");
        }
        assert!(PadLog::parse("").unwrap().is_empty());
    }
}
