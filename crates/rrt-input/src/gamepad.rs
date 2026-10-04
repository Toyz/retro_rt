//! The gamepad side of [`Input`]: gilrs, with no filters, one pad at a time.

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs};

use crate::keyboard::Side;
use crate::{Buttons, Keyboard, Pad, Stick, square_stick};

/// How a gamepad's round stick is carried to the pad's bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StickShape {
    /// As gilrs reports it: a diagonal reaches about 0.7 an axis.
    Round,
    /// Pushed out to the square, as a DualShock's saturating potentiometers
    /// read ([`square_stick`]). The default: games were tuned on DualShocks.
    #[default]
    Square,
}

/// What the pad's two motors do, as a DualShock takes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Motors {
    /// The small motor: on or off.
    pub small: bool,
    /// The large motor's power, 0 off to 255.
    pub large: u8,
}

/// gilrs buttons and the pad bits they press: where a DualShock has them.
const GAMEPAD: [(Button, Buttons); 16] = [
    (Button::DPadUp, Buttons::UP),
    (Button::DPadDown, Buttons::DOWN),
    (Button::DPadLeft, Buttons::LEFT),
    (Button::DPadRight, Buttons::RIGHT),
    (Button::South, Buttons::CROSS),
    (Button::East, Buttons::CIRCLE),
    (Button::West, Buttons::SQUARE),
    (Button::North, Buttons::TRIANGLE),
    (Button::LeftTrigger, Buttons::L1),
    (Button::RightTrigger, Buttons::R1),
    (Button::LeftTrigger2, Buttons::L2),
    (Button::RightTrigger2, Buttons::R2),
    (Button::Start, Buttons::START),
    (Button::Select, Buttons::SELECT),
    (Button::LeftThumb, Buttons::L3),
    (Button::RightThumb, Buttons::R3),
];

/// The keyboard and the gamepads, read together once a frame.
pub struct Input {
    gilrs: Option<Gilrs>,
    /// The pad read: the last to send a press or a push past half, kept
    /// while it stays connected.
    active: Option<GamepadId>,
    /// How the sticks are shaped before they become bytes.
    pub shape: StickShape,
    /// The keyboard's held keys and their mapping. The window layer feeds it
    /// with [`Keyboard::key`].
    pub keyboard: Keyboard,
    rumble: Option<Rumble>,
    motors: Motors,
    warned_no_ff: bool,
}

impl Default for Input {
    fn default() -> Self {
        Input::new()
    }
}

impl Input {
    /// Opens gilrs with no filters (its dead zone rescales the stick, and a
    /// game applies its own), the default [`crate::KeyMap`] and square sticks.
    /// With no gamepad support on the host it carries on with the keyboard.
    pub fn new() -> Input {
        let gilrs = gilrs::GilrsBuilder::new()
            .with_default_filters(false)
            .build()
            .map_err(|e| tracing::warn!("no gamepad support: {e}"))
            .ok();
        Input { gilrs, ..Input::keyboard_only() }
    }

    /// The keyboard alone, with gilrs never opened: for tools, tests and
    /// headless runs, where no gamepad should be read.
    pub fn keyboard_only() -> Input {
        Input {
            gilrs: None,
            active: None,
            shape: StickShape::default(),
            keyboard: Keyboard::default(),
            rumble: None,
            motors: Motors::default(),
            warned_no_ff: false,
        }
    }

    /// Whether a gamepad is connected.
    pub fn has_gamepad(&self) -> bool {
        self.gilrs.as_ref().is_some_and(|g| g.gamepads().any(|(_, p)| p.is_connected()))
    }

    /// Every gamepad gilrs knows, one line each: id, name, the OS's name,
    /// uuid, whether it is connected. For a log's header.
    pub fn gamepads(&self) -> Vec<String> {
        let Some(g) = &self.gilrs else { return Vec::new() };
        g.gamepads()
            .map(|(id, p)| {
                format!(
                    "gamepad {id}: {:?} ({:?}) uuid {:02x?} connected {}",
                    p.name(),
                    p.os_name(),
                    p.uuid(),
                    p.is_connected()
                )
            })
            .collect()
    }

    /// This frame's pad: the keyboard's buttons with the gamepad's, and the
    /// gamepad's sticks unless the keyboard's stick keys are held
    /// ([`compose`]). Drains gilrs' events, so call it once a frame.
    pub fn read(&mut self) -> Pad {
        let state = self.gilrs.as_mut().and_then(|g| {
            while let Some(ev) = g.next_event() {
                self.active = next_active(self.active, ev.id, seen(&ev.event));
            }
            choose(g, self.active).map(|id| GamepadState::from_gilrs(&g.gamepad(id)))
        });
        compose(&self.keyboard, state.as_ref(), self.shape)
    }

