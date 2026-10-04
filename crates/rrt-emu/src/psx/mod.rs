//! The PlayStation: the R3000A and its GTE, the BIOS calls, a ready machine.
//!
//! - [`R3000`]: the CPU behind [`crate::Cpu`], with its load and branch
//!   delay slots, COP0 as plain registers and the [`Gte`] as COP2. Its
//!   register numbering (GPRs, hi, lo, then the GTE's and COP0's registers)
//!   is on the type.
//! - [`Gte`]: the geometry coprocessor, bit-exact, usable on its own by a
//!   port that has to reproduce the original's GTE math.
//! - [`bios`]: the BIOS function tables (calls through 0xa0, 0xb0, 0xc0)
//!   answered on the host, extensible per game.
//! - [`machine`] and [`machine_with_exe`]: an R3000A over [`Memory::psx`]
//!   with the BIOS installed, the stack and heap where PsyQ programs expect
//!   them.
//!
//! Not here: the GPU, SPU, CD-ROM, DMA, timers and interrupts (the I/O
//! ports are an [`crate::IoLog`]), the BIOS ROM, and instruction timing.

pub mod bios;
pub mod gte;
pub mod r3000;

pub use bios::Bios;
pub use gte::Gte;
pub use r3000::R3000;

use crate::bus::Memory;
use crate::heap::Heap;
use crate::load::PsxExe;
use crate::machine::Machine;

/// Where [`machine`] puts the stack for calls: near the top of the 2 MB of
/// RAM, in KSEG0.
pub const STACK_TOP: u32 = 0x801f_ff00;
/// The start of [`machine`]'s HLE heap.
pub const HEAP_BASE: u32 = 0x8018_0000;
/// The size of [`machine`]'s HLE heap, in bytes.
pub const HEAP_SIZE: u32 = 0x8_0000;

/// An R3000A over [`Memory::psx`] with the stack at [`STACK_TOP`], an 8-byte
/// aligned heap of [`HEAP_SIZE`] bytes at [`HEAP_BASE`] (BIOS InitHeap
/// replaces it), the BIOS tables hooked ([`bios::install`]) and the
/// critical-section system calls answered ([`bios::critical_sections`]).
///
/// The [`Bios`] table is kept in [`Machine::ext`]: supply a function a game
/// needs beyond the standard set with
/// `m.ext.get::<Bios<Memory>>().unwrap().insert(0xa0, 0x2f, f)`.
pub fn machine() -> Machine<R3000, Memory> {
    let mut m = Machine::new(R3000::new(), Memory::psx());
    m.stack_top = STACK_TOP;
    m.heap = Heap::new(HEAP_BASE, HEAP_SIZE, 8);
    let bios = bios::install(&mut m);
    m.ext.insert(bios);
    bios::critical_sections(&mut m);
    m
}

