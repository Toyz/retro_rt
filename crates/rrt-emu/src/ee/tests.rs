//! The EE's tests: piney-eemu's `cpu.rs` and `machine.rs` tests ported onto
//! [`Machine`], plus 128-bit memory, MMI, COP1, VU0 data memory, register
//! naming, the presets and ELF loading.

use super::*;
use crate::bus::{Bus, BusExt};
use crate::cpu::Stop;
use crate::machine::{Change, HookResult};

const JR_RA: u32 = 0x03e0_0008;
const NOP: u32 = 0;
const AT: u32 = 0x1000;
const M64: u128 = u64::MAX as u128;

fn program<C: Cpu, B: Bus>(m: &mut Machine<C, B>, at: u32, words: &[u32]) {
    for (i, w) in words.iter().enumerate() {
        m.bus.write_u32(at + 4 * i as u32, *w).unwrap();
    }
}

/// Executes the single instruction `w`, placed at [`AT`].
fn exec(m: &mut Machine<Ee, Memory>, w: u32) -> Result<u32, Stop> {
    m.bus.write_u32(AT, w).unwrap();
    m.cpu.set_pc(AT);
    m.cpu.step(&mut m.bus)
}

fn from_words(w: [u32; 4]) -> u128 {
    w.iter().rev().fold(0, |v, &x| (v << 32) | u128::from(x))
}

fn from_halves(h: [u16; 8]) -> u128 {
    h.iter().rev().fold(0, |v, &x| (v << 16) | u128::from(x))
}

fn vu() -> Features {
    Features { vu: true, ..Features::default() }
}

// piney-eemu cpu.rs ------------------------------------------------------

#[test]
fn a_64_bit_write_keeps_the_upper_half_of_a_register() {
    let mut m = machine(Features::default());
    m.cpu.r[8] = u128::MAX;
    // addiu t0, zero, -1: a 32-bit result sign-extends to 64 bits only.
    exec(&mut m, 0x2408_ffff).unwrap();
    assert_eq!(m.cpu.r[8], u128::MAX);
    // lui t0, 1
    exec(&mut m, 0x3c08_0001).unwrap();
    assert_eq!(m.cpu.r[8], !M64 | 0x10000);
}

#[test]
fn mult_and_div_fill_lo_and_hi_and_ee_div_sets_them_on_a_zero_divisor() {
    let mut m = machine(Features::default());
    m.cpu.r[4] = 0xffff_ffff; // -1
    m.cpu.r[5] = 3;
    // multu a0, a1 -> 0x2_ffff_fffd
    exec(&mut m, 0x0085_0019).unwrap();
    assert_eq!((m.cpu.lo, m.cpu.hi), (0xffff_ffff_ffff_fffd, 2));
    // mult a0, a1 -> -3
    exec(&mut m, 0x0085_0018).unwrap();
    assert_eq!((m.cpu.lo, m.cpu.hi), (0xffff_ffff_ffff_fffd, 0xffff_ffff_ffff_ffff));
    // div a0, a1: -1 / 3 = 0 remainder -1
    exec(&mut m, 0x0085_001a).unwrap();
    assert_eq!((m.cpu.lo, m.cpu.hi), (0, 0xffff_ffff_ffff_ffff));
    // A zero divisor leaves them, unless ee_div.
    m.cpu.r[5] = 0;
    exec(&mut m, 0x0085_001a).unwrap();
    assert_eq!((m.cpu.lo, m.cpu.hi), (0, 0xffff_ffff_ffff_ffff));
    m.cpu.features.ee_div = true;
    m.cpu.r[4] = 5;
    exec(&mut m, 0x0085_001a).unwrap();
    assert_eq!((m.cpu.lo, m.cpu.hi), (0xffff_ffff_ffff_ffff, 5));
}

