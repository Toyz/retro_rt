//! Hostile input never panics: mutated ELFs and PS-X EXEs, images at the
//! bottom and the top of the address space, random bytes and random symbol
//! files, through parsing, analysis and listing. Run in the test profile,
//! where integer overflow panics, so a wrap is caught as well as an index.
//! `RRT_DIS_ROBUST_ROUNDS` raises the round count (default 400).

use rrt_dis::analysis::{Image, Program, find_gp};
use rrt_dis::exe::{Elf, PsxExe};
use rrt_dis::listing::Listing;
use rrt_dis::mips::Mips;
use rrt_dis::{Kind, Symbol, Symbols};

/// xorshift64: reproducible, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A small MIPS program: crt0 setting gp, a call with a string argument, a
/// switch through a table, and two returns.
fn program_words(base: u32) -> Vec<u32> {
    let hi = (base >> 16) as u16 as u32;
    vec![
        0x3c1c_0000 | hi, // lui gp, hi
        0x279c_0100,      // addiu gp, gp, 0x100
        0x3c04_0000 | hi, // lui a0, hi
        0x0c00_0000 | ((base.wrapping_add(0x20) >> 2) & 0x03ff_ffff),
        0x2484_0060,      // addiu a0, a0, 0x60
        0x2c82_0002,      // sltiu v0, a0, 2
        0x3c01_0000 | hi, // lui at, hi
        0x8c23_0050,      // lw v1, 0x50(at)
        0x0060_0008,      // 0x20: jr v1
        0,
        0x03e0_0008, // jr ra
        0,
        0x03e0_0008, // jr ra
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        base.wrapping_add(0x28), // 0x50: table
        base.wrapping_add(0x30),
        0,
        0,
        u32::from_le_bytes(*b"hell"), // 0x60
        u32::from_le_bytes([b'o', 0, 0, 0]),
    ]
}

fn psx_exe(base: u32, entry: u32) -> Vec<u8> {
    let words = program_words(base);
    let mut data = vec![0u8; 0x800];
    data[..8].copy_from_slice(b"PS-X EXE");
    data[0x10..0x14].copy_from_slice(&entry.to_le_bytes());
    data[0x18..0x1c].copy_from_slice(&base.to_le_bytes());
    data[0x1c..0x20].copy_from_slice(&(words.len() as u32 * 4).to_le_bytes());
    data.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    data
}

/// A little-endian ELF: one PT_LOAD of the program, and a symtab naming the
/// entry (func) and the table (object).
fn elf(base: u32) -> Vec<u8> {
    let code: Vec<u8> = program_words(base).iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut f = vec![0u8; 0x400];
    let put = |f: &mut Vec<u8>, at: usize, n: usize, v: u32| f[at..at + n].copy_from_slice(&v.to_le_bytes()[..n]);
    f[..4].copy_from_slice(b"\x7fELF");
    f[4] = 1;
    f[5] = 1;
    put(&mut f, 18, 2, 8);
    put(&mut f, 24, 4, base);
    put(&mut f, 28, 4, 0x34);
    put(&mut f, 32, 4, 0x300);
    put(&mut f, 42, 2, 32);
    put(&mut f, 44, 2, 1);
    put(&mut f, 46, 2, 40);
    put(&mut f, 48, 2, 3);
    put(&mut f, 0x34, 4, 1);
    put(&mut f, 0x38, 4, 0x100);
    put(&mut f, 0x3c, 4, base);
    put(&mut f, 0x44, 4, code.len() as u32);
    put(&mut f, 0x48, 4, code.len() as u32 + 0x40);
    f[0x100..0x100 + code.len()].copy_from_slice(&code);
    let (sh1, sh2) = (0x300 + 40, 0x300 + 80);
    put(&mut f, sh1 + 4, 4, 2);
    put(&mut f, sh1 + 16, 4, 0x200);
    put(&mut f, sh1 + 20, 4, 48);
    put(&mut f, sh1 + 24, 4, 2);
    put(&mut f, sh1 + 36, 4, 16);
    put(&mut f, sh2 + 4, 4, 3);
    put(&mut f, sh2 + 16, 4, 0x240);
    put(&mut f, sh2 + 20, 4, 12);
    f[0x240..0x24c].copy_from_slice(b"\0main\0table\0");
    put(&mut f, 0x210, 4, 1);
    put(&mut f, 0x214, 4, base);
    put(&mut f, 0x218, 4, 0x48);
    f[0x21c] = 2;
    put(&mut f, 0x21e, 2, 1);
    put(&mut f, 0x220, 4, 6);
    put(&mut f, 0x224, 4, base.wrapping_add(0x50));
    put(&mut f, 0x228, 4, 8);
    f[0x22c] = 1;
    put(&mut f, 0x22e, 2, 1);
    f
}

