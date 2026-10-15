//! A decoded instruction, the same shape for every instruction set, and the
//! [`Arch`] trait a backend implements.

use crate::ir::{BinOp, Effect, Expr, Lifted};

/// Byte order of an instruction set's code and data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Endian {
    /// Least significant byte first: MIPS as the PlayStations run it, x86,
    /// ARM as the GBA runs it, the 6502, the Z80.
    Little,
    /// Most significant byte first: the 68000, SH-2, SH-4 (as Sega ran them),
    /// PowerPC.
    Big,
}

/// Which register file a register is in. What each holds is the
/// architecture's; [`Arch::reg_name`] names them. More files may come with
/// more backends, so a match on it needs a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum RegFile {
    /// General-purpose registers.
    Gpr,
    /// Special registers outside the files (MIPS hi, lo, hi1, lo1, sa).
    Special,
    /// Coprocessor 0 (system control).
    Cop0,
    /// Floating-point registers (MIPS COP1).
    Fpr,
    /// Floating-point control registers.
    FpCtrl,
    /// Coprocessor 2 data registers (the PS1 GTE's).
    Cop2Data,
    /// Coprocessor 2 control registers (the PS1 GTE's).
    Cop2Ctrl,
    /// Vector float registers (the PS2 VU0's vf0-vf31).
    VecFloat,
    /// Vector integer registers (the PS2 VU0's vi0-vi15).
    VecInt,
    /// Condition flags, one register per flag (x86's carry, zero, sign,
    /// overflow; the 68000's condition codes).
    Flags,
    /// Segment registers (x86 real and protected mode).
    Segment,
}

/// A register: its file and number within it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Reg {
    /// The file.
    pub file: RegFile,
    /// The number within the file.
    pub index: u8,
}

impl Reg {
    /// General-purpose register `index`.
    pub const fn gpr(index: u8) -> Reg {
        Reg { file: RegFile::Gpr, index }
    }
}

/// A memory access an instruction makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Mem {
    /// The base register.
    pub base: Reg,
    /// The signed offset added to it.
    pub offset: i32,
    /// Bytes accessed (1, 2, 4, 8, 16); the unaligned MIPS forms give the
    /// full width they merge into.
    pub size: u8,
    /// A store, not a load.
    pub store: bool,
}

/// One operand, in the order the instruction is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operand {
    /// A register.
    Reg(Reg),
    /// An immediate value as the instruction means it (sign-extended where
    /// the instruction sign-extends).
    Imm(i64),
    /// A code address: a branch or jump target, absolute.
    Target(u32),
    /// A memory operand: `offset(base)`.
    Mem(Mem),
    /// A code field with no arithmetic meaning (MIPS `syscall`/`break`).
    Code(u32),
}

/// What an instruction does to control flow. More kinds may come with more
/// backends, so a match on it needs a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Flow {
    /// Falls through to the next instruction.
    Next,
    /// A conditional branch to `target`; `likely` when the delay slot runs
    /// only if taken (MIPS branch-likely).
    Branch {
        /// Where it goes when taken.
        target: u32,
        /// The delay slot is annulled when not taken.
        likely: bool,
    },
    /// An unconditional jump: to `target` when the instruction says
    /// (`j`, `b`), None through a register (`jr` other than a return: a
    /// jump table or a tail call).
    Jump {
        /// The target, when the instruction carries it.
        target: Option<u32>,
    },
    /// A call: to `target` when direct (`jal`, `bal`), None through a
    /// register (`jalr`). Control comes back after it.
    Call {
        /// The target, when the instruction carries it.
        target: Option<u32>,
    },
    /// A return to the caller (MIPS `jr ra`).
    Return,
    /// A trap to the system that comes back (`syscall`, the conditional
    /// traps).
    Trap,
    /// A trap that does not come back: the program stops (MIPS `break`, as
    /// compilers emit it after a call that does not return or to abort).
    /// The walk ends here.
    Halt,
    /// Not a valid instruction.
    Invalid,
}

