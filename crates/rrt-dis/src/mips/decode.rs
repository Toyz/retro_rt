//! The encoding tables: a word to a [`Def`] - mnemonic, operand tokens,
//! fields that must be zero, control-flow kind, the registers it touches
//! without naming them. Building an [`Insn`](crate::Insn) from a `Def` and
//! printing one are in the parent module; the VU0 macro tables are in
//! `vu0`, the GTE commands in `gte`.
//!
//! Must-be-zero fields are checked, as piney's decoder does, so data
//! decodes as `.word` rather than as a plausible instruction.

use super::regs::*;
use super::{Isa, gte, vu0};
use crate::insn::Reg;

/// How an operand register is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum A {
    /// Read.
    R,
    /// Written.
    W,
    /// Read and written (a merge or a conditional move).
    Rw,
}

/// An operand token: which field of the word it is and how it prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum T {
    /// GPR in rs (bits 21-25).
    Rs(A),
    /// GPR in rt (bits 16-20).
    Rt(A),
    /// GPR in rd (bits 11-15).
    Rd(A),
    /// Shift amount (bits 6-10), decimal.
    Sa,
    /// Sign-extended 16-bit immediate, decimal.
    Simm,
    /// Zero-extended 16-bit immediate, hex.
    Uimm,
    /// `offset(rs)`.
    Mem,
    /// PC-relative branch target.
    Br,
    /// 26-bit jump target.
    Jt,
    /// `syscall`'s 20-bit code.
    Syscall,
    /// `break`'s two 10-bit codes.
    Break,
    /// A trap's 10-bit code (bits 6-15).
    Tcode,
    /// The rt field as a hex number: `cache` op, `pref` hint.
    Hint,
    /// COP0 register in rd.
    C0(A),
    /// The EE breakpoint register the low bits of a COP0 move select (no
    /// operand of its own).
    C0Debug(A),
    /// The EE performance counter register field (bits 1-5), decimal.
    C0Perf(A),
    /// FPR in rd (fs).
    Fs(A),
    /// FPR in rt (ft).
    Ft(A),
    /// FPR in sa (fd).
    Fd(A),
    /// FPU control register in rd.
    Fcr(A),
    /// GTE data register in rd.
    GteD(A),
    /// GTE control register in rd.
    GteC(A),
    /// GTE data register in rt (`lwc2`/`swc2`).
    GteRt(A),
    /// A GTE command's sf and lm fields.
    Gte,
    /// `mvmva`'s sf, lm, mx, v and cv fields.
    Mvmva,
    /// VU0 float register in rd (`qmfc2`/`qmtc2`).
    VfRd(A),
    /// VU0 float register in rt (`lqc2`/`sqc2`, and ft of the macro ops).
    Vft(A),
    /// VU0 float register in rd (fs).
    Vfs(A),
    /// VU0 float register in sa (fd).
    Vfd(A),
    /// ft read with the broadcast field (bits 0-1) as a suffix: `vf3x`.
    VftBc,
    /// fs read with the fsf field (bits 21-22): `vf0w`.
    VfsF,
    /// ft read with the ftf field (bits 23-24): `vf5x`.
    VftF,
    /// VU0 integer register in rt.
    Vit(A),
    /// VU0 integer register in rd.
    Vis(A),
    /// VU0 integer register in sa.
    Vid(A),
    /// `(vis++)`.
    VisInc,
    /// `(vit++)`.
    VitInc,
    /// `(--vis)`.
    VisDec,
    /// `(--vit)`.
    VitDec,
    /// `(vis)`.
    VisAt,
    /// VU0 control register in rd (`cfc2`/`ctc2`, `vcallmsr`).
    VuCtrl(A),
    /// The VU0 accumulator.
    Acc(A),
    /// VU0 Q.
    Q(A),
    /// VU0 I.
    I,
    /// VU0 R (the random number register).
    Rand(A),
    /// `viaddi`'s signed 5-bit immediate (bits 6-10).
    Vimm5,
    /// `vcallms`'s 15-bit micro address (bits 6-20), printed as the byte
    /// address in micro memory (times 8).
    Vimm15,
}

/// What an instruction does to control flow, before the operands are known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// Falls through.
    Plain,
    /// A conditional branch (`beq zero, zero` and `bgez zero` become jumps).
    Branch {
        /// Branch-likely.
        likely: bool,
    },
    /// A branch that links: `bltzal`, `bgezal` and their likely forms.
    Link,
    /// `j`.
    Jump,
    /// `jal`.
    Call,
    /// `jr`.
    Jr,
    /// `jalr`.
    Jalr,
    /// `syscall`, `break`, the conditional traps.
    Trap,
    /// `eret`: back to EPC, no delay slot.
    Eret,
}

