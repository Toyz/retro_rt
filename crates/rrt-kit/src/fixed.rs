//! Fixed-point numbers as the consoles compute them.

use std::fmt;
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

/// A signed fixed-point number: an `i32` with `FRAC` fraction bits. Adding
/// and subtracting wrap as the hardware's 32-bit registers do; multiplying
/// widens to 64 bits and shifts right arithmetically, truncating toward
/// minus infinity as the PS1's GTE and its compilers' `>> 12` do.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fixed<const FRAC: u32>(pub i32);

/// 12 fraction bits: the PS1 GTE's 4.12 matrices and 20.12 positions
/// (4096 is 1.0).
pub type Q12 = Fixed<12>;

impl<const FRAC: u32> Fixed<FRAC> {
    /// 1.0.
    pub const ONE: Self = Fixed(1 << FRAC);
    /// 0.
    pub const ZERO: Self = Fixed(0);

    /// A value from its raw word as the console stores it (4096 is 1.0 in
    /// [`Q12`]).
    pub const fn from_raw(raw: i32) -> Self {
        Fixed(raw)
    }

    /// The raw value as the console stores it.
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// A whole number.
    pub const fn from_int(n: i32) -> Self {
        Fixed(n.wrapping_shl(FRAC))
    }

    /// The nearest representable value to `v`, saturating at the ends.
    pub fn from_f32(v: f32) -> Self {
        Fixed((v * (1u64 << FRAC) as f32).round() as i32)
    }

    /// The value as a float.
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / (1u64 << FRAC) as f32
    }

    /// The whole part, rounded toward minus infinity (an arithmetic shift).
    pub const fn floor(self) -> i32 {
        self.0 >> FRAC
    }
}

impl<const FRAC: u32> Add for Fixed<FRAC> {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Fixed(self.0.wrapping_add(o.0))
    }
}

impl<const FRAC: u32> AddAssign for Fixed<FRAC> {
    fn add_assign(&mut self, o: Self) {
        *self = *self + o;
    }
}

impl<const FRAC: u32> Sub for Fixed<FRAC> {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Fixed(self.0.wrapping_sub(o.0))
    }
}

impl<const FRAC: u32> SubAssign for Fixed<FRAC> {
    fn sub_assign(&mut self, o: Self) {
        *self = *self - o;
    }
}

impl<const FRAC: u32> Neg for Fixed<FRAC> {
    type Output = Self;
    fn neg(self) -> Self {
        Fixed(self.0.wrapping_neg())
    }
}

impl<const FRAC: u32> Mul for Fixed<FRAC> {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        Fixed(((i64::from(self.0) * i64::from(o.0)) >> FRAC) as i32)
    }
}

impl<const FRAC: u32> fmt::Debug for Fixed<FRAC> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({:#x})", self.to_f32(), self.0)
    }
}

impl<const FRAC: u32> fmt::Display for Fixed<FRAC> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_f32(), f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q12_holds_4096_as_one() {
        assert_eq!(Q12::ONE.raw(), 4096);
        assert_eq!(Q12::from_int(3).raw(), 3 * 4096);
        assert_eq!(Q12::from_f32(0.5).raw(), 2048);
        assert_eq!(Q12::from_raw(6144).to_f32(), 1.5);
        assert_eq!(Q12::from_f32(-1.5).floor(), -2, "floor goes toward minus infinity");
    }

    #[test]
    fn multiply_truncates_toward_minus_infinity() {
        let half = Q12::from_raw(2048);
        assert_eq!((Q12::from_int(3) * half).raw(), 6144);
        // -1/4096 * 1/2 is -1/8192: the shift gives -1, not 0.
        assert_eq!((Q12::from_raw(-1) * half).raw(), -1);
        assert_eq!((Q12::from_raw(1) * half).raw(), 0);
        // 20.12 positions times 4.12 rotations stay in 64 bits.
        assert_eq!((Q12::from_int(100_000) * Q12::ONE).raw(), Q12::from_int(100_000).raw());
    }

    #[test]
    fn add_and_sub_wrap_like_the_registers() {
        assert_eq!((Q12::from_raw(i32::MAX) + Q12::from_raw(1)).raw(), i32::MIN);
        assert_eq!((Q12::from_raw(i32::MIN) - Q12::from_raw(1)).raw(), i32::MAX);
        let mut x = Q12::ONE;
        x += Q12::ONE;
        x -= Q12::from_raw(1);
        assert_eq!(x.raw(), 8191);
        assert_eq!((-Q12::ONE).raw(), -4096);
    }
}