impl Flow {
    /// Whether control can continue to the next instruction (after any delay
    /// slot): everything but an unconditional jump, a return, a halt and
    /// an invalid word. A call continues; a [`Flow::Trap`] returns.
    pub fn falls_through(self) -> bool {
        !matches!(self, Flow::Jump { .. } | Flow::Return | Flow::Halt | Flow::Invalid)
    }

    /// The code address the instruction names, if any.
    pub fn target(self) -> Option<u32> {
        match self {
            Flow::Branch { target, .. } => Some(target),
            Flow::Jump { target } | Flow::Call { target } => target,
            _ => None,
        }
    }
}

/// A decoded instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insn {
    /// Its address.
    pub addr: u32,
    /// Its bytes, as a little or big-endian word per the [`Arch`] (the low
    /// `size` bytes are the instruction).
    pub word: u32,
    /// Bytes it takes.
    pub size: u8,
    /// The mnemonic as the manuals write it (`addiu`, `lq`, `rtps`); a
    /// renderer may print a pseudo-op instead (`move`, `li`, `nop`).
    pub mnemonic: &'static str,
    /// Operands, in written order.
    pub operands: Vec<Operand>,
    /// Its effect on control flow.
    pub flow: Flow,
    /// The next instruction executes before the transfer takes effect.
    pub delay_slot: bool,
    /// Registers it reads.
    pub reads: Vec<Reg>,
    /// Registers it writes (a write to a hard-wired zero is not listed).
    pub writes: Vec<Reg>,
    /// The memory it loads or stores, if any.
    pub mem: Option<Mem>,
}

impl Insn {
    /// Whether this is a valid instruction.
    pub fn is_valid(&self) -> bool {
        self.flow != Flow::Invalid
    }

    /// The immediate operand, if it has exactly one.
    pub fn imm(&self) -> Option<i64> {
        let mut imms = self.operands.iter().filter_map(|o| match o {
            Operand::Imm(v) => Some(*v),
            _ => None,
        });
        let first = imms.next();
        if imms.next().is_some() { None } else { first }
    }
}

/// A calling convention: which registers carry what across a call. The
/// decompiler counts a callee's arguments, knows what a call may change, and
/// recognises a prologue's saves by it; the analysis takes the stack and
/// global pointers from it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallConv {
    /// Registers that carry arguments, in order (MIPS a0-a3). Empty for a
    /// convention that passes everything on the stack.
    pub args: &'static [Reg],
    /// Registers that carry results, the main one first (MIPS v0, v1).
    pub results: &'static [Reg],
    /// Registers a callee preserves (MIPS s0-s7, fp, ra).
    pub saved: &'static [Reg],
    /// Registers a call may change (MIPS at, v0-v1, a0-a3, t0-t9, hi, lo).
    pub clobbered: &'static [Reg],
    /// The stack pointer.
    pub stack_pointer: Option<Reg>,
    /// The register small data is addressed from (MIPS gp).
    pub global_pointer: Option<Reg>,
    /// Where a call leaves its return address, if in a register (MIPS ra).
    pub return_address: Option<Reg>,
}

/// An instruction set: how to decode it, print it and say what it does.
///
/// A backend implements the required methods ([`name`](Arch::name),
/// [`endian`](Arch::endian), [`align`](Arch::align),
/// [`decode`](Arch::decode), [`reg_name`](Arch::reg_name),
/// [`render`](Arch::render)) for disassembly. [`lift`](Arch::lift) and
/// [`call_conv`](Arch::call_conv) add the analysis's constant tracking and
/// the decompiler: [`evaluate`](Arch::evaluate),
/// [`switch_bound`](Arch::switch_bound), [`stack_pointer`](Arch::stack_pointer)
/// and [`global_pointer`](Arch::global_pointer) are derived from them unless
/// overridden. `docs/guides/new-backend.md` walks through adding one, and
/// [`crate::conformance::check`] tests one.
pub trait Arch {
    /// Its name: "R3000A", "R5900".
    fn name(&self) -> &'static str;

    /// Byte order of its code.
    fn endian(&self) -> Endian;

    /// Instruction alignment in bytes (4 for MIPS; 2 for SH and Thumb).
    fn align(&self) -> u32;

