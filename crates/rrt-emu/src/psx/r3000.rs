//! The R3000A, the PlayStation's CPU: MIPS I with its load delay slot and
//! branch delay slot, COP0 as plain registers, and the GTE as COP2.
//!
//! Exceptions do not vector anywhere: a system call, a breakpoint, an
//! overflow, a misaligned or unmapped access, or an instruction the
//! interpreter does not know ends the step with a [`Stop`] for the harness
//! to deal with. Not here: the instruction cache, cycle timing (every
//! instruction counts 1), interrupts, the BIOS (see [`crate::psx::bios`]),
//! and COP0 behaviour beyond `mfc0`/`mtc0`/`rfe` on a register file.

use crate::bus::{Bus, BusExt};
use crate::cpu::{Abi, Cpu, Endian, ReturnAddress, Stop};

use super::gte::{GTE_CTRL, GTE_DATA, Gte};

/// The general registers' names, r0-r31.
pub const GPR: [&str; 32] = [
    "zero", "at", "v0", "v1", "a0", "a1", "a2", "a3", "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7", "s0", "s1", "s2",
    "s3", "s4", "s5", "s6", "s7", "t8", "t9", "k0", "k1", "gp", "sp", "fp", "ra",
];

/// The COP0 registers' names, by register number.
pub const COP0: [&str; 32] = [
    "c0r0", "c0r1", "c0r2", "bpc", "c0r4", "bda", "jumpdest", "dcic", "badvaddr", "bdam", "c0r10", "bpcm", "sr",
    "cause", "epc", "prid", "c0r16", "c0r17", "c0r18", "c0r19", "c0r20", "c0r21", "c0r22", "c0r23", "c0r24", "c0r25",
    "c0r26", "c0r27", "c0r28", "c0r29", "c0r30", "c0r31",
];

/// [`Cpu::reg`] index of `hi`.
pub const HI: usize = 32;
/// [`Cpu::reg`] index of `lo`.
pub const LO: usize = 33;
/// [`Cpu::reg`] index of GTE data register 0; data register `n` is
/// `GTE_DATA_BASE + n`.
pub const GTE_DATA_BASE: usize = 34;
/// [`Cpu::reg`] index of GTE control register 0; control register `n` is
/// `GTE_CTRL_BASE + n`.
pub const GTE_CTRL_BASE: usize = 66;
/// [`Cpu::reg`] index of COP0 register 0; COP0 register `n` is
/// `COP0_BASE + n`.
pub const COP0_BASE: usize = 98;
/// How many registers [`Cpu::reg`] reads.
pub const REG_COUNT: usize = 130;

/// The PlayStation's calling convention: MIPS o32 as PsyQ compiles it.
pub const ABI: Abi = Abi {
    args: &[4, 5, 6, 7],
    ret: 2,
    sp: 29,
    return_address: ReturnAddress::Register(31),
    stack_args_offset: 16,
    stack_slot: 4,
    stack_align: 8,
};

/// The R3000A interpreter.
///
/// [`Cpu::reg`] numbers its registers:
///
/// | index    | registers                                              |
/// |----------|--------------------------------------------------------|
/// | 0-31     | r0-r31: zero, at, v0, v1, a0-a3, t0-t7, s0-s7, t8, t9, k0, k1, gp, sp, fp, ra |
/// | 32       | hi                                                     |
/// | 33       | lo                                                     |
/// | 34-65    | GTE data registers 0-31 ([`GTE_DATA`] names: vxy0 ... lzcr), as `mfc2`/`mtc2` see them |
/// | 66-97    | GTE control registers 0-31 ([`GTE_CTRL`] names: r11r12 ... flag), as `cfc2`/`ctc2` see them |
/// | 98-129   | COP0 registers 0-31 ([`COP0`] names: sr, cause, epc ...) |
///
/// Writes to r0 are ignored. A GTE register written through
/// [`Cpu::set_reg`] behaves as `mtc2`/`ctc2` would (SXYP pushes the FIFO).
///
/// A fault other than a system call leaves the CPU as it was before the
/// faulting instruction: the program counter on it, a pending load still
/// pending. A system call completes: registers as they stand after it (a
/// pending load landed) and the program counter past it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct R3000 {
    /// r0-r31. r0 reads 0 as long as nothing writes this array directly. A
    /// load in flight is not here until it lands ([`Cpu::settle`]).
    pub r: [u32; 32],
    /// HI: the high word of a product, the remainder of a division.
    pub hi: u32,
    /// LO: the low word of a product, the quotient of a division.
    pub lo: u32,
    /// COP0's registers, plain storage: `mtc0` writes, `mfc0` reads (with
    /// the load delay), `rfe` pops SR's (12) interrupt/mode stack.
    pub cop0: [u32; 32],
    /// The geometry coprocessor, COP2.
    pub gte: Gte,
    /// The instruction about to execute.
    pc: u32,
    /// The one after it: pc + 4, or a branch target when pc is a delay slot.
    next_pc: u32,
    /// A load issued by the previous instruction, landing after this one:
    /// (register, value).
    load: Option<(usize, u32)>,
}