/// One decoded encoding.
#[derive(Clone, Copy, Debug)]
pub(super) struct Def {
    /// The mnemonic (VU0 ops: without the dest mask).
    pub mn: &'static str,
    /// Operand tokens in written order.
    pub toks: &'static [T],
    /// Bits that must be zero.
    pub zero: u32,
    /// Control flow.
    pub kind: Kind,
    /// Registers read without being named.
    pub imp_r: &'static [Reg],
    /// Registers written without being named.
    pub imp_w: &'static [Reg],
    /// The VU0 dest mask (bits 21-24) is part of the mnemonic.
    pub dest: bool,
    /// A GTE command (its register effects come from `gte`).
    pub gte: Option<gte::Command>,
}

pub(super) const RS: u32 = 0x03e0_0000;
pub(super) const RT: u32 = 0x001f_0000;
pub(super) const RD: u32 = 0x0000_f800;
pub(super) const SAF: u32 = 0x0000_07c0;
const R4: u32 = RS | RT | RD | SAF;

/// The fields a token reads its value from, for the must-be-zero check.
fn field(t: T) -> u32 {
    use T::*;
    match t {
        Rs(_) | Mem => RS,
        Rt(_) | Ft(_) | Vft(_) | Vit(_) | VftBc | VftF | VitInc | VitDec | Hint | GteRt(_) => RT,
        Rd(_) | Fs(_) | Fcr(_) | C0(_) | GteD(_) | GteC(_) | VfRd(_) | Vfs(_) | Vis(_) | VfsF | VisInc | VisDec
        | VisAt | VuCtrl(_) => RD,
        Sa | Fd(_) | Vfd(_) | Vid(_) | Vimm5 => SAF,
        Syscall | Break => R4,
        Tcode => RD | SAF,
        _ => 0,
    }
}

impl Def {
    fn new(mn: &'static str, toks: &'static [T]) -> Def {
        Def { mn, toks, zero: 0, kind: Kind::Plain, imp_r: &[], imp_w: &[], dest: false, gte: None }
    }

    /// The fields in `mask` that no token uses must be zero.
    fn fields(mut self, mask: u32) -> Def {
        let used = self.toks.iter().fold(0, |m, &t| m | field(t));
        self.zero |= mask & !used;
        self
    }

    pub(super) fn zero(mut self, mask: u32) -> Def {
        self.zero |= mask;
        self
    }

    fn kind(mut self, kind: Kind) -> Def {
        self.kind = kind;
        self
    }

    fn imp(mut self, r: &'static [Reg], w: &'static [Reg]) -> Def {
        self.imp_r = r;
        self.imp_w = w;
        self
    }

    pub(super) fn dest(mut self) -> Def {
        self.dest = true;
        self
    }
}

/// A Def whose unused rs/rt/rd/sa fields must be zero (R-type).
fn r(mn: &'static str, toks: &'static [T]) -> Def {
    Def::new(mn, toks).fields(R4)
}

/// A Def with no field checks.
pub(super) fn d(mn: &'static str, toks: &'static [T]) -> Def {
    Def::new(mn, toks)
}

/// A Def with no field checks and implicit registers (VU0 tables).
pub(super) fn di(mn: &'static str, toks: &'static [T], imp_r: &'static [Reg], imp_w: &'static [Reg]) -> Def {
    Def::new(mn, toks).imp(imp_r, imp_w)
}

/// A Def whose bits in `zero` must be zero (VU0 tables).
pub(super) fn dz(mn: &'static str, toks: &'static [T], zero: u32) -> Def {
    Def::new(mn, toks).zero(zero)
}

const HL: &[Reg] = &[HI, LO];
const HL1: &[Reg] = &[HI1, LO1];
const HL128: &[Reg] = &[HI, LO, HI1, LO1];

