//! The R5900 (the EE's CPU core) as an interpreter behind [`Cpu`]: piney_apples'
//! `piney-eemu` `cpu.rs` ported instruction for instruction. That is
//! `tools/eemu.py`'s machine, with `tools/test_anim.py`'s VuMachine and
//! `tools/test_stream_rs.py`'s Vu0Machine behind [`Features`], so a machine
//! stops on the instructions its piney counterpart stops on.
//!
//! The state is held as piney holds it: 128-bit GPRs, HI / LO and HI1 / LO1
//! whole, raw `u32` FPU and VU0 registers. Nothing checks alignment (`lw`
//! from an odd address reads the four bytes there; `lq` / `sq` clear the low
//! four address bits), and `add` / `addi` / `sub` / `dadd` wrap as their
//! unsigned forms do: no [`Stop::Overflow`], no [`Stop::Address`]. Memory is
//! the [`Bus`]; VU0's data memory is reached through it at [`VU0_DATA`].
//!
//! Where it differs from piney: `syscall` stops with [`Stop::Syscall`]
//! (complete, the program counter past it) and `break` with [`Stop::Break`]
//! (not executed), where piney stops on both as uninterpreted words; every
//! other word piney does not interpret stops with [`Stop::Reserved`],
//! leaving the program counter on it.
//!
//! Not here: exceptions and COP0, the TLB, the caches, `lwl` / `lwr` /
//! `swl` / `swr` / `sdl` / `sdr`, `mfsa` / `mtsa` / `qfsrv`, most of MMI,
//! VU0 micro mode and its flags, the instructions' timing (a step is one
//! cycle) - none of which piney interprets.

use crate::bus::{Bus, BusExt};
use crate::cpu::{Abi, Cpu, Endian, ReturnAddress, Stop};

use super::float;

/// `$sp` at a call's start: 32 MB less 4 KB (piney's `STACK_TOP`).
pub const STACK_TOP: u32 = (32 << 20) - 0x1000;
/// `$ra` at a call's start: returning there ends the call (piney's
/// `RETURN_SENTINEL`, the same as [`crate::Machine::return_to`]'s default).
pub const RETURN_SENTINEL: u32 = 0xffff_fff0;
/// Where the EE sees VU0's data memory: 0x1100_4000, [`VU0_DATA_SIZE`] bytes
/// (256 quadwords). The VU0 load / store instructions behind
/// [`Features::vi`] address it by quadword, wrapping at 256.
pub const VU0_DATA: u32 = 0x1100_4000;
/// VU0 data memory's size in bytes.
pub const VU0_DATA_SIZE: u32 = 4096;
/// FCR31's condition bit (bit 23): what `c.*.s` set and `bc1t` / `bc1f`
/// test.
pub const C_BIT: u32 = 1 << 23;

/// Register index of HI (pipeline 0's, 128 bits as piney holds it).
pub const HI: usize = 32;
/// Register index of LO.
pub const LO: usize = 33;
/// Register index of HI1, the second multiply / divide pipeline's HI.
pub const HI1: usize = 34;
/// Register index of LO1.
pub const LO1: usize = 35;
/// Register index of SA, the funnel shift amount (held; no interpreted
/// instruction reads or writes it).
pub const SA: usize = 36;
/// Register index of f0; f1-f31 follow.
pub const F0: usize = 37;
/// Register index of FCR31, the FPU control / status word.
pub const FCR31: usize = 69;
/// Register index of the FPU accumulator (`adda.s`, `madd.s` ...).
pub const ACC: usize = 70;
/// Register index of vf0 (VU0's float vectors, x in bits 0-31 up to w in
/// bits 96-127); vf1-vf31 follow. Present when [`Features::vu`] is.
pub const VF0: usize = 71;
/// Register index of VU0's accumulator, laid out as a vf register.
pub const VACC: usize = 103;
/// Register index of VU0's Q register (a float's bits).
pub const Q: usize = 104;
/// Register index of vi0, VU0's 16-bit integer registers; vi1-vi15 follow.
pub const VI0: usize = 105;
/// Registers without the VU0 ones.
pub const REGS: usize = 71;
/// Registers with the VU0 ones ([`Features::vu`]).
pub const REGS_VU: usize = 121;

const M64: u128 = u64::MAX as u128;
const ONE: u32 = 0x3f80_0000;

/// Which of piney's machines this one is: each flag widens the instruction
/// set or changes a behaviour piney's harnesses rely on. All off is
/// `tools/eemu.py`'s plain machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    /// test_anim's VuMachine: COP2 in macro mode (VU0 arithmetic,
    /// `qmfc2` / `qmtc2`), `lqc2` / `sqc2`, `ldl` / `ldr`, and the MMI
    /// instructions `pextlw`, `pextuw`, `pcpyld`, `pcpyud`, `pextlh`,
    /// `psubh`, `paddsh`, `ppach`, `psraw`, `pmthi`, `pmtlo`, `pmaddh`. Also
    /// exposes the VU0 registers ([`REGS_VU`]).
    pub vu: bool,
    /// test_stream_rs's Vu0Machine on top of `vu`: vi0-vi15, VU0 data
    /// memory (at [`VU0_DATA`] on the bus), `cfc2` / `ctc2`, `viadd` /
    /// `viaddi`, `vlqi` / `vsqi` / `vlqd` / `vsqd`.
    pub vi: bool,
    /// test_toppage_rs's `ee_machine`: `div` / `divu` by zero set LO to -1
    /// (or 1 for `div` of a negative dividend) and HI to the dividend, as
    /// the EE does, instead of leaving them unchanged.
    pub ee_div: bool,
}

/// What an instruction does to control flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Next,
    /// Jump there after the delay slot.
    Jump(u32),
    /// A branch-likely not taken: the delay slot is skipped.
    Annul,
    /// `syscall` with its code: complete, the run stops past it.
    Syscall(u32),
    /// `break` with its code: the run stops on it.
    Break(u32),
}

/// VU0 lane arithmetic, test_anim's `arith` kinds.
#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    Madd,
    Msub,
    Max,
    Mini,
    Mul,
}

const BC_OPS: [Op; 7] = [Op::Add, Op::Sub, Op::Madd, Op::Msub, Op::Max, Op::Mini, Op::Mul];

