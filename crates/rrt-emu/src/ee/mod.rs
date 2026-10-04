//! The PlayStation 2's Emotion Engine: the R5900 interpreter ([`Ee`]), its
//! float rules as bit-exact functions ports can call ([`float`]), kernel and
//! library stand-ins by symbol name ([`kernel`]), and a ready machine
//! ([`machine`], [`machine_with_elf`]).
//!
//! The interpreter is piney_apples' `piney-eemu` ported onto [`Cpu`] and
//! [`crate::Bus`]: memory is [`Memory::ps2`] plus VU0's data memory at
//! [`VU0_DATA`]; a call starts with `sp` at [`STACK_TOP`] and returns to
//! [`RETURN_SENTINEL`], as piney's do.
//!
//! Not here: the IOP, the GS, VIF, DMA, timers, VU1 and VU0 micro mode, the
//! EE kernel's system calls by number, the BIOS. Code that reaches them
//! stops, or is stubbed by name ([`kernel`]).

pub mod float;
pub mod kernel;
pub mod r5900;

pub use r5900::{
    ACC, C_BIT, Ee, F0, FCR31, Features, HI, HI1, LO, LO1, Q, REGS, REGS_VU, RETURN_SENTINEL, SA, STACK_TOP, VACC, VF0,
    VI0, VU0_DATA, VU0_DATA_SIZE,
};

use crate::bus::{Memory, Region};
use crate::cpu::Cpu;
use crate::heap::Heap;
use crate::load::Elf;
use crate::machine::Machine;

/// The HLE heap's first byte: 0x0100_0000 (16 MB), piney's
/// `tools/test_stream_rs.py` `HEAP0`, above where the games' images and
/// fixed blocks go.
pub const HEAP_BASE: u32 = 0x0100_0000;
/// One past the HLE heap's last byte: 0x01f0_0000, piney's `HEAP_END` (as in
/// `test_stream_rs`, `test_cinema_rs`, `test_chat_msg_rs`), leaving the
/// megabyte below [`STACK_TOP`] to the stack.
pub const HEAP_END: u32 = 0x01f0_0000;

/// A PS2 machine with nothing loaded: [`Memory::ps2`] (32 MB of RAM, the
/// scratchpad, I/O on an [`crate::IoLog`]) plus VU0's 4 KB data memory as
/// RAM at [`VU0_DATA`] (folded by 0x1fff_ffff like the RAM), an [`Ee`] with
/// `features`, the stack at [`STACK_TOP`], calls returning to
/// [`RETURN_SENTINEL`], and the HLE heap over [`HEAP_BASE`]..[`HEAP_END`]
/// with 16-byte blocks.
pub fn machine(features: Features) -> Machine<Ee, Memory> {
    let mut mem = Memory::ps2();
    mem.regions.push(Region::ram("vu0 data", VU0_DATA, VU0_DATA_SIZE as usize, VU0_DATA_SIZE, 0x1fff_ffff));
    let mut m = Machine::new(Ee::new(features), mem);
    m.stack_top = STACK_TOP;
    m.return_to = RETURN_SENTINEL;
    m.heap = Heap::new(HEAP_BASE, HEAP_END - HEAP_BASE, 16);
    m
}

/// [`machine`] with a PS2 executable loaded: its segments written, its
/// symbols copied into [`Machine::symbols`], `gp` set from the `_gp` symbol
/// when there is one (sign-extended, as the EE holds 32-bit values), the
/// program counter at the entry point, and the stand-ins installed by name:
/// [`kernel::install`] and [`crate::hle::libc`].
///
/// [`Machine::call`] does not reset registers, so `gp` holds for every call
/// unless the code changes it. Errors when the ELF is not little-endian
/// MIPS or a segment lies outside the memory map.
pub fn machine_with_elf(elf: &Elf, features: Features) -> Result<Machine<Ee, Memory>, String> {
    if elf.endian != crate::cpu::Endian::Little || elf.machine != 8 {
        return Err(format!("not a little-endian MIPS ELF (machine {}, {:?})", elf.machine, elf.endian));
    }
    let mut m = machine(features);
    elf.load(&mut m.bus)?;
    m.symbols = elf.symbols.clone();
    if let Some(gp) = m.symbols.addr("_gp") {
        m.cpu.set32(28, gp);
    }
    m.cpu.set_pc(elf.entry);
    kernel::install(&mut m);
    crate::hle::libc(&mut m);
    Ok(m)
}

#[cfg(test)]
mod tests;