/// The Def for `word`, or None when it is no instruction of `isa`.
pub(super) fn lookup(isa: Isa, w: u32) -> Option<Def> {
    let ee = isa == Isa::R5900;
    let def = match w >> 26 {
        0x00 => special(ee, w)?,
        0x01 => regimm(ee, w)?,
        0x02 => d("j", &[T::Jt]).kind(Kind::Jump),
        0x03 => d("jal", &[T::Jt]).kind(Kind::Call).imp(&[], &[RA]),
        0x04 => d("beq", &[T::Rs(A::R), T::Rt(A::R), T::Br]).kind(Kind::Branch { likely: false }),
        0x05 => d("bne", &[T::Rs(A::R), T::Rt(A::R), T::Br]).kind(Kind::Branch { likely: false }),
        0x06 => d("blez", &[T::Rs(A::R), T::Br]).zero(RT).kind(Kind::Branch { likely: false }),
        0x07 => d("bgtz", &[T::Rs(A::R), T::Br]).zero(RT).kind(Kind::Branch { likely: false }),
        0x08 => d("addi", &[T::Rt(A::W), T::Rs(A::R), T::Simm]),
        0x09 => d("addiu", &[T::Rt(A::W), T::Rs(A::R), T::Simm]),
        0x0a => d("slti", &[T::Rt(A::W), T::Rs(A::R), T::Simm]),
        0x0b => d("sltiu", &[T::Rt(A::W), T::Rs(A::R), T::Simm]),
        0x0c => d("andi", &[T::Rt(A::W), T::Rs(A::R), T::Uimm]),
        0x0d => d("ori", &[T::Rt(A::W), T::Rs(A::R), T::Uimm]),
        0x0e => d("xori", &[T::Rt(A::W), T::Rs(A::R), T::Uimm]),
        0x0f => d("lui", &[T::Rt(A::W), T::Uimm]).zero(RS),
        0x10 if ee => cop0_ee(w)?,
        0x10 => cop0_r3000(w)?,
        0x11 if ee => cop1(w)?,
        0x12 if ee => cop2_ee(w)?,
        0x12 => cop2_gte(w)?,
        0x14 if ee => d("beql", &[T::Rs(A::R), T::Rt(A::R), T::Br]).kind(Kind::Branch { likely: true }),
        0x15 if ee => d("bnel", &[T::Rs(A::R), T::Rt(A::R), T::Br]).kind(Kind::Branch { likely: true }),
        0x16 if ee => d("blezl", &[T::Rs(A::R), T::Br]).zero(RT).kind(Kind::Branch { likely: true }),
        0x17 if ee => d("bgtzl", &[T::Rs(A::R), T::Br]).zero(RT).kind(Kind::Branch { likely: true }),
        0x18 if ee => d("daddi", &[T::Rt(A::W), T::Rs(A::R), T::Simm]),
        0x19 if ee => d("daddiu", &[T::Rt(A::W), T::Rs(A::R), T::Simm]),
        0x1a if ee => d("ldl", &[T::Rt(A::Rw), T::Mem]),
        0x1b if ee => d("ldr", &[T::Rt(A::Rw), T::Mem]),
        0x1c if ee => mmi(w)?,
        0x1e if ee => d("lq", &[T::Rt(A::W), T::Mem]),
        0x1f if ee => d("sq", &[T::Rt(A::R), T::Mem]),
        0x20 => d("lb", &[T::Rt(A::W), T::Mem]),
        0x21 => d("lh", &[T::Rt(A::W), T::Mem]),
        0x22 => d("lwl", &[T::Rt(A::Rw), T::Mem]),
        0x23 => d("lw", &[T::Rt(A::W), T::Mem]),
        0x24 => d("lbu", &[T::Rt(A::W), T::Mem]),
        0x25 => d("lhu", &[T::Rt(A::W), T::Mem]),
        0x26 => d("lwr", &[T::Rt(A::Rw), T::Mem]),
        0x27 if ee => d("lwu", &[T::Rt(A::W), T::Mem]),
        0x28 => d("sb", &[T::Rt(A::R), T::Mem]),
        0x29 => d("sh", &[T::Rt(A::R), T::Mem]),
        0x2a => d("swl", &[T::Rt(A::R), T::Mem]),
        0x2b => d("sw", &[T::Rt(A::R), T::Mem]),
        0x2c if ee => d("sdl", &[T::Rt(A::R), T::Mem]),
        0x2d if ee => d("sdr", &[T::Rt(A::R), T::Mem]),
        0x2e => d("swr", &[T::Rt(A::R), T::Mem]),
        0x2f if ee => d("cache", &[T::Hint, T::Mem]),
        0x31 if ee => d("lwc1", &[T::Ft(A::W), T::Mem]),
        0x32 if !ee => d("lwc2", &[T::GteRt(A::W), T::Mem]),
        0x33 if ee => d("pref", &[T::Hint, T::Mem]),
        0x36 if ee => d("lqc2", &[T::Vft(A::W), T::Mem]),
        0x37 if ee => d("ld", &[T::Rt(A::W), T::Mem]),
        0x39 if ee => d("swc1", &[T::Ft(A::R), T::Mem]),
        0x3a if !ee => d("swc2", &[T::GteRt(A::R), T::Mem]),
        0x3e if ee => d("sqc2", &[T::Vft(A::R), T::Mem]),
        0x3f if ee => d("sd", &[T::Rt(A::R), T::Mem]),
        _ => return None,
    };
    (w & def.zero == 0).then_some(def)
}