/// The MMI functs the VuMachine takes over from the base machine's MMI.
fn vu_mmi(funct: u32) -> bool {
    matches!(funct, 0x08 | 0x09 | 0x28 | 0x29 | 0x34 | 0x36 | 0x37 | 0x3c | 0x3e | 0x3f)
}

/// The low word sign-extended to 64 bits, as the EE holds 32-bit results.
fn sext32(v: u64) -> u128 {
    u128::from(v as u32 as i32 as i64 as u64)
}

fn words(v: u128) -> [u32; 4] {
    std::array::from_fn(|k| (v >> (32 * k)) as u32)
}

fn from_words(w: [u32; 4]) -> u128 {
    w.iter().rev().fold(0, |v, &x| (v << 32) | u128::from(x))
}

fn halves(v: u128) -> [u16; 8] {
    std::array::from_fn(|k| (v >> (16 * k)) as u16)
}

fn from_halves(h: [u16; 8]) -> u128 {
    h.iter().rev().fold(0, |v, &x| (v << 16) | u128::from(x))
}

const GPR_NAMES: [&str; 32] = [
    "zero", "at", "v0", "v1", "a0", "a1", "a2", "a3", "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7", "s0", "s1", "s2",
    "s3", "s4", "s5", "s6", "s7", "t8", "t9", "k0", "k1", "gp", "sp", "fp", "ra",
];

const SPECIAL_NAMES: [&str; 5] = ["hi", "lo", "hi1", "lo1", "sa"];

const FPR_NAMES: [&str; 32] = [
    "f0", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "f13", "f14", "f15", "f16", "f17",
    "f18", "f19", "f20", "f21", "f22", "f23", "f24", "f25", "f26", "f27", "f28", "f29", "f30", "f31",
];

const VF_NAMES: [&str; 32] = [
    "vf0", "vf1", "vf2", "vf3", "vf4", "vf5", "vf6", "vf7", "vf8", "vf9", "vf10", "vf11", "vf12", "vf13", "vf14",
    "vf15", "vf16", "vf17", "vf18", "vf19", "vf20", "vf21", "vf22", "vf23", "vf24", "vf25", "vf26", "vf27", "vf28",
    "vf29", "vf30", "vf31",
];

const VI_NAMES: [&str; 16] = [
    "vi0", "vi1", "vi2", "vi3", "vi4", "vi5", "vi6", "vi7", "vi8", "vi9", "vi10", "vi11", "vi12", "vi13", "vi14",
    "vi15",
];

/// The Emotion Engine's R5900 core.
///
/// Registers, as [`Cpu::reg`] numbers them (each read as `u128`,
/// zero-extended):
///
/// | index | names | width |
/// |---|---|---|
/// | 0-31 | `zero at v0 v1 a0-a3 t0-t7 s0-s7 t8 t9 k0 k1 gp sp fp ra` | 128 |
/// | 32-36 | `hi lo hi1 lo1 sa` ([`HI`], [`LO`], [`HI1`], [`LO1`], [`SA`]) | 128, `sa` 32 |
/// | 37-68 | `f0`-`f31` ([`F0`]), raw float bits | 32 |
/// | 69, 70 | `fcr31`, `acc` ([`FCR31`], [`ACC`]) | 32 |
/// | 71-102 | `vf0`-`vf31` ([`VF0`]), x in bits 0-31 .. w in 96-127 | 128 |
/// | 103, 104 | `vacc`, `q` ([`VACC`], [`Q`]) | 128, 32 |
/// | 105-120 | `vi0`-`vi15` ([`VI0`]) | 16 |
///
/// Indices 71 on exist only with [`Features::vu`] ([`Cpu::reg_count`] is
/// [`REGS`] or [`REGS_VU`]). GPR names are the MIPS ones; the EABI passes
/// arguments five to eight in r8-r11, which [`Cpu::reg_index`] also finds
/// as `a4`-`a7` (and `s8` for `fp`, `rN` for any GPR, a leading `$`
/// ignored). Writes to `zero`, `vf0` (always 0, 0, 0, 1) and `vi0` are
/// ignored.
#[derive(Clone, Debug)]
pub struct Ee {
    /// The general purpose registers, 128 bits each; most instructions work
    /// on the low 64 and leave the upper 64 alone.
    pub r: [u128; 32],
    /// HI: pipeline 0's (`mult`, `div`, `mfhi`); `mthi` copies all 128 bits.
    pub hi: u128,
    /// LO: pipeline 0's.
    pub lo: u128,
    /// HI1: pipeline 1's (`mult1`, `div1`, `mfhi1`).
    pub hi1: u128,
    /// LO1: pipeline 1's.
    pub lo1: u128,
    /// SA, the funnel shift amount (held for completeness).
    pub sa: u32,
    /// The FPU registers, raw single-float bits ([`super::float`]).
    pub f: [u32; 32],
    /// FCR31: the condition bit is [`C_BIT`]. `cfc1` of FCR0 reads 0x2e30.
    pub fcr31: u32,
    /// The FPU accumulator, raw float bits.
    pub acc: u32,
    /// VU0's float registers, lanes x, y, z, w as raw float bits.
    pub vf: [[u32; 4]; 32],
    /// VU0's accumulator, lanes x, y, z, w.
    pub vacc: [u32; 4],
    /// VU0's Q register (`vdiv`, `vsqrt`, `vrsqrt`), raw float bits.
    pub q: u32,
    /// VU0's integer registers.
    pub vi: [u16; 16],
    /// Which instructions it interprets.
    pub features: Features,
    pc: u32,
    npc: u32,
}

impl Ee {
    /// A reset core with `features`: every register 0 but vf0 (0, 0, 0,
    /// 1.0), the program counter at 0.
    pub fn new(features: Features) -> Ee {
        let mut vf = [[0; 4]; 32];
        vf[0] = [0, 0, 0, ONE];
        Ee {
            r: [0; 32],
            hi: 0,
            lo: 0,
            hi1: 0,
            lo1: 0,
            sa: 0,
            f: [0; 32],
            fcr31: 0,
            acc: 0,
            vf,
            vacc: [0; 4],
            q: 0,
            vi: [0; 16],
            features,
            pc: 0,
            npc: 4,
        }
    }

    /// The address of the instruction after the next: the next one's
    /// successor, or a branch's target while its delay slot is next.
    pub fn next_pc(&self) -> u32 {
        self.npc
    }

