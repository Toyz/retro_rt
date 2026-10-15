//! MIPS: the PS1's R3000A and the PS2's R5900.
//!
//! [`Mips`] decodes either into an [`Insn`]: mnemonic and operands, control
//! flow, the registers read and written (named and implied), the memory
//! accessed. It prints them the way hwtr (R3000A) and piney (R5900) print
//! them: o32 register names without `$`, signed decimal for arithmetic
//! immediates and offsets, hex for logical immediates, `lui` and codes,
//! branch and jump targets as absolute addresses through a label lookup,
//! and pseudo-ops for the common idioms unless asked for canonical forms.
//!
//! - R3000A: MIPS I, COP0 as the PS1 has it (`mfc0`, `mtc0`, `rfe`) and
//!   COP2 as the GTE (`mfc2`/`cfc2`/`mtc2`/`ctc2`, `lwc2`/`swc2` and the
//!   commands by name with their sf/lm/mvmva fields). No COP1, no COP3.
//! - R5900: MIPS III's 64-bit integer set, the MIPS IV `movz`/`movn`/`pref`,
//!   the EE additions (`lq`/`sq`, three-operand multiplies, the pipeline-1
//!   `mult1`/`div1`/`madd1` family, `mfsa`/`mtsa`/`mtsab`/`mtsah`), all of
//!   MMI, the EE's COP0 (`eret`, `ei`, `di`, `tlb*`, `bc0*`, the breakpoint
//!   and performance counter moves), its single-precision COP1 with the
//!   accumulator, and COP2 as VU0 in macro mode. No doubles, no `ll`/`sc`,
//!   no 64-bit multiply or divide: the EE has none.
//!
//! Not here: VU microcode (VU0/VU1 micro mode). The analysis that uses
//! these decodes is [`crate::analysis`], the symbols [`crate::symbols`].
//!
//! Register numbering beyond the GPRs ([`RegFile`] index):
//!
//! | file | indices |
//! |------|---------|
//! | `Special` | 0 hi, 1 lo, 2 hi1, 3 lo1 (the EE's upper halves of HI/LO), 4 sa |
//! | `Cop0` | 0-31 the COP0 registers; R5900 also 32 + n the breakpoint registers `mfc0` reaches through register 24 with low bits n (32 bpc, 34 iab, 35 iabm, 36 dab, 37 dabm, 38 dvb, 39 dvbm), 40 + n performance counter n (`mfpc`); `mfps` is 25 |
//! | `Fpr` | 0-31 f0-f31, 32 the EE FPU accumulator (`ACC`) |
//! | `FpCtrl` | 0-31 fcr0-fcr31 (the compares write and `bc1*` read fcr31) |
//! | `Cop2Data` | R3000A: GTE data registers 0-31 (`vxy0` ... `lzcr`) |
//! | `Cop2Ctrl` | R3000A: GTE control registers 0-31 (`r11r12` ... `flag`). R5900: VU0 control registers 16-31 (`Status`, `MACflag`, `ClipFlag`, 20 `R`, 21 `I`, 22 `Q`, `TPC`, 27 `CMSAR0`, `FBRST`, 29 `VPU-STAT`, `CMSAR1`); 0-15 are `VecInt` |
//! | `VecFloat` | vf0-vf31, 32 the VU0 accumulator (`ACC`) |
//! | `VecInt` | vi0-vi15 |
//!
//! `reads` and `writes` leave out the hard-wired registers (r0, vf0, vi0):
//! reading one carries no value and writing one is lost. A 128-bit MMI
//! operation on HI/LO lists both halves (hi and hi1, lo and lo1). The
//! VU0's MAC and status flags, which every FMAC operation updates, are not
//! listed; the clipping flags `vclipw` sets are.
//!
//! Pseudo-ops, printed when [`Mips::pseudo`] is set (as hwtr and GNU
//! objdump do), canonical forms otherwise:
//!
//! | pseudo-op | for |
//! |-----------|-----|
//! | `nop` | `sll zero, zero, 0` |
//! | `move d, s` | `addu`/`or`/`daddu` with rt (or rs) zero |
//! | `li t, imm` | `addiu t, zero, imm` (decimal), `ori t, zero, imm` (hex) |
//! | `b target` | `beq zero, zero, target`, `bgez zero, target` |
//! | `bal target` | `bgezal zero, target` |
//! | `beqz`/`bnez`/`beqzl`/`bnezl s, target` | `beq`/`bne`/`beql`/`bnel s, zero, target` |
//! | `negu`/`neg`/`dnegu`/`dneg d, t` | `subu`/`sub`/`dsubu`/`dsub d, zero, t` |
//! | `not d, s` | `nor d, s, zero` |
//! | `jalr s` | `jalr ra, s` |
//! | `mult s, t` (and `multu`, `madd`, `maddu`, `mult1`, `multu1`, `madd1`, `maddu1`) | the R5900's three-operand form with rd zero |
//!
//! Operand conventions: `lui`'s immediate is the 16 bits as written (not
//! shifted); `syscall` carries one [`Operand::Code`], `break` two (the
//! high and low 10-bit codes) and the traps one, printed only when nonzero
//! as GNU objdump prints them; `cache` and `pref` carry a memory operand of
//! size 0 and no [`Insn::mem`]. A memory operand goes through the label
//! lookup only when its base is `zero` (the address is then the offset);
//! a GTE command's fields are immediates (sf, lm; for `mvmva` also mx, v,
//! cv). VU0 mnemonics carry their dest mask (`vadd.xyz`); the VU0 loads
//! and stores (`vlqi`, `vsqd`, `vilwr`, ...) address VU0 data memory, not
//! the EE's, so they have no [`Insn::mem`].
//!
//! Control flow: `jal`, `bal` and the linking branches are calls; `jalr`
//! is a call to an unknown target, or a jump (a return through ra) when rd
//! is zero; `jr ra` returns, other `jr` jump; `beq zero, zero` and
//! `bgez zero` (and their likely forms) are jumps; `syscall`, `break` and
//! the conditional traps trap; `eret` returns, with no delay slot.