/// (bytes, store) for the loads and stores; `cache` and `pref` take a
/// memory operand but access nothing (0 bytes).
pub(super) fn mem_access(mn: &str) -> Option<(u8, bool)> {
    Some(match mn {
        "lb" | "lbu" => (1, false),
        "lh" | "lhu" => (2, false),
        "lw" | "lwu" | "lwl" | "lwr" | "lwc1" | "lwc2" => (4, false),
        "ld" | "ldl" | "ldr" => (8, false),
        "lq" | "lqc2" => (16, false),
        "sb" => (1, true),
        "sh" => (2, true),
        "sw" | "swl" | "swr" | "swc1" | "swc2" => (4, true),
        "sd" | "sdl" | "sdr" => (8, true),
        "sq" | "sqc2" => (16, true),
        "cache" | "pref" => (0, false),
        _ => return None,
    })
}

fn special(ee: bool, w: u32) -> Option<Def> {
    use A::*;
    use T::*;
    let shift = &[Rd(W), Rt(R), Sa];
    let shiftv = &[Rd(W), Rt(R), Rs(R)];
    let three = &[Rd(W), Rs(R), Rt(R)];
    let trap = &[Rs(R), Rt(R), Tcode];
    Some(match w & 63 {
        0x00 => r("sll", shift),
        0x02 => r("srl", shift),
        0x03 => r("sra", shift),
        0x04 => r("sllv", shiftv),
        0x06 => r("srlv", shiftv),
        0x07 => r("srav", shiftv),
        0x08 => r("jr", &[Rs(R)]).kind(Kind::Jr),
        0x09 => r("jalr", &[Rd(W), Rs(R)]).kind(Kind::Jalr),
        0x0a if ee => r("movz", &[Rd(Rw), Rs(R), Rt(R)]),
        0x0b if ee => r("movn", &[Rd(Rw), Rs(R), Rt(R)]),
        0x0c => d("syscall", &[Syscall]).kind(Kind::Trap),
        0x0d => d("break", &[Break]).kind(Kind::Trap),
        0x0f if ee => match (w >> 6) & 31 {
            0x00 => r("sync", &[]),
            0x10 => d("sync.p", &[]).zero(RS | RT | RD),
            _ => return None,
        },
        0x10 => r("mfhi", &[Rd(W)]).imp(&[HI], &[]),
        0x11 => r("mthi", &[Rs(R)]).imp(&[], &[HI]),
        0x12 => r("mflo", &[Rd(W)]).imp(&[LO], &[]),
        0x13 => r("mtlo", &[Rs(R)]).imp(&[], &[LO]),
        0x14 if ee => r("dsllv", shiftv),
        0x16 if ee => r("dsrlv", shiftv),
        0x17 if ee => r("dsrav", shiftv),
        // The EE's mult/multu also write LO to rd.
        0x18 if ee => r("mult", three).imp(&[], HL),
        0x19 if ee => r("multu", three).imp(&[], HL),
        0x18 => r("mult", &[Rs(R), Rt(R)]).imp(&[], HL),
        0x19 => r("multu", &[Rs(R), Rt(R)]).imp(&[], HL),
        0x1a => r("div", &[Rs(R), Rt(R)]).imp(&[], HL),
        0x1b => r("divu", &[Rs(R), Rt(R)]).imp(&[], HL),
        0x20 => r("add", three),
        0x21 => r("addu", three),
        0x22 => r("sub", three),
        0x23 => r("subu", three),
        0x24 => r("and", three),
        0x25 => r("or", three),
        0x26 => r("xor", three),
        0x27 => r("nor", three),
        0x28 if ee => r("mfsa", &[Rd(W)]).imp(&[SA], &[]),
        0x29 if ee => r("mtsa", &[Rs(R)]).imp(&[], &[SA]),
        0x2a => r("slt", three),
        0x2b => r("sltu", three),
        0x2c if ee => r("dadd", three),
        0x2d if ee => r("daddu", three),
        0x2e if ee => r("dsub", three),
        0x2f if ee => r("dsubu", three),
        0x30 if ee => r("tge", trap).kind(Kind::Trap),
        0x31 if ee => r("tgeu", trap).kind(Kind::Trap),
        0x32 if ee => r("tlt", trap).kind(Kind::Trap),
        0x33 if ee => r("tltu", trap).kind(Kind::Trap),
        0x34 if ee => r("teq", trap).kind(Kind::Trap),
        0x36 if ee => r("tne", trap).kind(Kind::Trap),
        0x38 if ee => r("dsll", shift),
        0x3a if ee => r("dsrl", shift),
        0x3b if ee => r("dsra", shift),
        0x3c if ee => r("dsll32", shift),
        0x3e if ee => r("dsrl32", shift),
        0x3f if ee => r("dsra32", shift),
        _ => return None,
    })
}