    fn a32(&self, i: usize) -> i32 {
        self.r[i] as u32 as i32
    }

    fn u32r(&self, i: usize) -> u32 {
        self.r[i] as u32
    }

    fn a64(&self, i: usize) -> i64 {
        self.r[i] as u64 as i64
    }

    fn m64(&self, i: usize) -> u64 {
        self.r[i] as u64
    }

    /// Writes the low 64 bits of GPR `i`, keeping the upper 64.
    pub fn set(&mut self, i: usize, v: u64) {
        if i != 0 {
            self.r[i] = (self.r[i] & !M64) | u128::from(v);
        }
    }

    /// Writes all 128 bits of GPR `i`.
    pub fn set128(&mut self, i: usize, v: u128) {
        if i != 0 {
            self.r[i] = v;
        }
    }

    /// Writes a 32-bit result to GPR `i`, sign-extended to 64 bits, the
    /// upper 64 kept.
    pub fn set32(&mut self, i: usize, v: u32) {
        self.set(i, v as i32 as i64 as u64);
    }

    /// Writes the lanes of vf`i` that `mask` names (x = 8, y = 4, z = 2,
    /// w = 1); vf0 is never written.
    pub fn vset(&mut self, i: usize, mask: u32, vals: [u32; 4]) {
        if i != 0 {
            for (k, v) in vals.into_iter().enumerate() {
                if mask & (8 >> k) != 0 {
                    self.vf[i][k] = v;
                }
            }
        }
    }

    /// Writes vi`i & 15` (16 bits); vi0 stays 0.
    fn vset_i(&mut self, i: u32, v: u32) {
        if i & 15 != 0 {
            self.vi[(i & 15) as usize] = v as u16;
        }
    }

    /// The effective address of a load or store: `rs` + the sign-extended
    /// immediate, in 32 bits.
    fn ea(&self, w: u32) -> u32 {
        (i64::from(self.a32(((w >> 21) & 31) as usize)) + i64::from(w as u16 as i16)) as u32
    }

    fn advance(&mut self, flow: Flow) {
        match flow {
            Flow::Jump(t) => (self.pc, self.npc) = (self.npc, t),
            Flow::Annul => (self.pc, self.npc) = (self.npc.wrapping_add(4), self.npc.wrapping_add(8)),
            Flow::Next | Flow::Syscall(_) | Flow::Break(_) => {
                (self.pc, self.npc) = (self.npc, self.npc.wrapping_add(4))
            }
        }
    }

    /// One instruction with the machine's features.
    fn exec(&mut self, bus: &mut dyn Bus, pc: u32, w: u32) -> Result<Flow, Stop> {
        let op = w >> 26;
        let be = move |addr| Stop::Bus { pc, addr };
        if self.features.ee_div && op == 0 && matches!(w & 63, 0x1a | 0x1b) {
            let (rs, rt) = (((w >> 21) & 31) as usize, ((w >> 16) & 31) as usize);
            if self.u32r(rt) == 0 {
                let n = self.a32(rs);
                let q: i64 = if w & 63 == 0x1a && n < 0 { 1 } else { -1 };
                self.lo = u128::from(q as u64);
                self.hi = u128::from(i64::from(n) as u64);
                return Ok(Flow::Next);
            }
        }
        if self.features.vu {
            match op {
                0x12 => return self.cop2(bus, pc, w),
                0x36 => {
                    // lqc2
                    let v = bus.read_u128(self.ea(w) & !15).map_err(be)?;
                    self.vset(((w >> 16) & 31) as usize, 0xf, words(v));
                    return Ok(Flow::Next);
                }
                0x3e => {
                    // sqc2
                    let v = from_words(self.vf[((w >> 16) & 31) as usize]);
                    bus.write_u128(self.ea(w) & !15, v).map_err(be)?;
                    return Ok(Flow::Next);
                }
                0x1a | 0x1b => {
                    // ldl / ldr
                    let (rt, ea) = (((w >> 16) & 31) as usize, self.ea(w));
                    let k = ea & 7;
                    let dw = bus.read_u64(ea & !7).map_err(be)?;
                    let old = self.m64(rt);
                    let v = if op == 0x1a {
                        let sh = 8 * (7 - k);
                        (dw << sh) | (old & ((1u64 << sh) - 1))
                    } else {
                        let sh = 8 * k;
                        (dw >> sh) | (old & if sh == 0 { 0 } else { u64::MAX << (64 - sh) })
                    };
                    self.set(rt, v);
                    return Ok(Flow::Next);
                }
                0x1c if vu_mmi(w & 63) => return self.mmi(pc, w),
                _ => {}
            }
        }
        self.exec_ee(bus, pc, w)
    }

    fn branch(pc: u32, simm: i64, cond: bool, likely: bool) -> Flow {
        if cond {
            Flow::Jump(i64::from(pc).wrapping_add(4).wrapping_add(simm << 2) as u32)
        } else if likely {
            Flow::Annul
        } else {
            Flow::Next
        }
    }

    /// `(lo, hi)` of mult / multu (either pipeline): each half of the
    /// product sign-extended from 32 bits.
    fn multiply(&self, signed: bool, rs: usize, rt: usize) -> (u128, u128) {
        let (v, hi) = if signed {
            let v = i64::from(self.a32(rs)) * i64::from(self.a32(rt));
            (v as u64, (v >> 32) as u64)
        } else {
            let v = u64::from(self.u32r(rs)) * u64::from(self.u32r(rt));
            (v, v >> 32)
        };
        (sext32(v), sext32(hi))
    }

    /// `(lo, hi)` of div / divu (either pipeline), the quotient truncated
    /// toward zero; None for a zero divisor, which leaves LO and HI alone.
    fn divide(&self, signed: bool, rs: usize, rt: usize) -> Option<(u128, u128)> {
        let (n, d) = if signed {
            (i64::from(self.a32(rs)), i64::from(self.a32(rt)))
        } else {
            (i64::from(self.u32r(rs)), i64::from(self.u32r(rt)))
        };
        if d == 0 {
            return None;
        }
        let q = n / d;
        Some((sext32(q as u64), sext32((n - q * d) as u64)))
    }