mod decode;
mod gte;
mod regs;
mod vu0;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::insn::{Arch, Endian, Flow, Insn, Mem, Operand, Reg, RegFile};
use decode::{A, Def, Kind, T};
use regs::*;

/// Which MIPS CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Isa {
    /// The PS1's R3000A: MIPS I with the GTE as COP2.
    R3000,
    /// The PS2's Emotion Engine core: MIPS III/IV integer, MMI, single
    /// float COP1, VU0 as COP2.
    R5900,
}

/// A MIPS instruction set: decoding and printing for one CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Mips {
    /// The CPU.
    pub isa: Isa,
    /// Print pseudo-ops (`nop`, `move`, `li`, `b`, ...) for the idioms they
    /// name; false prints every instruction in its canonical form.
    pub pseudo: bool,
}

impl Mips {
    /// `isa`, with pseudo-ops.
    pub const fn new(isa: Isa) -> Mips {
        Mips { isa, pseudo: true }
    }

    /// The PS1's R3000A, with pseudo-ops.
    pub const fn r3000() -> Mips {
        Mips::new(Isa::R3000)
    }

    /// The PS2's R5900, with pseudo-ops.
    pub const fn r5900() -> Mips {
        Mips::new(Isa::R5900)
    }

    /// The same CPU printing canonical forms only (no pseudo-ops).
    pub const fn canonical(self) -> Mips {
        Mips { pseudo: false, ..self }
    }

    /// Decodes one instruction word at `addr`.
    pub fn decode_word(&self, word: u32, addr: u32) -> Insn {
        let Some(def) = decode::lookup(self.isa, word) else {
            return invalid(word, addr, 4);
        };
        let mut b = Build { ops: Vec::new(), reads: Vec::new(), writes: Vec::new(), mem: None };
        for &t in def.toks {
            b.token(&def, t, word, addr);
        }
        b.reads.extend_from_slice(def.imp_r);
        b.writes.extend_from_slice(def.imp_w);
        if let Some(c) = def.gte {
            let (mut dr, mut cr) = (c.data_r, c.ctrl_r);
            if c.name == "mvmva" {
                let (d, c) = gte::mvmva_reads(word);
                dr |= d;
                cr |= c;
            }
            b.reads.extend(gte::regs(dr, RegFile::Cop2Data).chain(gte::regs(cr, RegFile::Cop2Ctrl)));
            b.writes.extend(gte::regs(c.data_w, RegFile::Cop2Data).chain(gte::regs(c.ctrl_w, RegFile::Cop2Ctrl)));
        }
        let (flow, delay_slot) = flow(&def, word, addr);
        Insn {
            addr,
            word,
            size: 4,
            mnemonic: if def.dest { with_dest(def.mn, (word >> 21) & 15) } else { def.mn },
            operands: b.ops,
            flow,
            delay_slot,
            reads: tidy(b.reads),
            writes: tidy(b.writes),
            mem: b.mem,
        }
    }

