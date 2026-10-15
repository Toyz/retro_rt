//! The MIPS decoder against the vectors of the two decoders it replaces -
//! hwtr's R3000A (`hwtr-psx/src/mips.rs`) and piney's R5900
//! (`tools/test_mips.py`) - and the structure it fills in: operands, flow,
//! registers read and written, memory. The LLVM differential is in
//! `llvm.rs`.
//!
//! Adaptations from the sources, all cosmetic:
//! - piney prints registers with `$` (`$sp`, `$vf5y`, `$Q`, `$Status`);
//!   this decoder prints them without, so the expected texts have every `$`
//!   removed and are otherwise compared exactly, padding included.
//! - hwtr's tests compare with runs of whitespace collapsed; so do these.
//! - piney's `Insn` fields (`target`, `delay`, `cond`, `likely`, `call`,
//!   `base`/`offset`/`size`/`store`, `imm`) are checked through [`Flow`],
//!   [`Insn::delay_slot`], [`Insn::mem`] and [`Insn::imm`].

use rrt_dis::mips::{Isa, Mips};
use rrt_dis::{Arch, Flow, Insn, Mem, Operand, Reg, RegFile};

fn decode(m: Mips, word: u32, pc: u32) -> Insn {
    m.decode(&word.to_le_bytes(), pc)
}

fn text(m: Mips, word: u32, pc: u32) -> String {
    m.render(&decode(m, word, pc), &|_| None)
}