    /// The gamepad being read, as gilrs reports it this moment: its id, its
    /// sticks' and triggers' raw values, the hat. For a pad log; empty with
    /// no gamepad.
    pub fn describe(&self) -> String {
        let Some(g) = &self.gilrs else { return String::new() };
        let Some(id) = choose(g, self.active) else { return String::new() };
        let p = g.gamepad(id);
        let v = |a| p.value(a);
        format!(
            "pad {id} ls {:+.3} {:+.3} rs {:+.3} {:+.3} lz {:+.3} rz {:+.3} dpad {:+.3} {:+.3}",
            v(Axis::LeftStickX),
            v(Axis::LeftStickY),
            v(Axis::RightStickX),
            v(Axis::RightStickY),
            v(Axis::LeftZ),
            v(Axis::RightZ),
            v(Axis::DPadX),
            v(Axis::DPadY)
        )
    }

    /// Runs the pad's motors. Only a change is sent; the motors keep running
    /// until told otherwise, as a DualShock's do. A pad without force
    /// feedback, or none at all, is not an error.
    pub fn rumble(&mut self, motors: Motors) {
        if motors == self.motors {
            return;
        }
        self.motors = motors;
        self.send_motors(motors);
    }

    /// The motors as last set by [`Input::rumble`].
    pub fn motors(&self) -> Motors {
        self.motors
    }

    fn send_motors(&mut self, motors: Motors) {
        let Some(g) = &mut self.gilrs else { return };
        let read = choose(g, self.active);
        let target = read.filter(|id| g.gamepad(*id).is_ff_supported());
        if self.rumble.as_ref().map(|r| r.pad) != target {
            self.rumble = target.and_then(|id| Rumble::new(g, id));
            if let Some(id) = target.filter(|_| self.rumble.is_some()) {
                tracing::info!("rumble on gamepad {id} ({})", g.gamepad(id).name());
            }
        }
        // A pad that cannot rumble would otherwise fail in silence.
        if target.is_none() && motors != Motors::default() && !self.warned_no_ff {
            self.warned_no_ff = true;
            match read {
                Some(id) => tracing::warn!("gamepad {id} ({}) reports no force feedback", g.gamepad(id).name()),
                None => tracing::warn!("no gamepad to rumble"),
            }
        }
        if let Some(r) = &self.rumble {
            r.set(motors);
        }
    }
}

/// One gamepad at one moment: what [`compose`] turns into a [`Pad`]. Read
/// from gilrs by [`Input::read`]; public so a test or a replay can make one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GamepadState {
    /// The buttons gilrs reports pressed, mapped to where a DualShock has
    /// them.
    pub buttons: Buttons,
    /// The hat's x and y (`DPadX`, `DPadY`), -1..1, y up positive: the
    /// D-pad of a pad with no SDL mapping. 0 for one whose D-pad is buttons.
    pub hat: (f32, f32),
    /// The left stick's x and y, -1..1, y up positive.
    pub left: (f32, f32),
    /// The right stick's x and y, -1..1, y up positive.
    pub right: (f32, f32),
    /// L2's and R2's analog travel, 0..1, when the pad reports it.
    pub l2: Option<f32>,
    /// See [`GamepadState::l2`].
    pub r2: Option<f32>,
}

impl GamepadState {
    fn from_gilrs(p: &gilrs::Gamepad<'_>) -> GamepadState {
        let mut buttons = Buttons::NONE;
        for (b, bits) in GAMEPAD {
            if p.is_pressed(b) {
                buttons |= bits;
            }
        }
        let axes = |x, y| (p.value(x), p.value(y));
        let trigger = |b: Button| p.button_data(b).map(|d| d.value());
        GamepadState {
            buttons,
            hat: axes(Axis::DPadX, Axis::DPadY),
            left: axes(Axis::LeftStickX, Axis::LeftStickY),
            right: axes(Axis::RightStickX, Axis::RightStickY),
            l2: trigger(Button::LeftTrigger2),
            r2: trigger(Button::RightTrigger2),
        }
    }
}