fn regimm(ee: bool, w: u32) -> Option<Def> {
    use A::*;
    use T::*;
    let br = &[Rs(R), Br];
    let ti = &[Rs(R), Simm];
    let b = Kind::Branch { likely: false };
    let bl = Kind::Branch { likely: true };
    Some(match (w >> 16) & 31 {
        0x00 => d("bltz", br).kind(b),
        0x01 => d("bgez", br).kind(b),
        0x10 => d("bltzal", br).kind(Kind::Link).imp(&[], &[RA]),
        0x11 => d("bgezal", br).kind(Kind::Link).imp(&[], &[RA]),
        _ if !ee => return None,
        0x02 => d("bltzl", br).kind(bl),
        0x03 => d("bgezl", br).kind(bl),
        0x08 => d("tgei", ti).kind(Kind::Trap),
        0x09 => d("tgeiu", ti).kind(Kind::Trap),
        0x0a => d("tlti", ti).kind(Kind::Trap),
        0x0b => d("tltiu", ti).kind(Kind::Trap),
        0x0c => d("teqi", ti).kind(Kind::Trap),
        0x0e => d("tnei", ti).kind(Kind::Trap),
        0x12 => d("bltzall", br).kind(Kind::Link).imp(&[], &[RA]),
        0x13 => d("bgezall", br).kind(Kind::Link).imp(&[], &[RA]),
        0x18 => d("mtsab", ti).imp(&[], &[SA]),
        0x19 => d("mtsah", ti).imp(&[], &[SA]),
        _ => return None,
    })
}

fn cop0_r3000(w: u32) -> Option<Def> {
    use A::*;
    use T::*;
    Some(match (w >> 21) & 31 {
        0x00 => d("mfc0", &[Rt(W), C0(R)]).zero(0x7ff),
        0x04 => d("mtc0", &[Rt(R), C0(W)]).zero(0x7ff),
        // rfe pops the interrupt-enable/mode stack in SR.
        0x10 if w & 0x01ff_ffff == 0x10 => d("rfe", &[]).imp(&[SR], &[SR]),
        _ => return None,
    })
}

