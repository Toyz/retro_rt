//! What MIPS instructions compute, for [`Arch::lift`](crate::Arch::lift).
//!
//! Lifted: the integer ALU (including the EE's 64-bit forms, on their low 32
//! bits, so `daddu` is `move` as the EE compiler uses it), `hi`/`lo` and the
//! multiplies and divides, loads and stores of every width, the moves to and
//! from COP0, COP1 and the GTE, `lwc1`/`swc1`/`lwc2`/`swc2`, branch
//! conditions and register jump targets, and the EE's single-precision
//! COP1 (arithmetic, conversions, the accumulator forms, compares into the
//! condition bit). GTE commands and `syscall` are opaque statements; `break`
//! aborts; the conditional traps (GCC's divide checks on the EE) and
//! `sync`/`cache`/`pref` do nothing. Not lifted: the EE's 64-bit shifts and
//! MMI (the IR's values are 32 bits), VU0 macro operations, `movz`/`movn`
//! and the pipeline-1 multiplies, which the decompiler shows as comments.

use super::decode::lookup;
use super::regs::{FACC, FCR31, HI, LO};
use super::{Isa, Mips, fields, simm};
use crate::insn::{Arch, Flow, Insn, Reg, RegFile};
use crate::ir::{BinOp, Effect, Expr, Lifted, UnOp};

fn file(file: RegFile, i: u32) -> Reg {
    Reg { file, index: i as u8 }
}