impl R3000 {
    /// A CPU with every register 0, about to execute at 0.
    pub fn new() -> R3000 {
        R3000 { next_pc: 4, ..R3000::default() }
    }

    /// The address of the instruction after the next: pc + 4, or the target
    /// of a branch whose delay slot is next.
    pub fn next_pc(&self) -> u32 {
        self.next_pc
    }

    /// The load in flight, landing after the next instruction: (register,
    /// value).
    pub fn pending_load(&self) -> Option<(usize, u32)> {
        self.load
    }

    /// Executes one instruction; on error the caller decides what of the
    /// state to keep.
    fn execute(&mut self, bus: &mut dyn Bus) -> Result<(), Stop> {
        let pc = self.pc;
        if pc & 3 != 0 {
            return Err(Stop::Address { pc, addr: pc });
        }
        let word = bus.read_u32(pc).map_err(|addr| Stop::Bus { pc, addr })?;
        self.pc = self.next_pc;
        self.next_pc = self.pc.wrapping_add(4);

        // Reads see the registers before the pending load lands; writes go to
        // `out`, where the pending load has landed, so a write by this
        // instruction to the same register wins over it.
        let mut out = self.r;
        if let Some((reg, v)) = self.load.take() {
            out[reg] = v;
        }
        let mut new_load = None;
        let r = self.r;

        let opcode = word >> 26;
        let rs = ((word >> 21) & 31) as usize;
        let rt = ((word >> 16) & 31) as usize;
        let rd = ((word >> 11) & 31) as usize;
        let sa = (word >> 6) & 31;
        let funct = word & 63;
        let s = r[rs];
        let t = r[rt];
        let imm_s = word as u16 as i16 as i32 as u32;
        let imm_u = word & 0xffff;
        let addr = s.wrapping_add(imm_s);
        let branch_target = pc.wrapping_add(4).wrapping_add(imm_s << 2);
        let jump_target = (pc.wrapping_add(4) & 0xf000_0000) | ((word & 0x03ff_ffff) << 2);
        let reserved = Stop::Reserved { pc, word: u64::from(word) };
        let overflow = Stop::Overflow { pc };
        let bus_err = |addr| Stop::Bus { pc, addr };

        let aligned = |width: u32| if addr & (width - 1) == 0 { Ok(()) } else { Err(Stop::Address { pc, addr }) };

        match opcode {
            0 => match funct {
                0x00 => out[rd] = t << sa,
                0x02 => out[rd] = t >> sa,
                0x03 => out[rd] = ((t as i32) >> sa) as u32,
                0x04 => out[rd] = t << (s & 31),
                0x06 => out[rd] = t >> (s & 31),
                0x07 => out[rd] = ((t as i32) >> (s & 31)) as u32,
                0x08 => self.next_pc = s,
                0x09 => {
                    out[rd] = pc.wrapping_add(8);
                    self.next_pc = s;
                }
                0x0c => {
                    // The instruction is complete when the exception is
                    // taken: registers as they stand (a pending load landed),
                    // pc past it.
                    out[0] = 0;
                    self.r = out;
                    return Err(Stop::Syscall { pc, code: (word >> 6) & 0xfffff });
                }
                0x0d => return Err(Stop::Break { pc, code: (word >> 6) & 0xfffff }),
                0x10 => out[rd] = self.hi,
                0x11 => self.hi = s,
                0x12 => out[rd] = self.lo,
                0x13 => self.lo = s,
                0x18 => {
                    let p = (s as i32 as i64) * (t as i32 as i64);
                    self.lo = p as u32;
                    self.hi = (p >> 32) as u32;
                }
                0x19 => {
                    let p = (s as u64) * (t as u64);
                    self.lo = p as u32;
                    self.hi = (p >> 32) as u32;
                }
                0x1a => {
                    let (n, d) = (s as i32, t as i32);
                    if d == 0 {
                        self.hi = n as u32;
                        self.lo = if n >= 0 { 0xffff_ffff } else { 1 };
                    } else if n == i32::MIN && d == -1 {
                        self.hi = 0;
                        self.lo = n as u32;
                    } else {
                        self.hi = (n % d) as u32;
                        self.lo = (n / d) as u32;
                    }
                }
                0x1b => {
                    if t == 0 {
                        self.hi = s;
                        self.lo = 0xffff_ffff;
                    } else {
                        self.hi = s % t;
                        self.lo = s / t;
                    }
                }
                0x20 => out[rd] = (s as i32).checked_add(t as i32).ok_or(overflow)? as u32,
                0x21 => out[rd] = s.wrapping_add(t),
                0x22 => out[rd] = (s as i32).checked_sub(t as i32).ok_or(overflow)? as u32,
                0x23 => out[rd] = s.wrapping_sub(t),
                0x24 => out[rd] = s & t,
                0x25 => out[rd] = s | t,
                0x26 => out[rd] = s ^ t,
                0x27 => out[rd] = !(s | t),
                0x2a => out[rd] = ((s as i32) < (t as i32)) as u32,
                0x2b => out[rd] = (s < t) as u32,
                _ => return Err(reserved),
            },
            // REGIMM. The R3000A decodes it on bit 16 (bgez vs bltz) and on
            // bits 17-20 being 1000 (link); other rt values alias, but no
            // compiler emits them, so they are reserved here.
            1 => {
                let taken = match rt {
                    0x00 | 0x10 => (s as i32) < 0,
                    0x01 | 0x11 => (s as i32) >= 0,
                    _ => return Err(reserved),
                };
                if rt & 0x10 != 0 {
                    out[31] = pc.wrapping_add(8);
                }
                if taken {
                    self.next_pc = branch_target;
                }
            }
            0x02 => self.next_pc = jump_target,
            0x03 => {
                out[31] = pc.wrapping_add(8);
                self.next_pc = jump_target;
            }
            0x04..=0x07 => {
                let taken = match opcode {
                    0x04 => s == t,
                    0x05 => s != t,
                    0x06 => (s as i32) <= 0,
                    _ => (s as i32) > 0,
                };
                if taken {
                    self.next_pc = branch_target;
                }
            }
            0x08 => out[rt] = (s as i32).checked_add(imm_s as i32).ok_or(overflow)? as u32,
            0x09 => out[rt] = s.wrapping_add(imm_s),
            0x0a => out[rt] = ((s as i32) < (imm_s as i32)) as u32,
            0x0b => out[rt] = (s < imm_s) as u32,
            0x0c => out[rt] = s & imm_u,
            0x0d => out[rt] = s | imm_u,
            0x0e => out[rt] = s ^ imm_u,
            0x0f => out[rt] = imm_u << 16,
            // COP0
            0x10 => match rs {
                0x00 => new_load = Some((rt, self.cop0[rd])),
                0x04 => self.cop0[rd] = t,
                0x10 if funct == 0x10 => {
                    let sr = self.cop0[12];
                    self.cop0[12] = (sr & !0xf) | ((sr >> 2) & 0xf);
                }
                _ => return Err(reserved),
            },
            // COP2, the GTE. Register moves out of it go through the load
            // delay; moves in and commands take effect at once.
            0x12 => match rs {
                0x00 => new_load = Some((rt, self.gte.read_data(rd))),
                0x02 => new_load = Some((rt, self.gte.read_ctrl(rd))),
                0x04 => self.gte.write_data(rd, t),
                0x06 => self.gte.write_ctrl(rd, t),
                r if r & 0x10 != 0 => self.gte.command(word & 0x01ff_ffff),
                _ => return Err(reserved),
            },
            0x20 => {
                let v = bus.read_u8(addr).map_err(bus_err)?;
                new_load = Some((rt, v as i8 as i32 as u32));
            }
            0x21 => {
                aligned(2)?;
                let v = bus.read_u16(addr).map_err(bus_err)?;
                new_load = Some((rt, v as i16 as i32 as u32));
            }
            0x23 => {
                aligned(4)?;
                new_load = Some((rt, bus.read_u32(addr).map_err(bus_err)?));
            }
            0x24 => new_load = Some((rt, u32::from(bus.read_u8(addr).map_err(bus_err)?))),
            0x25 => {
                aligned(2)?;
                new_load = Some((rt, u32::from(bus.read_u16(addr).map_err(bus_err)?)));
            }
            // LWL, LWR
            0x22 | 0x26 => {
                let m = bus.read_u32(addr & !3).map_err(bus_err)?;
                // The unaligned loads merge with the register's value as it
                // will be after a pending load to it lands, which is what
                // lets an lwr/lwl pair run back to back.
                let cur = out[rt];
                let sh = (addr & 3) * 8;
                let v = if opcode == 0x22 {
                    let keep = if sh == 24 { 0 } else { 0x00ff_ffff >> sh };
                    (cur & keep) | (m << (24 - sh))
                } else {
                    let keep = if sh == 0 { 0 } else { 0xffff_ff00 << (24 - sh) };
                    (cur & keep) | (m >> sh)
                };
                new_load = Some((rt, v));
            }
            0x28 => bus.write_u8(addr, t as u8).map_err(bus_err)?,
            0x29 => {
                aligned(2)?;
                bus.write_u16(addr, t as u16).map_err(bus_err)?;
            }
            0x2b => {
                aligned(4)?;
                bus.write_u32(addr, t).map_err(bus_err)?;
            }
            // SWL, SWR
            0x2a | 0x2e => {
                let at = addr & !3;
                let m = bus.read_u32(at).map_err(bus_err)?;
                let sh = (addr & 3) * 8;
                let v = if opcode == 0x2a {
                    let keep = if sh == 24 { 0 } else { 0xffff_ff00 << sh };
                    (m & keep) | (t >> (24 - sh))
                } else {
                    let keep = if sh == 0 { 0 } else { 0x00ff_ffff >> (24 - sh) };
                    (m & keep) | (t << sh)
                };
                bus.write_u32(at, v).map_err(bus_err)?;
            }
            // LWC2
            0x32 => {
                aligned(4)?;
                let v = bus.read_u32(addr).map_err(bus_err)?;
                self.gte.write_data(rt, v);
            }
            // SWC2
            0x3a => {
                aligned(4)?;
                let v = self.gte.read_data(rt);
                bus.write_u32(addr, v).map_err(bus_err)?;
            }
            _ => return Err(reserved),
        }
        out[0] = 0;
        self.r = out;
        // A load into r0 is discarded: nothing lands after the next
        // instruction.
        self.load = new_load.filter(|&(reg, _)| reg != 0);
        Ok(())
    }
}