    /// The text of one operand token (none for an elided code, two for
    /// `break`'s pair).
    fn token_text(&self, t: T, w: u32, addr: u32, label: &dyn Fn(u32) -> Option<String>) -> Vec<String> {
        let (rs, rt, rd, sa) = fields(w);
        let gpr = |i: u32| GPR[i as usize].to_string();
        let at = |a: u32| label(a).unwrap_or_else(|| format!("0x{a:08x}"));
        let name = |r: Reg| self.reg_name(r);
        let vf = |i: u32| format!("vf{i}");
        let vi = |i: u32| self.reg_name(vu_int(i));
        let one = |s: String| vec![s];
        match t {
            T::Rs(_) => one(gpr(rs)),
            T::Rt(_) => one(gpr(rt)),
            T::Rd(_) => one(gpr(rd)),
            T::Sa => one(sa.to_string()),
            T::Simm => one(simm(w).to_string()),
            T::Uimm => one(format!("0x{:x}", w & 0xffff)),
            T::Mem => {
                let off = simm(w);
                let place = if rs == 0 { label(off as u32) } else { None };
                one(format!("{}({})", place.unwrap_or_else(|| off.to_string()), gpr(rs)))
            }
            T::Br => one(at(branch_target(w, addr))),
            T::Jt => one(at(jump_target(w, addr))),
            T::Syscall => {
                let code = (w >> 6) & 0xf_ffff;
                if code == 0 { vec![] } else { one(format!("0x{code:x}")) }
            }
            T::Break => {
                let (hi, lo) = ((w >> 16) & 0x3ff, (w >> 6) & 0x3ff);
                match (hi, lo) {
                    (0, 0) => vec![],
                    (_, 0) => one(format!("0x{hi:x}")),
                    _ => vec![format!("0x{hi:x}"), format!("0x{lo:x}")],
                }
            }
            T::Tcode => {
                let code = (w >> 6) & 0x3ff;
                if code == 0 { vec![] } else { one(format!("0x{code:x}")) }
            }
            T::Hint => one(format!("0x{rt:x}")),
            T::C0(_) => one(name(Reg { file: RegFile::Cop0, index: rd as u8 })),
            T::C0Debug(_) => vec![],
            T::C0Perf(_) => one(((w >> 1) & 31).to_string()),
            T::Fs(_) => one(format!("f{rd}")),
            T::Ft(_) => one(format!("f{rt}")),
            T::Fd(_) => one(format!("f{sa}")),
            T::Fcr(_) => one(format!("fcr{rd}")),
            T::GteD(_) => one(GTE_DATA[rd as usize].to_string()),
            T::GteC(_) => one(GTE_CTRL[rd as usize].to_string()),
            T::GteRt(_) => one(GTE_DATA[rt as usize].to_string()),
            T::Gte => one(format!("sf={}, lm={}", (w >> 19) & 1, (w >> 10) & 1)),
            T::Mvmva => one(format!(
                "sf={}, lm={}, mx={}, v={}, cv={}",
                (w >> 19) & 1,
                (w >> 10) & 1,
                (w >> 17) & 3,
                (w >> 15) & 3,
                (w >> 13) & 3
            )),
            T::VfRd(_) | T::Vfs(_) => one(vf(rd)),
            T::Vft(_) => one(vf(rt)),
            T::Vfd(_) => one(vf(sa)),
            T::VftBc => one(format!("vf{rt}{}", XYZW[(w & 3) as usize])),
            T::VfsF => one(format!("vf{rd}{}", XYZW[((w >> 21) & 3) as usize])),
            T::VftF => one(format!("vf{rt}{}", XYZW[((w >> 23) & 3) as usize])),
            T::Vit(_) => one(vi(rt)),
            T::Vis(_) => one(vi(rd)),
            T::Vid(_) => one(vi(sa)),
            T::VisInc => one(format!("({}++)", vi(rd))),
            T::VitInc => one(format!("({}++)", vi(rt))),
            T::VisDec => one(format!("(--{})", vi(rd))),
            T::VitDec => one(format!("(--{})", vi(rt))),
            T::VisAt => one(format!("({})", vi(rd))),
            T::VuCtrl(_) => one(vi(rd)),
            T::Acc(_) => one("ACC".into()),
            T::Q(_) => one("Q".into()),
            T::I => one("I".into()),
            T::Rand(_) => one("R".into()),
            T::Vimm5 => one(vimm5(w).to_string()),
            T::Vimm15 => one(format!("0x{:04x}", vimm15(w))),
        }
    }
}