    /// The base machine (`tools/eemu.py`'s `exec`).
    fn exec_ee(&mut self, bus: &mut dyn Bus, pc: u32, w: u32) -> Result<Flow, Stop> {
        let op = w >> 26;
        let rs = ((w >> 21) & 31) as usize;
        let rt = ((w >> 16) & 31) as usize;
        let rd = ((w >> 11) & 31) as usize;
        let sa = (w >> 6) & 31;
        let imm = w & 0xffff;
        let simm = i64::from(imm as u16 as i16);
        let ea = (i64::from(self.a32(rs)) + simm) as u32;
        let be = move |addr| Stop::Bus { pc, addr };
        let bad = Err(Stop::Reserved { pc, word: u64::from(w) });
        let store = |bus: &mut dyn Bus, at: u32, n: usize, v: u128| bus.write(at, &v.to_le_bytes()[..n]).map_err(be);

        match op {
            0 => {
                match w & 63 {
                    0x00 => self.set32(rd, self.u32r(rt) << sa),
                    0x02 => self.set32(rd, self.u32r(rt) >> sa),
                    0x03 => self.set32(rd, (self.a32(rt) >> sa) as u32),
                    0x04 => self.set32(rd, self.u32r(rt) << (self.r[rs] as u32 & 31)),
                    0x06 => self.set32(rd, self.u32r(rt) >> (self.r[rs] as u32 & 31)),
                    0x07 => self.set32(rd, (self.a32(rt) >> (self.r[rs] as u32 & 31)) as u32),
                    0x08 => return Ok(Flow::Jump(self.u32r(rs))),
                    0x09 => {
                        // The link is written first: jalr rd == rs jumps to it.
                        self.set(rd, u64::from(pc.wrapping_add(8)));
                        return Ok(Flow::Jump(self.u32r(rs)));
                    }
                    0x0a => {
                        if self.m64(rt) == 0 {
                            self.set(rd, self.m64(rs));
                        }
                    }
                    0x0b => {
                        if self.m64(rt) != 0 {
                            self.set(rd, self.m64(rs));
                        }
                    }
                    0x0c => return Ok(Flow::Syscall((w >> 6) & 0xf_ffff)),
                    0x0d => return Ok(Flow::Break((w >> 6) & 0xf_ffff)),
                    0x0f => {}
                    0x10 => self.set(rd, self.hi as u64),
                    0x12 => self.set(rd, self.lo as u64),
                    0x11 => self.hi = self.r[rs],
                    0x13 => self.lo = self.r[rs],
                    f @ (0x18 | 0x19) => {
                        (self.lo, self.hi) = self.multiply(f == 0x18, rs, rt);
                        self.set(rd, self.lo as u64);
                    }
                    f @ (0x1a | 0x1b) => {
                        if let Some((lo, hi)) = self.divide(f == 0x1a, rs, rt) {
                            (self.lo, self.hi) = (lo, hi);
                        }
                    }
                    0x20 | 0x21 => self.set32(rd, self.a32(rs).wrapping_add(self.a32(rt)) as u32),
                    0x22 | 0x23 => self.set32(rd, self.a32(rs).wrapping_sub(self.a32(rt)) as u32),
                    0x24 => self.set(rd, self.m64(rs) & self.m64(rt)),
                    0x25 => self.set(rd, self.m64(rs) | self.m64(rt)),
                    0x26 => self.set(rd, self.m64(rs) ^ self.m64(rt)),
                    0x27 => self.set(rd, !(self.m64(rs) | self.m64(rt))),
                    0x2a => self.set(rd, u64::from(self.a64(rs) < self.a64(rt))),
                    0x2b => self.set(rd, u64::from(self.m64(rs) < self.m64(rt))),
                    0x14 => self.set(rd, self.m64(rt) << (self.r[rs] as u32 & 63)),
                    0x16 => self.set(rd, self.m64(rt) >> (self.r[rs] as u32 & 63)),
                    0x17 => self.set(rd, (self.a64(rt) >> (self.r[rs] as u32 & 63)) as u64),
                    0x2c | 0x2d => self.set(rd, self.m64(rs).wrapping_add(self.m64(rt))),
                    0x2e | 0x2f => self.set(rd, self.m64(rs).wrapping_sub(self.m64(rt))),
                    0x38 => self.set(rd, self.m64(rt) << sa),
                    0x3a => self.set(rd, self.m64(rt) >> sa),
                    0x3b => self.set(rd, (self.a64(rt) >> sa) as u64),
                    0x3c => self.set(rd, self.m64(rt) << (sa + 32)),
                    0x3e => self.set(rd, self.m64(rt) >> (sa + 32)),
                    0x3f => self.set(rd, (self.a64(rt) >> (sa + 32)) as u64),
                    _ => return bad,
                }
                Ok(Flow::Next)
            }
            1 => {
                let cond = match rt {
                    0 | 2 | 0x10 => self.a64(rs) < 0,
                    1 | 3 | 0x11 => self.a64(rs) >= 0,
                    _ => return bad,
                };
                if rt >= 0x10 {
                    self.set(31, u64::from(pc.wrapping_add(8)));
                }
                Ok(Self::branch(pc, simm, cond, rt == 2 || rt == 3))
            }
            2 | 3 => {
                if op == 3 {
                    self.set(31, u64::from(pc.wrapping_add(8)));
                }
                Ok(Flow::Jump((pc.wrapping_add(4) & 0xf000_0000) | ((w & 0x03ff_ffff) << 2)))
            }
            4..=7 | 0x14..=0x17 => {
                let cond = match op & 3 {
                    0 => self.m64(rs) == self.m64(rt),
                    1 => self.m64(rs) != self.m64(rt),
                    2 => self.a64(rs) <= 0,
                    _ => self.a64(rs) > 0,
                };
                Ok(Self::branch(pc, simm, cond, op >= 0x14))
            }
            0x08 | 0x09 => {
                self.set32(rt, (i64::from(self.a32(rs)) + simm) as u32);
                Ok(Flow::Next)
            }
            0x0a => {
                self.set(rt, u64::from(self.a64(rs) < simm));
                Ok(Flow::Next)
            }
            0x0b => {
                self.set(rt, u64::from(self.m64(rs) < simm as u64));
                Ok(Flow::Next)
            }
            0x0c => {
                self.set(rt, self.m64(rs) & u64::from(imm));
                Ok(Flow::Next)
            }
            0x0d => {
                self.set(rt, self.m64(rs) | u64::from(imm));
                Ok(Flow::Next)
            }
            0x0e => {
                self.set(rt, self.m64(rs) ^ u64::from(imm));
                Ok(Flow::Next)
            }
            0x0f => {
                self.set32(rt, imm << 16);
                Ok(Flow::Next)
            }
            0x18 | 0x19 => {
                self.set(rt, self.a64(rs).wrapping_add(simm) as u64);
                Ok(Flow::Next)
            }
            0x1c => {
                // MMI: only pipeline 1's multiply, divide and moves.
                match w & 63 {
                    f @ (0x18 | 0x19) => {
                        (self.lo1, self.hi1) = self.multiply(f == 0x18, rs, rt);
                        self.set(rd, self.lo1 as u64);
                    }
                    f @ (0x1a | 0x1b) => {
                        if let Some((lo, hi)) = self.divide(f == 0x1a, rs, rt) {
                            (self.lo1, self.hi1) = (lo, hi);
                        }
                    }
                    0x10 => self.set(rd, self.hi1 as u64),
                    0x12 => self.set(rd, self.lo1 as u64),
                    0x11 => self.hi1 = self.r[rs],
                    0x13 => self.lo1 = self.r[rs],
                    _ => return bad,
                }
                Ok(Flow::Next)
            }
            0x11 => {
                match rs {
                    0x00 => self.set32(rt, self.f[rd]),
                    0x02 => self.set32(
                        rt,
                        if rd == 31 {
                            self.fcr31
                        } else if rd == 0 {
                            0x2e30
                        } else {
                            0
                        },
                    ),
                    0x04 => self.f[rd] = self.u32r(rt),
                    0x06 => {
                        if rd == 31 {
                            self.fcr31 = self.u32r(rt);
                        }
                    }
                    0x08 => {
                        if rt > 3 {
                            return bad;
                        }
                        let cond = (self.fcr31 & C_BIT != 0) == (rt & 1 != 0);
                        return Ok(Self::branch(pc, simm, cond, rt >= 2));
                    }
                    0x10 => return self.cop1_s(pc, w, rt, rd, sa as usize),
                    0x14 if w & 63 == 0x20 => self.f[sa as usize] = float::from_int(self.f[rd] as i32),
                    _ => return bad,
                }
                Ok(Flow::Next)
            }
            0x20 => {
                let v = bus.read_u8(ea).map_err(be)? as i8;
                self.set(rt, i64::from(v) as u64);
                Ok(Flow::Next)
            }
            0x24 => {
                self.set(rt, u64::from(bus.read_u8(ea).map_err(be)?));
                Ok(Flow::Next)
            }
            0x21 => {
                let v = bus.read_u16(ea).map_err(be)? as i16;
                self.set(rt, i64::from(v) as u64);
                Ok(Flow::Next)
            }
            0x25 => {
                self.set(rt, u64::from(bus.read_u16(ea).map_err(be)?));
                Ok(Flow::Next)
            }
            0x23 => {
                self.set32(rt, bus.read_u32(ea).map_err(be)?);
                Ok(Flow::Next)
            }
            0x27 => {
                self.set(rt, u64::from(bus.read_u32(ea).map_err(be)?));
                Ok(Flow::Next)
            }
            0x37 => {
                self.set(rt, bus.read_u64(ea).map_err(be)?);
                Ok(Flow::Next)
            }
            0x1e => {
                self.set128(rt, bus.read_u128(ea & !15).map_err(be)?);
                Ok(Flow::Next)
            }
            0x28 => store(bus, ea, 1, self.r[rt]).map(|_| Flow::Next),
            0x29 => store(bus, ea, 2, self.r[rt]).map(|_| Flow::Next),
            0x2b => store(bus, ea, 4, self.r[rt]).map(|_| Flow::Next),
            0x3f => store(bus, ea, 8, self.r[rt]).map(|_| Flow::Next),
            0x1f => store(bus, ea & !15, 16, self.r[rt]).map(|_| Flow::Next),
            0x31 => {
                self.f[rt] = bus.read_u32(ea).map_err(be)?;
                Ok(Flow::Next)
            }
            0x39 => store(bus, ea, 4, u128::from(self.f[rt])).map(|_| Flow::Next),
            // cache, pref
            0x2f | 0x33 => Ok(Flow::Next),
            _ => bad,
        }
    }

