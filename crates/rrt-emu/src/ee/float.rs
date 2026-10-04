//! The EE's single-precision float arithmetic, bit-exact, on raw `u32` bit
//! patterns: what the R5900's FPU (COP1) and VU0 in macro mode compute, so a
//! port can compute exactly what the original did, not just what the
//! interpreter does.
//!
//! The rules, from piney_apples' `piney-data` `field::ee` and `piney-eemu`
//! `fpu` (which model `tools/eemu.py`'s `f_*` functions):
//!
//! - No denormals: a pattern with exponent field 0 is zero, whatever its
//!   fraction bits, as an operand; a result below the smallest normal is a
//!   zero of the result's sign.
//! - No infinities or NaNs: exponent field 255 is an ordinary number. A
//!   result past the largest is +/-[`FMAX`]; so is division by zero.
//! - Every result is the exact one truncated toward zero to 24 significant
//!   bits (worked on the integer mantissas), not rounded to nearest.
//! - `madd.s` / `msub.s` truncate the product before the add ([`madd`],
//!   [`msub`]).
//!
//! Not here: double precision (the EE has none), the FPU's and the VU's
//! flags (overflow, underflow, MAC, clip), and the VU's own pipelines. These
//! are pure functions of the operands.

use std::cmp::Ordering;

/// The EE's largest float, 0x7fff_ffff (about 3.4028237e38): what overflow
/// and division by zero give, with the result's sign.
pub const FMAX: u32 = 0x7fff_ffff;
const SIGN: u32 = 0x8000_0000;
const FRACTION: u32 = 0x007f_ffff;
const HIDDEN: u32 = 0x0080_0000;
/// Exponent bias plus the 23 fraction bits: value = mantissa * 2^(exp - 150).
const BIAS: i32 = 150;

/// (sign bit, exponent field, mantissa with the hidden bit), the mantissa 0
/// when the exponent field is 0.
fn unpack(v: u32) -> (u32, i32, u64) {
    let e = (v >> 23) & 0xff;
    let m = if e == 0 { 0 } else { u64::from((v & FRACTION) | HIDDEN) };
    (v >> 31, e as i32, m)
}

/// The float of sign `sign` and exact magnitude `n * 2^e2`, truncated to 24
/// bits, saturating at [`FMAX`] and flushing to a signed zero.
fn round(sign: u32, n: u128, e2: i32) -> u32 {
    if n == 0 {
        return sign << 31;
    }
    let len = (128 - n.leading_zeros()) as i32;
    let m = if len >= 24 { n >> (len - 24) } else { n << (24 - len) } as u32;
    let exp = e2 + len - 24 + BIAS;
    if exp > 255 {
        return (sign << 31) | FMAX;
    }
    if exp < 1 {
        return sign << 31;
    }
    (sign << 31) | ((exp as u32) << 23) | (m & FRACTION)
}

/// `add.s`: `a + b`, truncated toward zero. A zero operand gives the other
/// operand back unchanged; two zeros give -0 only when both are -0. An
/// operand more than 64 binary places below the other leaves the larger as
/// it is (same signs) or one step toward zero (opposite signs).
pub fn add(a: u32, b: u32) -> u32 {
    let (sa, ea, ma) = unpack(a);
    let (sb, eb, mb) = unpack(b);
    match (ma, mb) {
        (0, 0) => return a & b & SIGN,
        (0, _) => return b,
        (_, 0) => return a,
        _ => {}
    }
    // x has the larger exponent.
    let ((sx, ex, mx, x), (sy, ey, my)) =
        if ea >= eb { ((sa, ea, ma, a), (sb, eb, mb)) } else { ((sb, eb, mb, b), (sa, ea, ma)) };
    let d = (ex - ey) as u32;
    if d > 64 {
        // y lies wholly below x's last bit: the same sign leaves x, the
        // other sign takes x one step toward zero (a bit pattern's
        // magnitude is monotonic, and x's exponent is far above 1).
        return if sx == sy { x } else { x - 1 };
    }
    let signed = |s: u32, m: i128| if s == 1 { -m } else { m };
    let n = signed(sx, i128::from(mx) << d) + signed(sy, i128::from(my));
    round(u32::from(n < 0), n.unsigned_abs(), ey - BIAS)
}

/// `sub.s`: `a - b`, as [`add`] with `b`'s sign flipped.
pub fn sub(a: u32, b: u32) -> u32 {
    add(a, b ^ SIGN)
}