#[test]
fn an_uninterpreted_word_stops_on_itself_and_the_vu_features_widen_the_set() {
    let mut m = machine(Features::default());
    // syscall completes and stops past itself; break stops on itself.
    assert_eq!(exec(&mut m, 0x0000_000c), Err(Stop::Syscall { pc: AT, code: 0 }));
    assert_eq!(m.cpu.pc(), AT + 4);
    assert_eq!(exec(&mut m, 0x0000_0dcd), Err(Stop::Break { pc: AT, code: 0x37 }));
    assert_eq!(m.cpu.pc(), AT);
    // lqc2 without the VU features.
    assert_eq!(exec(&mut m, 0xd800_0000), Err(Stop::Reserved { pc: AT, word: 0xd800_0000 }));
    assert_eq!(m.cpu.pc(), AT);
    m.cpu.features.vu = true;
    assert_eq!(exec(&mut m, 0xd800_0000), Ok(1));
    // psraw with rs set is not psraw to mips.py.
    assert_eq!(exec(&mut m, 0x7020_083f), Err(Stop::Reserved { pc: AT, word: 0x7020_083f }));
    assert_eq!(exec(&mut m, 0x7000_083f), Ok(1));
}

#[test]
fn a_store_past_ram_stops_with_a_bus_error_at_the_first_unmapped_byte() {
    let mut m = machine(Features::default());
    m.cpu.r[4] = u128::from((32u32 << 20) - 2);
    // sw zero, 0(a0)
    assert_eq!(exec(&mut m, 0xac80_0000), Err(Stop::Bus { pc: AT, addr: 32 << 20 }));
    assert_eq!(m.cpu.pc(), AT);
    // sb zero, 0(a0)
    assert_eq!(exec(&mut m, 0xa080_0000), Ok(1));
}

#[test]
fn a_branch_likely_not_taken_skips_its_delay_slot() {
    let mut m = machine(Features::default());
    m.cpu.r[4] = 1;
    // beql a0, zero, +4: not taken, so the next instruction is past the slot.
    exec(&mut m, 0x5080_0004).unwrap();
    assert_eq!((m.cpu.pc(), m.cpu.next_pc()), (AT + 8, AT + 12));
    // bne a0, zero, -1: taken back to itself after the slot.
    exec(&mut m, 0x1480_ffff).unwrap();
    assert_eq!((m.cpu.pc(), m.cpu.next_pc()), (AT + 4, AT));
}

// piney-eemu machine.rs ---------------------------------------------------

#[test]
fn a_call_returns_v0_with_the_stack_and_return_address_planted() {
    let mut m = machine(Features::default());
    // addu v0, a0, a1; jr ra; nop
    program(&mut m, 0x1000, &[0x0085_1021, JR_RA, NOP]);
    assert_eq!(m.call(0x1000, &[40, 2]), Ok(42));
    assert_eq!(m.steps, 3);
    assert_eq!(m.cpu.r[29], u128::from(STACK_TOP));
    assert_eq!(m.cpu.r[31], u128::from(RETURN_SENTINEL));
}

#[test]
fn delay_slots_run_and_a_likely_branch_not_taken_annuls_its_slot() {
    let mut m = machine(Features::default());
    // 0x1000 li v0, 1
    // 0x1004 beql zero, a0, +2 (to 0x1010), taken only when a0 is 0
    // 0x1008 addiu v0, v0, 10     delay slot: skipped when not taken
    // 0x100c addiu v0, v0, 100
    // 0x1010 jr ra
    // 0x1014 addiu v0, v0, 1000   delay slot of jr
    program(&mut m, 0x1000, &[0x2402_0001, 0x5004_0002, 0x2442_000a, 0x2442_0064, JR_RA, 0x2442_03e8]);
    assert_eq!(m.call(0x1000, &[0]), Ok(1011));
    assert_eq!(m.call(0x1000, &[1]), Ok(1101));
}

