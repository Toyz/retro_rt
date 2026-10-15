//! Register names and the fixed registers instructions touch without naming
//! them. The numbering of the files outside the GPRs is documented on
//! [`Mips`](super::Mips); the GTE names follow hwtr, the R5900 COP0 and VU0
//! control names follow the EE manuals as piney's disassembler prints them.
//! Encoding fields are not here: they are in `decode`.

use crate::insn::{Reg, RegFile};

/// The o32 names of r0-r31 (r8-r15 stay t0-t7 on the EE too).
pub(super) const GPR: [&str; 32] = [
    "zero", "at", "v0", "v1", "a0", "a1", "a2", "a3", "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7", "s0", "s1", "s2",
    "s3", "s4", "s5", "s6", "s7", "t8", "t9", "k0", "k1", "gp", "sp", "fp", "ra",
];

/// The GTE data registers, as COP2 data registers 0-31.
pub(super) const GTE_DATA: [&str; 32] = [
    "vxy0", "vz0", "vxy1", "vz1", "vxy2", "vz2", "rgbc", "otz", "ir0", "ir1", "ir2", "ir3", "sxy0", "sxy1", "sxy2",
    "sxyp", "sz0", "sz1", "sz2", "sz3", "rgb0", "rgb1", "rgb2", "res1", "mac0", "mac1", "mac2", "mac3", "irgb", "orgb",
    "lzcs", "lzcr",
];

/// The GTE control registers, as COP2 control registers 0-31.
pub(super) const GTE_CTRL: [&str; 32] = [
    "r11r12", "r13r21", "r22r23", "r31r32", "r33", "trx", "try", "trz", "l11l12", "l13l21", "l22l23", "l31l32", "l33",
    "rbk", "gbk", "bbk", "lr1lr2", "lr3lg1", "lg2lg3", "lb1lb2", "lb3", "rfc", "gfc", "bfc", "ofx", "ofy", "h", "dqa",
    "dqb", "zsf3", "zsf4", "flag",
];

/// The R3000A's COP0 registers.
pub(super) const COP0_R3000: [&str; 32] = [
    "c0r0", "c0r1", "c0r2", "bpc", "c0r4", "bda", "jumpdest", "dcic", "badvaddr", "bdam", "c0r10", "bpcm", "sr",
    "cause", "epc", "prid", "c0r16", "c0r17", "c0r18", "c0r19", "c0r20", "c0r21", "c0r22", "c0r23", "c0r24", "c0r25",
    "c0r26", "c0r27", "c0r28", "c0r29", "c0r30", "c0r31",
];

/// The R5900's COP0 registers.
pub(super) const COP0_R5900: [&str; 32] = [
    "Index", "Random", "EntryLo0", "EntryLo1", "Context", "PageMask", "Wired", "c0r7", "BadVAddr", "Count", "EntryHi",
    "Compare", "Status", "Cause", "EPC", "PRId", "Config", "c0r17", "c0r18", "c0r19", "c0r20", "c0r21", "c0r22",
    "BadPAddr", "Debug", "Perf", "c0r26", "c0r27", "TagLo", "TagHi", "ErrorEPC", "c0r31",
];

/// The EE's breakpoint registers, reached through COP0 register 24 by the
/// low bits of `mfc0`/`mtc0`; numbered COP0 32 + those bits.
pub(super) const COP0_DEBUG: [&str; 8] = ["bpc", "c0d1", "iab", "iabm", "dab", "dabm", "dvb", "dvbm"];

/// VU0's control registers 16-31 as `cfc2`/`ctc2` see them (0-15 are vi0-vi15).
pub(super) const VU0_CTRL: [&str; 16] = [
    "Status", "MACflag", "ClipFlag", "c2r19", "R", "I", "Q", "c2r23", "c2r24", "c2r25", "TPC", "CMSAR0", "FBRST",
    "VPU-STAT", "c2r30", "CMSAR1",
];

const fn reg(file: RegFile, index: u8) -> Reg {
    Reg { file, index }
}

/// hi.
pub(super) const HI: Reg = reg(RegFile::Special, 0);
/// lo.
pub(super) const LO: Reg = reg(RegFile::Special, 1);
/// hi1: the EE's pipeline-1 HI, the upper 64 bits of the 128-bit HI.
pub(super) const HI1: Reg = reg(RegFile::Special, 2);
/// lo1.
pub(super) const LO1: Reg = reg(RegFile::Special, 3);
/// The EE's funnel shift amount.
pub(super) const SA: Reg = reg(RegFile::Special, 4);
/// ra, written by the linking jumps and branches.
pub(super) const RA: Reg = Reg::gpr(31);
/// The EE FPU accumulator.
pub(super) const FACC: Reg = reg(RegFile::Fpr, 32);
/// FCR31, whose C bit the compares set and `bc1*` test.
pub(super) const FCR31: Reg = reg(RegFile::FpCtrl, 31);
/// The VU0 accumulator.
pub(super) const VACC: Reg = reg(RegFile::VecFloat, 32);
/// VU0 R (random), as COP2 control 20.
pub(super) const VR: Reg = reg(RegFile::Cop2Ctrl, 20);
/// VU0 I, as COP2 control 21.
pub(super) const VI_REG: Reg = reg(RegFile::Cop2Ctrl, 21);
/// VU0 Q, as COP2 control 22.
pub(super) const VQ: Reg = reg(RegFile::Cop2Ctrl, 22);
/// VU0 clipping flags, as COP2 control 18.
pub(super) const VCLIP: Reg = reg(RegFile::Cop2Ctrl, 18);
/// VPU-STAT, whose VU0 busy bit `bc2f`/`bc2t` test.
pub(super) const VPU_STAT: Reg = reg(RegFile::Cop2Ctrl, 29);
/// R3000A COP0 SR.
pub(super) const SR: Reg = reg(RegFile::Cop0, 12);
/// R5900 COP0 registers the system instructions touch.
pub(super) const C0_INDEX: Reg = reg(RegFile::Cop0, 0);
pub(super) const C0_RANDOM: Reg = reg(RegFile::Cop0, 1);
pub(super) const C0_ENTRYLO0: Reg = reg(RegFile::Cop0, 2);
pub(super) const C0_ENTRYLO1: Reg = reg(RegFile::Cop0, 3);
pub(super) const C0_PAGEMASK: Reg = reg(RegFile::Cop0, 5);
pub(super) const C0_ENTRYHI: Reg = reg(RegFile::Cop0, 10);
pub(super) const C0_STATUS: Reg = reg(RegFile::Cop0, 12);
pub(super) const C0_EPC: Reg = reg(RegFile::Cop0, 14);
pub(super) const C0_ERROREPC: Reg = reg(RegFile::Cop0, 30);

/// A VU0 integer register as a 5-bit field names it: vi0-vi15, and COP2
/// control 16-31 beyond (only `cfc2`/`ctc2` and `vcallmsr` reach those).
pub(super) fn vu_int(index: u32) -> Reg {
    if index < 16 { reg(RegFile::VecInt, index as u8) } else { reg(RegFile::Cop2Ctrl, index as u8) }
}

/// Whether `r` is hard-wired (r0, vf0, vi0): reads of it carry no value and
/// writes to it are lost, so neither is listed.
pub(super) fn hardwired(r: Reg) -> bool {
    r.index == 0 && matches!(r.file, RegFile::Gpr | RegFile::VecFloat | RegFile::VecInt)
}