    /// COP1's S format; the rt, rd and sa fields are ft, fs and fd.
    fn cop1_s(&mut self, pc: u32, w: u32, ft: usize, fs: usize, fd: usize) -> Result<Flow, Stop> {
        let (a, b) = (self.f[fs], self.f[ft]);
        match w & 63 {
            0x00 => self.f[fd] = float::add(a, b),
            0x01 => self.f[fd] = float::sub(a, b),
            0x02 => self.f[fd] = float::mul(a, b),
            0x03 => self.f[fd] = float::div(a, b),
            // The EE takes sqrt.s's operand from ft.
            0x04 => self.f[fd] = float::sqrt(b),
            0x05 => self.f[fd] = a & 0x7fff_ffff,
            0x06 => self.f[fd] = a,
            0x07 => self.f[fd] = a ^ 0x8000_0000,
            0x16 => self.f[fd] = float::rsqrt(a, b),
            0x18 => self.acc = float::add(a, b),
            0x19 => self.acc = float::sub(a, b),
            0x1a => self.acc = float::mul(a, b),
            0x1c => self.f[fd] = float::madd(self.acc, a, b),
            0x1d => self.f[fd] = float::msub(self.acc, a, b),
            0x1e => self.acc = float::madd(self.acc, a, b),
            0x1f => self.acc = float::msub(self.acc, a, b),
            0x24 => self.f[fd] = float::to_int(a) as u32,
            0x28 => self.f[fd] = float::max(a, b),
            0x29 => self.f[fd] = float::min(a, b),
            f @ (0x30 | 0x32 | 0x34 | 0x36) => {
                let cond = match f {
                    0x30 => false,
                    0x32 => float::cmp(a, b).is_eq(),
                    0x34 => float::lt(a, b),
                    _ => float::le(a, b),
                };
                self.fcr31 = if cond { self.fcr31 | C_BIT } else { self.fcr31 & !C_BIT };
            }
            _ => return Err(Stop::Reserved { pc, word: u64::from(w) }),
        }
        Ok(Flow::Next)
    }

