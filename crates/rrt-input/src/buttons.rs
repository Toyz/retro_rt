//! Pad buttons in the PlayStation controller's bit order.
//!
//! The order is the 16-bit word the pad shifts out over SIO, inverted so that
//! 1 is pressed: SELECT is bit 0, SQUARE bit 15. PS1 games read it in this
//! order. PS2 libpad games (`scePadRead`, a `direct` field) hold the same word
//! with its two bytes swapped, so L2 is bit 0 and LEFT bit 15:
//! [`Buttons::libpad`] and [`Buttons::from_libpad`] convert. Games for other
//! machines map from here with the face-button aliases
//! ([`Buttons::SOUTH`] and its neighbours), which name a button by where it
//! sits rather than by its PlayStation symbol.

use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not};

/// Held buttons, active high, in the PlayStation pad's bit order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Buttons(pub u16);

impl Buttons {
    /// No button.
    pub const NONE: Buttons = Buttons(0);
    /// Select (Create on a DualSense, View/Back on an Xbox pad).
    pub const SELECT: Buttons = Buttons(1 << 0);
    /// The left stick pressed in.
    pub const L3: Buttons = Buttons(1 << 1);
    /// The right stick pressed in.
    pub const R3: Buttons = Buttons(1 << 2);
    /// Start (Options on a DualSense, Menu on an Xbox pad).
    pub const START: Buttons = Buttons(1 << 3);
    /// D-pad up.
    pub const UP: Buttons = Buttons(1 << 4);
    /// D-pad right.
    pub const RIGHT: Buttons = Buttons(1 << 5);
    /// D-pad down.
    pub const DOWN: Buttons = Buttons(1 << 6);
    /// D-pad left.
    pub const LEFT: Buttons = Buttons(1 << 7);
    /// The lower left shoulder (LT on an Xbox pad).
    pub const L2: Buttons = Buttons(1 << 8);
    /// The lower right shoulder (RT on an Xbox pad).
    pub const R2: Buttons = Buttons(1 << 9);
    /// The upper left shoulder (LB on an Xbox pad).
    pub const L1: Buttons = Buttons(1 << 10);
    /// The upper right shoulder (RB on an Xbox pad).
    pub const R1: Buttons = Buttons(1 << 11);
    /// The top face button (Y on an Xbox pad).
    pub const TRIANGLE: Buttons = Buttons(1 << 12);
    /// The right face button (B on an Xbox pad).
    pub const CIRCLE: Buttons = Buttons(1 << 13);
    /// The bottom face button (A on an Xbox pad).
    pub const CROSS: Buttons = Buttons(1 << 14);
    /// The left face button (X on an Xbox pad).
    pub const SQUARE: Buttons = Buttons(1 << 15);

    /// The bottom face button, by position: [`Buttons::CROSS`].
    pub const SOUTH: Buttons = Buttons::CROSS;
    /// The right face button, by position: [`Buttons::CIRCLE`].
    pub const EAST: Buttons = Buttons::CIRCLE;
    /// The left face button, by position: [`Buttons::SQUARE`].
    pub const WEST: Buttons = Buttons::SQUARE;
    /// The top face button, by position: [`Buttons::TRIANGLE`].
    pub const NORTH: Buttons = Buttons::TRIANGLE;

    /// The four D-pad bits.
    pub const DPAD: Buttons = Buttons(0x00f0);
    /// The four face buttons.
    pub const FACE: Buttons = Buttons(0xf000);

    /// Every button with its lower-case name, in bit order. The names are
    /// the ones scripted presses and logs use (`cross`, `l1`, `start`).
    pub const NAMED: [(&'static str, Buttons); 16] = [
        ("select", Buttons::SELECT),
        ("l3", Buttons::L3),
        ("r3", Buttons::R3),
        ("start", Buttons::START),
        ("up", Buttons::UP),
        ("right", Buttons::RIGHT),
        ("down", Buttons::DOWN),
        ("left", Buttons::LEFT),
        ("l2", Buttons::L2),
        ("r2", Buttons::R2),
        ("l1", Buttons::L1),
        ("r1", Buttons::R1),
        ("triangle", Buttons::TRIANGLE),
        ("circle", Buttons::CIRCLE),
        ("cross", Buttons::CROSS),
        ("square", Buttons::SQUARE),
    ];

    /// The raw word.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// No bit is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Every bit of `other` is set.
    pub const fn contains(self, other: Buttons) -> bool {
        self.0 & other.0 == other.0
    }

    /// Any bit of `other` is set.
    pub const fn intersects(self, other: Buttons) -> bool {
        self.0 & other.0 != 0
    }

    /// The word in PS2 libpad's order (`scePadRead`'s buttons as a `u16`,
    /// what a game keeps as `direct`): this word with its bytes swapped.
    pub const fn libpad(self) -> u16 {
        self.0.swap_bytes()
    }

    /// A word in PS2 libpad's order. See [`Buttons::libpad`].
    pub const fn from_libpad(word: u16) -> Buttons {
        Buttons(word.swap_bytes())
    }

    /// The word as the pad sends it on the wire: active low, 0 pressed.
    pub const fn active_low(self) -> u16 {
        !self.0
    }

    /// A button by its name in [`Buttons::NAMED`], or `x` for cross and `o`
    /// for circle. None for anything else.
    pub fn by_name(name: &str) -> Option<Buttons> {
        match name {
            "x" => Some(Buttons::CROSS),
            "o" => Some(Buttons::CIRCLE),
            _ => Buttons::NAMED.iter().find(|(n, _)| *n == name).map(|&(_, b)| b),
        }
    }

    /// The names of the set bits, in bit order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Buttons::NAMED.into_iter().filter(move |(_, b)| self.contains(*b)).map(|(n, _)| n)
    }
}

impl BitOr for Buttons {
    type Output = Buttons;
    fn bitor(self, other: Buttons) -> Buttons {
        Buttons(self.0 | other.0)
    }
}

impl BitOrAssign for Buttons {
    fn bitor_assign(&mut self, other: Buttons) {
        self.0 |= other.0;
    }
}

impl BitAnd for Buttons {
    type Output = Buttons;
    fn bitand(self, other: Buttons) -> Buttons {
        Buttons(self.0 & other.0)
    }
}

impl BitAndAssign for Buttons {
    fn bitand_assign(&mut self, other: Buttons) {
        self.0 &= other.0;
    }
}

impl Not for Buttons {
    type Output = Buttons;
    fn not(self) -> Buttons {
        Buttons(!self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The PS2 order's constants as a libpad game declares them: L2 0x0001,
    /// UP 0x1000, SQUARE 0x0080, START 0x0800.
    #[test]
    fn libpad_order_is_the_byte_swapped_word() {
        assert_eq!(Buttons::L2.libpad(), 0x0001);
        assert_eq!(Buttons::UP.libpad(), 0x1000);
        assert_eq!(Buttons::SQUARE.libpad(), 0x0080);
        assert_eq!(Buttons::START.libpad(), 0x0800);
        assert_eq!(Buttons::from_libpad(0x0040), Buttons::CROSS);
    }

    #[test]
    fn names_round_trip() {
        for (name, b) in Buttons::NAMED {
            assert_eq!(Buttons::by_name(name), Some(b));
        }
        assert_eq!(Buttons::by_name("x"), Some(Buttons::CROSS));
        assert_eq!(Buttons::by_name("nope"), None);
        let both = Buttons::CROSS | Buttons::L1;
        assert_eq!(both.names().collect::<Vec<_>>(), ["l1", "cross"]);
    }
}
