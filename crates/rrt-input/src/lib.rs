//! The host's pad, read once a frame.
//!
//! [`Input`] merges the keyboard ([`Keyboard`], keys mapped by [`KeyMap`]) with
//! one gamepad (gilrs: a DualSense, an Xbox pad, anything with an SDL mapping
//! or a hat) into a [`Pad`]: [`Buttons`] in the PlayStation pad's bit order,
//! the two sticks as bytes with 0x80 at rest ([`Stick`]), and the analog
//! triggers. [`Input::rumble`] drives the pad's motors as a DualShock takes
//! them ([`Motors`]). Everything but the reading of gilrs is [`compose`], a
//! pure function of the keyboard and a [`GamepadState`]; a [`Script`] holds
//! buttons on given frames, for headless runs and replays.
//!
//! What a game does with the pad - edge detection with its own repeat delay,
//! dead zones, the stick pressing the D-pad - is the game's and stays in the
//! game's crate. This crate applies no dead zone and no filter of its own: a
//! game that reads raw sticks gets raw sticks. See `docs/crates/rrt-input.md`
//! and `docs/conventions/pad.md`.

pub mod buttons;
pub mod gamepad;
pub mod keyboard;
pub mod script;
pub mod stick;

pub use buttons::Buttons;
pub use gamepad::{GamepadState, Input, Motors, StickShape, compose};
pub use keyboard::{KeyMap, Keyboard, Side};
pub use script::{Press, Script};
pub use stick::{Stick, axis_byte, square_stick};

/// One frame of the pad as the host reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pad {
    /// Held buttons, active high, in the PlayStation pad's order.
    pub buttons: Buttons,
    /// The left stick.
    pub left: Stick,
    /// The right stick.
    pub right: Stick,
    /// L2 and R2 as analog values, 0 released to 255 fully pressed: the
    /// gamepad's trigger axes, or 255 while the button bit alone is held
    /// (a keyboard, a pad with digital triggers).
    pub l2: u8,
    /// See [`Pad::l2`].
    pub r2: u8,
    /// A gamepad is connected, so the sticks are real: a DualShock in analog
    /// mode. False for the keyboard alone, where the sticks only move by
    /// [`KeyMap`]'s stick keys.
    pub analog: bool,
}

impl Default for Pad {
    fn default() -> Pad {
        Pad { buttons: Buttons::NONE, left: Stick::CENTRE, right: Stick::CENTRE, l2: 0, r2: 0, analog: false }
    }
}

impl Pad {
    /// Every bit of `b` is held.
    pub fn held(&self, b: Buttons) -> bool {
        self.buttons.contains(b)
    }

    /// The buttons held now and not in `previous`: pressed this frame.
    pub fn pressed(&self, previous: &Pad) -> Buttons {
        self.buttons & !previous.buttons
    }

    /// The buttons held in `previous` and not now: released this frame.
    pub fn released(&self, previous: &Pad) -> Buttons {
        previous.buttons & !self.buttons
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_and_released_are_edges_against_the_previous_frame() {
        let before = Pad { buttons: Buttons::CROSS | Buttons::UP, ..Pad::default() };
        let now = Pad { buttons: Buttons::CROSS | Buttons::START, ..Pad::default() };
        assert_eq!(now.pressed(&before), Buttons::START);
        assert_eq!(now.released(&before), Buttons::UP);
        assert!(now.held(Buttons::CROSS));
    }
}