pub(super) fn lift(mips: &Mips, insn: &Insn) -> Option<Lifted> {
    use BinOp::*;
    if !insn.is_valid() {
        return None;
    }
    let w = insn.word;
    let (rs, rt, rd, sa) = fields(w);
    let ee = mips.isa == Isa::R5900;
    let gpr = |i: u32| if i == 0 { Expr::Const(0) } else { Expr::Reg(Reg::gpr(i as u8)) };
    let (s, t) = (gpr(rs), gpr(rt));
    let simm = Expr::Const(simm(w).into());
    let uimm = Expr::Const((w & 0xffff).into());
    let shamt = Expr::Const(sa.into());
    let bin = |op, a: &Expr, b: &Expr| Expr::bin(op, a.clone(), b.clone()).simplify();
    let addr = bin(Add, &s, &simm);
    // A write to `zero` is lost.
    let set = |r: u32, e: Expr| if r == 0 { vec![] } else { vec![Effect::Set(Reg::gpr(r as u8), e)] };
    let load = |size: u8, signed: bool| set(rt, Expr::load(size, signed, addr.clone()));
    let store = |size: u8| vec![Effect::Store { size, addr: addr.clone(), value: t.clone() }];
    let zero = Expr::Const(0);
    let effects = match insn.mnemonic {
        "sll" if w == 0 => vec![],
        "sll" | "dsll" => set(rd, bin(Shl, &t, &shamt)),
        "srl" => set(rd, bin(Shr, &t, &shamt)),
        "sra" | "dsra" => set(rd, bin(Sar, &t, &shamt)),
        "sllv" | "dsllv" => set(rd, bin(Shl, &t, &s)),
        "srlv" => set(rd, bin(Shr, &t, &s)),
        "srav" | "dsrav" => set(rd, bin(Sar, &t, &s)),
        "mfhi" => set(rd, Expr::Reg(HI)),
        "mflo" => set(rd, Expr::Reg(LO)),
        "mthi" => vec![Effect::Set(HI, s.clone())],
        "mtlo" => vec![Effect::Set(LO, s.clone())],
        "mult" | "multu" => {
            let signed = insn.mnemonic == "mult";
            let lo = bin(MulLo { signed }, &s, &t);
            let mut v = vec![Effect::Set(LO, lo.clone()), Effect::Set(HI, bin(MulHi { signed }, &s, &t))];
            // The EE's three-operand form also writes the low word to rd.
            if ee {
                v.extend(set(rd, lo));
            }
            v
        }
        "div" | "divu" => {
            let signed = insn.mnemonic == "div";
            vec![Effect::Set(LO, bin(Div { signed }, &s, &t)), Effect::Set(HI, bin(Rem { signed }, &s, &t))]
        }
        "add" | "addu" | "dadd" | "daddu" => set(rd, bin(Add, &s, &t)),
        "sub" | "subu" | "dsub" | "dsubu" if rs == 0 => set(rd, Expr::un(UnOp::Neg, t.clone())),
        "sub" | "subu" | "dsub" | "dsubu" => set(rd, bin(Sub, &s, &t)),
        "and" => set(rd, bin(And, &s, &t)),
        "or" => set(rd, bin(Or, &s, &t)),
        "xor" => set(rd, bin(Xor, &s, &t)),
        "nor" if rt == 0 => set(rd, Expr::un(UnOp::Not, s.clone())),
        "nor" => set(rd, Expr::un(UnOp::Not, bin(Or, &s, &t))),
        "slt" => set(rd, bin(Lt, &s, &t)),
        "sltu" => set(rd, bin(Ltu, &s, &t)),
        "addi" | "addiu" | "daddi" | "daddiu" => set(rt, bin(Add, &s, &simm)),
        "slti" => set(rt, bin(Lt, &s, &simm)),
        "sltiu" => set(rt, bin(Ltu, &s, &simm)),
        "andi" => set(rt, bin(And, &s, &uimm)),
        "ori" => set(rt, bin(Or, &s, &uimm)),
        "xori" => set(rt, bin(Xor, &s, &uimm)),
        "lui" => set(rt, Expr::Const(i64::from(((w & 0xffff) << 16) as i32))),
        "lb" => load(1, true),
        "lbu" => load(1, false),
        "lh" => load(2, true),
        "lhu" => load(2, false),
        // lwl/lwr merge into rt; as a pair they load the word.
        "lw" | "lwl" | "lwr" => load(4, true),
        "lwu" => load(4, false),
        "ld" | "ldl" | "ldr" => load(8, true),
        "lq" => load(16, true),
        "sb" => store(1),
        "sh" => store(2),
        "sw" | "swl" | "swr" => store(4),
        "sd" | "sdl" | "sdr" => store(8),
        "sq" => store(16),
        "lwc1" => vec![Effect::Set(file(RegFile::Fpr, rt), Expr::load(4, true, addr.clone()))],
        "swc1" => vec![Effect::Store { size: 4, addr: addr.clone(), value: Expr::Reg(file(RegFile::Fpr, rt)) }],
        // VU0's 128-bit registers from and to memory (vf0 is hard-wired).
        "lqc2" if rt == 0 => vec![],
        "lqc2" => vec![Effect::Set(file(RegFile::VecFloat, rt), Expr::load(16, false, addr.clone()))],
        "sqc2" if rt != 0 => {
            vec![Effect::Store { size: 16, addr: addr.clone(), value: Expr::Reg(file(RegFile::VecFloat, rt)) }]
        }
        // rd = rt == 0 ? rs : rd, and its opposite.
        "movz" => set(rd, Expr::Cond(bin(Eq, &t, &zero).into(), s.clone().into(), gpr(rd).into())),
        "movn" => set(rd, Expr::Cond(bin(Ne, &t, &zero).into(), s.clone().into(), gpr(rd).into())),
        "lwc2" if !ee => vec![Effect::Set(file(RegFile::Cop2Data, rt), Expr::load(4, true, addr.clone()))],
        "swc2" if !ee => {
            vec![Effect::Store { size: 4, addr: addr.clone(), value: Expr::Reg(file(RegFile::Cop2Data, rt)) }]
        }
        "mfc0" => set(rt, Expr::Reg(file(RegFile::Cop0, rd))),
        "mtc0" => vec![Effect::Set(file(RegFile::Cop0, rd), t.clone())],
        "mfc1" => set(rt, Expr::Reg(file(RegFile::Fpr, rd))),
        "mtc1" => vec![Effect::Set(file(RegFile::Fpr, rd), t.clone())],
        "mfc2" if !ee => set(rt, Expr::Reg(file(RegFile::Cop2Data, rd))),
        "cfc2" if !ee => set(rt, Expr::Reg(file(RegFile::Cop2Ctrl, rd))),
        "mtc2" if !ee => vec![Effect::Set(file(RegFile::Cop2Data, rd), t.clone())],
        "ctc2" if !ee => vec![Effect::Set(file(RegFile::Cop2Ctrl, rd), t.clone())],
        // The EE's single-precision COP1: fs in rd's field, ft in rt's, fd
        // in sa's; ACC is the accumulator.
        "add.s" | "sub.s" | "mul.s" | "div.s" | "max.s" | "min.s" | "rsqrt.s" | "madd.s" | "msub.s" | "mov.s"
        | "neg.s" | "abs.s" | "sqrt.s" | "cvt.s.w" | "cvt.w.s" | "adda.s" | "suba.s" | "mula.s" | "madda.s"
        | "msuba.s" | "c.f.s" | "c.eq.s" | "c.lt.s" | "c.le.s" => {
            let f = |i: u32| Expr::Reg(file(RegFile::Fpr, i));
            let (fs, ft, fd, acc) = (f(rd), f(rt), file(RegFile::Fpr, sa), Expr::Reg(FACC));
            let un = |op, a: &Expr| Expr::un(op, a.clone());
            let set = |e| vec![Effect::Set(fd, e)];
            // A compare sets the condition bit, 23, that bc1t/bc1f test.
            let cmp = |op| vec![Effect::Set(FCR31, bin(Shl, &bin(op, &fs, &ft), &Expr::Const(23)))];
            match insn.mnemonic {
                "add.s" => set(bin(FAdd, &fs, &ft)),
                "sub.s" => set(bin(FSub, &fs, &ft)),
                "mul.s" => set(bin(FMul, &fs, &ft)),
                "div.s" => set(bin(FDiv, &fs, &ft)),
                "max.s" => set(bin(FMax, &fs, &ft)),
                "min.s" => set(bin(FMin, &fs, &ft)),
                "rsqrt.s" => set(bin(FDiv, &fs, &un(UnOp::FSqrt, &ft))),
                "madd.s" => set(bin(FAdd, &acc, &bin(FMul, &fs, &ft))),
                "msub.s" => set(bin(FSub, &acc, &bin(FMul, &fs, &ft))),
                "mov.s" => set(fs),
                "neg.s" => set(un(UnOp::FNeg, &fs)),
                "abs.s" => set(un(UnOp::FAbs, &fs)),
                "sqrt.s" => set(un(UnOp::FSqrt, &ft)),
                "cvt.s.w" => set(un(UnOp::IntToFloat, &fs)),
                "cvt.w.s" => set(un(UnOp::FloatToInt, &fs)),
                "adda.s" => vec![Effect::Set(FACC, bin(FAdd, &fs, &ft))],
                "suba.s" => vec![Effect::Set(FACC, bin(FSub, &fs, &ft))],
                "mula.s" => vec![Effect::Set(FACC, bin(FMul, &fs, &ft))],
                "madda.s" => vec![Effect::Set(FACC, bin(FAdd, &acc, &bin(FMul, &fs, &ft)))],
                "msuba.s" => vec![Effect::Set(FACC, bin(FSub, &acc, &bin(FMul, &fs, &ft)))],
                "c.f.s" => vec![Effect::Set(FCR31, Expr::Const(0))],
                "c.eq.s" => cmp(FEq),
                "c.lt.s" => cmp(FLt),
                _ => cmp(FLe),
            }
        }
        "break" => vec![Effect::Abort],
        "syscall" => vec![Effect::Opaque(mips.render(insn, &|_| None))],
        "teq" | "tne" | "tge" | "tgeu" | "tlt" | "tltu" | "teqi" | "tnei" | "tgei" | "tgeiu" | "tlti" | "tltiu" => {
            vec![]
        }
        "sync" | "sync.l" | "sync.p" | "cache" | "pref" => vec![],
        _ if !ee && lookup(mips.isa, w).is_some_and(|d| d.gte.is_some()) => {
            let text = mips.render(insn, &|_| None);
            let (mn, ops) = text.split_once(' ').unwrap_or((&text, ""));
            vec![Effect::Opaque(format!("gte_{mn}({})", ops.trim()))]
        }
        // Branches and jumps carry no effects of their own (a link register
        // is the call's to write).
        _ if insn.flow != Flow::Next => vec![],
        _ => return None,
    };
    let cond = match insn.flow {
        Flow::Branch { .. } | Flow::Call { target: Some(_) } => match insn.mnemonic {
            "beq" | "beql" => Some(bin(Eq, &s, &t)),
            "bne" | "bnel" => Some(bin(Ne, &s, &t)),
            "blez" | "blezl" => Some(bin(Le, &s, &zero)),
            "bgtz" | "bgtzl" => Some(bin(Gt, &s, &zero)),
            "bltz" | "bltzl" | "bltzal" | "bltzall" => Some(bin(Lt, &s, &zero)),
            // bgezal zero is `bal`: always.
            "bgezal" | "bgezall" if rs == 0 => None,
            "bgez" | "bgezl" | "bgezal" | "bgezall" => Some(bin(Ge, &s, &zero)),
            // The FPU's condition bit, fcr31 bit 23.
            "bc1t" | "bc1tl" => Some(bin(Ne, &bin(And, &Expr::Reg(FCR31), &Expr::Const(1 << 23)), &zero)),
            "bc1f" | "bc1fl" => Some(bin(Eq, &bin(And, &Expr::Reg(FCR31), &Expr::Const(1 << 23)), &zero)),
            // A condition outside the registers (the EE's CPCOND0, VU0's
            // busy flag): named, never mistaken for an unconditional jump.
            mn if matches!(insn.flow, Flow::Branch { .. }) => Some(Expr::Var(format!("{mn}_taken"))),
            _ => None,
        },
        _ => None,
    };
    let target = match insn.flow {
        Flow::Jump { target: None } | Flow::Call { target: None } | Flow::Return
            if matches!(insn.mnemonic, "jr" | "jalr") =>
        {
            Some(s)
        }
        _ => None,
    };
    Some(Lifted { effects, cond, target })
}