    /// COP2 in macro mode: the Vu0Machine's additions first when enabled,
    /// then the VuMachine's.
    fn cop2(&mut self, bus: &mut dyn Bus, pc: u32, w: u32) -> Result<Flow, Stop> {
        if self.features.vi && self.cop2_vi(bus, pc, w)? {
            return Ok(Flow::Next);
        }
        self.cop2_vu(pc, w)
    }

    /// VU0 data memory quadword `a` (wrapping at 256 quadwords), through the
    /// bus.
    fn qword(bus: &mut dyn Bus, pc: u32, a: u16) -> Result<[u32; 4], Stop> {
        let at = VU0_DATA + u32::from(a & 0xff) * 16;
        bus.read_u128(at).map(words).map_err(|addr| Stop::Bus { pc, addr })
    }

    /// Writes the lanes of quadword `a` that `mask` names.
    fn store_qword(bus: &mut dyn Bus, pc: u32, a: u16, mask: u32, vals: [u32; 4]) -> Result<(), Stop> {
        let at = VU0_DATA + u32::from(a & 0xff) * 16;
        for (k, v) in vals.into_iter().enumerate() {
            if mask & (8 >> k) != 0 {
                bus.write_u32(at + 4 * k as u32, v).map_err(|addr| Stop::Bus { pc, addr })?;
            }
        }
        Ok(())
    }

