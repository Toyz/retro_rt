//! The MIPS decoder against LLVM's, word by word.
//!
//! Words covering every primary opcode, SPECIAL function, REGIMM rt, the
//! COP0/COP1/COP2 rs values and their function fields, MMI and the VU0
//! macro tables, with varied register and immediate fields (and the
//! must-be-zero fields zeroed often enough to hit valid encodings), are
//! assembled as `.word` with `llvm-mc -triple=mipsel` and disassembled by
//! `llvm-objdump --triple=mipsel`: `--mcpu=mips1` against the R3000A,
//! `--mcpu=mips3` against the R5900 (falling back to `--mcpu=mips4` for
//! the words mips3 rejects, for `movz`/`movn`/`pref`). Both sides are
//! compared in canonical form (this decoder with pseudo-ops off).
//!
//! llvm-objdump has no no-aliases option for MIPS, so its aliases are
//! turned back into the instructions they stand for (`nop`, `move`, `b`,
//! `bal`, `beqz`/`bnez`(`l`), `negu`/`neg`/`dnegu`/`dneg`, `not`, one-operand
//! `jalr`, `jr` for `jalr zero`, `div $zero, s, t`, the two-operand `mult`
//! on the R5900, a zero COP select). Then both texts are normalised the
//! same way: `$` dropped, register names to numbers, immediates to
//! decimal, `<symbol>` annotations dropped; mnemonic and operands must
//! match. Two LLVM printing differences are normalised as well: `j`/`jal`
//! targets are compared on their low 28 bits (llvm-objdump prints the
//! field without the PC's region), and LLVM's `c.olt.s`/`c.ole.s` are the
//! EE's `c.lt.s`/`c.le.s` (the same condition codes, 4 and 6).
//!
//! Exclusions, counted and printed:
//! - R3000A: what LLVM decodes at mips1 that MIPS I does not have (LLVM's
//!   mips1 decoder accepts MIPS II-IV and MIPS32 instructions; listed by
//!   mnemonic), all of COP1 (the PS1 has no FPU), COP moves with a nonzero
//!   MIPS32 select field; the other way, the GTE commands and `rfe`, which
//!   LLVM does not know.
//! - R5900: the EE extensions LLVM does not know or reads as something
//!   else - MMI (opcode 0x1c, which LLVM reads as SPECIAL2), `lq`/`sq`,
//!   `lqc2`/`sqc2`, COP2 (VU0), the EE's COP0 (`ei`, `di`, the debug and
//!   performance counter moves), the EE COP1 operations (accumulator ops,
//!   three-operand `rsqrt.s`, `sqrt.s` reading ft, `max.s`/`min.s`),
//!   `mfsa`/`mtsa`/`mtsab`/`mtsah`, `sync.p`, three-operand `mult`/`multu`;
//!   and what LLVM decodes at mips3/mips4 that the EE does not have
//!   (doubles and the other FPU formats and conditions, `ll`/`sc`, 64-bit
//!   multiply and divide, `dmfc*`/`dmtc*`, `ldc1`/`sdc1`, COP1X, `movf`/
//!   `movt`, COP3 and LWC2/SWC2-space loads, `jalx`, MIPS32 selects).
//!
//! The test skips, with a note, when the LLVM tools are not found.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use rrt_dis::mips::{Isa, Mips};
use rrt_dis::{Arch, Reg, RegFile};

/// Where the code is placed, so no branch target wraps below zero.
const VMA: u32 = 0x1000_0000;

