//! The generators games used, and one for a port's own randomness.

use std::borrow::Cow;

use super::Generator;

/// A 32-bit linear congruential generator: `state = state * mul + add`
/// (wrapping), output `(state >> shift) & mask`. Most compilers' `rand` is
/// one; the constants say which.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Lcg {
    /// The state.
    pub state: u32,
    /// The multiplier.
    pub mul: u32,
    /// The increment.
    pub add: u32,
    /// How far the state is shifted right for the output.
    pub shift: u32,
    /// How many bits of the shifted state are output.
    pub out_bits: u32,
}

impl Lcg {
    /// The C standard's example `rand`, which many compilers and console
    /// libraries ship: multiplier 1103515245 (0x41c64e6d), increment 12345,
    /// output bits 16-30 (`RAND_MAX` 32767). Seeded with 1 it gives 16838,
    /// 5758, 10113.
    pub const ANSI_C: Lcg = Lcg { state: 1, mul: 1_103_515_245, add: 12_345, shift: 16, out_bits: 15 };

    /// Microsoft C's `rand`: multiplier 214013, increment 2531011, output
    /// bits 16-30. Seeded with 1 it gives 41, 18467, 6334.
    pub const MSVC: Lcg = Lcg { state: 1, mul: 214_013, add: 2_531_011, shift: 16, out_bits: 15 };

    /// These constants with `seed` as the state: `Lcg::ANSI_C.seeded(s)` is
    /// `srand(s)`.
    pub const fn seeded(self, seed: u32) -> Lcg {
        Lcg { state: seed, ..self }
    }

    /// Steps `n` times at once, in O(log n): the affine step composed by
    /// squaring. The same state as `n` calls to [`Generator::next`].
    pub fn jump(&mut self, mut n: u64) {
        // (mul, add) of the step applied 2^k times, accumulated into
        // (acc_mul, acc_add) for every set bit of n.
        let (mut m, mut a) = (self.mul, self.add);
        let (mut acc_m, mut acc_a) = (1u32, 0u32);
        while n > 0 {
            if n & 1 == 1 {
                acc_m = acc_m.wrapping_mul(m);
                acc_a = acc_a.wrapping_mul(m).wrapping_add(a);
            }
            a = a.wrapping_mul(m).wrapping_add(a);
            m = m.wrapping_mul(m);
            n >>= 1;
        }
        self.state = self.state.wrapping_mul(acc_m).wrapping_add(acc_a);
    }
}

impl Generator for Lcg {
    type State = u32;

    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(self.mul).wrapping_add(self.add);
        (self.state >> self.shift) & mask(self.out_bits)
    }

    fn bits(&self) -> u32 {
        self.out_bits
    }

    fn state(&self) -> u32 {
        self.state
    }

    fn set_state(&mut self, state: u32) {
        self.state = state;
    }
}

/// The low `bits` bits set.
const fn mask(bits: u32) -> u32 {
    if bits >= 32 { u32::MAX } else { (1 << bits) - 1 }
}

/// A 64-bit-state linear congruential generator: `state = state * mul + add`
/// (wrapping), output `(state >> shift) & mask`, at most 32 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Lcg64 {
    /// The state.
    pub state: u64,
    /// The multiplier.
    pub mul: u64,
    /// The increment.
    pub add: u64,
    /// How far the state is shifted right for the output.
    pub shift: u32,
    /// How many bits of the shifted state are output, at most 32.
    pub out_bits: u32,
}

impl Lcg64 {
    /// newlib's `rand` (the C library of many GCC console toolchains): the
    /// 64-bit state stepped by multiplier 6364136223846793005 and increment
    /// 1, output bits 32-62 (`RAND_MAX` 0x7fffffff). `srand(s)` sets the
    /// state to `s`.
    pub const NEWLIB: Lcg64 = Lcg64 { state: 1, mul: 6_364_136_223_846_793_005, add: 1, shift: 32, out_bits: 31 };

    /// These constants with `seed` as the state.
    pub const fn seeded(self, seed: u64) -> Lcg64 {
        Lcg64 { state: seed, ..self }
    }
}

impl Generator for Lcg64 {
    type State = u64;

    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(self.mul).wrapping_add(self.add);
        ((self.state >> self.shift) as u32) & mask(self.out_bits.min(32))
    }

    fn bits(&self) -> u32 {
        self.out_bits.min(32)
    }

    fn state(&self) -> u64 {
        self.state
    }

    fn set_state(&mut self, state: u64) {
        self.state = state;
    }
}

/// Marsaglia's 32-bit xorshift: `x ^= x << a; x ^= x >> b; x ^= x << c`,
/// output the state. The shifts default to 13, 17, 5. A state of 0 stays 0,
/// so it is never seeded with 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Xorshift32 {
    /// The state; never 0.
    pub state: u32,
    /// The three shifts.
    pub shifts: (u32, u32, u32),
}