#[test]
fn hooks_stand_in_for_a_library_function_and_for_host_code() {
    let mut m = machine(Features::default());
    // move s0, ra; jal 0x2000; nop; jal 0x3000; move a0, v0; jr s0; nop
    program(&mut m, 0x1000, &[0x03e0_8021, 0x0c00_0800, NOP, 0x0c00_0c00, 0x0040_2021, 0x0200_0008, NOP]);
    m.bus.load(0x4000, b"hello\0");
    crate::hle::install(&mut m, 0x2000, "strlen");
    m.hook(0x3000, |c| {
        let a0 = c.arg32(0);
        HookResult::ret32(a0 * 2)
    });
    assert_eq!(m.call(0x1000, &[0x4000]), Ok(10));
    m.unhook(0x3000);
    // 0x3000 is now code: words of zeros (nop), on until the limit. A hook
    // is not a step here (piney counts it, and stops at 0x30b0).
    m.step_limit = 50;
    assert_eq!(m.call(0x1000, &[0x4000]), Err(Stop::StepLimit { pc: 0x30b4 }));
}

#[test]
fn a_call_stops_on_a_syscall_without_a_handler_and_on_an_unknown_word() {
    let mut m = machine(Features::default());
    program(&mut m, 0x1000, &[NOP, 0x0000_000c, JR_RA, NOP]);
    assert_eq!(m.call(0x1000, &[]), Err(Stop::Syscall { pc: 0x1004, code: 0 }));
    program(&mut m, 0x1100, &[NOP, 0x0000_0001]);
    assert_eq!(m.call(0x1100, &[]), Err(Stop::Reserved { pc: 0x1104, word: 1 }));
}

// Beyond piney's tests ------------------------------------------------------

#[test]
fn a_syscall_handler_answers_in_v0_and_the_run_continues_past_it() {
    let mut m = machine(Features::default());
    // li v1, 0x3c; syscall 0x10; jr ra; nop
    program(&mut m, 0x1000, &[0x2403_003c, 0x0000_040c, JR_RA, NOP]);
    m.on_syscall(|c, code| {
        let v1 = c.cpu.reg(3) as u64;
        HookResult::Return(v1 + u64::from(code))
    });
    // A newer handler that passes the call on leaves it to the older one.
    m.on_syscall(|_, _| HookResult::Continue);
    assert_eq!(m.call(0x1000, &[]), Ok(0x4c));
}

/// A hooked address reached as a branch's delay slot runs as code: it is not
/// a call to the hooked function.
#[test]
fn a_hook_does_not_fire_in_a_delay_slot() {
    let mut m = machine(Features::default());
    // beq zero, zero, +2; addiu v0, zero, 7 (delay slot, hooked);
    // addiu v0, zero, 1 (skipped); jr ra; nop
    program(&mut m, 0x1000, &[0x1000_0002, 0x2402_0007, 0x2402_0001, JR_RA, NOP]);
    m.stub(0x1004, 99);
    assert_eq!(m.call(0x1000, &[]), Ok(7));
    assert_eq!(m.call(0x1004, &[]), Ok(99), "called directly, it is a call");
    // A branch-likely not taken skips its slot: no delay slot is pending.
    m.cpu.r[4] = 1;
    exec(&mut m, 0x5080_0004).unwrap();
    assert!(!m.cpu.in_delay_slot());
    exec(&mut m, 0x1480_ffff).unwrap();
    assert!(m.cpu.in_delay_slot());
}

/// `sq a1, 0(a0)`; `lq v0, 0(a0)`; `jr ra`; `nop`.
const SQ_LQ: [u32; 4] = [0x7c85_0000, 0x7882_0000, JR_RA, NOP];

#[test]
fn lq_and_sq_move_a_whole_128_bit_register_through_memory() {
    let mut m = machine(Features::default());
    program(&mut m, 0x1000, &SQ_LQ);
    let v = u128::from_le_bytes(std::array::from_fn(|i| i as u8 + 1));
    m.cpu.r[5] = v;
    // The low four address bits are ignored: 0x2004 is the quadword at 0x2000.
    assert_eq!(m.call(0x1000, &[0x2004]), Ok(v as u64));
    assert_eq!(m.cpu.r[2], v);
    assert_eq!(m.bus.read_u128(0x2000), Ok(v));
    assert_eq!(m.bus.read_u128(0xa000_2000), Ok(v), "KSEG1 sees the same RAM");
}