/// [`machine`] with `exe`'s image loaded at its address and gp set from its
/// header when the header gives one (when it does not, the program's crt0
/// sets gp, and a function called directly may need it set by hand). The
/// program counter is left alone: [`Machine::call`] sets it.
pub fn machine_with_exe(exe: &PsxExe) -> Result<Machine<R3000, Memory>, String> {
    let mut m = machine();
    exe.load(&mut m.bus)?;
    if exe.gp0 != 0 {
        m.cpu.r[28] = exe.gp0;
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{Bus, BusExt};
    use crate::cpu::{Cpu, Stop};
    use crate::machine::HookResult;

    const V0: u32 = 2;
    const A0: u32 = 4;
    const A1: u32 = 5;
    const T1: u32 = 9;
    const T2: u32 = 10;
    const SP: u32 = 29;
    const RA: u32 = 31;

    const F: u32 = 0x8001_0000;
    const DATA: u32 = 0x8002_0000;
    const NOP: u32 = 0;

    fn special(rs: u32, rt: u32, rd: u32, funct: u32) -> u32 {
        (rs << 21) | (rt << 16) | (rd << 11) | funct
    }

    fn imm(op: u32, rs: u32, rt: u32, imm: i16) -> u32 {
        (op << 26) | (rs << 21) | (rt << 16) | u32::from(imm as u16)
    }

    fn jr_ra() -> u32 {
        special(RA, 0, 0, 0x08)
    }

    fn put(m: &mut Machine<R3000, Memory>, at: u32, words: &[u32]) {
        for (i, w) in words.iter().enumerate() {
            m.bus.write_u32(at + 4 * i as u32, *w).unwrap();
        }
    }

    #[test]
    fn a_call_runs_a_function_and_returns_v0() {
        let mut m = machine();
        // addu v0, a0, a1; jr ra; nop
        put(&mut m, F, &[special(A0, A1, V0, 0x21), jr_ra(), NOP]);
        assert_eq!(m.call(F, &[2, 3]), Ok(5));
        assert_eq!(m.steps, 3);
    }

    #[test]
    fn the_fifth_argument_is_read_from_sp_plus_16() {
        let mut m = machine();
        // lw v0, 16(sp); nop; addu v0, v0, a0; jr ra; nop
        put(&mut m, F, &[imm(0x23, SP, V0, 16), NOP, special(V0, A0, V0, 0x21), jr_ra(), NOP]);
        assert_eq!(m.call(F, &[1, 2, 3, 4, 50]), Ok(51));
        assert_eq!(m.cpu.reg(29) % 8, 0, "the stack stays 8-byte aligned");
    }

    #[test]
    fn signed_overflow_in_add_stops_with_overflow() {
        let mut m = machine();
        // add v0, a0, a1; jr ra; nop
        put(&mut m, F, &[special(A0, A1, V0, 0x20), jr_ra(), NOP]);
        assert_eq!(m.call(F, &[0x7fff_ffff, 1]), Err(Stop::Overflow { pc: F }));
        assert_eq!(m.cpu.pc(), F, "the faulting instruction did not execute");
        assert_eq!(m.call(F, &[0x7fff_fffe, 1]), Ok(0x7fff_ffff));
    }

    #[test]
    fn a_syscall_stops_past_itself_and_on_syscall_answers_it() {
        let mut m = machine();
        // syscall 0x42; addiu v0, v0, 1; jr ra; nop
        put(&mut m, F, &[(0x42 << 6) | 0x0c, imm(0x09, V0, V0, 1), jr_ra(), NOP]);
        // a0 = 0 is not a critical section call, so the standard handler
        // stops on it.
        assert_eq!(m.call(F, &[0]), Err(Stop::Syscall { pc: F, code: 0x42 }));
        assert_eq!(m.cpu.pc(), F + 4, "the syscall is complete");
        m.on_syscall(|_, code| HookResult::Return(u64::from(code)));
        assert_eq!(m.call(F, &[0]), Ok(0x43), "v0 from the handler, then addiu");
    }

    #[test]
    fn critical_section_syscalls_report_whether_interrupts_were_on() {
        let mut m = machine();
        // syscall; jr ra; nop
        put(&mut m, F, &[0x0c, jr_ra(), NOP]);
        assert_eq!(m.call(F, &[1]), Ok(1), "Enter: they were on");
        assert_eq!(m.call(F, &[1]), Ok(0), "Enter again: they were off");
        assert_eq!(m.call(F, &[2]), Ok(0), "Exit: they were off");
        assert_eq!(m.call(F, &[1]), Ok(1));
    }

    #[test]
    fn strlen_through_the_a0_table_returns_the_length() {
        let mut m = machine();
        m.bus.write(DATA, b"hello, world\0").unwrap();
        // The PsyQ stub: addiu sp, sp, -24; sw ra, 16(sp); li t2, 0xa0;
        // jalr t2; li t1, 0x1b (delay slot); lw ra, 16(sp); nop; jr ra;
        // addiu sp, sp, 24
        put(
            &mut m,
            F,
            &[
                imm(0x09, SP, SP, -24),
                imm(0x2b, SP, RA, 16),
                imm(0x09, 0, T2, 0xa0),
                special(T2, 0, RA, 0x09),
                imm(0x09, 0, T1, 0x1b),
                imm(0x23, SP, RA, 16),
                NOP,
                jr_ra(),
                imm(0x09, SP, SP, 24),
            ],
        );
        assert_eq!(m.call(F, &[u64::from(DATA)]), Ok(12));

        // From KUSEG a plain `jal 0xa0` reaches the table too: move t3, ra;
        // jal 0xa0; li t1, 0x1b; jr t3; nop
        let g = 0x0003_0000;
        let t3 = 11;
        put(
            &mut m,
            g,
            &[
                special(RA, 0, t3, 0x21),
                (0x03 << 26) | (0xa0 >> 2),
                imm(0x09, 0, T1, 0x1b),
                special(t3, 0, 0, 0x08),
                NOP,
            ],
        );
        assert_eq!(m.call(g, &[u64::from(DATA)]), Ok(12));
    }

    #[test]
    fn an_unknown_bios_function_stops_naming_it() {
        let mut m = machine();
        m.cpu.r[T1 as usize] = 0x2f;
        match m.call(0xa0, &[]) {
            Err(Stop::Hle { pc: 0xa0, what }) => assert!(what.starts_with("BIOS A0:2f not provided"), "{what}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_game_supplies_a_bios_function_the_table_lacks() {
        let mut m = machine();
        let bios = m.ext.get::<Bios<Memory>>().expect("machine() keeps its BIOS table").clone();
        assert!(!bios.provides(0xa0, 0x2f));
        bios.insert(0xa0, 0x2f, |c| HookResult::Return(c.arg(0) * 2));
        m.cpu.r[T1 as usize] = 0x2f;
        assert_eq!(m.call(0xa0, &[21]), Ok(42));
    }

    /// A hooked address reached as a branch's delay slot runs as code: it is
    /// not a call to the hooked function.
    #[test]
    fn a_hook_does_not_fire_in_a_delay_slot() {
        let mut m = machine();
        // beq zero, zero, +2; addiu v0, zero, 7 (delay slot, hooked);
        // addiu v0, zero, 1 (skipped); jr ra; nop
        put(&mut m, F, &[0x1000_0002, imm(0x09, 0, V0, 7), imm(0x09, 0, V0, 1), jr_ra(), NOP]);
        m.stub(F + 4, 99);
        assert_eq!(m.call(F, &[]), Ok(7));
        assert_eq!(m.call(F + 4, &[]), Ok(99), "called directly, it is a call");
    }

    #[test]
    fn the_bios_heap_allocates_from_what_init_heap_gave_it() {
        let mut m = machine();
        m.cpu.r[T1 as usize] = 0x39;
        m.call(0xa0, &[0x8010_0000, 0x1000]).unwrap();
        m.cpu.r[T1 as usize] = 0x33;
        assert_eq!(m.call(0xa0, &[16]), Ok(0x8010_0000));
        assert_eq!(m.call(0xa0, &[16]), Ok(0x8010_0010));
    }

    #[test]
    fn call_recorded_reports_the_word_a_sw_wrote() {
        let mut m = machine();
        // sw a1, 0(a0); jr ra; nop
        put(&mut m, F, &[imm(0x2b, A0, A1, 0), jr_ra(), NOP]);
        let o = m.call_recorded(F, &[u64::from(DATA), 0xdead_beef]).unwrap();
        // Changes are reported at canonical addresses: KSEG0 folds to 0.
        let at = m.canonical(DATA);
        assert_eq!(at, DATA & 0x1fff_ffff);
        assert!(o.wrote(at, 4));
        assert_eq!(o.changes[0].addr, at);
        assert_eq!(o.changes.len(), 1, "{}", o.report());
        assert_eq!(o.changes[0].after, vec![0xef, 0xbe, 0xad, 0xde]);
        assert_eq!(o.changes[0].before, vec![0; 4]);
    }

    #[test]
    fn machine_with_exe_loads_the_image_and_sets_gp() {
        let mut data = vec![0u8; 0x800];
        data[..8].copy_from_slice(b"PS-X EXE");
        let w = |d: &mut Vec<u8>, at: usize, v: u32| d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        w(&mut data, 0x10, F);
        w(&mut data, 0x14, 0x8009_0000);
        w(&mut data, 0x18, F);
        w(&mut data, 0x1c, 12);
        for word in [special(A0, A1, V0, 0x23), jr_ra(), NOP] {
            data.extend_from_slice(&word.to_le_bytes());
        }
        let exe = PsxExe::parse(&data).unwrap();
        let mut m = machine_with_exe(&exe).unwrap();
        assert_eq!(m.cpu.r[28], 0x8009_0000);
        assert_eq!(m.call(F, &[10, 3]), Ok(7));
    }
}