/// `mul.s`: `a * b`, the exact product truncated toward zero; overflow
/// gives +/-[`FMAX`], underflow a signed zero, a zero operand a signed zero.
pub fn mul(a: u32, b: u32) -> u32 {
    let (sa, ea, ma) = unpack(a);
    let (sb, eb, mb) = unpack(b);
    round(sa ^ sb, u128::from(ma * mb), ea + eb - 2 * BIAS)
}

/// `div.s`: `a / b`, truncated toward zero. A zero divisor (any exponent-0
/// pattern) gives +/-[`FMAX`] with the operands' sign product, 0/0
/// included.
pub fn div(a: u32, b: u32) -> u32 {
    let (sa, ea, ma) = unpack(a);
    let (sb, eb, mb) = unpack(b);
    if mb == 0 {
        return ((sa ^ sb) << 31) | FMAX;
    }
    round(sa ^ sb, (u128::from(ma) << 60) / u128::from(mb), ea - eb - 60)
}

/// `madd.s` / `madda.s`: `acc + a * b`, the product truncated first (two
/// truncations, as the EE does).
pub fn madd(acc: u32, a: u32, b: u32) -> u32 {
    add(acc, mul(a, b))
}

/// `msub.s` / `msuba.s`: `acc - a * b`, the product truncated first.
pub fn msub(acc: u32, a: u32, b: u32) -> u32 {
    sub(acc, mul(a, b))
}

/// `sqrt.s`: the square root of `v`'s magnitude (the sign is ignored, so
/// sqrt(-4) is 2), truncated toward zero; exponent 0 gives +0.
pub fn sqrt(v: u32) -> u32 {
    let (_, e, m) = unpack(v);
    let t = 60 + ((e - BIAS - 60) & 1);
    let root = (u128::from(m) << t).isqrt();
    round(0, root, (e - BIAS - t) / 2)
}

/// `rsqrt.s`: `a / sqrt(b)` rounded once (truncated toward zero), with
/// `a`'s sign and `b`'s sign ignored; computed as
/// `floor(A / sqrt(B)) = isqrt(A * A / B)`. A zero `b` gives +/-[`FMAX`]
/// with `a`'s sign; a zero `a` gives +0.
pub fn rsqrt(a: u32, b: u32) -> u32 {
    let (sa, ea, ma) = unpack(a);
    let (_, eb, mb) = unpack(b);
    if mb == 0 {
        return (sa << 31) | FMAX;
    }
    let (ma, mb) = (u128::from(ma), u128::from(mb));
    // The numerator's 2^120 and the denominator's 2^t cancel exactly, so
    // the quotient fits in 128 bits.
    let t = 60 + ((eb - BIAS - 60) & 1);
    let q = ((ma * ma) << (120 - t)) / mb;
    round(sa, q.isqrt(), ea - BIAS - 60 - (eb - BIAS - t) / 2)
}

/// `max.s` (and VU0 `vmax`): `a` when it is not below `b` by [`cmp`], else
/// `b`; of two equal values (+0 and -0, say) `a`'s pattern comes back.
pub fn max(a: u32, b: u32) -> u32 {
    if cmp(a, b) != Ordering::Less { a } else { b }
}

/// `min.s` (and VU0 `vmini`): `a` when it is not above `b` by [`cmp`],
/// else `b`; of two equal values `a`'s pattern comes back.
pub fn min(a: u32, b: u32) -> u32 {
    if cmp(a, b) != Ordering::Greater { a } else { b }
}

/// `cvt.s.w`: an `int` to a float, truncated toward zero to 24 bits (so
/// 16_777_217 gives 16_777_216.0, as does -16_777_217 with its sign).
pub fn from_int(i: i32) -> u32 {
    round(u32::from(i < 0), u128::from(i.unsigned_abs()), 0)
}

/// `cvt.w.s`: a float to an `int`, truncated toward zero and saturating at
/// `i32::MIN` / `i32::MAX`; exponent 0 gives 0.
pub fn to_int(v: u32) -> i32 {
    let (s, e, m) = unpack(v);
    let e = e - BIAS;
    let mag: u128 = if m == 0 {
        0
    } else if e >= 0 {
        if e > 40 { u128::MAX } else { u128::from(m) << e }
    } else if e <= -64 {
        0
    } else {
        u128::from(m >> -e)
    };
    if s == 1 && m != 0 {
        if mag > 0x8000_0000 { i32::MIN } else { (mag as i64).wrapping_neg() as i32 }
    } else if mag > 0x7fff_ffff {
        i32::MAX
    } else {
        mag as i32
    }
}