    /// Decodes the instruction at the start of `bytes`, which is at `addr`.
    /// Too few bytes, or bits that are no instruction, give an [`Insn`] whose
    /// flow is [`Flow::Invalid`].
    fn decode(&self, bytes: &[u8], addr: u32) -> Insn;

    /// A register's name: "v0", "f12", "vf3".
    fn reg_name(&self, reg: Reg) -> String;

    /// A register as pseudo-code writes it: by default its
    /// [`reg_name`](Arch::reg_name). A backend prefixes a coprocessor's
    /// registers so they read as the device they are (MIPS: `gte.sz1`).
    fn var_name(&self, reg: Reg) -> String {
        self.reg_name(reg)
    }

    /// The instruction as text: mnemonic (or a pseudo-op), then operands,
    /// with `label` asked for a name for every code or data address it
    /// writes (None prints the address in hex).
    fn render(&self, insn: &Insn, label: &dyn Fn(u32) -> Option<String>) -> String;

    /// What `insn` computes, as [`Effect`]s over [`Expr`]s, with its branch
    /// condition and indirect target. None for an instruction the backend
    /// does not lift: the decompiler prints it as a comment and takes the
    /// registers it writes as unknown. By default nothing is lifted.
    fn lift(&self, insn: &Insn) -> Option<Lifted> {
        let _ = insn;
        None
    }

    /// The calling convention, if the backend describes one.
    fn call_conv(&self) -> Option<CallConv> {
        None
    }

    /// The size of every instruction when they are all one size (MIPS: 4);
    /// None for a variable-length set (x86), whose analysis then steps back
    /// through the instructions it has decoded rather than by arithmetic.
    fn fixed_size(&self) -> Option<u32> {
        None
    }

    /// Bytes in a code or data pointer: what a jump table's entries and a
    /// function pointer slot hold (4; 2 for 16-bit code).
    fn pointer_size(&self) -> u32 {
        4
    }

    /// The value `insn` writes, when it writes one register and that can be
    /// computed from constants and from registers whose values `known`
    /// gives. The analysis uses it to follow addresses built over several
    /// instructions. By default, from [`Arch::lift`]: an instruction lifted
    /// to a single [`Effect::Set`] is evaluated.
    fn evaluate(&self, insn: &Insn, known: &dyn Fn(Reg) -> Option<u32>) -> Option<u32> {
        match self.lift(insn)?.effects.as_slice() {
            [Effect::Set(_, e)] => e.eval(known),
            _ => None,
        }
    }

    /// The register that holds the global pointer: by default the calling
    /// convention's.
    fn global_pointer(&self) -> Option<Reg> {
        self.call_conv()?.global_pointer
    }

    /// The stack pointer register: by default the calling convention's. An
    /// instruction lowering it by a constant opens a function's frame,
    /// which the analysis takes as a sign of a function's start.
    fn stack_pointer(&self) -> Option<Reg> {
        self.call_conv()?.stack_pointer
    }

    /// When `insn` compares a switch's index against a constant bound, that
    /// bound: how many entries a jump table behind it has at most. By
    /// default, from [`Arch::lift`]: a single set to an unsigned
    /// less-than against a constant (MIPS `sltiu t, idx, N`).
    fn switch_bound(&self, insn: &Insn) -> Option<u32> {
        match self.lift(insn)?.effects.as_slice() {
            [Effect::Set(_, Expr::Bin(BinOp::Ltu, _, n))] => match n.as_ref() {
                Expr::Const(n) => Some(*n as u32),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_says_where_control_goes() {
        assert!(Flow::Next.falls_through() && Flow::Call { target: None }.falls_through());
        assert!(Flow::Branch { target: 8, likely: false }.falls_through());
        assert!(!Flow::Return.falls_through() && !Flow::Jump { target: Some(4) }.falls_through());
        assert_eq!(Flow::Call { target: Some(0x100) }.target(), Some(0x100));
        assert_eq!(Flow::Jump { target: None }.target(), None);
        assert_eq!(Flow::Trap.target(), None);
        assert!(Flow::Trap.falls_through() && !Flow::Halt.falls_through());
    }
}