    /// test_stream_rs's Vu0Machine.cop2 before it defers: true if handled.
    fn cop2_vi(&mut self, bus: &mut dyn Bus, pc: u32, w: u32) -> Result<bool, Stop> {
        let (rs, rt, rd) = ((w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31);
        if rs == 0x02 {
            // cfc2 rt, vi(rd)
            let v = if rd < 16 { self.vi[rd as usize] } else { 0 };
            self.set(rt as usize, u64::from(v));
            return Ok(true);
        }
        if rs == 0x06 {
            // ctc2 rt, vi(rd)
            if rd < 16 {
                self.vset_i(rd, self.r[rt as usize] as u32);
            }
            return Ok(true);
        }
        if rs & 0x10 == 0 {
            return Ok(false);
        }
        let (funct, dest) = (w & 63, (w >> 21) & 0xf);
        let (it, is, id) = ((rt & 15) as usize, (rd & 15) as usize, (w >> 6) & 15);
        Ok(match funct {
            0x30 => {
                // viadd
                self.vset_i(id, u32::from(self.vi[is].wrapping_add(self.vi[it])));
                true
            }
            0x32 => {
                // viaddi
                let imm = (w >> 6) & 31;
                let imm = if imm & 16 != 0 { imm as i32 - 32 } else { imm as i32 };
                self.vset_i(rt, (i32::from(self.vi[is]) + imm) as u32);
                true
            }
            0x3c.. => match (((w >> 6) & 31) << 2) | (w & 3) {
                0x34 => {
                    // vlqi vf(ft), (vi(fs)++)
                    let q = Self::qword(bus, pc, self.vi[is])?;
                    self.vset(rt as usize, dest, q);
                    self.vset_i(rd, u32::from(self.vi[is]) + 1);
                    true
                }
                0x35 => {
                    // vsqi vf(fs), (vi(ft)++)
                    Self::store_qword(bus, pc, self.vi[it], dest, self.vf[rd as usize])?;
                    self.vset_i(rt, u32::from(self.vi[it]) + 1);
                    true
                }
                0x36 => {
                    // vlqd vf(ft), (--vi(fs))
                    self.vset_i(rd, u32::from(self.vi[is]).wrapping_sub(1));
                    let q = Self::qword(bus, pc, self.vi[is])?;
                    self.vset(rt as usize, dest, q);
                    true
                }
                0x37 => {
                    // vsqd vf(fs), (--vi(ft))
                    self.vset_i(rt, u32::from(self.vi[it]).wrapping_sub(1));
                    Self::store_qword(bus, pc, self.vi[it], dest, self.vf[rd as usize])?;
                    true
                }
                _ => false,
            },
            _ => false,
        })
    }

    fn arith(&self, op: Op, a: [u32; 4], y: [u32; 4]) -> [u32; 4] {
        std::array::from_fn(|k| match op {
            Op::Add => float::add(a[k], y[k]),
            Op::Sub => float::sub(a[k], y[k]),
            Op::Mul => float::mul(a[k], y[k]),
            Op::Madd => float::madd(self.vacc[k], a[k], y[k]),
            Op::Msub => float::msub(self.vacc[k], a[k], y[k]),
            Op::Max => float::max(a[k], y[k]),
            Op::Mini => float::min(a[k], y[k]),
        })
    }

    /// test_anim's VuMachine.cop2.
    fn cop2_vu(&mut self, pc: u32, w: u32) -> Result<Flow, Stop> {
        let (rs, rt, rd) = ((w >> 21) & 31, ((w >> 16) & 31) as usize, ((w >> 11) & 31) as usize);
        let bad = Err(Stop::Reserved { pc, word: u64::from(w) });
        if rs == 0x01 {
            // qmfc2
            self.set128(rt, from_words(self.vf[rd]));
            return Ok(Flow::Next);
        }
        if rs == 0x05 {
            // qmtc2
            self.vset(rd, 0xf, words(self.r[rt]));
            return Ok(Flow::Next);
        }
        if rs & 0x10 == 0 {
            return bad;
        }
        let dest = (w >> 21) & 0xf;
        let (ft, fs, fd) = (rt, rd, ((w >> 6) & 31) as usize);
        let (a, b) = (self.vf[fs], self.vf[ft]);
        let bcv = [b[(w & 3) as usize]; 4];
        let qv = [self.q; 4];
        let funct = w & 63;
        if funct < 0x3c {
            let res = match funct {
                0x00..=0x1b => self.arith(BC_OPS[(funct >> 2) as usize], a, bcv),
                0x1c => self.arith(Op::Mul, a, qv),
                0x20 => self.arith(Op::Add, a, qv),
                0x21 => self.arith(Op::Madd, a, qv),
                0x24 => self.arith(Op::Sub, a, qv),
                0x25 => self.arith(Op::Msub, a, qv),
                0x28 => self.arith(Op::Add, a, b),
                0x29 => self.arith(Op::Madd, a, b),
                0x2a => self.arith(Op::Mul, a, b),
                0x2b => self.arith(Op::Max, a, b),
                0x2c => self.arith(Op::Sub, a, b),
                0x2d => self.arith(Op::Msub, a, b),
                0x2f => self.arith(Op::Mini, a, b),
                0x2e => {
                    // vopmsub
                    let prod = [float::mul(a[1], b[2]), float::mul(a[2], b[0]), float::mul(a[0], b[1])];
                    let v = std::array::from_fn(|k| if k < 3 { float::sub(self.vacc[k], prod[k]) } else { 0 });
                    self.vset(fd, dest & 0xe, v);
                    return Ok(Flow::Next);
                }
                _ => return bad,
            };
            self.vset(fd, dest, res);
            return Ok(Flow::Next);
        }
        let code = (((w >> 6) & 31) << 2) | (w & 3);
        let acc = match code {
            0x00..=0x0f => Some((BC_OPS[(code >> 2) as usize], bcv)),
            0x18..=0x1b => Some((Op::Mul, bcv)),
            0x1c => Some((Op::Mul, qv)),
            0x20 => Some((Op::Add, qv)),
            0x21 => Some((Op::Madd, qv)),
            0x24 => Some((Op::Sub, qv)),
            0x25 => Some((Op::Msub, qv)),
            0x28 => Some((Op::Add, b)),
            0x29 => Some((Op::Madd, b)),
            0x2a => Some((Op::Mul, b)),
            0x2c => Some((Op::Sub, b)),
            0x2d => Some((Op::Msub, b)),
            _ => None,
        };
        if let Some((op, y)) = acc {
            let res = self.arith(op, a, y);
            for (k, v) in res.into_iter().enumerate() {
                if dest & (8 >> k) != 0 {
                    self.vacc[k] = v;
                }
            }
            return Ok(Flow::Next);
        }
        let shift = |code: u32| [0, 4, 12, 15][(code & 3) as usize];
        match code {
            0x10..=0x13 => {
                // vitof0/4/12/15
                let sh = shift(code);
                let res = a.map(|x| float::from_int(x as i32));
                let res = if sh == 0 { res } else { res.map(|x| float::div(x, float::from_int(1 << sh))) };
                self.vset(ft, dest, res);
            }
            0x14..=0x17 => {
                // vftoi0/4/12/15
                let sh = shift(code);
                let res = a.map(|x| float::to_int(if sh == 0 { x } else { float::mul(x, float::from_int(1 << sh)) }));
                self.vset(ft, dest, res.map(|x| x as u32));
            }
            // vabs
            0x1d => self.vset(ft, dest, a.map(|x| x & 0x7fff_ffff)),
            0x2e => {
                // vopmula
                self.vacc[0] = float::mul(a[1], b[2]);
                self.vacc[1] = float::mul(a[2], b[0]);
                self.vacc[2] = float::mul(a[0], b[1]);
            }
            // vmove
            0x30 => self.vset(ft, dest, a),
            // vmr32
            0x31 => self.vset(ft, dest, [a[1], a[2], a[3], a[0]]),
            // vdiv, vsqrt, vrsqrt: fsf in bits 21-22, ftf in 23-24.
            0x38 => self.q = float::div(a[((w >> 21) & 3) as usize], b[((w >> 23) & 3) as usize]),
            0x39 => self.q = float::sqrt(b[((w >> 23) & 3) as usize]),
            0x3a => self.q = float::rsqrt(a[((w >> 21) & 3) as usize], b[((w >> 23) & 3) as usize]),
            // vnop, vwaitq
            0x2f | 0x3b => {}
            _ => return bad,
        }
        Ok(Flow::Next)
    }

    /// test_anim's VuMachine.mmi. The instruction is matched on the fields
    /// `mips.decode` names it by, so an encoding with a must-be-zero field
    /// set stops, as it does there.
    fn mmi(&mut self, pc: u32, w: u32) -> Result<Flow, Stop> {
        let (rs, rt, rd, sa) =
            (((w >> 21) & 31) as usize, ((w >> 16) & 31) as usize, ((w >> 11) & 31) as usize, (w >> 6) & 31);
        let (a, b) = (self.r[rs], self.r[rt]);
        let hi = self.hi | (self.hi1 << 64);
        let lo = self.lo | (self.lo1 << 64);
        let v = match (w & 63, sa) {
            (0x08, 0x12) => {
                // pextlw
                let (a, b) = (words(a), words(b));
                from_words([b[0], a[0], b[1], a[1]])
            }
            (0x28, 0x12) => {
                // pextuw
                let (a, b) = (words(a), words(b));
                from_words([b[2], a[2], b[3], a[3]])
            }
            // pcpyld
            (0x09, 0x0e) => (b & M64) | ((a & M64) << 64),
            // pcpyud
            (0x29, 0x0e) => (a >> 64) | ((b >> 64) << 64),
            (0x08, 0x16) => {
                // pextlh
                let (a, b) = (halves(a), halves(b));
                from_halves([b[0], a[0], b[1], a[1], b[2], a[2], b[3], a[3]])
            }
            (0x08, 0x05) => {
                // psubh
                let (a, b) = (halves(a), halves(b));
                from_halves(std::array::from_fn(|i| a[i].wrapping_sub(b[i])))
            }
            (0x08, 0x14) => {
                // paddsh
                let (a, b) = (halves(a), halves(b));
                from_halves(std::array::from_fn(|i| (a[i] as i16).saturating_add(b[i] as i16) as u16))
            }
            (0x08, 0x17) => {
                // ppach
                let (a, b) = (halves(a), halves(b));
                from_halves([b[0], b[2], b[4], b[6], a[0], a[2], a[4], a[6]])
            }
            // psraw
            (0x3f, _) if rs == 0 => from_words(words(b).map(|x| ((x as i32) >> sa) as u32)),
            (0x29, 0x08 | 0x09) if rt == 0 && rd == 0 => {
                // pmthi / pmtlo: the other pair's upper half folds in its
                // second pipeline's (piney's `hi >> 64` of `hi | hi1 << 64`).
                let (h, h1, l, l1) = if sa == 0x08 {
                    (a & M64, a >> 64, self.lo & M64, (self.lo >> 64) | self.lo1)
                } else {
                    (self.hi & M64, (self.hi >> 64) | self.hi1, a & M64, a >> 64)
                };
                (self.hi, self.hi1, self.lo, self.lo1) = (h, h1, l, l1);
                return Ok(Flow::Next);
            }
            (0x09, 0x10) => {
                // pmaddh
                let (a, b) = (halves(a), halves(b));
                let mut h = words(hi).map(|x| i64::from(x as i32));
                let mut l = words(lo).map(|x| i64::from(x as i32));
                for (k, (is_hi, j)) in
                    [(false, 0), (false, 1), (true, 0), (true, 1), (false, 2), (false, 3), (true, 2), (true, 3)]
                        .into_iter()
                        .enumerate()
                {
                    let p = i64::from(a[k] as i16) * i64::from(b[k] as i16);
                    if is_hi {
                        h[j] += p;
                    } else {
                        l[j] += p;
                    }
                }
                let hi = from_words(h.map(|x| x as u32));
                let lo = from_words(l.map(|x| x as u32));
                (self.hi, self.hi1, self.lo, self.lo1) = (hi & M64, hi >> 64, lo & M64, lo >> 64);
                from_words([l[0] as u32, h[0] as u32, l[2] as u32, h[2] as u32])
            }
            _ => return Err(Stop::Reserved { pc, word: u64::from(w) }),
        };
        if rd != 0 {
            self.r[rd] = v;
        }
        Ok(Flow::Next)
    }
}

impl Default for Ee {
    fn default() -> Self {
        Ee::new(Features::default())
    }
}

impl Cpu for Ee {
    fn name(&self) -> &'static str {
        "R5900"
    }