fn squeezed(m: Mips, word: u32, pc: u32) -> String {
    text(m, word, pc).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn reg(file: RegFile, index: u8) -> Reg {
    Reg { file, index }
}

const HI: Reg = Reg { file: RegFile::Special, index: 0 };
const LO: Reg = Reg { file: RegFile::Special, index: 1 };
const HI1: Reg = Reg { file: RegFile::Special, index: 2 };
const LO1: Reg = Reg { file: RegFile::Special, index: 3 };
const SA: Reg = Reg { file: RegFile::Special, index: 4 };

fn g(i: u8) -> Reg {
    Reg::gpr(i)
}

// hwtr-psx/src/mips.rs tests.

#[test]
fn hwtr_common_forms() {
    let m = Mips::r3000();
    assert_eq!(squeezed(m, 0x0000_0000, 0), "nop");
    assert_eq!(squeezed(m, 0x27bd_ffe8, 0), "addiu sp, sp, -24");
    assert_eq!(squeezed(m, 0xafbf_0010, 0), "sw ra, 16(sp)");
    assert_eq!(squeezed(m, 0x03e0_0008, 0), "jr ra");
    assert_eq!(squeezed(m, 0x3c02_8001, 0), "lui v0, 0x8001");
    assert_eq!(squeezed(m, 0x0080_1021, 0), "move v0, a0");
    assert_eq!(squeezed(m, 0x2402_0005, 0), "li v0, 5");
    // jal 0x80012340 from 0x80010000
    assert_eq!(squeezed(m, 0x0c00_48d0, 0x8001_0000), "jal 0x80012340");
    // beq zero, zero, -1 instructions: branch to itself
    assert_eq!(squeezed(m, 0x1000_ffff, 0x8001_0000), "b 0x80010000");
    assert_eq!(squeezed(m, 0x1440_0003, 0x8001_0000), "bnez v0, 0x80010010");
}

#[test]
fn hwtr_gte() {
    let m = Mips::r3000();
    // RTPS is cop2 0x0180001
    assert_eq!(squeezed(m, 0x4a18_0001, 0), "rtps sf=1, lm=0");
    assert_eq!(squeezed(m, 0x4a28_0030, 0), "rtpt sf=1, lm=0");
    assert_eq!(squeezed(m, 0x4b40_0006, 0), "nclip sf=0, lm=0");
    assert_eq!(squeezed(m, 0x4888_4800, 0), "mtc2 t0, ir1");
    assert_eq!(squeezed(m, 0xc880_0000, 0), "lwc2 vxy0, 0(a0)");
}

#[test]
fn hwtr_cop0() {
    let m = Mips::r3000();
    assert_eq!(squeezed(m, 0x4002_6000, 0), "mfc0 v0, sr");
    assert_eq!(squeezed(m, 0x4200_0010, 0), "rfe");
}

// piney_apples tools/test_mips.py vectors, `$` removed.

const PINEY_BASE: &[(u32, &str)] = &[
    (0x27bdffe0, "addiu   $sp, $sp, -32"),
    (0xffbf0010, "sd      $ra, 16($sp)"),
    (0x0080282d, "move    $a1, $a0"),
    (0x00802825, "move    $a1, $a0"),
    (0x3c020038, "lui     $v0, 0x38"),
    (0x34038000, "li      $v1, 0x8000"),
    (0x2403ffff, "li      $v1, -1"),
    (0x306200ff, "andi    $v0, $v1, 0xff"),
    (0x8f828df0, "lw      $v0, -29200($gp)"),
    (0x90860043, "lbu     $a2, 67($a0)"),
    (0xa2000041, "sb      $zero, 65($s0)"),
    (0xdfbf0010, "ld      $ra, 16($sp)"),
    (0x68820007, "ldl     $v0, 7($a0)"),
    (0x6c820000, "ldr     $v0, 0($a0)"),
    (0xb0820007, "sdl     $v0, 7($a0)"),
    (0x9c820004, "lwu     $v0, 4($a0)"),
    (0xc4800008, "lwc1    $f0, 8($a0)"),
    (0xe7acfffc, "swc1    $f12, -4($sp)"),
    (0x000317fc, "dsll32  $v0, $v1, 31"),
    (0x000310ff, "dsra32  $v0, $v1, 3"),
    (0x00831014, "dsllv   $v0, $v1, $a0"),
    (0x00031100, "sll     $v0, $v1, 4"),
    (0x0064100a, "movz    $v0, $v1, $a0"),
    (0x0064100b, "movn    $v0, $v1, $a0"),
    (0x00031023, "negu    $v0, $v1"),
    (0x00601027, "not     $v0, $v1"),
    (0x2c620064, "sltiu   $v0, $v1, 100"),
    (0x38621234, "xori    $v0, $v1, 0x1234"),
    (0x0062001a, "div     $v1, $v0"),
    (0x0062001b, "divu    $v1, $v0"),
    (0x00001010, "mfhi    $v0"),
    (0x00400011, "mthi    $v0"),
    (0x03e00008, "jr      $ra"),
    (0x0320f809, "jalr    $t9"),
    (0x03201009, "jalr    $v0, $t9"),
    (0x00000000, "nop"),
    (0x0000000c, "syscall"),
    (0x0007000d, "break   0x7"),
    (0x004301f4, "teq     $v0, $v1, 0x7"),
    (0x00400036, "tne     $v0, $zero"),
    (0x044c0005, "teqi    $v0, 5"),
    (0xcc800040, "pref    0x0, 64($a0)"),
    (0xbc870000, "cache   0x7, 0($a0)"),
    (0x44820000, "mtc1    $v0, $f0"),
    (0x4442f800, "cfc1    $v0, $fcr31"),
    (0x46020800, "add.s   $f0, $f1, $f2"),
    (0x46000806, "mov.s   $f0, $f1"),
    (0x46010032, "c.eq.s  $f0, $f1"),
    (0x6462fff8, "daddiu  $v0, $v1, -8"),
    (0x0003107a, "dsrl    $v0, $v1, 1"),
];

const PINEY_EE: &[(u32, &str)] = &[
    // 128-bit loads/stores, three-operand multiplies, pipeline 1, SA
    (0x7fb30030, "sq      $s3, 48($sp)"),
    (0x7bb30030, "lq      $s3, 48($sp)"),
    (0x00631818, "mult    $v1, $v1, $v1"),
    (0x00640018, "mult    $v1, $a0"),
    (0x70851018, "mult1   $v0, $a0, $a1"),
    (0x7123001a, "div1    $t1, $v1"),
    (0x70001812, "mflo1   $v1"),
    (0x70850000, "madd    $a0, $a1"),
    (0x70851000, "madd    $v0, $a0, $a1"),
    (0x70801004, "plzcw   $v0, $a0"),
    (0x00001028, "mfsa    $v0"),
    (0x00400029, "mtsa    $v0"),
    (0x04980003, "mtsab   $a0, 3"),
    (0x0000040f, "sync.p"),
    (0x0000000f, "sync"),
    // MMI0-3 and the shifts
    (0x70851008, "paddw   $v0, $a0, $a1"),
    (0x70851488, "pextlw  $v0, $a0, $a1"),
    (0x70431b89, "pcpyld  $v1, $v0, $v1"),
    (0x708516e8, "qfsrv   $v0, $a0, $a1"),
    (0x708514e9, "pnor    $v0, $a0, $a1"),
    (0x700517a9, "pexcw   $v0, $a1"),
    (0x700a46e9, "pcpyh   $t0, $t2"),
    (0x70001030, "pmfhl.lw $v0"),
    (0x70001070, "pmfhl.uw $v0"),
    (0x700510f4, "psllh   $v0, $a1, 3"),
    (0x710b41e8, "pminh   $t0, $t0, $t3"),
    (0x70000229, "pmthi   $zero"),
    // COP0
    (0x40116000, "mfc0    $s1, $Status"),
    (0x4002c803, "mfpc    $v0, 1"),
    (0x42000038, "ei"),
    (0x42000039, "di"),
    (0x42000018, "eret"),
    // EE FPU
    (0x4617d01c, "madd.s  $f0, $f26, $f23"),
    (0x4618b01a, "mula.s  $f22, $f24"),
    (0x46000818, "adda.s  $f1, $f0"),
    (0x46000834, "c.lt.s  $f1, $f0"),
    (0x46010004, "sqrt.s  $f0, $f1"),
    (0x46020816, "rsqrt.s $f0, $f1, $f2"),
    (0x46020828, "max.s   $f0, $f1, $f2"),
    (0x46000824, "cvt.w.s $f0, $f1"),
    (0x46800820, "cvt.s.w $f0, $f1"),
    // COP2 moves and VU0 macro mode (sceVu0Normalize, hand-assembled rest)
    (0xd8a40000, "lqc2    $vf4, 0($a1)"),
    (0xf8860000, "sqc2    $vf6, 0($a0)"),
    (0x48a20801, "qmtc2.i $v0, $vf1"),
    (0x48220800, "qmfc2.ni $v0, $vf1"),
    (0x4844e000, "cfc2.ni $a0, $FBRST"),
    (0x4bc4216a, "vmul.xyz $vf5, $vf4, $vf4"),
    (0x4b052941, "vaddy.x $vf5, $vf5, $vf5y"),
    (0x4a0503bd, "vsqrt   $Q, $vf5x"),
    (0x4a0003bf, "vwaitq"),
    (0x4b000160, "vaddq.x $vf5, $vf0, $Q"),
    (0x4a0002ff, "vnop"),
    (0x4a6503bc, "vdiv    $Q, $vf0w, $vf5x"),
    (0x4bc0219c, "vmulq.xyz $vf6, $vf4, $Q"),
    (0x4bc31068, "vadd.xyz $vf1, $vf2, $vf3"),
    (0x4be31040, "vaddx.xyzw $vf1, $vf2, $vf3x"),
    (0x4be821bc, "vmulax.xyzw $ACC, $vf4, $vf8x"),
    (0x4bc209ff, "vclipw.xyz $vf1, $vf2w"),
    (0x4be1137c, "vlqi.xyzw $vf1, ($vi2++)"),
    (0x4b0113fe, "vilwr.x $vi1, ($vi2)"),
    (0x4b0113fd, "vmfir.x $vf1, $vi2"),
    (0x4a0117f2, "viaddi  $vi1, $vi2, -1"),
    (0x4a0000b8, "vcallms 0x0010"),
    (0x4a26333d, "vmr32.w $vf6, $vf6"),
];

const PINEY_NOT_EE: &[u32] = &[
    0x00ff00ff, // dsra32 with rs set: the 128-bit constant in _copyRefImage
    0x4c000000, // COP1X
    0x46200000, // add.d: no doubles on the EE
    0xc1111111, // ll
    0xd4000000, // ldc1
    0x0000001c, // dmult
    0x40000003, // mfc0 with a MIPS32 select
    0x70000002, // hole in the MMI table
];

fn check(table: &[(u32, &str)]) {
    let m = Mips::r5900();
    let mut bad = Vec::new();
    for &(w, want) in table {
        let want = want.replace('$', "");
        let got = text(m, w, 0x100000);
        if got != want {
            bad.push(format!("{w:08x}: got {got:?}, want {want:?}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn piney_base() {
    check(PINEY_BASE);
}

#[test]
fn piney_ee() {
    check(PINEY_EE);
}

#[test]
fn piney_word_fallback() {
    let m = Mips::r5900();
    for &w in PINEY_NOT_EE {
        let insn = decode(m, w, 0);
        assert!(!insn.is_valid(), "{w:08x} -> {}", m.render(&insn, &|_| None));
        assert_eq!(insn.flow, Flow::Invalid);
        assert_eq!(m.render(&insn, &|_| None), format!(".word   0x{w:08x}"));
    }
}

#[test]
fn piney_never_panics() {
    // piney decodes 100,000 random words; both CPUs and both renderings here.
    let mut x = 5900u64;
    for _ in 0..100_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let (w, pc) = (x as u32, ((x >> 32) as u32) & !3);
        for m in [Mips::r3000(), Mips::r5900(), Mips::r3000().canonical(), Mips::r5900().canonical()] {
            let insn = decode(m, w, pc);
            let _ = m.render(&insn, &|a| Some(format!("L{a:x}")));
        }
    }
}

#[test]
fn piney_branches() {
    let m = Mips::r5900();
    let i = decode(m, 0x10c300ef, 0x102698); // beq a2, v1
    assert_eq!(i.flow, Flow::Branch { target: 0x102a58, likely: false });
    assert!(i.delay_slot);
    let i = decode(m, 0x10000135, 0x102700); // b
    assert_eq!(m.render(&i, &|_| None), "b       0x00102bd8");
    assert_eq!(i.flow, Flow::Jump { target: Some(0x102bd8) });
    assert!(!i.flow.falls_through());
    let i = decode(m, 0x0c04b436, 0x102740); // jal
    assert_eq!(i.flow, Flow::Call { target: Some(0x12d0d8) });
    assert!(i.delay_slot && i.writes.contains(&g(31)));
    let i = decode(m, 0x5045fffb, 0x11e6c8); // beql
    assert!(matches!(i.flow, Flow::Branch { likely: true, .. }));
    assert_eq!(decode(m, 0x4901fff9, 0x13ea1c).flow.target(), Some(0x13ea04)); // bc2t
    assert_eq!(decode(m, 0x03e00008, 0).flow.target(), None);
}

#[test]
fn piney_fields() {
    let m = Mips::r5900();
    let i = decode(m, 0x8f828df0, 0); // lw v0, -29200(gp)
    assert_eq!(i.mem, Some(Mem { base: g(28), offset: -29200, size: 4, store: false }));
    assert_eq!((i.reads.as_slice(), i.writes.as_slice()), (&[g(28)][..], &[g(2)][..]));
    let i = decode(m, 0x7fb30030, 0); // sq
    assert_eq!(i.mem.map(|m| (m.size, m.store)), Some((16, true)));
    assert_eq!(decode(m, 0x3c020038, 0).imm(), Some(0x38));
    assert_eq!(decode(m, 0x2403ffff, 0).imm(), Some(-1));
    let i = decode(m, 0x70851018, 0); // mult1
    let mut w = i.writes.clone();
    w.sort();
    assert_eq!(w, [g(2), HI1, LO1]);
    let i = decode(m, 0x4be821bc, 0); // vmulax
    let vf = |n| reg(RegFile::VecFloat, n);
    assert_eq!((i.reads.as_slice(), i.writes.as_slice()), (&[vf(4), vf(8)][..], &[vf(32)][..]));
}

// The structure, class by class.

#[test]
fn names_and_traits() {
    let (a, b) = (Mips::r3000(), Mips::r5900());
    assert_eq!((a.name(), b.name()), ("R3000A", "R5900"));
    assert_eq!((a.isa, b.isa, a.pseudo, a.canonical().pseudo), (Isa::R3000, Isa::R5900, true, false));
    assert_eq!(a.align(), 4);
    assert_eq!(a.endian(), rrt_dis::Endian::Little);
    assert_eq!(a.reg_name(g(30)), "fp");
    assert_eq!(a.reg_name(reg(RegFile::Cop2Data, 9)), "ir1");
    assert_eq!(a.reg_name(reg(RegFile::Cop2Ctrl, 31)), "flag");
    assert_eq!(a.reg_name(reg(RegFile::Cop0, 12)), "sr");
    assert_eq!(b.reg_name(reg(RegFile::Cop0, 12)), "Status");
    assert_eq!(b.reg_name(reg(RegFile::Cop0, 34)), "iab");
    assert_eq!(b.reg_name(reg(RegFile::Cop2Ctrl, 22)), "Q");
    assert_eq!(b.reg_name(reg(RegFile::VecFloat, 32)), "ACC");
    assert_eq!(b.reg_name(reg(RegFile::VecInt, 7)), "vi7");
    assert_eq!(b.reg_name(reg(RegFile::Fpr, 12)), "f12");
    let names: Vec<_> = (0..5).map(|i| b.reg_name(reg(RegFile::Special, i))).collect();
    assert_eq!(names, ["hi", "lo", "hi1", "lo1", "sa"]);
    // Too few bytes: invalid, not a panic.
    let i = a.decode(&[0x08, 0x00], 0);
    assert!(!i.is_valid() && i.size == 2);
}

#[test]
fn canonical_forms_without_pseudo_ops() {
    let c3 = Mips::r3000().canonical();
    let c5 = Mips::r5900().canonical();
    let cases: &[(Mips, u32, &str)] = &[
        (c3, 0x0000_0000, "sll     zero, zero, 0"),
        (c3, 0x0080_1021, "addu    v0, a0, zero"),
        (c3, 0x2402_0005, "addiu   v0, zero, 5"),
        (c3, 0x3403_8000, "ori     v1, zero, 0x8000"),
        (c3, 0x1000_ffff, "beq     zero, zero, 0x00000000"),
        (c3, 0x1440_0003, "bne     v0, zero, 0x00000010"),
        (c3, 0x0411_0003, "bgezal  zero, 0x00000010"),
        (c3, 0x0003_1023, "subu    v0, zero, v1"),
        (c3, 0x0060_1027, "nor     v0, v1, zero"),
        (c3, 0x0320_f809, "jalr    ra, t9"),
        (c5, 0x0064_0018, "mult    zero, v1, a0"),
        (c5, 0x7085_0000, "madd    zero, a0, a1"),
        (c5, 0x0080_282d, "daddu   a1, a0, zero"),
    ];
    for &(m, w, want) in cases {
        assert_eq!(text(m, w, 0), want, "{w:08x}");
    }
    // The pseudo-ops for the same words.
    let p = Mips::r3000();
    assert_eq!(text(p, 0x0411_0003, 0), "bal     0x00000010");
    assert_eq!(text(p, 0x0401_0003, 0), "b       0x00000010");
    assert_eq!(text(Mips::r5900(), 0x5040_0003, 0), "beqzl   v0, 0x00000010");
    assert_eq!(text(Mips::r5900(), 0x0003_102f, 0), "dnegu   v0, v1");
}

#[test]
fn labels_name_targets_and_absolute_addresses() {
    let m = Mips::r3000();
    let label = |a: u32| (a == 0x8001_2340).then(|| "main".to_string());
    assert_eq!(m.render(&decode(m, 0x0c00_48d0, 0x8001_0000), &label), "jal     main");
    // An address through zero is absolute.
    let lw = 0x8c02_0000 | 0x1234;
    let label = |a: u32| (a == 0x1234).then(|| "table".to_string());
    assert_eq!(m.render(&decode(m, lw, 0), &label), "lw      v0, table(zero)");
    assert_eq!(text(m, lw, 0), "lw      v0, 4660(zero)");
}

#[test]
fn integer_reads_writes_and_memory() {
    let m = Mips::r3000();
    // addu v0, a0, a0: one read listed once.
    let i = decode(m, 0x0084_1021, 0);
    assert_eq!((i.reads, i.writes), (vec![g(4)], vec![g(2)]));
    // addiu zero, sp, 4: the write to zero is not listed.
    let i = decode(m, 0x27a0_0004, 0);
    assert!(i.writes.is_empty() && i.reads == [g(29)]);
    // lwl merges: reads its target.
    let i = decode(m, 0x8885_0003, 0);
    assert_eq!(i.reads, [g(5), g(4)]);
    assert_eq!(i.writes, [g(5)]);
    assert_eq!(i.mem, Some(Mem { base: g(4), offset: 3, size: 4, store: false }));
    // sh: a store reads both.
    let i = decode(m, 0xa485_fffe, 0);
    assert_eq!(i.reads, [g(5), g(4)]);
    assert!(i.writes.is_empty());
    assert_eq!(i.mem, Some(Mem { base: g(4), offset: -2, size: 2, store: true }));
    // mult/mflo through hi and lo.
    let i = decode(m, 0x0085_0018, 0);
    assert_eq!((i.reads.clone(), i.writes.clone()), (vec![g(4), g(5)], vec![HI, LO]));
    assert_eq!(decode(m, 0x0000_1012, 0).reads, [LO]);
    assert_eq!(decode(m, 0x0080_0011, 0).writes, [HI]);
    // Operands in written order.
    let i = decode(m, 0x0003_1100, 0);
    assert_eq!(i.operands, [Operand::Reg(g(2)), Operand::Reg(g(3)), Operand::Imm(4)]);
    let i = decode(m, 0x0007_004d, 0);
    assert_eq!(i.operands, [Operand::Code(7), Operand::Code(1)]);
    assert_eq!(text(m, 0x0007_004d, 0), "break   0x7, 0x1");
    // ldl and lq on the EE.
    let e = Mips::r5900();
    assert_eq!(decode(e, 0x6882_0007, 0).mem.map(|m| m.size), Some(8));
    assert_eq!(decode(e, 0x7bb3_0030, 0).mem, Some(Mem { base: g(29), offset: 48, size: 16, store: false }));
    // cache: a memory operand, no access.
    let i = decode(e, 0xbc87_0000, 0);
    assert!(i.mem.is_none() && matches!(i.operands[1], Operand::Mem(Mem { size: 0, .. })));
}

#[test]
fn flows_by_class() {
    let m = Mips::r3000();
    let f = |w: u32| decode(m, w, 0x1000);
    assert_eq!(f(0x03e0_0008).flow, Flow::Return);
    assert_eq!(f(0x0040_0008).flow, Flow::Jump { target: None });
    assert_eq!(f(0x0320_f809).flow, Flow::Call { target: None });
    assert_eq!(f(0x0320_f809).writes, [g(31)]);
    assert_eq!(f(0x0800_0400).flow, Flow::Jump { target: Some(0x1000) });
    assert_eq!(f(0x0c00_0400).flow, Flow::Call { target: Some(0x1000) });
    assert_eq!(f(0x0411_ffff).flow, Flow::Call { target: Some(0x1000) });
    assert_eq!(f(0x1000_ffff).flow, Flow::Jump { target: Some(0x1000) });
    assert_eq!(f(0x0401_ffff).flow, Flow::Jump { target: Some(0x1000) });
    assert_eq!(f(0x1c40_ffff).flow, Flow::Branch { target: 0x1000, likely: false });
    assert_eq!(f(0x0000_000c).flow, Flow::Trap);
    assert_eq!(f(0x0000_000d).flow, Flow::Trap);
    assert_eq!(f(0xffff_ffff).flow, Flow::Invalid);
    for w in [0x03e0_0008, 0x0320_f809, 0x0800_0400, 0x1000_ffff, 0x1c40_ffff] {
        assert!(f(w).delay_slot, "{w:08x}");
    }
    for w in [0x0000_0000, 0x0000_000c, 0xffff_ffff] {
        assert!(!f(w).delay_slot, "{w:08x}");
    }
    let e = Mips::r5900();
    let f = |w: u32| decode(e, w, 0x1000);
    assert_eq!(f(0x5440_ffff).flow, Flow::Branch { target: 0x1000, likely: true });
    assert_eq!(f(0x0413_ffff).flow, Flow::Call { target: Some(0x1000) });
    assert_eq!(f(0x4501_ffff).flow, Flow::Branch { target: 0x1000, likely: false });
    assert_eq!(f(0x4503_ffff).flow, Flow::Branch { target: 0x1000, likely: true });
    assert_eq!(f(0x004301f4).flow, Flow::Trap);
    assert_eq!(f(0x4200_0018).flow, Flow::Return);
    assert!(!f(0x4200_0018).delay_slot);
}

#[test]
fn cop0_and_gte_registers() {
    let m = Mips::r3000();
    let c0 = |n| reg(RegFile::Cop0, n);
    let i = decode(m, 0x4002_6000, 0);
    assert_eq!((i.reads, i.writes), (vec![c0(12)], vec![g(2)]));
    let i = decode(m, 0x4200_0010, 0);
    assert_eq!((i.reads, i.writes), (vec![c0(12)], vec![c0(12)]));
    let d = |n| reg(RegFile::Cop2Data, n);
    let c = |n| reg(RegFile::Cop2Ctrl, n);
    let i = decode(m, 0x4888_4800, 0); // mtc2 t0, ir1
    assert_eq!((i.reads, i.writes), (vec![g(8)], vec![d(9)]));
    let i = decode(m, 0x48c8_f800, 0); // ctc2 t0, flag
    assert_eq!(i.writes, [c(31)]);
    let i = decode(m, 0xc880_0000, 0); // lwc2 vxy0, 0(a0)
    assert_eq!(i.writes, [d(0)]);
    assert_eq!(i.mem.map(|m| m.size), Some(4));
    // nclip: the screen XYs to mac0 and flag.
    let i = decode(m, 0x4b40_0006, 0);
    assert_eq!((i.reads, i.writes), (vec![d(12), d(13), d(14)], vec![d(24), c(31)]));
    assert_eq!(i.operands, [Operand::Imm(0), Operand::Imm(0)]);
    // mvmva sf=1, mx=1 (light), v=3 (ir), cv=0 (translation).
    let w = 0x4a00_0012 | (1 << 19) | (1 << 17) | (3 << 15);
    let i = decode(m, w, 0);
    assert_eq!(text(m, w, 0), "mvmva   sf=1, lm=0, mx=1, v=3, cv=0");
    assert!(i.reads.contains(&c(8)) && i.reads.contains(&d(9)) && i.reads.contains(&c(5)));
    assert!(!i.reads.contains(&c(0)) && i.writes.contains(&d(25)));
    // An unknown GTE function is not a command.
    assert!(!decode(m, 0x4a00_0000, 0).is_valid());
}

#[test]
fn r3000_is_mips_i_only() {
    let m = Mips::r3000();
    for w in [
        0x0064_100a, // movz
        0x5040_0003, // beql
        0xdfbf_0010, // ld
        0x7085_1008, // paddw
        0x4602_0800, // add.s
        0x0000_000f, // sync
        0x004301f4,  // teq
        0x4200_0018, // eret
        0xd8a4_0000, // lqc2
        0x0021_0800, // sll with rs set
    ] {
        assert!(!decode(m, w, 0).is_valid(), "{w:08x}");
    }
}

#[test]
fn ee_hi_lo_sa_and_fpu() {
    let m = Mips::r5900();
    // madd reads and writes hi/lo and writes rd.
    let i = decode(m, 0x7085_1000, 0);
    assert_eq!((i.reads, i.writes), (vec![g(4), g(5), HI, LO], vec![g(2), HI, LO]));
    // pmfhi reads the whole 128-bit HI; pmultw writes both pairs.
    assert_eq!(decode(m, 0x7000_1209, 0).reads, [HI, HI1]);
    assert_eq!(decode(m, 0x7085_1309, 0).writes, [g(2), HI, LO, HI1, LO1]);
    // mtsab and qfsrv through sa.
    assert_eq!(decode(m, 0x0498_0003, 0).writes, [SA]);
    assert!(decode(m, 0x7085_16e8, 0).reads.contains(&SA));
    // FPU: the compares set fcr31's condition, bc1t reads it; the
    // accumulator ops.
    let fcr31 = reg(RegFile::FpCtrl, 31);
    let f = |n| reg(RegFile::Fpr, n);
    let acc = f(32);
    assert_eq!(decode(m, 0x4600_0834, 0).writes, [fcr31]);
    assert_eq!(decode(m, 0x4501_0003, 0).reads, [fcr31]);
    assert_eq!(decode(m, 0x4618_b01a, 0).writes, [acc]);
    let i = decode(m, 0x4617_d01c, 0); // madd.s f0, f26, f23
    assert_eq!((i.reads, i.writes), (vec![f(26), f(23), acc], vec![f(0)]));
    let i = decode(m, 0x4601_0004, 0); // sqrt.s f0, f1 (ft)
    assert_eq!((i.reads, i.writes), (vec![f(1)], vec![f(0)]));
    // mfc0 Status, the debug and performance moves.
    let i = decode(m, 0x4011_6000, 0);
    assert_eq!(i.reads, [reg(RegFile::Cop0, 12)]);
    let i = decode(m, 0x4002_c803, 0); // mfpc v0, 1
    assert_eq!(i.reads, [reg(RegFile::Cop0, 41)]);
    let i = decode(m, 0x4002_c002, 0); // mfiab v0
    assert_eq!(text(m, 0x4002_c002, 0), "mfiab   v0");
    assert_eq!(i.reads, [reg(RegFile::Cop0, 34)]);
}

#[test]
fn vu0_macro_mode() {
    let m = Mips::r5900();
    let vf = |n| reg(RegFile::VecFloat, n);
    let vi = |n| reg(RegFile::VecInt, n);
    let q = reg(RegFile::Cop2Ctrl, 22);
    let acc = vf(32);
    // The dest mask is part of the mnemonic.
    let i = decode(m, 0x4bc4_216a, 0);
    assert_eq!(i.mnemonic, "vmul.xyz");
    assert_eq!((i.reads, i.writes), (vec![vf(4)], vec![vf(5)]));
    // vaddq.x vf5, vf0, Q: vf0 is hard-wired and not listed; Q is read.
    let i = decode(m, 0x4b00_0160, 0);
    assert_eq!((i.reads, i.writes), (vec![q], vec![vf(5)]));
    // vdiv writes Q; vwaitq reads it.
    assert_eq!(decode(m, 0x4a65_03bc, 0).writes, [q]);
    assert_eq!(decode(m, 0x4a00_03bf, 0).reads, [q]);
    // vlqi reads and bumps vi2.
    let i = decode(m, 0x4be1_137c, 0);
    assert_eq!((i.reads, i.writes), (vec![vi(2)], vec![vf(1), vi(2)]));
    assert!(i.mem.is_none());
    // vmaddax reads the accumulator it adds into.
    let w = 0x4be0_0000 | (2 << 6) | 0x3c | (4 << 11) | (8 << 16);
    let i = decode(m, w, 0);
    assert_eq!(i.mnemonic, "vmaddax.xyzw");
    assert_eq!((i.reads, i.writes), (vec![acc, vf(4), vf(8)], vec![acc]));
    // vclipw writes the clip flags.
    assert_eq!(decode(m, 0x4bc2_09ff, 0).writes, [reg(RegFile::Cop2Ctrl, 18)]);
    // qmtc2/qmfc2, ctc2 to a VU0 control register, lqc2.
    assert_eq!(decode(m, 0x48a2_0801, 0).writes, [vf(1)]);
    assert_eq!(decode(m, 0x4822_0800, 0).reads, [vf(1)]);
    assert_eq!(decode(m, 0x48c4_d800, 0).writes, [reg(RegFile::Cop2Ctrl, 27)]);
    assert_eq!(decode(m, 0x48c4_0800, 0).writes, [vi(1)]);
    assert_eq!(decode(m, 0xd8a4_0000, 0).mem.map(|m| m.size), Some(16));
    // vcallmsr names CMSAR0.
    assert_eq!(text(m, 0x4a00_d839, 0), "vcallmsr CMSAR0");
    // Holes in both tables are invalid.
    assert!(!decode(m, 0x4a00_0033, 0).is_valid());
    assert!(!decode(m, 0x4a00_02bf, 0).is_valid());
    // vnop with a field set is not vnop.
    assert!(!decode(m, 0x4a01_02ff, 0).is_valid());
}

#[test]
fn evaluate_builds_addresses() {
    let m = Mips::r3000();
    let none = |_: Reg| None;
    // 0x8001a345: lui v0, 0x8002 ; addiu v0, v0, -0x5cbb (a negative low half).
    let lui = decode(m, 0x3c02_8002, 0);
    assert_eq!(m.evaluate(&lui, &none), Some(0x8002_0000));
    let v0 = |r: Reg| (r == g(2)).then_some(0x8002_0000);
    assert_eq!(m.evaluate(&decode(m, 0x2442_a345, 4), &v0), Some(0x8001_a345));
    // 0x80012345: a positive one.
    let v0 = |r: Reg| (r == g(2)).then_some(0x8001_0000);
    assert_eq!(m.evaluate(&decode(m, 0x2442_2345, 4), &v0), Some(0x8001_2345));
    // ori zero-extends.
    assert_eq!(m.evaluate(&decode(m, 0x3442_8000, 0), &|_| Some(0x1f80_0000)), Some(0x1f80_8000));
    // li and move through zero, which is never asked for.
    assert_eq!(m.evaluate(&decode(m, 0x2404_0005, 0), &none), Some(5));
    assert_eq!(m.evaluate(&decode(m, 0x3403_8000, 0), &none), Some(0x8000));
    let a0 = |r: Reg| (r == g(4)).then_some(0x1234);
    assert_eq!(m.evaluate(&decode(m, 0x0080_1021, 0), &a0), Some(0x1234));
    // An unknown input, a load, a write to zero.
    assert_eq!(m.evaluate(&decode(m, 0x2442_0004, 0), &none), None);
    assert_eq!(m.evaluate(&decode(m, 0x8c82_0000, 0), &|_| Some(0)), None);
    assert_eq!(m.evaluate(&decode(m, 0x3c00_8001, 0), &none), None);
    // sll and subu; the EE's daddiu/daddu on the low 32 bits.
    assert_eq!(m.evaluate(&decode(m, 0x0004_1080, 0), &a0), Some(0x1234 << 2));
    assert_eq!(m.evaluate(&decode(m, 0x0004_1023, 0), &a0), Some(0u32.wrapping_sub(0x1234)));
    let e = Mips::r5900();
    assert_eq!(e.evaluate(&decode(e, 0x6482_fff8, 0), &a0), Some(0x122c));
    assert_eq!(e.evaluate(&decode(e, 0x0080_102d, 0), &a0), Some(0x1234));
    assert_eq!(e.evaluate(&decode(e, 0xffff_ffff, 0), &a0), None);
}

#[test]
fn pointers_and_switch_bounds() {
    let m = Mips::r5900();
    assert_eq!(m.global_pointer(), Some(g(28)));
    assert_eq!(m.stack_pointer(), Some(g(29)));
    assert_eq!(m.switch_bound(&decode(m, 0x2c62_0064, 0)), Some(100)); // sltiu v0, v1, 100
    assert_eq!(m.switch_bound(&decode(m, 0x2c62_ffff, 0)), Some(0xffff_ffff));
    assert_eq!(m.switch_bound(&decode(m, 0x2862_0064, 0)), None); // slti
}