fn cop0_ee(w: u32) -> Option<Def> {
    use A::*;
    use T::*;
    const SYS: u32 = 0x01ff_ffc0;
    let (rs, rd, low) = ((w >> 21) & 31, (w >> 11) & 31, w & 0x7ff);
    let to = rs == 0x04;
    Some(match rs {
        0x00 | 0x04 => match rd {
            // The breakpoint registers hide behind rd 24, the performance
            // counters behind rd 25, selected by the low bits.
            24 => {
                const MF: [&str; 8] = ["mfbpc", "", "mfiab", "mfiabm", "mfdab", "mfdabm", "mfdvb", "mfdvbm"];
                const MT: [&str; 8] = ["mtbpc", "", "mtiab", "mtiabm", "mtdab", "mtdabm", "mtdvb", "mtdvbm"];
                if low >= 8 || low == 1 {
                    return None;
                }
                let k = low as usize;
                if to { d(MT[k], &[Rt(R), C0Debug(W)]) } else { d(MF[k], &[Rt(W), C0Debug(R)]) }
            }
            25 if low & 0x7c0 == 0 => match (to, low & 1) {
                (false, 0) => d("mfps", &[Rt(W), C0Perf(R)]),
                (false, _) => d("mfpc", &[Rt(W), C0Perf(R)]),
                (true, 0) => d("mtps", &[Rt(R), C0Perf(W)]),
                (true, _) => d("mtpc", &[Rt(R), C0Perf(W)]),
            },
            25 => return None,
            _ if to => d("mtc0", &[Rt(R), C0(W)]).zero(0x7ff),
            _ => d("mfc0", &[Rt(W), C0(R)]).zero(0x7ff),
        },
        0x08 => {
            let k = match (w >> 16) & 31 {
                0 => ("bc0f", false),
                1 => ("bc0t", false),
                2 => ("bc0fl", true),
                3 => ("bc0tl", true),
                _ => return None,
            };
            d(k.0, &[Br]).kind(Kind::Branch { likely: k.1 })
        }
        0x10 => {
            const TLB: &[Reg] = &[C0_ENTRYHI, C0_ENTRYLO0, C0_ENTRYLO1, C0_PAGEMASK];
            let def = match w & 63 {
                0x01 => d("tlbr", &[]).imp(&[C0_INDEX], TLB),
                0x02 => d("tlbwi", &[]).imp(&[C0_INDEX, C0_ENTRYHI, C0_ENTRYLO0, C0_ENTRYLO1, C0_PAGEMASK], &[]),
                0x06 => d("tlbwr", &[]).imp(&[C0_RANDOM, C0_ENTRYHI, C0_ENTRYLO0, C0_ENTRYLO1, C0_PAGEMASK], &[]),
                0x08 => d("tlbp", &[]).imp(&[C0_ENTRYHI], &[C0_INDEX]),
                0x18 => d("eret", &[]).kind(Kind::Eret).imp(&[C0_STATUS, C0_EPC, C0_ERROREPC], &[C0_STATUS]),
                0x38 => d("ei", &[]).imp(&[C0_STATUS], &[C0_STATUS]),
                0x39 => d("di", &[]).imp(&[C0_STATUS], &[C0_STATUS]),
                _ => return None,
            };
            def.zero(SYS)
        }
        _ => return None,
    })
}

fn cop1(w: u32) -> Option<Def> {
    use A::*;
    use T::*;
    const F3: u32 = RT | RD | SAF;
    let s3 = &[Fd(W), Fs(R), Ft(R)];
    let s2 = &[Fd(W), Fs(R)];
    let acc = &[Fs(R), Ft(R)];
    Some(match (w >> 21) & 31 {
        0x00 => d("mfc1", &[Rt(W), Fs(R)]).zero(0x7ff),
        0x02 => d("cfc1", &[Rt(W), Fcr(R)]).zero(0x7ff),
        0x04 => d("mtc1", &[Rt(R), Fs(W)]).zero(0x7ff),
        0x06 => d("ctc1", &[Rt(R), Fcr(W)]).zero(0x7ff),
        0x08 => {
            let (mn, likely) = match (w >> 16) & 31 {
                0 => ("bc1f", false),
                1 => ("bc1t", false),
                2 => ("bc1fl", true),
                3 => ("bc1tl", true),
                _ => return None,
            };
            d(mn, &[Br]).kind(Kind::Branch { likely }).imp(&[FCR31], &[])
        }
        0x10 => {
            let def = match w & 63 {
                0x00 => d("add.s", s3),
                0x01 => d("sub.s", s3),
                0x02 => d("mul.s", s3),
                0x03 => d("div.s", s3),
                // The EE's sqrt.s takes its operand in ft, not fs.
                0x04 => d("sqrt.s", &[Fd(W), Ft(R)]),
                0x05 => d("abs.s", s2),
                0x06 => d("mov.s", s2),
                0x07 => d("neg.s", s2),
                0x16 => d("rsqrt.s", s3),
                0x18 => d("adda.s", acc).imp(&[], &[FACC]),
                0x19 => d("suba.s", acc).imp(&[], &[FACC]),
                0x1a => d("mula.s", acc).imp(&[], &[FACC]),
                0x1c => d("madd.s", s3).imp(&[FACC], &[]),
                0x1d => d("msub.s", s3).imp(&[FACC], &[]),
                0x1e => d("madda.s", acc).imp(&[FACC], &[FACC]),
                0x1f => d("msuba.s", acc).imp(&[FACC], &[FACC]),
                0x24 => d("cvt.w.s", s2),
                0x28 => d("max.s", s3),
                0x29 => d("min.s", s3),
                0x30 => d("c.f.s", acc).imp(&[], &[FCR31]),
                0x32 => d("c.eq.s", acc).imp(&[], &[FCR31]),
                0x34 => d("c.lt.s", acc).imp(&[], &[FCR31]),
                0x36 => d("c.le.s", acc).imp(&[], &[FCR31]),
                _ => return None,
            };
            def.fields(F3)
        }
        0x14 if w & 63 == 0x20 => d("cvt.s.w", s2).fields(F3),
        _ => return None,
    })
}