#[test]
fn call_recorded_reports_the_sixteen_bytes_an_sq_wrote() {
    let mut m = machine(Features::default());
    program(&mut m, 0x1000, &SQ_LQ);
    let bytes: [u8; 16] = std::array::from_fn(|i| 0xf0 | i as u8);
    m.cpu.r[5] = u128::from_le_bytes(bytes);
    let o = m.call_recorded(0x1000, &[0x2008]).unwrap();
    assert_eq!(o.changes, [Change { addr: 0x2000, before: vec![0; 16], after: bytes.to_vec() }]);
    assert!(o.wrote(0x200c, 1) && !o.wrote(0x2010, 16));
    assert_eq!(o.ret, u64::from_le_bytes(bytes[..8].try_into().unwrap()));
    assert!(o.regs.iter().any(|&(name, v)| name == "a1" && v == u128::from_le_bytes(bytes)));
    // Through KSEG0 the change is reported at its canonical (physical)
    // address; the scratchpad keeps its own.
    m.cpu.r[5] = !m.cpu.r[5];
    let o = m.call_recorded(0x1000, &[0x8000_2000]).unwrap();
    assert_eq!(o.changes.len(), 1);
    assert_eq!(o.changes[0].addr, m.canonical(0x8000_2000));
    assert_eq!(o.changes[0].addr, 0x2000);
    let o = m.call_recorded(0x1000, &[0x7000_0010]).unwrap();
    assert_eq!(o.changes[0].addr, 0x7000_0010);
}

#[test]
fn mmi_instructions_work_on_lanes_of_the_128_bit_registers() {
    let mut m = machine(vu());
    let a = from_halves([0x7fff, 0x8000, 1, 2, 3, 4, 5, 6]);
    let b = from_halves([1, 0xffff, 1, 1, 1, 1, 1, 0xfffe]);
    m.cpu.r[4] = a;
    m.cpu.r[5] = b;
    // pcpyld v0, a0, a1: a1's low doubleword below a0's.
    exec(&mut m, 0x7085_1389).unwrap();
    assert_eq!(m.cpu.r[2], (b & M64) | ((a & M64) << 64));
    // pextlw v0, a0, a1: the low words interleaved, a1's first.
    exec(&mut m, 0x7085_1488).unwrap();
    assert_eq!(m.cpu.r[2], from_words([b as u32, a as u32, (b >> 32) as u32, (a >> 32) as u32]));
    // paddsh v0, a0, a1: halfwords added, saturating.
    exec(&mut m, 0x7085_1508).unwrap();
    assert_eq!(m.cpu.r[2], from_halves([0x7fff, 0x8000, 2, 3, 4, 5, 6, 4]));
    // psraw v0, a1, 4: each word shifted arithmetically.
    m.cpu.r[5] = from_words([0x8000_0000, 0x10, 0xffff_fff0, 0x7fff_ffff]);
    exec(&mut m, 0x7005_113f).unwrap();
    assert_eq!(m.cpu.r[2], from_words([0xf800_0000, 0x1, 0xffff_ffff, 0x07ff_ffff]));
    // Without the VU features pcpyld is not interpreted.
    m.cpu.features.vu = false;
    assert_eq!(exec(&mut m, 0x7085_1389), Err(Stop::Reserved { pc: AT, word: 0x7085_1389 }));
}

#[test]
fn cop1_arithmetic_follows_the_ee_float_rules() {
    let mut m = machine(Features::default());
    m.cpu.f[1] = float::bits(1.0);
    m.cpu.f[2] = float::bits(3.0);
    // div.s f0, f1, f2: 1/3 truncates where IEEE rounds up to 0x3eaaaaab.
    exec(&mut m, 0x4602_0803).unwrap();
    assert_eq!(m.cpu.f[0], 0x3eaa_aaaa);
    // mul.s f5, f3, f4 past the largest float clamps to it: no infinity.
    m.cpu.f[3] = 0x7f00_0000;
    m.cpu.f[4] = 0x7f00_0000;
    exec(&mut m, 0x4604_1942).unwrap();
    assert_eq!(m.cpu.f[5], float::FMAX);
    // c.lt.s f1, f2 sets the condition; bc1t +3 is then taken.
    exec(&mut m, 0x4602_0834).unwrap();
    assert_ne!(m.cpu.fcr31 & C_BIT, 0);
    exec(&mut m, 0x4501_0003).unwrap();
    assert_eq!(m.cpu.next_pc(), AT + 16);
}