/// The dest mask's letters, x first (bit 24).
const XYZW: [&str; 4] = ["x", "y", "z", "w"];

fn fields(w: u32) -> (u32, u32, u32, u32) {
    ((w >> 21) & 31, (w >> 16) & 31, (w >> 11) & 31, (w >> 6) & 31)
}

fn simm(w: u32) -> i32 {
    i32::from(w as u16 as i16)
}

fn vimm5(w: u32) -> i32 {
    let v = ((w >> 6) & 31) as i32;
    if v & 16 != 0 { v - 32 } else { v }
}

/// `vcallms`' micro address: the field counts 64-bit micro instructions;
/// the byte address is what microcode listings use.
fn vimm15(w: u32) -> u32 {
    ((w >> 6) & 0x7fff) * 8
}

fn branch_target(w: u32, addr: u32) -> u32 {
    addr.wrapping_add(4).wrapping_add((simm(w) << 2) as u32)
}

fn jump_target(w: u32, addr: u32) -> u32 {
    (addr.wrapping_add(4) & 0xf000_0000) | ((w & 0x03ff_ffff) << 2)
}

fn invalid(word: u32, addr: u32, size: u8) -> Insn {
    Insn {
        addr,
        word,
        size,
        mnemonic: ".word",
        operands: Vec::new(),
        flow: Flow::Invalid,
        delay_slot: false,
        reads: Vec::new(),
        writes: Vec::new(),
        mem: None,
    }
}

/// Drops hard-wired registers and repeats, keeping first-seen order.
fn tidy(regs: Vec<Reg>) -> Vec<Reg> {
    let mut out = Vec::with_capacity(regs.len());
    for r in regs {
        if !hardwired(r) && !out.contains(&r) {
            out.push(r);
        }
    }
    out
}