fn cop2_gte(w: u32) -> Option<Def> {
    use A::*;
    use T::*;
    Some(match (w >> 21) & 31 {
        0x00 => d("mfc2", &[Rt(W), GteD(R)]).zero(0x7ff),
        0x02 => d("cfc2", &[Rt(W), GteC(R)]).zero(0x7ff),
        0x04 => d("mtc2", &[Rt(R), GteD(W)]).zero(0x7ff),
        0x06 => d("ctc2", &[Rt(R), GteC(W)]).zero(0x7ff),
        rs if rs & 0x10 != 0 => {
            // The command word's other bits (20-24 carry the "fake" command
            // number the libraries emit) are ignored by the GTE.
            let c = gte::command(w & 63)?;
            let toks: &'static [T] = if c.name == "mvmva" { &[Mvmva] } else { &[Gte] };
            Def { gte: Some(c), ..d(c.name, toks) }
        }
        _ => return None,
    })
}

fn cop2_ee(w: u32) -> Option<Def> {
    use A::*;
    use T::*;
    // Bit 0 of the COP2 moves is the VU0 interlock: .i waits for a running
    // micro program to finish, .ni does not.
    let i = w & 1 != 0;
    let pick = |a: &'static str, b: &'static str| if i { a } else { b };
    Some(match (w >> 21) & 31 {
        0x01 => d(pick("qmfc2.i", "qmfc2.ni"), &[Rt(W), VfRd(R)]).zero(0x7fe),
        0x02 => d(pick("cfc2.i", "cfc2.ni"), &[Rt(W), VuCtrl(R)]).zero(0x7fe),
        0x05 => d(pick("qmtc2.i", "qmtc2.ni"), &[Rt(R), VfRd(W)]).zero(0x7fe),
        0x06 => d(pick("ctc2.i", "ctc2.ni"), &[Rt(R), VuCtrl(W)]).zero(0x7fe),
        0x08 => {
            let (mn, likely) = match (w >> 16) & 31 {
                0 => ("bc2f", false),
                1 => ("bc2t", false),
                2 => ("bc2fl", true),
                3 => ("bc2tl", true),
                _ => return None,
            };
            d(mn, &[Br]).kind(Kind::Branch { likely }).imp(&[VPU_STAT], &[])
        }
        rs if rs & 0x10 != 0 => vu0::macro_op(w)?,
        _ => return None,
    })
}