#[test]
fn vu0_macro_instructions_reach_vu0_data_memory_through_the_bus() {
    let mut m = machine(Features { vu: true, vi: true, ..Features::default() });
    // qmtc2 a0, vf1; ctc2 a1, vi2; vsqi vf1, (vi2++); cfc2 v0, vi2; jr ra; nop
    program(&mut m, 0x1000, &[0x48a4_0800, 0x48c5_1000, 0x4be2_0b7d, 0x4842_1000, JR_RA, NOP]);
    let v = from_words([float::bits(1.0), float::bits(2.0), float::bits(3.0), float::bits(4.0)]);
    m.cpu.r[4] = v;
    m.cpu.r[5] = 3;
    assert_eq!(m.call(0x1000, &[]), Ok(4), "vi2 incremented past quadword 3");
    assert_eq!(m.bus.read_u128(VU0_DATA + 3 * 16), Ok(v));
    assert_eq!(m.cpu.reg(VF0 + 1), v);
    // Without `vi`, ctc2 is not interpreted.
    m.cpu.features.vi = false;
    assert_eq!(m.call(0x1000, &[]), Err(Stop::Reserved { pc: 0x1004, word: 0x48c5_1000 }));
}

#[test]
fn registers_are_found_by_mips_name_eabi_alias_and_number() {
    let cpu = Ee::new(Features::default());
    assert_eq!(cpu.name(), "R5900");
    assert_eq!(cpu.reg_index("v0"), Some(2));
    assert_eq!(cpu.reg_name(2), "v0");
    assert_eq!(cpu.reg_index("a4"), Some(8));
    assert_eq!(cpu.reg_index("t0"), Some(8));
    assert_eq!(cpu.reg_index("$sp"), Some(29));
    assert_eq!(cpu.reg_index("s8"), Some(30));
    assert_eq!(cpu.reg_index("r31"), Some(31));
    assert_eq!(cpu.reg_index("ra"), Some(31));
    assert_eq!(cpu.reg_index("hi1"), Some(HI1));
    assert_eq!(cpu.reg_index("f0"), Some(F0));
    assert_eq!(cpu.reg_index("fcr31"), Some(FCR31));
    assert_eq!(cpu.reg_index("acc"), Some(ACC));
    assert_eq!((cpu.reg_count(), cpu.reg_index("vf1")), (REGS, None));
    let mut cpu = Ee::new(vu());
    assert_eq!(cpu.reg_count(), REGS_VU);
    assert_eq!(cpu.reg_index("vf1"), Some(VF0 + 1));
    assert_eq!(cpu.reg_index("vi15"), Some(REGS_VU - 1));
    assert_eq!(cpu.reg_index("q"), Some(Q));
    // zero, vf0 and vi0 ignore writes.
    cpu.set_reg(0, 5);
    cpu.set_reg(VF0, 5);
    cpu.set_reg(VI0, 5);
    assert_eq!((cpu.reg(0), cpu.reg(VF0), cpu.reg(VI0)), (0, u128::from(0x3f80_0000u32) << 96, 0));
    cpu.set_reg(VACC, 7);
    assert_eq!(cpu.vacc, [7, 0, 0, 0]);
}

#[test]
fn the_preset_machine_has_the_stack_heap_sentinel_and_vu0_memory() {
    let mut m = machine(Features::default());
    assert_eq!((m.stack_top, m.return_to), (0x01ff_f000, 0xffff_fff0));
    assert_eq!(m.cpu.abi().args, &[4, 5, 6, 7, 8, 9, 10, 11]);
    assert_eq!(m.heap.alloc(1), HEAP_BASE);
    assert!(m.bus.write_u32(VU0_DATA + VU0_DATA_SIZE - 4, 1).is_ok());
    assert_eq!(m.bus.write_u32(VU0_DATA + VU0_DATA_SIZE, 1), Err(VU0_DATA + VU0_DATA_SIZE));
}