impl Xorshift32 {
    /// Seeded with `seed`, or 1 for 0; shifts 13, 17, 5.
    pub const fn new(seed: u32) -> Xorshift32 {
        Xorshift32 { state: if seed == 0 { 1 } else { seed }, shifts: (13, 17, 5) }
    }
}

impl Generator for Xorshift32 {
    type State = u32;

    fn next(&mut self) -> u32 {
        let (a, b, c) = self.shifts;
        let mut x = self.state;
        x ^= x << a;
        x ^= x >> b;
        x ^= x << c;
        self.state = x;
        x
    }

    fn bits(&self) -> u32 {
        32
    }

    fn state(&self) -> u32 {
        self.state
    }

    fn set_state(&mut self, state: u32) {
        self.state = state;
    }
}

/// A Galois linear-feedback shift register, `width` bits: each step shifts
/// right one and, when the bit shifted out was 1, XORs in `taps`. Output is
/// the new state. With maximal taps it visits every non-zero state before
/// repeating; 16-bit with taps 0xb400 is the textbook one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Lfsr {
    /// The register; never 0 for a maximal sequence.
    pub state: u32,
    /// The feedback taps, within `width`.
    pub taps: u32,
    /// Bits in the register, 1-32.
    pub width: u32,
}

impl Lfsr {
    /// A `width`-bit register with `taps`, starting at `seed`.
    pub const fn new(seed: u32, taps: u32, width: u32) -> Lfsr {
        Lfsr { state: seed & mask(width), taps: taps & mask(width), width }
    }

    /// One step returning only the bit shifted out: for games that build
    /// their numbers a bit at a time.
    pub fn next_bit(&mut self) -> u32 {
        let out = self.state & 1;
        self.state >>= 1;
        if out == 1 {
            self.state ^= self.taps;
        }
        out
    }
}

impl Generator for Lfsr {
    type State = u32;

    fn next(&mut self) -> u32 {
        self.next_bit();
        self.state
    }

    fn bits(&self) -> u32 {
        self.width
    }

    fn state(&self) -> u32 {
        self.state
    }

    fn set_state(&mut self, state: u32) {
        self.state = state & mask(self.width);
    }
}

/// Values read in turn from a fixed table, wrapping at its end: the
/// table-driven randomness of games that shipped one. The state is the
/// index. The table is the game's, supplied by the port (from the
/// executable, say).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TableRng {
    /// The values, read in order.
    pub table: Cow<'static, [u8]>,
    /// The index of the next value.
    pub index: usize,
    /// Step the index before reading (true, DOOM's `P_Random` order) or
    /// after (false).
    pub step_first: bool,
}

impl TableRng {
    /// Reads `table` from index 0, stepping before each read.
    ///
    /// # Panics
    ///
    /// When `table` is empty.
    pub fn new(table: impl Into<Cow<'static, [u8]>>) -> TableRng {
        let table = table.into();
        assert!(!table.is_empty(), "an empty random table");
        TableRng { table, index: 0, step_first: true }
    }
}

impl Generator for TableRng {
    type State = usize;

    fn next(&mut self) -> u32 {
        let len = self.table.len();
        if self.step_first {
            self.index = (self.index + 1) % len;
            u32::from(self.table[self.index])
        } else {
            let v = self.table[self.index % len];
            self.index = (self.index + 1) % len;
            u32::from(v)
        }
    }

    fn bits(&self) -> u32 {
        8
    }

    fn state(&self) -> usize {
        self.index
    }

    fn set_state(&mut self, index: usize) {
        self.index = index % self.table.len();
    }
}

/// O'Neill's PCG32 (XSH RR, 64-bit state, odd increment): good statistics
/// and small state, for randomness a port adds of its own rather than
/// randomness it must reproduce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    const MUL: u64 = 6_364_136_223_846_793_005;

    /// Seeded as the reference `pcg32_srandom(seed, stream)` does: `stream`
    /// picks one of 2^63 independent sequences.
    pub fn new(seed: u64, stream: u64) -> Pcg32 {
        let mut p = Pcg32 { state: 0, inc: (stream << 1) | 1 };
        p.next();
        p.state = p.state.wrapping_add(seed);
        p.next();
        p
    }
}

impl Generator for Pcg32 {
    type State = (u64, u64);

    fn next(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(Self::MUL).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    fn bits(&self) -> u32 {
        32
    }

    fn state(&self) -> (u64, u64) {
        (self.state, self.inc)
    }

    fn set_state(&mut self, (state, inc): (u64, u64)) {
        self.state = state;
        self.inc = inc | 1;
    }
}