/// MMI0-MMI3: the sa field selects; rs, rt, rd as each names them.
fn mmi_sub(group: u32, sa: u32) -> Option<Def> {
    const MMI0: [&str; 32] = [
        "paddw", "psubw", "pcgtw", "pmaxw", "paddh", "psubh", "pcgth", "pmaxh", "paddb", "psubb", "pcgtb", "", "", "",
        "", "", "paddsw", "psubsw", "pextlw", "ppacw", "paddsh", "psubsh", "pextlh", "ppach", "paddsb", "psubsb",
        "pextlb", "ppacb", "", "", "pext5", "ppac5",
    ];
    const MMI1: [&str; 32] = [
        "", "pabsw", "pceqw", "pminw", "padsbh", "pabsh", "pceqh", "pminh", "", "", "pceqb", "", "", "", "", "",
        "padduw", "psubuw", "pextuw", "", "padduh", "psubuh", "pextuh", "", "paddub", "psubub", "pextub", "qfsrv", "",
        "", "", "",
    ];
    const MMI2: [&str; 32] = [
        "pmaddw", "", "psllvw", "psrlvw", "pmsubw", "", "", "", "pmfhi", "pmflo", "pinth", "", "pmultw", "pdivw",
        "pcpyld", "", "pmaddh", "phmadh", "pand", "pxor", "pmsubh", "phmsbh", "", "", "", "", "pexeh", "prevh",
        "pmulth", "pdivbw", "pexew", "prot3w",
    ];
    const MMI3: [&str; 32] = [
        "pmadduw", "", "", "psravw", "", "", "", "", "pmthi", "pmtlo", "pinteh", "", "pmultuw", "pdivuw", "pcpyud", "",
        "", "", "por", "pnor", "", "", "", "", "", "", "pexch", "pcpyh", "", "", "pexcw", "",
    ];
    use A::*;
    use T::*;
    let mn = [MMI0, MMI1, MMI2, MMI3][group as usize][sa as usize];
    if mn.is_empty() {
        return None;
    }
    // 128-bit HI and LO: the P-ops use both halves, hi/lo and hi1/lo1.
    const HI128: &[Reg] = &[HI, HI1];
    const LO128: &[Reg] = &[LO, LO1];
    let def = match mn {
        "pabsw" | "pabsh" | "pext5" | "ppac5" | "pexeh" | "prevh" | "pexew" | "prot3w" | "pexch" | "pcpyh"
        | "pexcw" => d(mn, &[Rd(W), Rt(R)]),
        "pmfhi" => d(mn, &[Rd(W)]).imp(HI128, &[]),
        "pmflo" => d(mn, &[Rd(W)]).imp(LO128, &[]),
        "pmthi" => d(mn, &[Rs(R)]).imp(&[], HI128),
        "pmtlo" => d(mn, &[Rs(R)]).imp(&[], LO128),
        "psllvw" | "psrlvw" | "psravw" => d(mn, &[Rd(W), Rt(R), Rs(R)]),
        "pdivw" | "pdivuw" | "pdivbw" => d(mn, &[Rs(R), Rt(R)]).imp(&[], HL128),
        "pmultw" | "pmultuw" | "pmulth" | "phmadh" | "phmsbh" => d(mn, &[Rd(W), Rs(R), Rt(R)]).imp(&[], HL128),
        "pmaddw" | "pmadduw" | "pmaddh" | "pmsubw" | "pmsubh" => d(mn, &[Rd(W), Rs(R), Rt(R)]).imp(HL128, HL128),
        "qfsrv" => d(mn, &[Rd(W), Rs(R), Rt(R)]).imp(&[SA], &[]),
        _ => d(mn, &[Rd(W), Rs(R), Rt(R)]),
    };
    Some(def.fields(RS | RT | RD))
}

fn mmi(w: u32) -> Option<Def> {
    use A::*;
    use T::*;
    let three = &[Rd(W), Rs(R), Rt(R)];
    let sa = (w >> 6) & 31;
    Some(match w & 63 {
        0x00 => r("madd", three).imp(HL, HL),
        0x01 => r("maddu", three).imp(HL, HL),
        0x04 => r("plzcw", &[Rd(W), Rs(R)]),
        0x08 => mmi_sub(0, sa)?,
        0x09 => mmi_sub(2, sa)?,
        0x10 => r("mfhi1", &[Rd(W)]).imp(&[HI1], &[]),
        0x11 => r("mthi1", &[Rs(R)]).imp(&[], &[HI1]),
        0x12 => r("mflo1", &[Rd(W)]).imp(&[LO1], &[]),
        0x13 => r("mtlo1", &[Rs(R)]).imp(&[], &[LO1]),
        0x18 => r("mult1", three).imp(&[], HL1),
        0x19 => r("multu1", three).imp(&[], HL1),
        0x1a => r("div1", &[Rs(R), Rt(R)]).imp(&[], HL1),
        0x1b => r("divu1", &[Rs(R), Rt(R)]).imp(&[], HL1),
        0x20 => r("madd1", three).imp(HL1, HL1),
        0x21 => r("maddu1", three).imp(HL1, HL1),
        0x28 => mmi_sub(1, sa)?,
        0x29 => mmi_sub(3, sa)?,
        0x30 => {
            const PMFHL: [&str; 5] = ["pmfhl.lw", "pmfhl.uw", "pmfhl.slw", "pmfhl.lh", "pmfhl.sh"];
            let mn = *PMFHL.get(sa as usize)?;
            d(mn, &[Rd(W)]).fields(RS | RT).imp(HL128, &[])
        }
        // pmthl.lw replaces words 0 and 2 of HI and LO, keeping the others.
        0x31 if sa == 0 => d("pmthl.lw", &[Rs(R)]).fields(RT | RD).imp(HL128, HL128),
        0x34 => r("psllh", &[Rd(W), Rt(R), Sa]),
        0x36 => r("psrlh", &[Rd(W), Rt(R), Sa]),
        0x37 => r("psrah", &[Rd(W), Rt(R), Sa]),
        0x3c => r("psllw", &[Rd(W), Rt(R), Sa]),
        0x3e => r("psrlw", &[Rd(W), Rt(R), Sa]),
        0x3f => r("psraw", &[Rd(W), Rt(R), Sa]),
        _ => return None,
    })
}
