//! Sticks as the PlayStation pad reports them: one byte an axis, 0 left or up,
//! 0x80 centre, 0xff right or down.

/// One stick's two axes as bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Stick {
    /// 0 left, 0x80 centre, 0xff right.
    pub x: u8,
    /// 0 up, 0x80 centre, 0xff down.
    pub y: u8,
}

impl Stick {
    /// At rest.
    pub const CENTRE: Stick = Stick { x: 0x80, y: 0x80 };

    /// From gilrs-style axes (-1 left or down, 1 right or up).
    pub fn from_axes(x: f32, y: f32) -> Stick {
        Stick { x: axis_byte(x, false), y: axis_byte(y, true) }
    }

    /// The offset from centre of each axis, -128 to 127, y down positive.
    pub fn offset(self) -> (i32, i32) {
        (i32::from(self.x) - 0x80, i32::from(self.y) - 0x80)
    }
}

impl Default for Stick {
    fn default() -> Stick {
        Stick::CENTRE
    }
}

/// A stick axis from gilrs (-1 left or down, 1 right or up) as the pad's byte
/// (0 left or up). `flip` turns gilrs' up-positive into down-positive, for a
/// y axis. 0x80 is the centre, so there are 128 steps below it and 127 above:
/// -1 reads 0, 0 reads 0x80, 1 reads 0xff.
pub fn axis_byte(v: f32, flip: bool) -> u8 {
    let v = if flip { -v } else { v }.clamp(-1.0, 1.0);
    let scale = if v < 0.0 { 128.0 } else { 127.0 };
    (128.0 + v * scale).round() as u8
}

/// A modern round-gated stick as a DualShock reports it. A DualShock's
/// potentiometers saturate before its round gate, so a full push in any
/// direction reads at or near the ends of both axes, and games tune their
/// thresholds to that: a round stick pushed diagonally reaches only 0.7 an
/// axis. The vector is scaled so its larger axis is its length (at most 1),
/// carrying the circle to the square. No dead zone is applied.
pub fn square_stick(x: f32, y: f32) -> (f32, f32) {
    let r = (x * x + y * y).sqrt().min(1.0);
    let m = x.abs().max(y.abs());
    if m <= 0.0 { (0.0, 0.0) } else { (x * r / m, y * r / m) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_bytes_hit_both_ends_and_the_centre() {
        assert_eq!(axis_byte(0.0, false), 0x80);
        assert_eq!(axis_byte(1.0, false), 0xff);
        assert_eq!(axis_byte(-1.0, false), 0);
        assert_eq!(axis_byte(1.0, true), 0, "stick up is 0");
        assert_eq!(axis_byte(7.0, false), 0xff, "out of range clamps");
    }

    #[test]
    fn square_stick_saturates_diagonals() {
        let d = std::f32::consts::FRAC_1_SQRT_2;
        let (x, y) = square_stick(d, d);
        assert!((x - 1.0).abs() < 1e-5 && (y - 1.0).abs() < 1e-5);
        assert_eq!(square_stick(0.0, 0.0), (0.0, 0.0));
        assert_eq!(square_stick(0.5, 0.0), (0.5, 0.0), "on an axis nothing changes");
    }
}