/// `base.dest` for a VU0 op, interned: each of the few hundred names is
/// built once and lives for the program, so [`Insn::mnemonic`] stays a
/// `&'static str`.
fn with_dest(base: &'static str, dest: u32) -> &'static str {
    if dest == 0 {
        return base;
    }
    static NAMES: OnceLock<Mutex<HashMap<(&'static str, u32), &'static str>>> = OnceLock::new();
    let mut names = NAMES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    names.entry((base, dest)).or_insert_with(|| {
        let mask: String = (0..4).filter(|k| dest & (8 >> k) != 0).map(|k| XYZW[k]).collect();
        Box::leak(format!("{base}.{mask}").into_boxed_str())
    })
}

/// Operands and register accesses as the tokens are walked.
struct Build {
    ops: Vec<Operand>,
    reads: Vec<Reg>,
    writes: Vec<Reg>,
    mem: Option<Mem>,
}

impl Build {
    fn access(&mut self, r: Reg, a: A) {
        if a != A::W {
            self.reads.push(r);
        }
        if a != A::R {
            self.writes.push(r);
        }
    }

    fn reg(&mut self, r: Reg, a: A) {
        self.ops.push(Operand::Reg(r));
        self.access(r, a);
    }

    fn token(&mut self, def: &Def, t: T, w: u32, addr: u32) {
        let (rs, rt, rd, sa) = fields(w);
        let file = |file: RegFile, i: u32| Reg { file, index: i as u8 };
        let vf = |i: u32| file(RegFile::VecFloat, i);
        match t {
            T::Rs(a) => self.reg(Reg::gpr(rs as u8), a),
            T::Rt(a) => self.reg(Reg::gpr(rt as u8), a),
            T::Rd(a) => self.reg(Reg::gpr(rd as u8), a),
            T::Sa => self.ops.push(Operand::Imm(sa.into())),
            T::Simm => self.ops.push(Operand::Imm(simm(w).into())),
            T::Uimm => self.ops.push(Operand::Imm((w & 0xffff).into())),
            T::Mem => {
                let base = Reg::gpr(rs as u8);
                let (size, store) = decode::mem_access(def.mn).unwrap_or((0, false));
                let m = Mem { base, offset: simm(w), size, store };
                self.ops.push(Operand::Mem(m));
                self.reads.push(base);
                if size > 0 {
                    self.mem = Some(m);
                }
            }
            T::Br => self.ops.push(Operand::Target(branch_target(w, addr))),
            T::Jt => self.ops.push(Operand::Target(jump_target(w, addr))),
            T::Syscall => self.ops.push(Operand::Code((w >> 6) & 0xf_ffff)),
            T::Break => {
                self.ops.push(Operand::Code((w >> 16) & 0x3ff));
                self.ops.push(Operand::Code((w >> 6) & 0x3ff));
            }
            T::Tcode => self.ops.push(Operand::Code((w >> 6) & 0x3ff)),
            T::Hint => self.ops.push(Operand::Imm(rt.into())),
            T::C0(a) => self.reg(file(RegFile::Cop0, rd), a),
            T::C0Debug(a) => self.access(file(RegFile::Cop0, 32 + (w & 7)), a),
            T::C0Perf(a) => {
                let n = (w >> 1) & 31;
                self.ops.push(Operand::Imm(n.into()));
                let r = if w & 1 == 0 { 25 } else { 40 + n };
                self.access(file(RegFile::Cop0, r), a);
            }
            T::Fs(a) => self.reg(file(RegFile::Fpr, rd), a),
            T::Ft(a) => self.reg(file(RegFile::Fpr, rt), a),
            T::Fd(a) => self.reg(file(RegFile::Fpr, sa), a),
            T::Fcr(a) => self.reg(file(RegFile::FpCtrl, rd), a),
            T::GteD(a) => self.reg(file(RegFile::Cop2Data, rd), a),
            T::GteC(a) => self.reg(file(RegFile::Cop2Ctrl, rd), a),
            T::GteRt(a) => self.reg(file(RegFile::Cop2Data, rt), a),
            T::Gte => {
                self.ops.push(Operand::Imm(((w >> 19) & 1).into()));
                self.ops.push(Operand::Imm(((w >> 10) & 1).into()));
            }
            T::Mvmva => {
                for v in [(w >> 19) & 1, (w >> 10) & 1, (w >> 17) & 3, (w >> 15) & 3, (w >> 13) & 3] {
                    self.ops.push(Operand::Imm(v.into()));
                }
            }
            T::VfRd(a) | T::Vfs(a) => self.reg(vf(rd), a),
            T::Vft(a) => self.reg(vf(rt), a),
            T::Vfd(a) => self.reg(vf(sa), a),
            T::VftBc | T::VftF => self.reg(vf(rt), A::R),
            T::VfsF => self.reg(vf(rd), A::R),
            T::Vit(a) => self.reg(vu_int(rt), a),
            T::Vis(a) => self.reg(vu_int(rd), a),
            T::Vid(a) => self.reg(vu_int(sa), a),
            T::VisInc | T::VisDec => self.reg(vu_int(rd), A::Rw),
            T::VitInc | T::VitDec => self.reg(vu_int(rt), A::Rw),
            T::VisAt => self.reg(vu_int(rd), A::R),
            T::VuCtrl(a) => self.reg(vu_int(rd), a),
            T::Acc(a) => self.reg(VACC, a),
            T::Q(a) => self.reg(VQ, a),
            T::I => self.reg(VI_REG, A::R),
            T::Rand(a) => self.reg(VR, a),
            T::Vimm5 => self.ops.push(Operand::Imm(vimm5(w).into())),
            T::Vimm15 => self.ops.push(Operand::Imm(vimm15(w).into())),
        }
    }
}

/// The flow and whether there is a delay slot.
fn flow(def: &Def, w: u32, addr: u32) -> (Flow, bool) {
    let (rs, rt, rd, _) = fields(w);
    match def.kind {
        Kind::Plain => (Flow::Next, false),
        Kind::Trap => (Flow::Trap, false),
        Kind::Eret => (Flow::Return, false),
        Kind::Branch { likely } => {
            let target = branch_target(w, addr);
            // beq zero, zero and bgez zero are always taken: GNU's `b`.
            let always = match def.mn {
                "beq" | "beql" => rs == 0 && rt == 0,
                "bgez" | "bgezl" => rs == 0,
                _ => false,
            };
            let f = if always { Flow::Jump { target: Some(target) } } else { Flow::Branch { target, likely } };
            (f, true)
        }
        Kind::Link => (Flow::Call { target: Some(branch_target(w, addr)) }, true),
        Kind::Jump => (Flow::Jump { target: Some(jump_target(w, addr)) }, true),
        Kind::Call => (Flow::Call { target: Some(jump_target(w, addr)) }, true),
        Kind::Jr | Kind::Jalr if def.kind == Kind::Jr || rd == 0 => {
            (if rs == 31 { Flow::Return } else { Flow::Jump { target: None } }, true)
        }
        Kind::Jalr | Kind::Jr => (Flow::Call { target: None }, true),
    }
}

impl Arch for Mips {
    fn name(&self) -> &'static str {
        match self.isa {
            Isa::R3000 => "R3000A",
            Isa::R5900 => "R5900",
        }
    }

    fn endian(&self) -> Endian {
        Endian::Little
    }

    fn align(&self) -> u32 {
        4
    }

    fn decode(&self, bytes: &[u8], addr: u32) -> Insn {
        match bytes.first_chunk::<4>() {
            Some(b) => self.decode_word(u32::from_le_bytes(*b), addr),
            None => {
                let word = bytes.iter().rev().fold(0, |w, &b| (w << 8) | u32::from(b));
                invalid(word, addr, bytes.len() as u8)
            }
        }
    }

    fn reg_name(&self, reg: Reg) -> String {
        let i = reg.index as usize;
        let ee = self.isa == Isa::R5900;
        match reg.file {
            RegFile::Gpr if i < 32 => GPR[i].into(),
            RegFile::Special if i < 5 => ["hi", "lo", "hi1", "lo1", "sa"][i].into(),
            RegFile::Cop0 if i < 32 => if ee { COP0_R5900[i] } else { COP0_R3000[i] }.into(),
            RegFile::Cop0 if ee && i < 40 => COP0_DEBUG[i - 32].into(),
            RegFile::Cop0 if ee => format!("pcr{}", i - 40),
            RegFile::Fpr if i == 32 => "ACC".into(),
            RegFile::Fpr => format!("f{i}"),
            RegFile::FpCtrl => format!("fcr{i}"),
            RegFile::Cop2Data if !ee && i < 32 => GTE_DATA[i].into(),
            RegFile::Cop2Ctrl if !ee && i < 32 => GTE_CTRL[i].into(),
            RegFile::Cop2Ctrl if ee && (16..32).contains(&i) => VU0_CTRL[i - 16].into(),
            RegFile::Cop2Ctrl if ee && i < 16 => format!("vi{i}"),
            RegFile::VecFloat if i == 32 => "ACC".into(),
            RegFile::VecFloat => format!("vf{i}"),
            RegFile::VecInt => format!("vi{i}"),
            file => format!("{file:?}{i}").to_lowercase(),
        }
    }

    fn render(&self, insn: &Insn, label: &dyn Fn(u32) -> Option<String>) -> String {
        let w = insn.word;
        let def = if insn.is_valid() { decode::lookup(self.isa, w) } else { None };
        let Some(def) = def else {
            return format!(".word   0x{w:08x}");
        };
        let mut ops: Vec<String> = def.toks.iter().flat_map(|&t| self.token_text(t, w, insn.addr, label)).collect();
        let mut mn = insn.mnemonic;
        if self.pseudo {
            let (rs, rt, rd, _) = fields(w);
            let ee = self.isa == Isa::R5900;
            match def.mn {
                "sll" if w == 0 => {
                    mn = "nop";
                    ops.clear();
                }
                "addu" | "or" | "daddu" if rt == 0 => {
                    mn = "move";
                    ops.remove(2);
                }
                "addu" | "or" | "daddu" if rs == 0 => {
                    mn = "move";
                    ops.remove(1);
                }
                "addiu" | "ori" if rs == 0 => {
                    mn = "li";
                    ops.remove(1);
                }
                "beq" if rs == 0 && rt == 0 => {
                    mn = "b";
                    ops.drain(..2);
                }
                "beq" | "bne" | "beql" | "bnel" if rt == 0 => {
                    mn = match def.mn {
                        "beq" => "beqz",
                        "bne" => "bnez",
                        "beql" => "beqzl",
                        _ => "bnezl",
                    };
                    ops.remove(1);
                }
                "bgez" | "bgezal" if rs == 0 => {
                    mn = if def.mn == "bgez" { "b" } else { "bal" };
                    ops.remove(0);
                }
                "sub" | "subu" | "dsub" | "dsubu" if rs == 0 => {
                    mn = match def.mn {
                        "sub" => "neg",
                        "subu" => "negu",
                        "dsub" => "dneg",
                        _ => "dnegu",
                    };
                    ops.remove(1);
                }
                "nor" if rt == 0 => {
                    mn = "not";
                    ops.remove(2);
                }
                "jalr" if rd == 31 => {
                    ops.remove(0);
                }
                "mult" | "multu" | "madd" | "maddu" | "mult1" | "multu1" | "madd1" | "maddu1" if ee && rd == 0 => {
                    ops.remove(0);
                }
                _ => {}
            }
        }
        if ops.is_empty() { mn.to_string() } else { format!("{mn:<7} {}", ops.join(", ")) }
    }

    /// The value written to the destination GPR of `lui`, `addiu`/`addi`/
    /// `daddiu`/`daddi`, `ori`, `addu`/`add`/`daddu`/`dadd`, `or` (so
    /// `move` and `li`), `subu`/`sub`/`dsubu`/`dsub` and `sll`, on the low
    /// 32 bits with wrapping. `zero` is known as 0 without asking `known`.
    fn evaluate(&self, insn: &Insn, known: &dyn Fn(Reg) -> Option<u32>) -> Option<u32> {
        if !insn.is_valid() || !insn.writes.iter().any(|r| r.file == RegFile::Gpr) {
            return None;
        }
        let w = insn.word;
        let (rs, rt, _, sa) = fields(w);
        let get = |i: u32| if i == 0 { Some(0) } else { known(Reg::gpr(i as u8)) };
        let imm = simm(w) as u32;
        Some(match insn.mnemonic {
            "lui" => (w & 0xffff) << 16,
            "addiu" | "addi" | "daddiu" | "daddi" => get(rs)?.wrapping_add(imm),
            "ori" => get(rs)? | (w & 0xffff),
            "addu" | "add" | "daddu" | "dadd" => get(rs)?.wrapping_add(get(rt)?),
            "or" => get(rs)? | get(rt)?,
            "subu" | "sub" | "dsubu" | "dsub" => get(rs)?.wrapping_sub(get(rt)?),
            "sll" => get(rt)? << sa,
            _ => return None,
        })
    }

    fn global_pointer(&self) -> Option<Reg> {
        Some(Reg::gpr(28))
    }

    fn stack_pointer(&self) -> Option<Reg> {
        Some(Reg::gpr(29))
    }

    /// `sltiu t, s, N`'s N, sign-extended to 32 bits as the compare uses it.
    fn switch_bound(&self, insn: &Insn) -> Option<u32> {
        (insn.is_valid() && insn.mnemonic == "sltiu").then(|| simm(insn.word) as u32)
    }
}
