//! Text listings of an analysed program: labels, symbol names in place of
//! addresses, and comments saying who calls a function, what a load reads,
//! which string an argument points at.
//!
//! Names come from the [`Symbols`] first; anything the analysis found but
//! nobody named gets a generated one, in the style splat and objdump
//! readers know: `func_80010020` for a function, `.L80010040` for a branch
//! or jump table target inside one, `str_80010070` for a string,
//! `D_80018010` for other data.
//!
//! ```text
//! func_80010020:                     # 1 caller: 80010000
//!   80010020  2c820002  sltiu   v0, a0, 2
//!   80010030  8c230060  lw      v1, 0x60(at)       # D_80010060
//!   80010038  00600008  jr      v1                 # switch, 2 cases at D_80010060
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::analysis::{Program, Ref};
use crate::insn::{Arch, Flow};
use crate::symbols::Symbols;

/// How many callers or references a label's comment lists before it counts
/// the rest.
const XREFS_SHOWN: usize = 4;

/// Renders an analysed [`Program`] with a project's [`Symbols`].
pub struct Listing<'p, 'a, A: Arch + ?Sized> {
    program: &'p Program<'a, A>,
    symbols: &'p Symbols,
    /// Site to the addresses it refers to and how.
    by_site: BTreeMap<u32, Vec<(u32, Ref)>>,
    /// Branch and jump table targets inside functions.
    locals: BTreeSet<u32>,
    /// The strings the code takes the address of.
    strings: BTreeSet<u32>,
    /// Print each instruction's address and encoding (default on).
    pub addresses: bool,
}

impl<'p, 'a, A: Arch + ?Sized> Listing<'p, 'a, A> {
    /// A listing of `program`, naming addresses from `symbols`.
    pub fn new(program: &'p Program<'a, A>, symbols: &'p Symbols) -> Self {
        let mut by_site: BTreeMap<u32, Vec<(u32, Ref)>> = BTreeMap::new();
        let mut locals = BTreeSet::new();
        for (&target, sites) in &program.refs {
            for &(site, how) in sites {
                by_site.entry(site).or_default().push((target, how));
                if how == Ref::Jump && program.owner.contains_key(&target) && !program.funcs.contains_key(&target) {
                    locals.insert(target);
                }
            }
        }
        for jt in program.jump_tables.values() {
            locals.extend(jt.targets.iter().filter(|t| !program.funcs.contains_key(t)));
        }
        let strings = program.strings(4).into_keys().collect();
        Listing { program, symbols, by_site, locals, strings, addresses: true }
    }

    /// The label at exactly `addr`: its symbol, else a generated name for
    /// a function, a local target, a string or data the code refers to.
    pub fn label(&self, addr: u32) -> Option<String> {
        if let Some(name) = self.symbols.name(addr) {
            return Some(name.to_string());
        }
        let p = self.program;
        if p.funcs.contains_key(&addr) {
            return Some(format!("func_{addr:08x}"));
        }
        if self.locals.contains(&addr) {
            return Some(format!(".L{addr:08x}"));
        }
        if p.refs.contains_key(&addr) && !p.owner.contains_key(&addr) {
            if self.strings.contains(&addr) {
                return Some(format!("str_{addr:08x}"));
            }
            return Some(format!("D_{addr:08x}"));
        }
        None
    }

    /// A name for any address: its label, else `symbol+0xN` inside a
    /// symbol's size, else the hex address.
    pub fn name(&self, addr: u32) -> String {
        if let Some(l) = self.label(addr) {
            return l;
        }
        if let Some(sym) = self.symbols.iter().filter(|s| s.contains(addr)).last() {
            return format!("{}+0x{:x}", sym.name, addr - sym.addr);
        }
        format!("0x{addr:08x}")
    }

    /// The listing of the function starting at `start`, from its entry to
    /// its end. None when no function starts there.
    pub fn function(&self, start: u32) -> Option<String> {
        let f = self.program.funcs.get(&start)?;
        Some(self.range(f.start, f.end))
    }

    /// The listing of `from..to`: owned instructions decoded, everything
    /// else as data - a string the code refers to as `.asciz`, other words
    /// as `.word`, named when they point at a label.
    pub fn range(&self, from: u32, to: u32) -> String {
        let mut out = String::new();
        let p = self.program;
        let endian = self.program.arch.endian();
        let mut pc = from;
        while pc < to && p.image.contains(pc) {
            self.label_line(&mut out, pc);
            if p.owner.contains_key(&pc) {
                let bytes = p.image.slice(pc, p.image.end().saturating_sub(pc) as usize).unwrap_or_default();
                let insn = p.arch.decode(bytes, pc);
                let text = p.arch.render(&insn, &|a| Some(self.name(a)));
                let comment = self.insn_comment(pc, &insn.flow);
                self.line(&mut out, pc, Some((insn.word, insn.size)), &text, &comment);
                pc += insn.size.max(1) as u32;
                continue;
            }
            if self.strings.contains(&pc)
                && let Some(s) = p.image.string(pc, 4)
            {
                self.line(&mut out, pc, None, &format!(".asciz  {s:?}"), "");
                pc += s.len() as u32 + 1;
                // Back onto the word grid, unless a label sits between.
                while !pc.is_multiple_of(4) && pc < to && self.label(pc).is_none() {
                    pc += 1;
                }
                continue;
            }
            match p.image.word(pc, endian).filter(|_| pc.is_multiple_of(4) && to.saturating_sub(pc) >= 4) {
                Some(w) => {
                    let text = match self.label(w) {
                        Some(l) if p.image.contains(w) => format!(".word   {l}"),
                        _ => format!(".word   0x{w:08x}"),
                    };
                    self.line(&mut out, pc, Some((w, 4)), &text, "");
                    pc += 4;
                }
                None => {
                    let b = p.image.slice(pc, 1).unwrap()[0];
                    self.line(&mut out, pc, Some((b as u32, 1)), &format!(".byte   0x{b:02x}"), "");
                    pc += 1;
                }
            }
        }
        out
    }

