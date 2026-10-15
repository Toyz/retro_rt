//! A decoded instruction, the same shape for every instruction set.

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
/// architecture's; [`Arch::reg_name`] names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// What an instruction does to control flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// A trap to the system (`syscall`, `break`).
    Trap,
    /// Not a valid instruction.
    Invalid,
}

impl Flow {
    /// Whether control can continue to the next instruction (after any delay
    /// slot): everything but an unconditional jump, a return and an invalid
    /// word. A call continues; a trap is assumed to return.
    pub fn falls_through(self) -> bool {
        !matches!(self, Flow::Jump { .. } | Flow::Return | Flow::Invalid)
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

/// An instruction set: how to decode it and how to print it.
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

    /// The instruction as text: mnemonic (or a pseudo-op), then operands,
    /// with `label` asked for a name for every code or data address it
    /// writes (None prints the address in hex).
    fn render(&self, insn: &Insn, label: &dyn Fn(u32) -> Option<String>) -> String;

    /// The value `insn` writes to its first written register, when it can
    /// be computed from constants and from registers whose values `known`
    /// gives (MIPS: `lui`, `addiu`/`addi`, `ori`, `addu`/`or` of known
    /// registers, `move`). The analysis uses it to resolve addresses built
    /// over several instructions. None when not computable, and by default.
    fn evaluate(&self, insn: &Insn, known: &dyn Fn(Reg) -> Option<u32>) -> Option<u32> {
        let _ = (insn, known);
        None
    }

    /// The register that holds the global pointer, if the architecture's
    /// calling convention has one (MIPS `gp`).
    fn global_pointer(&self) -> Option<Reg> {
        None
    }

    /// The stack pointer register (MIPS `sp`): an instruction lowering it
    /// by a constant opens a function's frame, which the analysis takes as a
    /// sign of a function's start.
    fn stack_pointer(&self) -> Option<Reg> {
        None
    }

    /// When `insn` compares a switch's index against a constant bound
    /// (MIPS `sltiu t, idx, N`), that bound: how many entries a jump table
    /// behind it has at most.
    fn switch_bound(&self, insn: &Insn) -> Option<u32> {
        let _ = insn;
        None
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
    }
}
