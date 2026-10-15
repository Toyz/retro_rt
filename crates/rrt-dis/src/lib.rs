//! Disassembly for reverse engineering.
//!
//! - [`Arch`]: an instruction set, decoding bytes into an [`Insn`] - its
//!   mnemonic and [`Operand`]s, its control [`Flow`], the registers it reads
//!   and writes, the memory it touches - and rendering one as text with
//!   symbol names in place of addresses. [`mips::Mips`] is the PS1's R3000A
//!   (with the GTE) and the PS2's R5900 (with MMI, COP1 and VU0 macro mode).
//! - [`Symbols`]: names for addresses as a reverse engineering project keeps
//!   them - kind, size, how sure, a note - read from and written to the text
//!   file a project commits.
//! - [`exe`]: the PS-X EXE and ELF an original shipped as, parsed.
//! - [`analysis`]: the whole program by recursive descent - functions and
//!   their extents, calls and callers, jump tables, interface slots, the
//!   data and strings the code refers to.
//! - [`listing`]: text listings with names in place of addresses and
//!   comments saying who calls what and what a load reads.
//!
//! Decoding is structural so tools can reason about code, not just print it:
//! what a function calls, where a register came from, which address a
//! `lui`/`addiu` pair builds. Nothing outside [`mips`] names an opcode, so
//! another architecture is one more [`Arch`].
//!
//! With the `cli` feature this crate is also the `rrt-dis` command.

#![forbid(unsafe_code)]

pub mod analysis;
pub mod exe;
pub mod insn;
pub mod listing;
pub mod mips;
pub mod symbols;

pub use insn::{Arch, Endian, Flow, Insn, Mem, Operand, Reg, RegFile};
pub use symbols::{Confidence, Kind, Symbol, Symbols};