/// A little-endian MIPS ELF: `code` loaded at `at`, entry `at`, and a
/// symbol table of `syms`.
fn elf(at: u32, code: &[u32], syms: &[(&str, u32)]) -> Vec<u8> {
    fn put(f: &mut [u8], at: usize, n: usize, v: u32) {
        f[at..at + n].copy_from_slice(&v.to_le_bytes()[..n]);
    }
    let code: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut strtab = vec![0u8];
    let mut symtab = vec![0u8; 16];
    for (name, value) in syms {
        let mut e = [0u8; 16];
        put(&mut e, 0, 4, strtab.len() as u32);
        put(&mut e, 4, 4, *value);
        e[12] = 2;
        put(&mut e, 14, 2, 1);
        symtab.extend_from_slice(&e);
        strtab.extend_from_slice(name.as_bytes());
        strtab.push(0);
    }
    let code_off = 0x100;
    let sym_off = (code_off + code.len()).next_multiple_of(16);
    let str_off = sym_off + symtab.len();
    let sh_off = (str_off + strtab.len()).next_multiple_of(4);
    let mut f = vec![0u8; sh_off + 3 * 40];
    f[..4].copy_from_slice(b"\x7fELF");
    f[4] = 1;
    f[5] = 1;
    put(&mut f, 18, 2, 8);
    put(&mut f, 24, 4, at);
    put(&mut f, 28, 4, 0x34);
    put(&mut f, 32, 4, sh_off as u32);
    put(&mut f, 42, 2, 32);
    put(&mut f, 44, 2, 1);
    put(&mut f, 46, 2, 40);
    put(&mut f, 48, 2, 3);
    put(&mut f, 0x34, 4, 1);
    put(&mut f, 0x38, 4, code_off as u32);
    put(&mut f, 0x3c, 4, at);
    put(&mut f, 0x44, 4, code.len() as u32);
    put(&mut f, 0x48, 4, code.len() as u32);
    f[code_off..code_off + code.len()].copy_from_slice(&code);
    f[sym_off..str_off].copy_from_slice(&symtab);
    f[str_off..str_off + strtab.len()].copy_from_slice(&strtab);
    let (s1, s2) = (sh_off + 40, sh_off + 80);
    put(&mut f, s1 + 4, 4, 2);
    put(&mut f, s1 + 16, 4, sym_off as u32);
    put(&mut f, s1 + 20, 4, symtab.len() as u32);
    put(&mut f, s1 + 24, 4, 2);
    put(&mut f, s1 + 36, 4, 16);
    put(&mut f, s2 + 4, 4, 3);
    put(&mut f, s2 + 16, 4, str_off as u32);
    put(&mut f, s2 + 20, 4, strtab.len() as u32);
    f
}

#[test]
fn an_elf_machine_loads_symbols_sets_gp_and_stubs_kernel_and_libc_by_name() {
    const BREAK: u32 = 0x0000_000d;
    let mut code = vec![
        0x03e0_8021, // move s0, ra
        0x0c04_0040, // jal CreateSema (0x100100)
        NOP,
        0x005c_1821, // addu v1, v0, gp
        0x0200_0008, // jr s0
        0x0060_1021, // move v0, v1
    ];
    code.resize(0x40, NOP);
    code.extend([BREAK, BREAK]);
    let syms = [("f", 0x0010_0000), ("CreateSema", 0x0010_0100), ("memcpy", 0x0010_0104), ("_gp", 0x0010_8000)];
    let elf = Elf::parse(&elf(0x0010_0000, &code, &syms)).unwrap();
    let mut m = machine_with_elf(&elf, Features::default()).unwrap();
    assert_eq!(m.cpu.pc(), 0x0010_0000);
    assert_eq!(m.symbols.addr("f"), Some(0x0010_0000));
    assert_eq!(m.cpu.r[28], 0x0010_8000);
    assert!(m.is_hooked(0x0010_0100) && m.is_hooked(0x0010_0104));
    assert_eq!(m.call(0x0010_0000, &[]), Ok(0x0010_8001), "CreateSema gave 1, plus gp");
}