/// A frame's pad from the keyboard and, when one is connected, a gamepad:
/// both's buttons and the hat's directions together; the gamepad's sticks,
/// shaped by `shape`, unless the keyboard's stick keys push them; L2 and R2's
/// analog travel, or 255 for a trigger held only as a button. This is all of
/// [`Input::read`] but the reading of gilrs.
pub fn compose(keyboard: &Keyboard, gamepad: Option<&GamepadState>, shape: StickShape) -> Pad {
    let mut pad = Pad { buttons: keyboard.buttons(), ..Pad::default() };
    if let Some(g) = gamepad {
        pad.analog = true;
        pad.buttons |= g.buttons | hat(g.hat.0, g.hat.1);
        let shaped = |(x, y): (f32, f32)| match shape {
            StickShape::Round => (x, y),
            StickShape::Square => square_stick(x, y),
        };
        let (lx, ly) = shaped(g.left);
        let (rx, ry) = shaped(g.right);
        pad.left = Stick::from_axes(lx, ly);
        pad.right = Stick::from_axes(rx, ry);
        let travel = |v: Option<f32>| v.map_or(0, |v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        pad.l2 = travel(g.l2);
        pad.r2 = travel(g.r2);
    }
    if let Some(s) = keyboard.stick(Side::Left) {
        pad.left = s;
    }
    if let Some(s) = keyboard.stick(Side::Right) {
        pad.right = s;
    }
    if pad.buttons.contains(Buttons::L2) && pad.l2 == 0 {
        pad.l2 = 0xff;
    }
    if pad.buttons.contains(Buttons::R2) && pad.r2 == 0 {
        pad.r2 = 0xff;
    }
    pad
}

/// What a gilrs event says about which pad is in use.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Seen {
    /// A button went down.
    Press,
    /// A button's or axis' value changed to this.
    Moved(f32),
    /// The pad went away.
    Disconnected,
    /// Anything else.
    Other,
}

fn seen(event: &EventType) -> Seen {
    match *event {
        EventType::ButtonPressed(..) => Seen::Press,
        EventType::ButtonChanged(_, v, _) | EventType::AxisChanged(_, v, _) => Seen::Moved(v),
        EventType::Disconnected => Seen::Disconnected,
        _ => Seen::Other,
    }
}

/// The pad read after pad `id` did what `seen` says: it takes over with a
/// press or a push past half (a resting stick's drift on another pad does
/// not); the active pad going away leaves none chosen.
fn next_active<T: PartialEq + Copy>(active: Option<T>, id: T, seen: Seen) -> Option<T> {
    match seen {
        Seen::Press => Some(id),
        Seen::Moved(v) if v.abs() > 0.5 => Some(id),
        Seen::Disconnected if active == Some(id) => None,
        _ => active,
    }
}

/// The active pad while it is connected, else the first connected one (an
/// Xbox pad can come with other devices listed before it).
fn choose(g: &Gilrs, active: Option<GamepadId>) -> Option<GamepadId> {
    active
        .filter(|id| g.connected_gamepad(*id).is_some())
        .or_else(|| g.gamepads().find(|(_, p)| p.is_connected()).map(|(id, _)| id))
}

/// A D-pad that gilrs reports as a hat (`DPadX`, `DPadY`: pads with no SDL
/// mapping, DirectInput) rather than as buttons. Y is up positive.
fn hat(x: f32, y: f32) -> Buttons {
    let mut b = Buttons::NONE;
    if x > 0.5 {
        b |= Buttons::RIGHT;
    } else if x < -0.5 {
        b |= Buttons::LEFT;
    }
    if y > 0.5 {
        b |= Buttons::UP;
    } else if y < -0.5 {
        b |= Buttons::DOWN;
    }
    b
}

/// Two force-feedback effects on one pad, one per motor, each repeating
/// until stopped.
struct Rumble {
    pad: GamepadId,
    strong: gilrs::ff::Effect,
    weak: gilrs::ff::Effect,
}

impl Rumble {
    fn new(gilrs: &mut Gilrs, id: GamepadId) -> Option<Rumble> {
        use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
        let effect = |kind: BaseEffectType, gilrs: &mut Gilrs| {
            EffectBuilder::new()
                .add_effect(BaseEffect {
                    kind,
                    scheduling: Replay { play_for: Ticks::from_ms(1000), ..Default::default() },
                    ..Default::default()
                })
                .gamepads(&[id])
                .finish(gilrs)
                .map_err(|e| tracing::warn!("rumble: {e}"))
                .ok()
        };
        let strong = effect(BaseEffectType::Strong { magnitude: u16::MAX }, gilrs)?;
        let weak = effect(BaseEffectType::Weak { magnitude: u16::MAX }, gilrs)?;
        Some(Rumble { pad: id, strong, weak })
    }

