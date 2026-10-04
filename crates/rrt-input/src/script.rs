//! Scripted presses: a pad driven by a list of frames, for headless runs,
//! tests and replays.
//!
//! The text form is the one the ports' `--press` flags take:
//! `FRAME:BUTTONS[:FRAMES]`, comma separated. `BUTTONS` is one or more names
//! from [`Buttons::NAMED`] (or `x`, `o`) joined by `+`; `FRAMES` is how long
//! they are held, 1 when left out. `1300:down,1340:x,1600:start:30,0:l1+r1:2`.

use std::fmt;

use crate::Buttons;

/// Buttons held from `frame` for `frames` frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    /// The first frame held, counted from 0 as [`crate::Pad`] frames are.
    pub frame: u64,
    /// What is held.
    pub buttons: Buttons,
    /// How many frames, at least 1.
    pub frames: u64,
}

/// A list of [`Press`]es; presses that overlap add up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Script {
    /// The presses, in the order given.
    pub presses: Vec<Press>,
}

/// Why a script did not parse: the item, and what was wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

impl Script {
    /// Parses the text form (see the module docs). An empty string is an
    /// empty script.
    pub fn parse(text: &str) -> Result<Script, ParseError> {
        let mut presses = Vec::new();
        for item in text.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            let bad = |why: &str| ParseError(format!("{item:?}: {why}; want FRAME:BUTTONS[:FRAMES]"));
            let mut parts = item.split(':');
            let frame = parts.next().and_then(|f| f.parse().ok()).ok_or_else(|| bad("no frame number"))?;
            let names = parts.next().ok_or_else(|| bad("no buttons"))?;
            let mut buttons = Buttons::NONE;
            for name in names.split('+') {
                buttons |= Buttons::by_name(&name.to_ascii_lowercase())
                    .ok_or_else(|| bad(&format!("no button called {name:?}")))?;
            }
            let frames = match parts.next() {
                None => 1,
                Some(n) => n.parse().ok().filter(|n| *n > 0).ok_or_else(|| bad("the hold must be 1 or more frames"))?,
            };
            if parts.next().is_some() {
                return Err(bad("too many fields"));
            }
            presses.push(Press { frame, buttons, frames });
        }
        Ok(Script { presses })
    }

    /// The buttons the script holds on `frame`.
    pub fn buttons_at(&self, frame: u64) -> Buttons {
        self.presses
            .iter()
            .filter(|p| (p.frame..p.frame + p.frames).contains(&frame))
            .fold(Buttons::NONE, |acc, p| acc | p.buttons)
    }

    /// The first frame after the last press ends: when the script is done.
    pub fn end(&self) -> u64 {
        self.presses.iter().map(|p| p.frame + p.frames).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frames_buttons_and_holds() {
        let s = Script::parse("1300:down, 1340:x,1600:Start:30,0:l1+r1:2").unwrap();
        assert_eq!(
            s.presses,
            [
                Press { frame: 1300, buttons: Buttons::DOWN, frames: 1 },
                Press { frame: 1340, buttons: Buttons::CROSS, frames: 1 },
                Press { frame: 1600, buttons: Buttons::START, frames: 30 },
                Press { frame: 0, buttons: Buttons::L1 | Buttons::R1, frames: 2 },
            ]
        );
        assert_eq!(s.end(), 1630);
        assert_eq!(Script::parse("").unwrap(), Script::default());
    }

    #[test]
    fn holds_cover_their_frames_and_overlaps_add_up() {
        let s = Script::parse("10:x:3,11:o").unwrap();
        assert_eq!(s.buttons_at(9), Buttons::NONE);
        assert_eq!(s.buttons_at(10), Buttons::CROSS);
        assert_eq!(s.buttons_at(11), Buttons::CROSS | Buttons::CIRCLE);
        assert_eq!(s.buttons_at(12), Buttons::CROSS);
        assert_eq!(s.buttons_at(13), Buttons::NONE);
    }

    #[test]
    fn bad_items_say_what_is_wrong() {
        for (text, why) in [
            ("x:cross", "no frame number"),
            ("10", "no buttons"),
            ("10:jump", "no button called"),
            ("10:x:0", "1 or more"),
            ("10:x:2:3", "too many"),
        ] {
            let e = Script::parse(text).unwrap_err();
            assert!(e.0.contains(why), "{text}: {e}");
        }
    }
}
