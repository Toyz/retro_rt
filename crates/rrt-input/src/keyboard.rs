//! The keyboard as a pad: which physical keys press which [`Buttons`], and
//! which push a stick.

use std::collections::HashSet;

use winit::keyboard::{KeyCode, PhysicalKey};

use crate::{Buttons, Stick};

/// Which stick a key pushes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The left stick.
    Left,
    /// The right stick.
    Right,
}

/// Physical keys to pad buttons and stick pushes. Keys are by position
/// ([`KeyCode`]), so the layout holds on any keyboard language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyMap {
    /// A key and the buttons it holds. Several keys may hold the same button.
    pub buttons: Vec<(KeyCode, Buttons)>,
    /// A key, the stick it pushes, and the direction: x and y, each -1, 0 or
    /// 1, y down positive. Held keys add up; a diagonal reads the corner.
    pub sticks: Vec<(KeyCode, Side, i8, i8)>,
}

impl Default for KeyMap {
    /// The layout every retro_rt game starts from: arrows the D-pad, Z X A S
    /// cross circle square triangle, Q W L1 R1, 1 2 L2 R2, Enter Start, Shift
    /// or Backspace Select, C V L3 R3, and I J K L the right stick.
    fn default() -> KeyMap {
        use KeyCode::*;
        KeyMap {
            buttons: vec![
                (ArrowUp, Buttons::UP),
                (ArrowDown, Buttons::DOWN),
                (ArrowLeft, Buttons::LEFT),
                (ArrowRight, Buttons::RIGHT),
                (KeyZ, Buttons::CROSS),
                (KeyX, Buttons::CIRCLE),
                (KeyA, Buttons::SQUARE),
                (KeyS, Buttons::TRIANGLE),
                (KeyQ, Buttons::L1),
                (KeyW, Buttons::R1),
                (Digit1, Buttons::L2),
                (Digit2, Buttons::R2),
                (Enter, Buttons::START),
                (ShiftLeft, Buttons::SELECT),
                (ShiftRight, Buttons::SELECT),
                (Backspace, Buttons::SELECT),
                (KeyC, Buttons::L3),
                (KeyV, Buttons::R3),
            ],
            sticks: vec![
                (KeyI, Side::Right, 0, -1),
                (KeyK, Side::Right, 0, 1),
                (KeyJ, Side::Right, -1, 0),
                (KeyL, Side::Right, 1, 0),
            ],
        }
    }
}

/// The keys held now, by physical position, and the map that reads them.
#[derive(Clone, Debug, Default)]
pub struct Keyboard {
    /// The mapping [`Keyboard::buttons`] and [`Keyboard::stick`] read with.
    pub map: KeyMap,
    held: HashSet<KeyCode>,
}

impl Keyboard {
    /// A keyboard read through `map`.
    pub fn new(map: KeyMap) -> Keyboard {
        Keyboard { map, held: HashSet::new() }
    }

    /// Records a key going down or up, as winit reports it.
    pub fn key(&mut self, key: PhysicalKey, pressed: bool) {
        let PhysicalKey::Code(code) = key else { return };
        if pressed {
            self.held.insert(code);
        } else {
            self.held.remove(&code);
        }
    }

    /// Lets go of every key: for when the window loses focus and the
    /// releases will never arrive.
    pub fn release_all(&mut self) {
        self.held.clear();
    }

    /// Whether `code` is held.
    pub fn is_held(&self, code: KeyCode) -> bool {
        self.held.contains(&code)
    }

    /// The buttons the held keys press.
    pub fn buttons(&self) -> Buttons {
        let mut b = Buttons::NONE;
        for (k, bits) in &self.map.buttons {
            if self.held.contains(k) {
                b |= *bits;
            }
        }
        b
    }

    /// Where the held keys push `side`'s stick, at full deflection (a
    /// diagonal in the corner, as a DualShock's saturates); None with none of
    /// its keys held, so a gamepad's stick is left alone.
    pub fn stick(&self, side: Side) -> Option<Stick> {
        let keys = self.map.sticks.iter().filter(|(k, s, ..)| *s == side && self.held.contains(k));
        let (n, x, y) = keys.fold((0, 0i32, 0i32), |(n, x, y), &(_, _, dx, dy)| (n + 1, x + dx as i32, y + dy as i32));
        let byte = |v: i32| match v.signum() {
            -1 => 0,
            1 => 0xff,
            _ => 0x80,
        };
        (n > 0).then(|| Stick { x: byte(x), y: byte(y) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(k: &mut Keyboard, code: KeyCode, down: bool) {
        k.key(PhysicalKey::Code(code), down);
    }

    /// Two keys on one button: letting go of one leaves the button held.
    #[test]
    fn a_button_stays_held_while_any_of_its_keys_is() {
        let mut k = Keyboard::default();
        press(&mut k, KeyCode::ShiftLeft, true);
        press(&mut k, KeyCode::Backspace, true);
        press(&mut k, KeyCode::ShiftLeft, false);
        assert_eq!(k.buttons(), Buttons::SELECT);
        press(&mut k, KeyCode::Backspace, false);
        assert_eq!(k.buttons(), Buttons::NONE);
    }

    #[test]
    fn stick_keys_push_to_the_corner() {
        let mut k = Keyboard::default();
        assert_eq!(k.stick(Side::Right), None);
        press(&mut k, KeyCode::KeyI, true);
        press(&mut k, KeyCode::KeyJ, true);
        assert_eq!(k.stick(Side::Right), Some(Stick { x: 0, y: 0 }));
        press(&mut k, KeyCode::KeyL, true);
        assert_eq!(k.stick(Side::Right), Some(Stick { x: 0x80, y: 0 }), "left and right cancel");
        assert_eq!(k.stick(Side::Left), None);
    }
}