/// `c.eq.s` / `c.lt.s` / `c.le.s` as an ordering of exact values: every
/// exponent-0 pattern (either zero, any fraction) is equal to every other,
/// and exponent 255 is a number above the rest of its sign.
pub fn cmp(a: u32, b: u32) -> Ordering {
    key(a).cmp(&key(b))
}

/// A float's value order: for normalised numbers the pattern's magnitude
/// bits order like the value, exponent 255 included.
fn key(v: u32) -> i64 {
    if v & 0x7f80_0000 == 0 {
        return 0;
    }
    let mag = i64::from(v & !SIGN);
    if v & SIGN != 0 { -mag } else { mag }
}

/// `c.lt.s`: `a < b` by [`cmp`].
pub fn lt(a: u32, b: u32) -> bool {
    cmp(a, b) == Ordering::Less
}

/// `c.le.s`: `a <= b` by [`cmp`].
pub fn le(a: u32, b: u32) -> bool {
    cmp(a, b) != Ordering::Greater
}

/// newlib's `sqrtf` as the EE games link it (`__ieee754_sqrtf`, e.g. `INF
/// SLUS_202.67:0x00124ab0`): the library function, not the instruction. A
/// bit-by-bit square root rounding to nearest, unlike [`sqrt`]; exponent 0
/// comes back unchanged and a negative number gives [`FMAX`] (the EE's
/// 0/0).
pub fn sqrtf(v: u32) -> u32 {
    if v & 0x7f80_0000 == 0 {
        return v;
    }
    if v & SIGN != 0 {
        return FMAX;
    }
    let mut ix = (v >> 23) as i32 - 127;
    let mut m = u64::from((v & FRACTION) | HIDDEN) << (ix & 1);
    ix >>= 1;
    m <<= 1;
    let (mut q, mut s, mut r) = (0u64, 0u64, 0x0100_0000u64);
    while r != 0 {
        let t = s + r;
        if m >= t {
            m -= t;
            s = t + r;
            q += r;
        }
        m <<= 1;
        r >>= 1;
    }
    if m != 0 {
        q += q & 1;
    }
    ((q >> 1) as i64 + 0x3f00_0000 + (i64::from(ix) << 23)) as u32
}

