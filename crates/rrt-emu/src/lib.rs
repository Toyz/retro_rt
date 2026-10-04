//! The originals, run for comparison.
//!
//! A port is checked against the game it ports by running the original's
//! code on the same inputs and comparing. This crate is what that takes, for
//! any console:
//!
//! - [`Cpu`]: an interpreter behind one trait - step, the program counter,
//!   registers, the calling convention ([`Abi`]). [`psx::R3000`] (with its
//!   GTE) and [`ee::Ee`] implement it.
//! - [`Bus`] and [`Memory`]: memory as the CPU sees it, regions mapped by
//!   address (RAM with mirrors, scratchpads, I/O behind a [`Device`]), with
//!   presets for the consoles ([`Memory::psx`], [`Memory::ps2`]).
//! - [`Machine`]: a CPU and its memory with HLE: [`Machine::hook`] runs host
//!   code instead of the function at an address (a library call, the BIOS, a
//!   function outside the part under test), [`Machine::on_syscall`] answers
//!   system calls, and [`Machine::call`] calls an original function with
//!   arguments and runs it until it returns.
//! - The oracle: [`Machine::call_recorded`] returns an [`Outcome`] - the
//!   return value and every byte the call changed - to compare with what the
//!   port computes from the same inputs; [`Machine::check`] watches every
//!   call of a function while the whole original runs, comparing at entry
//!   and return ([`CheckStats`]).
//! - Hardware data formats several games of a console share: [`spu`] (the
//!   PS1/PS2 SPU's ADPCM).
//!
//! Nothing here is needed at play time: a port runs its own Rust code. See
//! `docs/crates/rrt-emu.md`.

#![forbid(unsafe_code)]

pub mod bus;
pub mod cpu;
pub mod ee;
pub mod heap;
pub mod hle;
pub mod load;
pub mod machine;
pub mod psx;
pub mod spu;
pub mod symbols;

pub use bus::{Backing, Bus, BusExt, Device, IoLog, Memory, Region, SharedBus};
pub use cpu::{Abi, Cpu, Endian, ReturnAddress, Stop};
pub use heap::Heap;
pub use load::{Elf, PsxExe};
pub use machine::{Call, Change, Check, CheckExit, CheckStats, Hook, HookResult, Machine, Outcome, SyscallHook};
pub use symbols::Symbols;
