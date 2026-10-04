//! The CPU trait: what the harness needs from any interpreter, whatever the
//! architecture.
//!
//! Nothing here assumes MIPS. A CPU says its byte order ([`Endian`]), how a
//! call passes arguments and where it leaves the return address ([`Abi`]:
//! a register on MIPS, ARM, SH and PowerPC; the stack on the 68000, x86,
//! the 6502 and Z80), and how many registers it has and how wide (up to the
//! EE's 128 bits). The trait is object safe, so a system with several CPUs
//! can hold them as `Box<dyn Cpu>`.

use std::fmt;

use crate::bus::Bus;

/// Byte order of a CPU's loads and stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endian {
    /// Least significant byte first: MIPS as the PlayStations run it, x86,
    /// ARM as the GBA runs it, the 6502, the Z80.
    Little,
    /// Most significant byte first: the 68000, SH-2, SH-4 (as Sega ran them),
    /// PowerPC.
    Big,
}

/// Where a call leaves its return address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnAddress {
    /// In this register (MIPS `ra`, ARM `lr`, SH `pr`, PowerPC `lr` as a
    /// register index of the CPU's own numbering).
    Register(usize),
    /// Pushed on the stack, `bytes` wide, at the stack pointer on entry
    /// (68000 and x86: 4; 6502 and Z80: 2).
    Stack {
        /// Width of the pushed address.
        bytes: u8,
    },
}

/// A calling convention: enough for [`crate::Machine::call`] to pass
/// arguments, find the return value and return from a hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Abi {
    /// Registers that carry the first integer arguments, in order.
    pub args: &'static [usize],
    /// The register the return value comes back in.
    pub ret: usize,
    /// The stack pointer.
    pub sp: usize,
    /// Where the return address goes.
    pub return_address: ReturnAddress,
    /// Where the first stack argument is, from the stack pointer at entry:
    /// 16 on MIPS o32 (the callee may store a0-a3 below it), 0 on EABI, 4
    /// past a stack return address on the 68000.
    pub stack_args_offset: u32,
    /// Bytes each stack argument takes.
    pub stack_slot: u32,
    /// The stack pointer's alignment at a call, in bytes.
    pub stack_align: u32,
}

/// Why a step or a run stopped. Common to every CPU, so the harness handles
/// them alike; CPU-specific detail rides in the fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A system call instruction (MIPS `syscall`, 68000 `trap`, SH `trapa`).
    /// The instruction is complete and the program counter is past it, so
    /// a handler that answers it lets the run carry on.
    Syscall {
        /// The instruction's address.
        pc: u32,
        /// The code the instruction carries (MIPS: bits 6-25).
        code: u32,
    },
    /// A breakpoint instruction.
    Break {
        /// The instruction's address.
        pc: u32,
        /// Its code.
        code: u32,
    },
    /// An arithmetic overflow exception (MIPS `add`, `addi`, `sub`).
    Overflow {
        /// The instruction's address.
        pc: u32,
    },
    /// A misaligned access or fetch.
    Address {
        /// The instruction's address.
        pc: u32,
        /// The address accessed.
        addr: u32,
    },
    /// Nothing mapped at the address.
    Bus {
        /// The instruction's address.
        pc: u32,
        /// The first unmapped address.
        addr: u32,
    },
    /// An instruction the interpreter does not know or does not implement.
    Reserved {
        /// The instruction's address.
        pc: u32,
        /// Its bits.
        word: u64,
    },
    /// The machine's step budget ran out.
    StepLimit {
        /// Where it got to.
        pc: u32,
    },
    /// A hook asked the run to stop.
    Halt {
        /// Where it stopped.
        pc: u32,
    },
    /// Something the harness or an HLE function could not do, as text.
    Hle {
        /// Where.
        pc: u32,
        /// What.
        what: String,
    },
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stop::Syscall { pc, code } => write!(f, "{pc:#010x}: syscall {code:#x}"),
            Stop::Break { pc, code } => write!(f, "{pc:#010x}: break {code:#x}"),
            Stop::Overflow { pc } => write!(f, "{pc:#010x}: arithmetic overflow"),
            Stop::Address { pc, addr } => write!(f, "{pc:#010x}: misaligned access at {addr:#010x}"),
            Stop::Bus { pc, addr } => write!(f, "{pc:#010x}: nothing mapped at {addr:#010x}"),
            Stop::Reserved { pc, word } => write!(f, "{pc:#010x}: instruction {word:#x} is not interpreted"),
            Stop::StepLimit { pc } => write!(f, "{pc:#010x}: step limit reached"),
            Stop::Halt { pc } => write!(f, "{pc:#010x}: halted"),
            Stop::Hle { pc, what } => write!(f, "{pc:#010x}: {what}"),
        }
    }
}

impl std::error::Error for Stop {}

impl Stop {
    /// The address the stop happened at.
    pub fn pc(&self) -> u32 {
        match self {
            Stop::Syscall { pc, .. }
            | Stop::Break { pc, .. }
            | Stop::Overflow { pc }
            | Stop::Address { pc, .. }
            | Stop::Bus { pc, .. }
            | Stop::Reserved { pc, .. }
            | Stop::StepLimit { pc }
            | Stop::Halt { pc }
            | Stop::Hle { pc, .. } => *pc,
        }
    }
}

/// An interpreter. Registers are numbered by the CPU (MIPS: r0-r31, then
/// hi and lo, then whatever coprocessor state it exposes) and read as
/// `u128`, the widest any supported CPU has; [`Cpu::reg_name`] names them
/// for messages and [`Cpu::reg_index`] finds one by name.
pub trait Cpu {
    /// The CPU's name: "R3000A", "R5900".
    fn name(&self) -> &'static str;

    /// Its byte order.
    fn endian(&self) -> Endian;

    /// The calling convention the harness uses for calls and hooks.
    fn abi(&self) -> Abi;

    /// Executes one instruction and returns the cycles it took (1 where the
    /// interpreter does not count). An `Err` leaves the CPU as the
    /// [`Stop`] describes.
    fn step(&mut self, bus: &mut dyn Bus) -> Result<u32, Stop>;

    /// The address of the next instruction to execute.
    fn pc(&self) -> u32;

    /// Continues at `pc`, abandoning any branch or delay in flight.
    fn set_pc(&mut self, pc: u32);

    /// Registers [`Cpu::reg`] can read: indices `0..reg_count()`.
    fn reg_count(&self) -> usize;

    /// Register `i`, zero-extended.
    fn reg(&self, i: usize) -> u128;

    /// Sets register `i`; a write to a hard-wired register (MIPS r0) is
    /// ignored.
    fn set_reg(&mut self, i: usize, v: u128);

    /// Register `i`'s name: "v0", "sp", "hi".
    fn reg_name(&self, i: usize) -> &'static str;

    /// The register named `name`.
    fn reg_index(&self, name: &str) -> Option<usize> {
        (0..self.reg_count()).find(|&i| self.reg_name(i) == name)
    }

    /// Whether the next instruction is a branch's delay slot: the harness
    /// does not fire hooks or checks there, since reaching a hooked address
    /// as a delay slot is not a call to it. False for CPUs without delay
    /// slots.
    fn in_delay_slot(&self) -> bool {
        false
    }

    /// Lets anything still in flight land (a MIPS load delay), so registers
    /// read now are what the next instruction would see. The harness calls
    /// it before reading arguments or results.
    fn settle(&mut self) {}

    /// Every register as (name, value): for an oracle's report.
    fn regs(&self) -> Vec<(&'static str, u128)> {
        (0..self.reg_count()).map(|i| (self.reg_name(i), self.reg(i))).collect()
    }
}