/// An `f32` constant's bit pattern, for the constants code loads with
/// `lui` / `ori`.
pub const fn bits(x: f32) -> u32 {
    x.to_bits()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: u32 = 0x3f80_0000;

    #[test]
    fn results_truncate_toward_zero_where_ieee_rounds() {
        // 1 + 2^-24 is below 1's last bit: stays 1; 1 - 2^-24 truncates to
        // the float below 1, not back up to 1.
        let tiny = 0x3380_0000; // 2^-24
        assert_eq!(add(ONE, tiny), ONE);
        assert_eq!(sub(ONE, tiny), 0x3f7f_ffff);
        // 1/3 truncates (IEEE rounds to 0x3eaaaaab).
        assert_eq!(div(ONE, bits(3.0)), 0x3eaa_aaaa);
        assert_eq!(div(bits(-1.0), bits(3.0)), 0xbeaa_aaaa);
        // A product truncated: (1 + 2^-23)^2 = 1 + 2^-22 + 2^-46.
        assert_eq!(mul(0x3f80_0001, 0x3f80_0001), 0x3f80_0002);
        // madd truncates the product, then the sum.
        assert_eq!(madd(ONE, 0x3f80_0001, 0x3f80_0001), add(ONE, 0x3f80_0002));
        assert_eq!(msub(bits(2.0), ONE, ONE), ONE);
    }

    #[test]
    fn an_operand_far_below_the_other_leaves_it_or_steps_it_toward_zero() {
        let big = bits(1.0e30);
        assert_eq!(add(big, ONE), big);
        assert_eq!(sub(big, ONE), big - 1);
        assert_eq!(add(bits(-1.0e30), ONE), bits(-1.0e30) - 1);
        // A power of two drops into the binade below.
        assert_eq!(sub(0x7000_0000, ONE), 0x6fff_ffff);
    }

    #[test]
    fn overflow_clamps_to_the_largest_float_and_there_is_no_infinity() {
        assert_eq!(mul(0x7f00_0000, 0x7f00_0000), FMAX);
        assert_eq!(mul(0xff00_0000, 0x7f00_0000), SIGN | FMAX);
        assert_eq!(add(FMAX, FMAX), FMAX);
        assert_eq!(div(bits(-2.0), 0), SIGN | FMAX);
        assert_eq!(div(0, 0), FMAX);
        // Exponent 255 is a number, not an infinity or a NaN.
        assert_eq!(cmp(0x7f80_0000, 0x7f7f_ffff), Ordering::Greater);
        assert_eq!(sub(0x7f80_0000, 0x7f80_0000), 0);
    }

    #[test]
    fn denormals_are_zero_and_underflow_flushes_to_a_signed_zero() {
        // Exponent 0 is zero whatever the fraction.
        assert_eq!(add(0x0000_0001, ONE), ONE);
        assert_eq!(mul(0x0000_1234, ONE), 0);
        assert_eq!(cmp(0x0000_1234, 0x8000_0000), Ordering::Equal);
        assert_eq!(mul(0x0080_0000, 0x0080_0000), 0);
        assert_eq!(mul(0x8080_0000, 0x0080_0000), SIGN);
        // Zeros: -0 only from -0 + -0.
        assert_eq!(add(SIGN, SIGN), SIGN);
        assert_eq!(add(SIGN, 0), 0);
        assert_eq!(sub(ONE, ONE), 0);
    }

    #[test]
    fn conversions_truncate_and_saturate() {
        assert_eq!(from_int(600), bits(600.0));
        assert_eq!(from_int(-7), bits(-7.0));
        assert_eq!(from_int(0x0100_0001), bits(16_777_216.0));
        assert_eq!(from_int(-0x0100_0001), bits(-16_777_216.0));
        assert_eq!(from_int(i32::MIN), 0xcf00_0000);
        assert_eq!(from_int(-7), 0xc0e0_0000);
        assert_eq!(to_int(0xc0e0_0000), -7);
        assert_eq!(to_int(bits(27.99)), 27);
        assert_eq!(to_int(bits(-27.99)), -27);
        assert_eq!(to_int(bits(3.0e9)), i32::MAX);
        assert_eq!(to_int(bits(-3.0e9)), i32::MIN);
        assert_eq!(to_int(bits(0.5)), 0);
        assert_eq!(to_int(SIGN), 0);
    }

    #[test]
    fn comparisons_treat_both_zeros_as_equal() {
        assert_eq!(cmp(SIGN, 0), Ordering::Equal);
        assert_eq!(cmp(ONE, 0), Ordering::Greater);
        assert!(lt(bits(-1.0), 0) && le(SIGN, 0) && !lt(SIGN, 0));
        assert_eq!(max(SIGN, 0), SIGN, "equal values: the first operand");
        assert_eq!(min(bits(2.0), ONE), ONE);
        assert_eq!(max(bits(-2.0), ONE), ONE);
    }

    #[test]
    fn sqrt_truncates_and_newlib_sqrtf_rounds_to_nearest() {
        assert_eq!(sqrtf(bits(4.0)), bits(2.0));
        assert_eq!(sqrt(bits(4.0)), bits(2.0));
        // sqrt(2): newlib rounds to nearest (0x3fb504f3), sqrt.s truncates
        // to the same pattern; sqrt(5) is where they differ.
        assert_eq!(sqrtf(bits(2.0)), 0x3fb5_04f3);
        assert_eq!(sqrt(bits(2.0)), 0x3fb5_04f3);
        assert_eq!(sqrtf(bits(3.0)), 0x3fdd_b3d7);
        assert_eq!(sqrt(bits(3.0)), 0x3fdd_b3d7);
        assert_eq!(sqrtf(bits(5.0)), 0x400f_1bbd);
        assert_eq!(sqrt(bits(5.0)), 0x400f_1bbc);
        assert_eq!(sqrtf(bits(-4.0)), FMAX);
        assert_eq!(sqrt(bits(-4.0)), bits(2.0));
        assert_eq!(sqrtf(0x0000_0005), 0x0000_0005);
        assert_eq!(sqrt(0x0000_0005), 0);
    }

    #[test]
    fn rsqrt_divides_by_a_square_root_rounded_once() {
        let four = 0x4080_0000;
        let half = 0x3f00_0000;
        assert_eq!(rsqrt(ONE, four), half);
        assert_eq!(rsqrt(ONE, ONE), ONE);
        assert_eq!(rsqrt(0xbf80_0000, four), 0xbf00_0000);
        // Division by a zero (any exponent-0 pattern) is +/-Fmax.
        assert_eq!(rsqrt(ONE, 0x0000_1234), FMAX);
        assert_eq!(rsqrt(0xbf80_0000, 0), SIGN | FMAX);
        // A zero numerator is a zero.
        assert_eq!(rsqrt(0, four), 0);
    }
}