fn analyse(base: u32, bytes: &[u8], entry: u32, symbols: &Symbols) {
    for arch in [Mips::r3000(), Mips::r5900()] {
        let image = Image { base, bytes };
        let gp = find_gp(&arch, image, entry);
        let p = Program::analyze(&arch, image, entry, gp, symbols);
        let listing = Listing::new(&p, symbols);
        let _ = listing.range(base, base.saturating_add(0x200));
        for &start in p.funcs.keys() {
            let _ = listing.function(start);
        }
        let _ = (p.strings(4), p.unowned_words());
    }
}

fn mutate(rng: &mut Rng, data: &mut Vec<u8>, edits: u64) {
    for _ in 0..1 + rng.below(edits) {
        let at = rng.below(data.len() as u64) as usize;
        match rng.below(4) {
            0 => data[at] = rng.next() as u8,
            1 => data[at] ^= 1 << rng.below(8),
            2 => data[at] = [0, 0xff, 0x7f, 0x80][rng.below(4) as usize],
            _ => {
                let word = (at & !3).min(data.len().saturating_sub(4));
                let v = [0u32, u32::MAX, 0x8000_0000, 0x7fff_ffff, 0xffff_fff0][rng.below(5) as usize];
                data[word..word + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
    if rng.below(8) == 0 {
        data.truncate(rng.below(data.len() as u64) as usize);
    }
}

/// Bases that stress the edges: zero, the top of the address space, the
/// consoles' usual, and anywhere.
fn base(rng: &mut Rng) -> u32 {
    match rng.below(5) {
        0 => 0,
        1 => (u32::MAX - rng.below(0x100) as u32) & !3,
        2 => 0x8001_0000,
        3 => 0x0010_0000,
        _ => rng.next() as u32 & !3,
    }
}

#[test]
fn hostile_input_never_panics() {
    let rounds: u64 = std::env::var("RRT_DIS_ROBUST_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(400);
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..rounds {
        let b = base(&mut rng);

        let mut e = elf(b);
        mutate(&mut rng, &mut e, 12);
        if let Ok(parsed) = Elf::parse(&e)
            && let Ok((at, bytes)) = parsed.flatten()
        {
            analyse(at, &bytes[..bytes.len().min(0x1000)], parsed.entry, &parsed.symbols);
        }

        let entry = if rng.below(2) == 0 { b } else { rng.next() as u32 };
        let mut x = psx_exe(b, entry);
        mutate(&mut rng, &mut x, 24);
        if let Ok(parsed) = PsxExe::parse(&x) {
            analyse(parsed.t_addr, &parsed.image, parsed.pc0, &Symbols::new());
        }

        let raw: Vec<u8> = (0..rng.below(700)).map(|_| rng.next() as u8).collect();
        let mut symbols = Symbols::new();
        for _ in 0..rng.below(6) {
            let at = b.wrapping_add(rng.below(0x400) as u32);
            let kind = [Kind::Func, Kind::Data, Kind::Label, Kind::Unknown][rng.below(4) as usize];
            let size = [None, Some(0), Some(4), Some(u32::MAX)][rng.below(4) as usize];
            symbols.mark(Symbol { kind, size, ..Symbol::new(format!("s{at:x}"), at) });
        }
        analyse(b, &raw, b.wrapping_add(rng.below(64) as u32 & !3), &symbols);
        let _ = Symbols::parse(&String::from_utf8_lossy(&raw));
    }
}

#[test]
fn the_unmutated_inputs_analyse_as_written() {
    for b in [0u32, 0x8001_0000, 0x0010_0000] {
        let parsed = Elf::parse(&elf(b)).unwrap();
        assert_eq!(parsed.symbols.get("main").map(|s| (s.addr, s.kind, s.size)), Some((b, Kind::Func, Some(0x48))));
        let (at, bytes) = parsed.flatten().unwrap();
        let arch = Mips::r3000();
        let image = Image { base: at, bytes: &bytes };
        let p = Program::analyze(&arch, image, b, find_gp(&arch, image, b), &parsed.symbols);
        assert_eq!(p.jump_tables[&(b + 0x20)].targets, [b + 0x28, b + 0x30], "base {b:#x}");
        assert_eq!(p.gp, Some(b + 0x100));
    }
    // At the very top the image is cut short below 4 GiB, and still runs.
    let x = PsxExe::parse(&psx_exe(0xffff_ff80, 0xffff_ff80)).unwrap();
    analyse(x.t_addr, &x.image, x.pc0, &Symbols::new());
}