/// `name` on PATH, or the highest versioned `name-N`.
fn find_tool(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let mut best: Option<(u32, PathBuf)> = None;
    for dir in std::env::split_paths(&path) {
        let plain = dir.join(name);
        if plain.is_file() {
            return Some(plain);
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let file = e.file_name();
            let Some(v) = file.to_str().and_then(|f| f.strip_prefix(name)?.strip_prefix('-')?.parse::<u32>().ok())
            else {
                continue;
            };
            if best.as_ref().is_none_or(|(b, _)| v > *b) {
                best = Some((v, e.path()));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// A small deterministic generator (xorshift64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

const RS: u32 = 0x03e0_0000;
const RT: u32 = 0x001f_0000;
const RD: u32 = 0x0000_f800;
const SA: u32 = 0x0000_07c0;

/// The test words: `n` per encoding slot, each with a pattern of fields
/// cleared so the strict encodings are hit as well as the noise.
fn words() -> Vec<u32> {
    let mut rng = Rng(0x5900_3000);
    let mut out = Vec::new();
    let clears = [0, SA, RD | SA, RS, RS | SA, 0x7ff, RT | RD | SA, RS | RT, RS | RD | SA, 0x7fe, 0x01ff_ffc0, RT];
    let mut slot = |fixed: u32, keep: u32, n: usize, rng: &mut Rng| {
        for i in 0..n {
            let w = (rng.next() & keep & !clears[i % clears.len()]) | fixed;
            out.push(w);
        }
    };
    for op in 0..64 {
        slot(op << 26, 0x03ff_ffff, 160, &mut rng);
    }
    for f in 0..64 {
        slot(f, 0x03ff_ffc0, 96, &mut rng);
        // MMI (opcode 0x1c).
        slot((0x1c << 26) | f, 0x03ff_ffc0, 24, &mut rng);
    }
    for rt in 0..32 {
        slot((1 << 26) | (rt << 16), RS | 0xffff, 48, &mut rng);
    }
    for cop in [0x10u32, 0x11, 0x12] {
        for rs in 0..32 {
            slot((cop << 26) | (rs << 21), 0x001f_ffff, 24, &mut rng);
            if rs >= 0x10 {
                for f in 0..64 {
                    slot((cop << 26) | (rs << 21) | f, 0x001f_ffc0, 12, &mut rng);
                }
            }
        }
        // The branch-on-condition forms by rt.
        for rt in 0..32 {
            slot((cop << 26) | (8 << 21) | (rt << 16), 0xffff, 12, &mut rng);
        }
    }
    // MMI0-3 and the VU0 SPECIAL2 table by their sa/code fields.
    for f in [0x08u32, 0x09, 0x28, 0x29, 0x30, 0x31] {
        for sa in 0..32 {
            slot((0x1c << 26) | (sa << 6) | f, RS | RT | RD, 6, &mut rng);
        }
    }
    for code in 0..128u32 {
        slot((0x12 << 26) | (1 << 25) | ((code >> 2) << 6) | 0x3c | (code & 3), 0x01ff_f800, 6, &mut rng);
    }
    out
}

/// LLVM's reading of each word: (mnemonic, operands), None when it gives up.
type Llvm = Vec<Option<(String, Vec<String>)>>;

fn run(cmd: &mut Command) -> Vec<u8> {
    let out = cmd.output().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(out.status.success(), "{cmd:?}: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn llvm(mc: &Path, objdump: &Path, dir: &Path, words: &[u32], mcpu: &str) -> Llvm {
    let src = dir.join("words.s");
    let obj = dir.join("words.o");
    let mut s = String::from(".text\n");
    for w in words {
        let _ = writeln!(s, ".word 0x{w:08x}");
    }
    std::fs::write(&src, s).unwrap();
    run(Command::new(mc).args(["-triple=mipsel", "-filetype=obj", "-o"]).arg(&obj).arg(&src));
    let text = run(Command::new(objdump)
        .args(["-d", "--no-show-raw-insn", "--triple=mipsel"])
        .arg(format!("--mcpu={mcpu}"))
        .arg(format!("--adjust-vma=0x{VMA:x}"))
        .arg(&obj));
    let mut out: Llvm = vec![None; words.len()];
    for line in String::from_utf8_lossy(&text).lines() {
        let Some((addr, rest)) = line.split_once(":") else { continue };
        let Ok(addr) = u32::from_str_radix(addr.trim(), 16) else { continue };
        let rest = rest.trim();
        let rest = rest.split(" <").next().unwrap_or(rest).trim();
        if rest.is_empty() || rest.starts_with("<unknown>") {
            continue;
        }
        let (mn, ops) = rest.split_once('\t').unwrap_or((rest, ""));
        let ops =
            if ops.trim().is_empty() { Vec::new() } else { ops.split(',').map(|o| o.trim().to_string()).collect() };
        let i = ((addr - VMA) / 4) as usize;
        if i < out.len() {
            out[i] = Some((mn.trim().to_string(), ops));
        }
    }
    out
}

/// LLVM's aliases back to the instructions they stand for.
fn unalias(isa: Isa, word: u32, mn: &str, mut ops: Vec<String>) -> (String, Vec<String>) {
    let zero = || "$zero".to_string();
    let rt = (word >> 16) & 31;
    let mn = match mn {
        "nop" => {
            ops = vec![zero(), zero(), "0".into()];
            "sll"
        }
        "move" => {
            if rt == 0 {
                ops.push(zero());
            } else {
                ops.insert(1, zero());
            }
            match word & 63 {
                0x21 => "addu",
                0x25 => "or",
                0x2d => "daddu",
                _ => "move",
            }
        }
        "b" if word >> 26 == 1 => {
            ops.insert(0, zero());
            "bgez"
        }
        "b" => {
            ops.splice(0..0, [zero(), zero()]);
            "beq"
        }
        "bal" => {
            ops.insert(0, zero());
            "bgezal"
        }
        "beqz" | "bnez" | "beqzl" | "bnezl" => {
            ops.insert(1, zero());
            match mn {
                "beqz" => "beq",
                "bnez" => "bne",
                "beqzl" => "beql",
                _ => "bnel",
            }
        }
        "negu" | "neg" | "dnegu" | "dneg" => {
            ops.insert(1, zero());
            match mn {
                "negu" => "subu",
                "neg" => "sub",
                "dnegu" => "dsubu",
                _ => "dsub",
            }
        }
        "not" => {
            ops.push(zero());
            "nor"
        }
        "jalr" if ops.len() == 1 => {
            ops.insert(0, "$ra".into());
            "jalr"
        }
        "jr" if word & 63 == 9 => {
            ops.insert(0, zero());
            "jalr"
        }
        "div" | "divu" | "ddiv" | "ddivu" if ops.len() == 3 && ops[0] == "$zero" => {
            ops.remove(0);
            mn
        }
        "mult" | "multu" if isa == Isa::R5900 && ops.len() == 2 => {
            ops.insert(0, zero());
            mn
        }
        "mfc0" | "mtc0" | "mfc2" | "mtc2" | "dmfc0" | "dmtc0" if ops.len() == 3 && num(&ops[2]) == Some(0) => {
            ops.pop();
            mn
        }
        "c.olt.s" => "c.lt.s",
        "c.ole.s" => "c.le.s",
        _ => mn,
    };
    (mn.to_string(), ops)
}

fn num(s: &str) -> Option<i64> {
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let v = match s.strip_prefix("0x") {
        Some(hex) => i64::from_str_radix(hex, 16).ok()?,
        None => s.parse().ok()?,
    };
    Some(if neg { -v } else { v })
}

/// Register names (ours and LLVM's) to numbers, for one CPU.
fn names(m: Mips) -> HashMap<String, i64> {
    let mut map = HashMap::new();
    let mut files = vec![RegFile::Gpr, RegFile::Cop0, RegFile::Fpr, RegFile::FpCtrl];
    if m.isa == Isa::R3000 {
        files.extend([RegFile::Cop2Data, RegFile::Cop2Ctrl]);
    }
    for file in files {
        for i in 0..32u8 {
            let name = m.reg_name(Reg { file, index: i });
            let old = map.insert(name.clone(), i64::from(i));
            assert!(old.is_none_or(|o| o == i64::from(i)), "{name} names two numbers");
        }
    }
    map.insert("s8".into(), 30);
    map
}

/// One operand, normalised: `$` gone, registers and numbers as decimal.
fn norm(op: &str, names: &HashMap<String, i64>) -> String {
    let op = op.replace('$', "");
    let one = |t: &str| {
        let t = t.trim();
        if let Some(v) = num(t) {
            return v.to_string();
        }
        if let Some(&v) = names.get(t) {
            return v.to_string();
        }
        t.to_string()
    };
    match op.split_once('(') {
        Some((off, base)) => format!("{}({})", one(off), one(base.trim_end_matches(')'))),
        None => one(&op),
    }
}

/// Mnemonic and normalised operands; jump targets on their low 28 bits.
fn normalise(mn: &str, ops: &[String], names: &HashMap<String, i64>) -> String {
    let mut ops: Vec<String> = ops.iter().map(|o| norm(o, names)).collect();
    if matches!(mn, "j" | "jal") {
        ops = ops.iter().map(|o| (num(o).unwrap_or(0) & 0x0fff_ffff).to_string()).collect();
    }
    format!("{mn} {}", ops.join(", "))
}

/// Our canonical text split as LLVM's is.
fn ours(m: Mips, word: u32, addr: u32) -> Option<(String, Vec<String>)> {
    let insn = m.decode(&word.to_le_bytes(), addr);
    if !insn.is_valid() {
        return None;
    }
    let text = m.render(&insn, &|_| None);
    let (mn, ops) = text.split_once(' ').unwrap_or((&text, ""));
    let ops = if ops.trim().is_empty() { Vec::new() } else { ops.split(',').map(|o| o.trim().to_string()).collect() };
    Some((mn.to_string(), ops))
}

/// MIPS I as LLVM names it (after unaliasing), without COP1.
const MIPS1: &[&str] = &[
    "add", "addi", "addiu", "addu", "and", "andi", "beq", "bgez", "bgezal", "bgtz", "blez", "bltz", "bltzal", "bne",
    "break", "div", "divu", "j", "jal", "jalr", "jr", "lb", "lbu", "lh", "lhu", "lui", "lw", "lwl", "lwr", "mfhi",
    "mflo", "mthi", "mtlo", "mult", "multu", "nor", "or", "ori", "sb", "sh", "sll", "sllv", "slt", "slti", "sltiu",
    "sltu", "sra", "srav", "srl", "srlv", "sub", "subu", "sw", "swl", "swr", "syscall", "xor", "xori", "mfc0", "mtc0",
    "mfc2", "mtc2", "cfc2", "ctc2", "lwc2", "swc2",
];

fn is_cop1(word: u32, mn: &str) -> bool {
    matches!(word >> 26, 0x11 | 0x13 | 0x31 | 0x35 | 0x39 | 0x3d)
        || mn.ends_with(".s")
        || mn.ends_with(".d")
        || mn.ends_with(".w")
        || mn.ends_with(".l")
        || mn.ends_with(".ps")
}

/// Why an R3000A word may differ from LLVM, if it may.
fn r3000_exclusion(word: u32, llvm_mn: Option<&str>, mine: bool) -> Option<String> {
    let (op, rs) = (word >> 26, (word >> 21) & 31);
    if mine {
        if op == 0x12 && rs & 0x10 != 0 {
            return Some("GTE command (LLVM has no GTE)".into());
        }
        if op == 0x10 && rs == 0x10 {
            return Some("rfe (R3000 COP0; LLVM has none)".into());
        }
        if op == 0x12 && matches!(rs, 2 | 6) {
            return Some("cfc2/ctc2 (LLVM has none)".into());
        }
        return None;
    }
    let mn = llvm_mn?;
    if is_cop1(word, mn) {
        return Some("COP1: the PS1 has no FPU".into());
    }
    if matches!(op, 0x10 | 0x12) && matches!(rs, 0 | 4 | 1 | 5) && word & 0x7ff != 0 {
        return Some("COP move with a MIPS32 select or nonzero low bits".into());
    }
    if !MIPS1.contains(&mn) {
        return Some(format!("not on the PS1's R3000A (MIPS II+, MIPS32, TLB): {mn}"));
    }
    None
}

/// Why an R5900 word may differ from LLVM, if it may.
fn r5900_exclusion(word: u32, llvm: Option<&(String, Vec<String>)>, my_mn: Option<&str>) -> Option<String> {
    let llvm_mn = llvm.map(|l| l.0.as_str());
    let (op, rs, rt, rd, funct) = (word >> 26, (word >> 21) & 31, (word >> 16) & 31, (word >> 11) & 31, word & 63);
    // EE extensions: LLVM cannot read them, or reads them as something else.
    if my_mn.is_some() {
        let ext = match op {
            0x1c => Some("MMI (LLVM reads SPECIAL2)"),
            0x1e | 0x1f => Some("lq/sq"),
            0x36 | 0x3e => Some("lqc2/sqc2"),
            0x12 => Some("COP2 as VU0"),
            0x10 if rs == 0x10 && matches!(funct, 0x38 | 0x39) => Some("EE COP0 ei/di"),
            0x10 if matches!(rs, 0 | 4) && matches!(rd, 24 | 25) => Some("EE COP0 debug/perf moves"),
            0x11 if rs == 0x10 && matches!(funct, 0x04 | 0x16 | 0x18..=0x1f | 0x28 | 0x29) => {
                Some("EE COP1 (accumulator, rsqrt.s, sqrt.s on ft, max/min)")
            }
            0x00 if matches!(funct, 0x28 | 0x29) => Some("mfsa/mtsa"),
            0x00 if funct == 0x0f && (word >> 6) & 31 == 0x10 => Some("sync.p"),
            0x00 if matches!(funct, 0x18 | 0x19) && rd != 0 => Some("three-operand mult/multu"),
            0x01 if matches!(rt, 0x18 | 0x19) => Some("mtsab/mtsah"),
            0x10 if rs == 8 => Some("bc0f/bc0t/bc0fl/bc0tl (LLVM has none)"),
            _ => None,
        };
        if let Some(why) = ext {
            return Some(format!("EE extension: {why}"));
        }
    }
    let mn = llvm_mn?;
    if my_mn.is_some() {
        return None;
    }
    let why = if llvm.is_some_and(|l| l.1.iter().any(|o| o.contains("fcc"))) {
        "MIPS IV FP condition code field (the EE has one condition bit)".to_string()
    } else if mn.starts_with("sync") {
        "sync with a stype or nonzero fields (the EE has sync and sync.p)".to_string()
    } else if mn.ends_with(".d") || mn.ends_with(".l") || mn.ends_with(".ps") {
        "FPU format the EE lacks (double, long, paired)".to_string()
    } else if op == 0x11 && (mn.starts_with("c.") || mn.ends_with(".w") || mn.ends_with(".s")) {
        format!("FPU operation or condition the EE lacks: {mn}")
    } else if matches!(op, 0x10..=0x12) && matches!(rs, 0 | 4 | 1 | 5 | 2 | 6) && word & 0x7ff != 0 {
        "COP move with a MIPS32 select or nonzero low bits".to_string()
    } else {
        format!("not on the EE: {mn}")
    };
    Some(why)
}

struct Tally {
    compared: usize,
    both_invalid: usize,
    excluded: BTreeMap<String, usize>,
    examples: BTreeMap<String, Vec<String>>,
    failures: Vec<String>,
}

fn differential(m: Mips, words: &[u32], llvm: &Llvm) -> Tally {
    let names = names(m);
    let mut t = Tally {
        compared: 0,
        both_invalid: 0,
        excluded: BTreeMap::new(),
        examples: BTreeMap::new(),
        failures: Vec::new(),
    };
    let canon = m.canonical();
    for (i, &w) in words.iter().enumerate() {
        let addr = VMA + 4 * i as u32;
        let mine = ours(canon, w, addr);
        let theirs = llvm[i].clone().map(|(mn, ops)| unalias(m.isa, w, &mn, ops));
        let exclusion = match m.isa {
            Isa::R3000 => r3000_exclusion(w, theirs.as_ref().map(|t| t.0.as_str()), mine.is_some()),
            Isa::R5900 => r5900_exclusion(w, theirs.as_ref(), mine.as_ref().map(|t| t.0.as_str())),
        };
        let show = |mine: &Option<(String, Vec<String>)>, theirs: &Option<(String, Vec<String>)>| {
            format!(
                "{w:08x}: ours {:?}, llvm {:?}",
                mine.as_ref().map(|(a, o)| normalise(a, o, &names)),
                theirs.as_ref().map(|(a, o)| normalise(a, o, &names))
            )
        };
        let mut exclude = |why: String| {
            let ex = t.examples.entry(why.clone()).or_default();
            if ex.len() < 3 {
                ex.push(show(&mine, &theirs));
            }
            *t.excluded.entry(why).or_default() += 1;
        };
        match (&mine, &theirs) {
            (None, None) => t.both_invalid += 1,
            (Some((a, ao)), Some((b, bo))) => {
                let (x, y) = (normalise(a, ao, &names), normalise(b, bo, &names));
                if x == y {
                    t.compared += 1;
                } else if let Some(why) = exclusion {
                    exclude(why);
                } else {
                    t.failures.push(format!("{w:08x}: ours `{x}`, llvm `{y}`"));
                }
            }
            _ => match exclusion {
                Some(why) => exclude(why),
                None => t.failures.push(show(&mine, &theirs)),
            },
        }
    }
    t
}

#[test]
fn matches_llvm_objdump() {
    let (Some(mc), Some(objdump)) = (find_tool("llvm-mc"), find_tool("llvm-objdump")) else {
        println!("note: llvm-mc or llvm-objdump not found; skipping the LLVM differential");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rrt-dis-llvm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let words = words();
    let mips1 = llvm(&mc, &objdump, &dir, &words, "mips1");
    let mips3 = llvm(&mc, &objdump, &dir, &words, "mips3");
    let mips4 = llvm(&mc, &objdump, &dir, &words, "mips4");
    let _ = std::fs::remove_dir_all(&dir);
    let ee: Llvm = mips3.into_iter().zip(mips4).map(|(a, b)| a.or(b)).collect();

    let mut failed = Vec::new();
    for (m, llvm, cpu) in [(Mips::r3000(), &mips1, "mips1"), (Mips::r5900(), &ee, "mips3/mips4")] {
        let t = differential(m, &words, llvm);
        println!(
            "{} vs llvm-objdump --mcpu={cpu}: {} words, {} compared equal, {} invalid in both, {} excluded",
            m.name(),
            words.len(),
            t.compared,
            t.both_invalid,
            t.excluded.values().sum::<usize>()
        );
        // RRT_DIS_LLVM_VERBOSE=1 shows up to three words for each exclusion.
        let verbose = std::env::var_os("RRT_DIS_LLVM_VERBOSE").is_some();
        for (why, n) in &t.excluded {
            println!("  excluded {n:>6}  {why}");
            if verbose {
                for ex in &t.examples[why] {
                    println!("             {ex}");
                }
            }
        }
        for f in t.failures.iter().take(60) {
            println!("  MISMATCH {f}");
        }
        if !t.failures.is_empty() {
            failed.push(format!("{}: {} mismatches", m.name(), t.failures.len()));
        }
        assert!(t.compared > 1000, "{}: too few words compared", m.name());
    }
    assert!(failed.is_empty(), "{}", failed.join("; "));
}