    fn endian(&self) -> Endian {
        Endian::Little
    }

    /// The EABI: arguments in a0-a3 and r8-r11 (`a4`-`a7`), the return
    /// value in v0, `ra` the link, stack arguments 8 bytes each from `sp`
    /// with no home area, `sp` 16-byte aligned.
    fn abi(&self) -> Abi {
        Abi {
            args: &[4, 5, 6, 7, 8, 9, 10, 11],
            ret: 2,
            sp: 29,
            return_address: ReturnAddress::Register(31),
            stack_args_offset: 0,
            stack_slot: 8,
            stack_align: 16,
        }
    }

    /// Executes the instruction at the program counter. A bus error and an
    /// uninterpreted word leave the program counter on the instruction; a
    /// `syscall` completes and moves past it, a `break` does not.
    fn step(&mut self, bus: &mut dyn Bus) -> Result<u32, Stop> {
        let pc = self.pc;
        let w = bus.read_u32(pc).map_err(|addr| Stop::Bus { pc, addr })?;
        let flow = self.exec(bus, pc, w)?;
        match flow {
            Flow::Break(code) => Err(Stop::Break { pc, code }),
            Flow::Syscall(code) => {
                self.advance(flow);
                Err(Stop::Syscall { pc, code })
            }
            _ => {
                self.advance(flow);
                Ok(1)
            }
        }
    }

    fn pc(&self) -> u32 {
        self.pc
    }

    fn set_pc(&mut self, pc: u32) {
        self.pc = pc;
        self.npc = pc.wrapping_add(4);
    }

    /// True between a taken branch or jump and its delay slot: the
    /// instruction after the next is not the next one's successor. A
    /// branch-likely not taken skips its slot, so is never in one.
    fn in_delay_slot(&self) -> bool {
        self.npc != self.pc.wrapping_add(4)
    }

    fn reg_count(&self) -> usize {
        if self.features.vu { REGS_VU } else { REGS }
    }

    /// # Panics
    ///
    /// When `i` is [`REGS_VU`] or more.
    fn reg(&self, i: usize) -> u128 {
        match i {
            0..=31 => self.r[i],
            HI => self.hi,
            LO => self.lo,
            HI1 => self.hi1,
            LO1 => self.lo1,
            SA => u128::from(self.sa),
            F0..FCR31 => u128::from(self.f[i - F0]),
            FCR31 => u128::from(self.fcr31),
            ACC => u128::from(self.acc),
            VF0..VACC => from_words(self.vf[i - VF0]),
            VACC => from_words(self.vacc),
            Q => u128::from(self.q),
            VI0..REGS_VU => u128::from(self.vi[i - VI0]),
            _ => panic!("the R5900 has no register {i}"),
        }
    }

    /// # Panics
    ///
    /// When `i` is [`REGS_VU`] or more.
    fn set_reg(&mut self, i: usize, v: u128) {
        match i {
            0..=31 => self.set128(i, v),
            HI => self.hi = v,
            LO => self.lo = v,
            HI1 => self.hi1 = v,
            LO1 => self.lo1 = v,
            SA => self.sa = v as u32,
            F0..FCR31 => self.f[i - F0] = v as u32,
            FCR31 => self.fcr31 = v as u32,
            ACC => self.acc = v as u32,
            VF0..VACC => self.vset(i - VF0, 0xf, words(v)),
            VACC => self.vacc = words(v),
            Q => self.q = v as u32,
            VI0..REGS_VU => self.vset_i((i - VI0) as u32, v as u32),
            _ => panic!("the R5900 has no register {i}"),
        }
    }

    /// # Panics
    ///
    /// When `i` is [`REGS_VU`] or more.
    fn reg_name(&self, i: usize) -> &'static str {
        match i {
            0..=31 => GPR_NAMES[i],
            HI..F0 => SPECIAL_NAMES[i - HI],
            F0..FCR31 => FPR_NAMES[i - F0],
            FCR31 => "fcr31",
            ACC => "acc",
            VF0..VACC => VF_NAMES[i - VF0],
            VACC => "vacc",
            Q => "q",
            VI0..REGS_VU => VI_NAMES[i - VI0],
            _ => panic!("the R5900 has no register {i}"),
        }
    }

    /// Finds a register by its name, an EABI alias (`a4`-`a7` for r8-r11,
    /// `s8` for `fp`) or its number (`r0`-`r31`); a leading `$` is
    /// ignored.
    fn reg_index(&self, name: &str) -> Option<usize> {
        let name = name.strip_prefix('$').unwrap_or(name);
        let alias = match name {
            "a4" => Some(8),
            "a5" => Some(9),
            "a6" => Some(10),
            "a7" => Some(11),
            "s8" => Some(30),
            _ => name.strip_prefix('r').and_then(|n| n.parse::<usize>().ok()).filter(|&n| n < 32),
        };
        alias.or_else(|| (0..self.reg_count()).find(|&i| self.reg_name(i) == name))
    }
}