    fn set(&self, m: Motors) {
        let r = if m.large > 0 {
            self.strong.set_gain(f32::from(m.large) / 255.0).and_then(|_| self.strong.play())
        } else {
            self.strong.stop()
        };
        let r = r.and_then(|_| if m.small { self.weak.play() } else { self.weak.stop() });
        if let Err(e) = r {
            tracing::warn!("rumble: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use winit::keyboard::{KeyCode, PhysicalKey};

    use super::*;

    #[test]
    fn a_hat_presses_the_dpad() {
        assert_eq!(hat(1.0, 1.0), Buttons::RIGHT | Buttons::UP);
        assert_eq!(hat(-1.0, -1.0), Buttons::LEFT | Buttons::DOWN);
        assert_eq!(hat(0.2, -0.3), Buttons::NONE);
    }

    #[test]
    fn every_gilrs_button_maps_to_its_own_pad_bit() {
        let all = GAMEPAD.iter().fold(Buttons::NONE, |acc, (_, b)| acc | *b);
        assert_eq!(all, Buttons(0xffff), "all 16 bits, none twice");
        assert_eq!(GAMEPAD.iter().find(|(g, _)| *g == Button::South).map(|p| p.1), Some(Buttons::CROSS));
        assert_eq!(GAMEPAD.iter().find(|(g, _)| *g == Button::LeftTrigger).map(|p| p.1), Some(Buttons::L1));
        assert_eq!(GAMEPAD.iter().find(|(g, _)| *g == Button::LeftTrigger2).map(|p| p.1), Some(Buttons::L2));
    }

    #[test]
    fn the_keyboard_alone_is_a_digital_pad() {
        let mut k = Keyboard::default();
        k.key(PhysicalKey::Code(KeyCode::KeyZ), true);
        k.key(PhysicalKey::Code(KeyCode::Digit1), true);
        let pad = compose(&k, None, StickShape::Square);
        assert_eq!(pad.buttons, Buttons::CROSS | Buttons::L2);
        assert!(!pad.analog);
        assert_eq!((pad.left, pad.right), (Stick::CENTRE, Stick::CENTRE));
        assert_eq!((pad.l2, pad.r2), (0xff, 0), "a digital L2 reads fully pressed");
    }

    #[test]
    fn a_gamepad_and_the_keyboard_merge() {
        let mut k = Keyboard::default();
        k.key(PhysicalKey::Code(KeyCode::Enter), true);
        let g = GamepadState {
            buttons: Buttons::CROSS,
            hat: (-1.0, 0.0),
            left: (1.0, 1.0),
            right: (0.0, -1.0),
            l2: Some(0.5),
            r2: None,
        };
        let pad = compose(&k, Some(&g), StickShape::Round);
        assert_eq!(pad.buttons, Buttons::CROSS | Buttons::START | Buttons::LEFT);
        assert!(pad.analog);
        assert_eq!(pad.left, Stick { x: 0xff, y: 0 }, "right and up");
        assert_eq!(pad.right, Stick { x: 0x80, y: 0xff }, "down");
        assert_eq!((pad.l2, pad.r2), (128, 0));
    }

    #[test]
    fn square_sticks_reach_the_corner_and_round_ones_do_not() {
        let d = std::f32::consts::FRAC_1_SQRT_2;
        let g = GamepadState { left: (d, d), ..GamepadState::default() };
        let k = Keyboard::default();
        assert_eq!(compose(&k, Some(&g), StickShape::Square).left, Stick { x: 0xff, y: 0 });
        assert_eq!(compose(&k, Some(&g), StickShape::Round).left, Stick { x: 218, y: 37 });
    }

    #[test]
    fn stick_keys_override_the_gamepad_stick() {
        let mut k = Keyboard::default();
        k.key(PhysicalKey::Code(KeyCode::KeyL), true);
        let g = GamepadState { right: (-1.0, 0.0), ..GamepadState::default() };
        assert_eq!(compose(&k, Some(&g), StickShape::Square).right, Stick { x: 0xff, y: 0x80 });
    }

    #[test]
    fn the_active_pad_changes_on_a_press_or_a_real_push() {
        assert_eq!(next_active(None, 2, Seen::Press), Some(2));
        assert_eq!(next_active(Some(1), 2, Seen::Moved(0.3)), Some(1), "drift does not take over");
        assert_eq!(next_active(Some(1), 2, Seen::Moved(-0.8)), Some(2));
        assert_eq!(next_active(Some(1), 2, Seen::Disconnected), Some(1), "another pad leaving");
        assert_eq!(next_active(Some(2), 2, Seen::Disconnected), None);
        assert_eq!(next_active(Some(1), 2, Seen::Other), Some(1));
    }

    #[test]
    fn rumble_remembers_the_motors_with_no_gamepad() {
        let mut input = Input::keyboard_only();
        let on = Motors { small: true, large: 200 };
        input.rumble(on);
        assert_eq!(input.motors(), on);
        assert!(!input.has_gamepad());
        assert!(input.gamepads().is_empty() && input.describe().is_empty());
    }
}