impl Cpu for R3000 {
    fn name(&self) -> &'static str {
        "R3000A"
    }

    fn endian(&self) -> Endian {
        Endian::Little
    }

    fn abi(&self) -> Abi {
        ABI
    }

    fn step(&mut self, bus: &mut dyn Bus) -> Result<u32, Stop> {
        let (pc, next_pc, load) = (self.pc, self.next_pc, self.load);
        match self.execute(bus) {
            Ok(()) => Ok(1),
            Err(stop @ Stop::Syscall { .. }) => Err(stop),
            Err(stop) => {
                // Every other fault happens before the instruction changes
                // anything but the program counter and the load slot, so
                // putting those back leaves it unexecuted.
                self.pc = pc;
                self.next_pc = next_pc;
                self.load = load;
                Err(stop)
            }
        }
    }

    fn pc(&self) -> u32 {
        self.pc
    }

    /// Continues at `pc`, abandoning a branch in flight. A pending load
    /// lands first: the instruction that issued it has completed.
    fn set_pc(&mut self, pc: u32) {
        self.settle();
        self.pc = pc;
        self.next_pc = pc.wrapping_add(4);
    }

    fn reg_count(&self) -> usize {
        REG_COUNT
    }

    fn reg(&self, i: usize) -> u128 {
        u128::from(match i {
            0..32 => self.r[i],
            HI => self.hi,
            LO => self.lo,
            GTE_DATA_BASE..GTE_CTRL_BASE => self.gte.read_data(i - GTE_DATA_BASE),
            GTE_CTRL_BASE..COP0_BASE => self.gte.read_ctrl(i - GTE_CTRL_BASE),
            COP0_BASE..REG_COUNT => self.cop0[i - COP0_BASE],
            _ => panic!("R3000A register {i}"),
        })
    }

    /// Sets register `i` (see [`R3000`] for the numbering). A pending load
    /// to the same general register is cancelled, as an instruction writing
    /// it in the load's delay slot would.
    fn set_reg(&mut self, i: usize, v: u128) {
        let v = v as u32;
        match i {
            0 => {}
            1..32 => {
                self.r[i] = v;
                if self.load.is_some_and(|(reg, _)| reg == i) {
                    self.load = None;
                }
            }
            HI => self.hi = v,
            LO => self.lo = v,
            GTE_DATA_BASE..GTE_CTRL_BASE => self.gte.write_data(i - GTE_DATA_BASE, v),
            GTE_CTRL_BASE..COP0_BASE => self.gte.write_ctrl(i - GTE_CTRL_BASE, v),
            COP0_BASE..REG_COUNT => self.cop0[i - COP0_BASE] = v,
            _ => panic!("R3000A register {i}"),
        }
    }

    fn reg_name(&self, i: usize) -> &'static str {
        match i {
            0..32 => GPR[i],
            HI => "hi",
            LO => "lo",
            GTE_DATA_BASE..GTE_CTRL_BASE => GTE_DATA[i - GTE_DATA_BASE],
            GTE_CTRL_BASE..COP0_BASE => GTE_CTRL[i - GTE_CTRL_BASE],
            COP0_BASE..REG_COUNT => COP0[i - COP0_BASE],
            _ => panic!("R3000A register {i}"),
        }
    }

    fn in_delay_slot(&self) -> bool {
        self.next_pc != self.pc.wrapping_add(4)
    }

    fn settle(&mut self) {
        if let Some((r, v)) = self.load.take() {
            self.r[r] = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Memory;

    const BASE: u32 = 0x8001_0000;

    /// Steps `words` from [`BASE`] once each and lets the last load land.
    fn run_on(bus: &mut Memory, words: &[u32], setup: impl FnOnce(&mut R3000)) -> R3000 {
        for (i, w) in words.iter().enumerate() {
            bus.write_u32(BASE + i as u32 * 4, *w).unwrap();
        }
        let mut cpu = R3000::new();
        cpu.set_pc(BASE);
        setup(&mut cpu);
        for _ in 0..words.len() {
            cpu.step(bus).unwrap();
        }
        cpu.settle();
        cpu
    }

    fn run(words: &[u32], setup: impl FnOnce(&mut R3000)) -> R3000 {
        run_on(&mut Memory::psx(), words, setup)
    }

    #[test]
    fn the_load_delay_slot_sees_the_old_value() {
        // sw a0, 0(sp); lw v0, 0(sp); move v1, v0; nop
        let cpu = run(&[0xafa4_0000, 0x8fa2_0000, 0x0040_1821, 0], |c| {
            c.r[29] = 0x8010_0000;
            c.r[4] = 7;
            c.r[2] = 3;
        });
        assert_eq!(cpu.r[3], 3, "the delay slot reads v0 before the load lands");
        assert_eq!(cpu.r[2], 7);
    }

    #[test]
    fn a_write_in_the_load_delay_slot_beats_the_load() {
        // lw v0, 0(sp); li v0, 5; nop
        let cpu = run(&[0x8fa2_0000, 0x2402_0005, 0], |c| c.r[29] = 0x8010_0000);
        assert_eq!(cpu.r[2], 5);
    }

    #[test]
    fn the_delay_slot_runs_before_the_branch_lands() {
        // b +2; li v0, 1; li v0, 2; li v1, 3
        let cpu = run(&[0x1000_0002, 0x2402_0001, 0x2402_0002, 0x2403_0003], |_| {});
        assert_eq!(cpu.r[2], 1);
        assert_eq!(cpu.r[3], 3);
    }

    #[test]
    fn lwr_then_lwl_load_an_unaligned_word() {
        let mut bus = Memory::psx();
        bus.write(0x8010_0000, &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]).unwrap();
        // lwr v0, 1(a0); lwl v0, 4(a0); nop
        let cpu = run_on(&mut bus, &[0x9882_0001, 0x8882_0004, 0], |c| c.r[4] = 0x8010_0000);
        assert_eq!(cpu.r[2], 0x5544_3322);
    }

    #[test]
    fn swr_then_swl_store_an_unaligned_word() {
        let mut bus = Memory::psx();
        // swr a1, 1(a0); swl a1, 4(a0)
        run_on(&mut bus, &[0xb885_0001, 0xa885_0004], |c| {
            c.r[4] = 0x8010_0000;
            c.r[5] = 0x5544_3322;
        });
        assert_eq!(bus.bytes(0x8010_0000, 6), Ok(vec![0, 0x22, 0x33, 0x44, 0x55, 0]));
    }

    #[test]
    fn division_by_zero_leaves_the_hardware_results() {
        // div a0, a1; mfhi v0; mflo v1
        let div = [0x0085_001a, 0x0000_1010, 0x0000_1812];
        let cpu = run(&div, |c| (c.r[4], c.r[5]) = (7, 0));
        assert_eq!((cpu.r[2], cpu.r[3]), (7, 0xffff_ffff), "positive / 0: hi = n, lo = -1");
        let cpu = run(&div, |c| (c.r[4], c.r[5]) = (-7i32 as u32, 0));
        assert_eq!((cpu.r[2], cpu.r[3]), (-7i32 as u32, 1), "negative / 0: hi = n, lo = 1");
        let cpu = run(&div, |c| (c.r[4], c.r[5]) = (0x8000_0000, u32::MAX));
        assert_eq!((cpu.r[2], cpu.r[3]), (0, 0x8000_0000), "i32::MIN / -1: hi = 0, lo = i32::MIN");
        // divu a0, a1; mfhi v0; mflo v1
        let cpu = run(&[0x0085_001b, 0x0000_1010, 0x0000_1812], |c| (c.r[4], c.r[5]) = (9, 0));
        assert_eq!((cpu.r[2], cpu.r[3]), (9, 0xffff_ffff), "divu by 0: hi = n, lo = all ones");
    }

    #[test]
    fn a_fault_leaves_the_instruction_unexecuted() {
        let mut bus = Memory::psx();
        // lw v0, 0(sp); lw v1, 1(sp)
        bus.write_u32(BASE, 0x8fa2_0000).unwrap();
        bus.write_u32(BASE + 4, 0x8fa3_0001).unwrap();
        bus.write_u32(0x8010_0000, 42).unwrap();
        let mut cpu = R3000::new();
        cpu.set_pc(BASE);
        cpu.r[29] = 0x8010_0000;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.step(&mut bus), Err(Stop::Address { pc: BASE + 4, addr: 0x8010_0001 }));
        assert_eq!(cpu.pc(), BASE + 4);
        assert_eq!(cpu.pending_load(), Some((2, 42)), "the first load is still in flight");
    }

    #[test]
    fn gte_registers_move_through_cop2_and_mfc2_has_a_load_delay() {
        // mtc2 a0, ir1; mfc2 v0, ir1; move v1, v0; nop
        let cpu = run(&[0x4884_4800, 0x4802_4800, 0x0040_1821, 0], |c| c.r[4] = 0x1234);
        assert_eq!(cpu.gte.ir[1], 0x1234);
        assert_eq!(cpu.r[3], 0, "the delay slot sees v0 before mfc2 lands");
        assert_eq!(cpu.r[2], 0x1234);
        assert_eq!(cpu.reg(GTE_DATA_BASE + 9), 0x1234);
        assert_eq!(cpu.reg_index("ir1"), Some(GTE_DATA_BASE + 9));
        assert_eq!(cpu.reg_index("flag"), Some(GTE_CTRL_BASE + 31));
        assert_eq!(cpu.reg_index("sr"), Some(COP0_BASE + 12));
    }

    #[test]
    fn a_gte_command_runs_from_the_cop2_instruction() {
        // nclip
        let cpu = run(&[0x4b40_0006], |c| {
            c.gte.sxy = [[0, 0], [10, 0], [0, 10]];
        });
        assert_eq!(cpu.gte.mac[0], 100);
    }
}