    fn label_line(&self, out: &mut String, addr: u32) {
        let Some(label) = self.label(addr) else { return };
        let p = self.program;
        let is_func = p.funcs.contains_key(&addr);
        let sites: Vec<u32> = if is_func {
            p.callers.get(&addr).map(|c| c.iter().copied().collect()).unwrap_or_default()
        } else {
            p.refs
                .get(&addr)
                .map(|r| r.iter().map(|(s, _)| *s).collect::<BTreeSet<_>>().into_iter().collect())
                .unwrap_or_default()
        };
        if is_func && !out.is_empty() {
            out.push('\n');
        }
        let mut comment = String::new();
        if !sites.is_empty() {
            let what = if is_func { "caller" } else { "ref" };
            let s = if sites.len() == 1 { "" } else { "s" };
            let _ = write!(comment, "{} {what}{s}:", sites.len());
            for site in sites.iter().take(XREFS_SHOWN) {
                let _ = write!(comment, " {site:08x}");
            }
            if sites.len() > XREFS_SHOWN {
                comment.push_str(" ...");
            }
        }
        if let Some(note) = self.symbols.at(addr).map(|s| s.note.as_str()).filter(|n| !n.is_empty()) {
            if !comment.is_empty() {
                comment.push_str("; ");
            }
            comment.push_str(note);
        }
        if comment.is_empty() {
            let _ = writeln!(out, "{label}:");
        } else {
            let _ = writeln!(out, "{:<34} # {comment}", format!("{label}:"));
        }
    }

    fn insn_comment(&self, pc: u32, flow: &Flow) -> String {
        let p = self.program;
        let mut parts: Vec<String> = Vec::new();
        if let Some(jt) = p.jump_tables.get(&pc) {
            parts.push(format!("switch, {} cases at {}", jt.targets.len(), self.name(jt.table)));
        }
        if *flow == (Flow::Call { target: None })
            && let Some(slot) = p.indirect.get(&pc)
        {
            match p.slots.get(slot).filter(|t| t.len() == 1).and_then(|t| t.first()) {
                Some(&t) => parts.push(format!("-> {} via {}", self.name(t), self.name(*slot))),
                None => parts.push(format!("via {}", self.name(*slot))),
            }
        }
        for &(target, how) in self.by_site.get(&pc).into_iter().flatten() {
            match how {
                Ref::Call | Ref::Jump => {}
                Ref::Read | Ref::Write => parts.push(self.name(target)),
                Ref::Addr => match p.image.string(target, 4).filter(|_| self.strings.contains(&target)) {
                    Some(s) => parts.push(format!("&{} {s:?}", self.name(target))),
                    None => parts.push(format!("&{}", self.name(target))),
                },
            }
        }
        parts.join("; ")
    }

    fn line(&self, out: &mut String, pc: u32, raw: Option<(u32, u8)>, text: &str, comment: &str) {
        let mut l = String::from("  ");
        if self.addresses {
            let _ = write!(l, "{pc:08x}  ");
            match raw {
                Some((w, size)) => {
                    let _ = write!(l, "{:<10}", format!("{w:0width$x}", width = size as usize * 2));
                }
                None => l.push_str(&" ".repeat(10)),
            }
        }
        l.push_str(text);
        if !comment.is_empty() {
            let width: usize = if self.addresses { 54 } else { 34 };
            let _ = write!(l, "{:pad$} # {comment}", "", pad = width.saturating_sub(l.len()));
        }
        out.push_str(l.trim_end());
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Image, find_gp};
    use crate::mips::Mips;
    use crate::{Kind, Symbol};

    #[test]
    fn names_calls_switches_and_strings() {
        let words: [u32; 16] = [
            0x3c04_8001, // 80010000 lui a0, 0x8001
            0x0c00_4006, //          jal 0x80010018
            0x2484_0030, //          addiu a0, a0, 0x30
            0x03e0_0008, //          jr ra
            0,
            0,
            0x03e0_0008, // 80010018 jr ra
            0,
            0,
            0,
            0,
            0,
            u32::from_le_bytes(*b"hi t"), // 80010030
            u32::from_le_bytes(*b"here"),
            0,
            0,
        ];
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let arch = Mips::r3000();
        let img = Image { base: 0x8001_0000, bytes: &bytes };
        let mut syms = Symbols::new();
        syms.mark(Symbol { kind: Kind::Func, note: "prints a line".into(), ..Symbol::new("puts", 0x8001_0018) });
        let p = Program::analyze(&arch, img, 0x8001_0000, find_gp(&arch, img, 0x8001_0000), &syms);
        let l = Listing::new(&p, &syms);
        assert_eq!(l.label(0x8001_0000).as_deref(), Some("func_80010000"));
        assert_eq!(l.label(0x8001_0018).as_deref(), Some("puts"));
        assert_eq!(l.label(0x8001_0030).as_deref(), Some("str_80010030"));
        let text = l.range(0x8001_0000, 0x8001_0040);
        assert!(text.contains("puts:"), "{text}");
        assert!(text.contains("# 1 caller: 80010004; prints a line"), "{text}");
        assert!(text.contains("puts"), "the call names its target: {text}");
        assert!(text.contains("# &str_80010030 \"hi there\""), "{text}");
        assert!(text.contains(".asciz  \"hi there\""), "{text}");
        assert_eq!(l.function(0x8001_0018).unwrap().lines().count(), 3, "label, jr, delay slot");
    }
}
