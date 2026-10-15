//! Whole-program analysis by recursive descent: functions with their real
//! extents, the calls between them, jump tables, the data the code reaches,
//! and the code nothing seeded.
//!
//! Ported from hwtr's `hwtr-psx` analysis, which was proven on Hot Wheels
//! Turbo Racing's executable, and made architecture-neutral: it follows each
//! instruction's [`Flow`] and asks the [`Arch`] what a register holds
//! ([`Arch::evaluate`]), which register is the stack pointer and the global
//! pointer, and what bounds a switch. Nothing in it names a MIPS opcode.
//!
//! The certain functions are walked first - the entry, the symbols marked
//! `func`, and everything reachable from them by direct calls - which fixes
//! where the code ends. Heuristic seeds are believed only below that: a call
//! instruction a linear scan finds, a frame opened right after a return, an
//! unowned stretch of code, a data word pointing at unowned code. Each
//! function is walked along both arms of every branch until a return, a
//! tail jump, or a jump table, whose targets are read and walked too. Then
//! heuristic starts that another function's walk reaches are dropped, and
//! ownership is rebuilt, certain functions first.

use std::collections::{BTreeMap, BTreeSet};

use crate::insn::{Arch, Endian, Flow, Insn, Reg};
use crate::symbols::{Kind, Symbols};

/// An executable image: bytes loaded at `base`.
#[derive(Clone, Copy, Debug)]
pub struct Image<'a> {
    /// Where the first byte loads.
    pub base: u32,
    /// The bytes.
    pub bytes: &'a [u8],
}

impl<'a> Image<'a> {
    /// One past the last address, at most `u32::MAX`: bytes that would lie
    /// past the top of the address space are not part of the image.
    pub fn end(&self) -> u32 {
        u32::try_from(u64::from(self.base) + self.bytes.len() as u64).unwrap_or(u32::MAX)
    }

    /// Whether `addr` is inside.
    pub fn contains(&self, addr: u32) -> bool {
        addr >= self.base && addr < self.end()
    }

    /// `len` bytes from `addr`, if all inside.
    pub fn slice(&self, addr: u32, len: usize) -> Option<&'a [u8]> {
        let at = addr.checked_sub(self.base)? as usize;
        self.bytes.get(at..at.checked_add(len)?)
    }

    /// The 32-bit word at `addr` in byte order `endian`.
    pub fn word(&self, addr: u32, endian: Endian) -> Option<u32> {
        let b: [u8; 4] = self.slice(addr, 4)?.try_into().ok()?;
        Some(match endian {
            Endian::Little => u32::from_le_bytes(b),
            Endian::Big => u32::from_be_bytes(b),
        })
    }

    /// The `size`-byte (1, 2 or 4) unsigned value at `addr` in byte order
    /// `endian`.
    pub fn read(&self, addr: u32, size: u32, endian: Endian) -> Option<u32> {
        let b = self.slice(addr, size as usize)?;
        Some(match endian {
            Endian::Little => b.iter().rev().fold(0, |a, x| (a << 8) | u32::from(*x)),
            Endian::Big => b.iter().fold(0, |a, x| (a << 8) | u32::from(*x)),
        })
    }

    /// The NUL-terminated printable string at `addr` (ASCII 0x20-0x7e, tab,
    /// newline, carriage return), at least `min` characters, at most 4096.
    pub fn string(&self, addr: u32, min: usize) -> Option<String> {
        let at = addr.checked_sub(self.base)? as usize;
        let rest = self.bytes.get(at..)?;
        let end = rest.iter().take(4097).position(|b| *b == 0)?;
        let s = &rest[..end];
        let printable = s.iter().all(|b| (0x20..0x7f).contains(b) || matches!(b, b'\t' | b'\n' | b'\r'));
        (printable && s.len() >= min).then(|| String::from_utf8_lossy(s).into_owned())
    }
}

/// How a function's start was found, most certain first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Found {
    /// The program's entry.
    Entry,
    /// A symbol marked `func`.
    Symbol,
    /// Reached by a direct call from walked code.
    Call,
    /// A call instruction found by a linear scan, which may sit in data.
    Scan,
    /// A frame opened right after a return.
    Prologue,
    /// A data word pointing at unowned code.
    Pointer,
    /// The start of an unowned stretch of code.
    Gap,
}

impl Found {
    /// Whether the start is certain: entry, symbol or a call from walked code.
    pub fn is_certain(self) -> bool {
        matches!(self, Found::Entry | Found::Symbol | Found::Call)
    }
}

/// A function, as the walk found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Func {
    /// Its entry.
    pub start: u32,
    /// One past the highest instruction its walk reached.
    pub end: u32,
    /// How the start was found.
    pub found: Found,
    /// Instructions owned.
    pub insns: u32,
    /// Direct call targets.
    pub calls: BTreeSet<u32>,
    /// Jumps to another function's start.
    pub tail_calls: BTreeSet<u32>,
    /// Calls through a register.
    pub indirect_calls: u32,
    /// Jumps through a register that are neither a return nor a resolved
    /// jump table.
    pub unresolved_jumps: u32,
    /// The walk met an invalid instruction.
    pub hit_invalid: bool,
}

/// A switch's jump table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpTable {
    /// The jump through a register that dispatches through it.
    pub site: u32,
    /// The table's address.
    pub table: u32,
    /// Its targets, in order.
    pub targets: Vec<u32>,
    /// The register holding the case index, when the bound check names it
    /// and nothing changes it before the jump.
    pub index: Option<Reg>,
}

/// How an instruction refers to an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ref {
    /// A direct call.
    Call,
    /// A jump or branch.
    Jump,
    /// A load from it.
    Read,
    /// A store to it.
    Write,
    /// Its address built into a register (`lui`/`addiu`, a `$gp` offset)
    /// and not loaded or stored by the same instruction.
    Addr,
}

/// The analysed program.
pub struct Program<'a, A: Arch + ?Sized> {
    /// The instruction set.
    pub arch: &'a A,
    /// The image.
    pub image: Image<'a>,
    /// The global pointer's value, when known.
    pub gp: Option<u32>,
    /// Functions by start.
    pub funcs: BTreeMap<u32, Func>,
    /// Instruction address to the start of the function that owns it.
    pub owner: BTreeMap<u32, u32>,
    /// Jump tables by the jump that uses them.
    pub jump_tables: BTreeMap<u32, JumpTable>,
    /// Callee to its call sites.
    pub callers: BTreeMap<u32, BTreeSet<u32>>,
    /// Where the code is: the functions' extents merged wherever less than
    /// [`REGION_GAP`] apart, the first from the image's base. Heuristic
    /// seeds and their walks stay inside these and out of data symbols.
    pub code: Vec<(u32, u32)>,
    /// One past the highest owned instruction: where the code ends.
    pub code_end: u32,
    /// Fixed words that hold function addresses: those the code stores a
    /// function's address into (interface tables filled at start-up), and
    /// words in the image that start out holding one and that code calls
    /// through. Slot to the functions it may hold.
    pub slots: BTreeMap<u32, BTreeSet<u32>>,
    /// Calls through a register loaded from a fixed slot: site to slot.
    pub indirect: BTreeMap<u32, u32>,
    /// Every address the code refers to, with the sites and how.
    pub refs: BTreeMap<u32, BTreeSet<(u32, Ref)>>,
    reach: BTreeMap<u32, Vec<u32>>,
    /// The entry and the `func` symbols: a jump to one is a tail call even
    /// before its walk.
    known: BTreeSet<u32>,
    /// Sized `data` symbols and the `@data` ranges: start to end.
    data: BTreeMap<u32, u32>,
    /// Sized `func` symbols: start to end. A heuristic start strictly
    /// inside one is dead code of that function, not another.
    sized: BTreeMap<u32, u32>,
    /// Every instruction start the walks decoded, for stepping backwards
    /// through a variable-length instruction set (empty for a fixed-size
    /// one, which steps by arithmetic).
    starts: BTreeSet<u32>,
}

/// Bytes [`Program::analyze`] leaves between the image's end and the top of
/// the address space: an image loaded closer is cut short.
const HEADROOM: u32 = 16;

/// How far apart two functions can be and still be in one code region.
/// Wider gaps are data: a linker that interleaves code and data (Metrowerks
/// on the PS2 puts both in one section) leaves megabytes between.
pub const REGION_GAP: u32 = 256 << 10;

impl<'a, A: Arch + ?Sized> Program<'a, A> {
    /// Analyses `image` from `entry` (cut short 16 bytes below 4 GiB), with `gp` the global pointer's value
    /// when known ([`find_gp`]), the symbols marked `func` taken as certain
    /// starts.
    pub fn analyze(arch: &'a A, image: Image<'a>, entry: u32, gp: Option<u32>, symbols: &Symbols) -> Program<'a, A> {
        // Room above the image for an instruction, its delay slot and the
        // next, so stepping through it never wraps the address space.
        let room = (u32::MAX - HEADROOM).saturating_sub(image.base) as usize;
        let image = Image { base: image.base, bytes: &image.bytes[..image.bytes.len().min(room)] };
        let mut p = Program {
            arch,
            image,
            gp,
            funcs: BTreeMap::new(),
            owner: BTreeMap::new(),
            jump_tables: BTreeMap::new(),
            callers: BTreeMap::new(),
            code: Vec::new(),
            code_end: image.base,
            slots: BTreeMap::new(),
            indirect: BTreeMap::new(),
            refs: BTreeMap::new(),
            reach: BTreeMap::new(),
            known: BTreeSet::new(),
            data: symbols
                .iter()
                .filter(|s| s.kind == Kind::Data)
                .filter_map(|s| Some((s.addr, s.addr.checked_add(s.size.filter(|n| *n > 0)?)?)))
                .chain(symbols.data_ranges().iter().copied())
                .collect(),
            sized: symbols
                .iter()
                .filter(|s| s.kind == Kind::Func)
                .filter_map(|s| Some((s.addr, s.addr.checked_add(s.size.filter(|n| *n > 0)?)?)))
                .collect(),
            starts: BTreeSet::new(),
        };
        let mut certain: Vec<(u32, Found)> = symbols
            .iter()
            .filter(|s| s.kind == Kind::Func && image.contains(s.addr))
            .map(|s| (s.addr, Found::Symbol))
            .collect();
        // Last, so walked first: the entry is reported as the entry even
        // when a symbol names it.
        certain.push((entry, Found::Entry));
        p.known = certain.iter().map(|&(a, _)| a).collect();
        // Everything reachable by direct calls from the certain starts is
        // certain too; walking it first fixes where the code is, and the
        // heuristic seeds are believed only inside that, which keeps data
        // that happens to decode out of the code.
        while let Some((start, found)) = certain.pop() {
            if p.funcs.contains_key(&start) || !p.image.contains(start) {
                continue;
            }
            p.walk(start, found);
            let f = &p.funcs[&start];
            certain.extend(f.calls.iter().chain(&f.tail_calls).map(|&t| (t, Found::Call)));
        }
        p.regions();
        let mut pending = p.scan_seeds();
        let mut tried: BTreeSet<u32> = BTreeSet::new();
        // Starts skipped because another function owns their code.
        let mut owned: BTreeSet<u32> = BTreeSet::new();
        for _ in 0..32 {
            if pending.is_empty() {
                // The walks have settled: look in what is left.
                p.regions();
                pending = p.gap_seeds().into_iter().filter(|s| !tried.contains(&s.0)).collect();
                if pending.is_empty() {
                    break;
                }
            }
            for (start, found) in std::mem::take(&mut pending) {
                tried.insert(start);
                if p.funcs.contains_key(&start) || p.owner.contains_key(&start) {
                    owned.insert(start);
                    continue;
                }
                if found != Found::Call && (!p.in_code(start) || p.inside_func_symbol(start)) {
                    continue;
                }
                p.walk(start, found);
            }
            // Calls made by anything walked are as certain as the caller.
            let called: BTreeSet<u32> = p
                .funcs
                .values()
                .flat_map(|f| f.calls.iter().chain(&f.tail_calls).copied())
                // A target skipped for being inside code another function
                // owns is not queued again: that would spend every round
                // and starve the gap seeds.
                .filter(|t| p.image.contains(*t) && !p.funcs.contains_key(t) && !owned.contains(t))
                .collect();
            pending = called.into_iter().map(|t| (t, Found::Call)).collect();
        }
        p.regions();
        p.prune();
        p.resolve_slots();
        p.collect_refs();
        p
    }

    /// Merges the functions' extents into [`Program::code`].
    fn regions(&mut self) {
        let mut code: Vec<(u32, u32)> = Vec::new();
        let mut extents: Vec<(u32, u32)> = self.funcs.values().map(|f| (f.start, f.end)).collect();
        extents.sort();
        for (s, e) in extents {
            match code.last_mut() {
                Some(last) if s <= last.1.saturating_add(REGION_GAP) => last.1 = last.1.max(e),
                Some(_) => code.push((s, e)),
                None if s - self.image.base <= REGION_GAP => code.push((self.image.base, e)),
                None => code.push((s, e)),
            }
        }
        self.code_end = code.last().map_or(self.image.base, |r| r.1);
        self.code = code;
    }

    /// Whether `addr` is inside a sized data symbol.
    fn in_data(&self, addr: u32) -> bool {
        self.data.range(..=addr).next_back().is_some_and(|(_, &end)| addr < end)
    }

    /// Whether `addr` is past the start of a sized `func` symbol and before
    /// its end.
    fn inside_func_symbol(&self, addr: u32) -> bool {
        self.sized.range(..addr).next_back().is_some_and(|(_, &end)| addr < end)
    }

    /// Whether `addr` is in a code region and not in a data symbol.
    pub fn in_code(&self, addr: u32) -> bool {
        let i = self.code.partition_point(|r| r.1 <= addr);
        self.code.get(i).is_some_and(|r| addr >= r.0) && !self.in_data(addr)
    }

    fn decode(&self, addr: u32) -> Option<Insn> {
        let at = addr.checked_sub(self.image.base)? as usize;
        let bytes = self.image.bytes.get(at..)?;
        let insn = self.arch.decode(bytes, addr);
        insn.is_valid().then_some(insn)
    }

    /// The decoded instruction at `addr`, if valid.
    pub fn insn(&self, addr: u32) -> Option<Insn> {
        self.decode(addr)
    }

    /// The start of the instruction `k` before `site` in address order: by
    /// arithmetic for a fixed-size instruction set, else through the
    /// instruction starts the walks decoded.
    fn back(&self, site: u32, k: u32) -> Option<u32> {
        match self.arch.fixed_size() {
            Some(n) => site.checked_sub(k.checked_mul(n)?),
            None => (0..k).try_fold(site, |at, _| self.prev(at)),
        }
    }

    /// The start of the instruction right before `at`: by arithmetic for a
    /// fixed-size set; else a decoded start whose instruction ends exactly
    /// at `at`, so a straight line is never followed across a gap.
    fn prev(&self, at: u32) -> Option<u32> {
        match self.arch.fixed_size() {
            Some(n) => at.checked_sub(n),
            None => {
                let &p = self.starts.range(..at).next_back()?;
                (p + self.decode(p)?.size as u32 == at).then_some(p)
            }
        }
    }

    /// Whether the instruction ending right before `pc` is owned.
    fn owned_before(&self, pc: u32) -> bool {
        match self.arch.fixed_size() {
            Some(n) => pc.checked_sub(n).is_some_and(|b| self.owner.contains_key(&b)),
            None => self
                .owner
                .range(..pc)
                .next_back()
                .is_some_and(|(&a, _)| self.decode(a).is_some_and(|i| a + i.size as u32 == pc)),
        }
    }

    /// Whether `insn` lowers the stack pointer by a constant: a frame
    /// opening. Asked of the architecture through [`Arch::evaluate`] (the
    /// stack pointer goes down), else read from the immediate.
    fn opens_frame(&self, insn: &Insn) -> bool {
        const SP: u32 = 0x1000_0000;
        let Some(sp) = self.arch.stack_pointer() else { return false };
        if insn.writes != [sp] || !insn.reads.contains(&sp) || insn.mem.is_some() {
            return false;
        }
        match self.arch.evaluate(insn, &|r| (r == sp).then_some(SP)) {
            Some(v) => v < SP && SP - v < 0x10_0000,
            None => insn.imm().is_some_and(|v| v < 0),
        }
    }

    /// Seeds a linear scan finds: call targets, frames opened right after a
    /// return, and data words pointing at a frame opening.
    fn scan_seeds(&self) -> Vec<(u32, Found)> {
        let mut out: BTreeMap<u32, Found> = BTreeMap::new();
        let step = self.arch.align();
        let mut pc = self.image.base;
        while pc < self.image.end() {
            if !self.in_code(pc) {
                pc += step;
                continue;
            }
            if let Some(insn) = self.decode(pc) {
                if let Flow::Call { target: Some(t) } = insn.flow
                    && self.image.contains(t)
                {
                    out.entry(t).or_insert(Found::Scan);
                }
                if insn.flow == Flow::Return {
                    let next = pc + insn.size as u32 * if insn.delay_slot { 2 } else { 1 };
                    if self.decode(next).is_some_and(|i| self.opens_frame(&i)) {
                        out.entry(next).or_insert(Found::Prologue);
                    }
                }
            }
            pc += step;
        }
        let endian = self.arch.endian();
        let psize = self.arch.pointer_size();
        let mut pc = self.image.base;
        while self.image.end().saturating_sub(pc) >= psize {
            let w = self.image.read(pc, psize, endian).unwrap();
            if w.is_multiple_of(step) && self.image.contains(w) && self.decode(w).is_some_and(|i| self.opens_frame(&i))
            {
                out.entry(w).or_insert(Found::Pointer);
            }
            pc += psize;
        }
        // Scanned calls first: they are the strongest of the weak seeds.
        let mut v: Vec<(u32, Found)> = out.into_iter().collect();
        v.sort_by_key(|&(a, f)| (f != Found::Scan, a));
        v
    }

    /// New seeds once the walks settle: the start of each unowned stretch
    /// of code inside the code regions, and data words that point at
    /// unowned code.
    fn gap_seeds(&self) -> Vec<(u32, Found)> {
        let mut out = Vec::new();
        let step = self.arch.align();
        let endian = self.arch.endian();
        for &(start, end) in &self.code {
            let mut pc = start;
            while pc < end {
                if self.owner.contains_key(&pc) || self.image.word(pc, endian) == Some(0) || !self.in_code(pc) {
                    pc += step;
                    continue;
                }
                if let Some(i) = self.decode(pc)
                    && (self.opens_frame(&i) || self.owned_before(pc) || pc == self.image.base)
                {
                    out.push((pc, Found::Gap));
                }
                while pc < end && !self.owner.contains_key(&pc) {
                    pc += step;
                }
            }
        }
        let psize = self.arch.pointer_size();
        let mut pc = self.image.base;
        while self.image.end().saturating_sub(pc) >= psize {
            if self.in_code(pc) {
                pc += psize;
                continue;
            }
            let w = self.image.read(pc, psize, endian).unwrap();
            if w.is_multiple_of(step)
                && self.in_code(w)
                && !self.owner.contains_key(&w)
                && self.image.word(w, endian) != Some(0)
                && self.decode(w).is_some()
            {
                out.push((w, Found::Pointer));
            }
            pc += psize;
        }
        out.sort();
        out.dedup_by_key(|s| s.0);
        out
    }

    fn walk(&mut self, start: u32, found: Found) {
        let mut f = Func {
            start,
            end: start,
            found,
            insns: 0,
            calls: BTreeSet::new(),
            tail_calls: BTreeSet::new(),
            indirect_calls: 0,
            unresolved_jumps: 0,
            hit_invalid: false,
        };
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        let mut work = vec![start];
        while let Some(mut pc) = work.pop() {
            let mut sequential = false;
            loop {
                if seen.contains(&pc) || !self.image.contains(pc) {
                    break;
                }
                // Heuristic functions stay inside the code the calls fixed.
                if !found.is_certain() && !self.in_code(pc) {
                    f.hit_invalid = true;
                    break;
                }
                // Fell into the next function: only a certain start stops
                // the walk; a heuristic one may be a false start inside this
                // function, which prune() removes.
                if sequential && self.funcs.get(&pc).is_some_and(|g| g.found.is_certain()) {
                    break;
                }
                sequential = true;
                let Some(insn) = self.decode(pc) else {
                    f.hit_invalid = true;
                    break;
                };
                seen.insert(pc);
                if self.arch.fixed_size().is_none() {
                    self.starts.insert(pc);
                }
                let size = insn.size as u32;
                let delay = pc + size;
                match insn.flow {
                    Flow::Call { target } => {
                        match target {
                            Some(t) => {
                                f.calls.insert(t);
                                self.callers.entry(t).or_default().insert(pc);
                            }
                            // Through a register that holds a constant (a
                            // BIOS vector, `li t2, 0xa0; jalr t2`): direct.
                            None => match insn.reads.first().and_then(|r| self.value_before(pc, *r)) {
                                Some(t) => {
                                    f.calls.insert(t);
                                    self.callers.entry(t).or_default().insert(pc);
                                }
                                None => f.indirect_calls += 1,
                            },
                        }
                        pc += size;
                    }
                    Flow::Jump { target: Some(t) } => {
                        if insn.delay_slot {
                            seen.insert(delay);
                        }
                        if t != start && (self.funcs.contains_key(&t) || self.known.contains(&t) || t < start) {
                            f.tail_calls.insert(t);
                            self.callers.entry(t).or_default().insert(pc);
                        } else {
                            work.push(t);
                        }
                        break;
                    }
                    Flow::Jump { target: None } => {
                        if insn.delay_slot {
                            seen.insert(delay);
                        }
                        let reg = insn.reads.first().copied();
                        match reg.and_then(|r| self.jump_table(pc, r)) {
                            Some(jt) => {
                                work.extend(jt.targets.iter().copied());
                                self.jump_tables.insert(pc, jt);
                            }
                            // Through a register that holds a constant: a tail
                            // call to it (the PS1's BIOS stubs, `li t2, 0xa0;
                            // jr t2; li t1, N`).
                            None => match reg.and_then(|r| self.value_before(pc, r)) {
                                Some(t) => {
                                    f.tail_calls.insert(t);
                                    self.callers.entry(t).or_default().insert(pc);
                                }
                                None => f.unresolved_jumps += 1,
                            },
                        }
                        break;
                    }
                    Flow::Return | Flow::Halt | Flow::Invalid => {
                        if insn.delay_slot {
                            seen.insert(delay);
                        }
                        break;
                    }
                    Flow::Branch { target, likely } => {
                        work.push(target);
                        if likely && insn.delay_slot {
                            // Not taken, the delay slot is skipped.
                            seen.insert(delay);
                            pc = delay + size;
                        } else {
                            pc += size;
                        }
                    }
                    Flow::Next | Flow::Trap => pc += size,
                }
            }
        }
        f.insns = seen.len() as u32;
        f.end =
            seen.iter().next_back().map_or(start, |&a| a + self.decode(a).map_or(self.arch.align(), |i| i.size as u32));
        for &a in &seen {
            self.owner.entry(a).or_insert(start);
        }
        self.reach.insert(start, seen.into_iter().collect());
        self.funcs.insert(start, f);
    }

    /// Drops heuristic starts another function's walk reaches, then rebuilds
    /// instruction ownership, certain functions first.
    fn prune(&mut self) {
        let mut inside = BTreeSet::new();
        for (&start, addrs) in &self.reach {
            for a in addrs {
                if *a != start && self.funcs.get(a).is_some_and(|f| !f.found.is_certain()) {
                    inside.insert(*a);
                }
            }
        }
        for a in &inside {
            self.funcs.remove(a);
            self.reach.remove(a);
        }
        self.owner.clear();
        let mut order: Vec<u32> = self.funcs.keys().copied().collect();
        order.sort_by_key(|a| (!self.funcs[a].found.is_certain(), *a));
        for start in order {
            for &a in &self.reach[&start] {
                self.owner.entry(a).or_insert(start);
            }
        }
    }

    /// The constant `reg` holds just before the instruction at `site`, when
    /// the instructions before it build one (within 160 instructions back,
    /// six steps deep): through [`Arch::evaluate`], `$gp` as [`Program::gp`].
    pub fn value_before(&self, site: u32, reg: Reg) -> Option<u32> {
        self.const_before(site, 0, reg, 0)
    }

    fn const_before(&self, site: u32, mut k: u32, reg: Reg, depth: u32) -> Option<u32> {
        if Some(reg) == self.arch.global_pointer()
            && let Some(gp) = self.gp
        {
            return Some(gp);
        }
        let mut later = self.back(site, k).unwrap_or(site);
        let mut at = later;
        while k < 160 && depth < 6 {
            k += 1;
            at = self.prev(at)?;
            // Not across the start of the function: before it is another.
            if later < site && self.funcs.contains_key(&later) {
                return None;
            }
            later = at;
            let insn = self.decode(at)?;
            if !insn.writes.contains(&reg) {
                continue;
            }
            return self.arch.evaluate(&insn, &|r| self.const_before(at, 0, r, depth + 1));
        }
        None
    }

    /// The constant in `reg` on entry to the callee of the call at `site`,
    /// its delay slot included.
    pub fn arg_at_call(&self, site: u32, reg: Reg) -> Option<u32> {
        match self.decode(site) {
            Some(i) if i.delay_slot => {
                let delay = site + i.size as u32;
                self.value_before(delay + self.decode(delay).map_or(i.size, |d| d.size) as u32, reg)
            }
            _ => self.value_before(site, reg),
        }
    }

    /// Whether `insn` adds its two register inputs: asked of the
    /// architecture by evaluating it on two distinct values.
    fn adds(&self, insn: &Insn) -> bool {
        let [a, b] = insn.reads[..] else { return false };
        insn.mem.is_none()
            && insn.imm().is_none()
            && self.arch.evaluate(insn, &|r| {
                Some(if r == a {
                    0x100
                } else if r == b {
                    0x20
                } else {
                    0
                })
            }) == Some(0x120)
    }

    /// A switch dispatch ending at a jump through `reg` at `site`: a
    /// pointer-sized load into `reg` from a table whose address the code
    /// builds (possibly plus a scaled index added in), and a bound that
    /// limits the entries.
    fn jump_table(&self, site: u32, reg: Reg) -> Option<JumpTable> {
        let step = self.arch.align();
        let psize = self.arch.pointer_size();
        let back = |k: u32| self.decode(self.back(site, k)?);
        let (k, load) = (1..=6).find_map(|k| {
            let i = back(k)?;
            (i.writes.contains(&reg) && i.mem.is_some_and(|m| !m.store && u32::from(m.size) == psize)).then_some((k, i))
        })?;
        let m = load.mem.unwrap();
        let at = load.addr;
        // The table's base: when an add builds the load's base register
        // (`addu at, at, v0` after `lui at, hi`), one of its inputs is the
        // table and the other the scaled index; either may also evaluate to
        // something stale from another path, so every candidate is tried,
        // the add's inputs first, and the first that reads as a table wins.
        let mut bases = Vec::new();
        let writer = (k + 1..k + 12).find_map(|ka| back(ka).filter(|i| i.writes.contains(&m.base)));
        if let Some(add) = writer
            && self.adds(&add)
        {
            let add_at = add.addr;
            bases.extend(add.reads.iter().filter_map(|r| self.const_before(add_at, 0, *r, 0)));
        }
        bases.extend(self.const_before(at, 0, m.base, 0));
        let check = (k + 1..k + 24).find_map(|kb| back(kb).and_then(|i| Some((self.arch.switch_bound(&i)?, i))));
        let bound = check.as_ref().map(|(n, _)| *n);
        // The index: the register the bound check compares, unless an
        // instruction between the check and the jump writes it.
        let index = check.and_then(|(_, i)| {
            let Some(crate::ir::Effect::Set(_, crate::ir::Expr::Bin(_, x, _))) =
                self.arch.lift(&i)?.effects.first().cloned()
            else {
                return None;
            };
            let crate::ir::Expr::Reg(x) = x.as_ref() else { return None };
            let between = (1..).map_while(|k| self.back(site, k).filter(|a| *a > i.addr));
            let changed =
                between.collect::<Vec<_>>().iter().any(|a| self.decode(*a).is_some_and(|d| d.writes.contains(x)));
            (!changed).then_some(*x)
        });
        let limit = bound.unwrap_or(256).min(1024);
        let endian = self.arch.endian();
        bases.into_iter().find_map(|base| {
            let table = base.wrapping_add(m.offset as u32);
            let mut targets = Vec::new();
            // Entries are code addresses: not in data, and not in the table
            // itself (which may come before its code, as rodata between
            // hand-written assembly and the compiler's output does, or after).
            let table_end = table.wrapping_add(limit * psize);
            for n in 0..limit {
                let Some(t) = self.image.read(table.wrapping_add(n * psize), psize, endian) else { break };
                if !t.is_multiple_of(step)
                    || !self.image.contains(t)
                    || self.in_data(t)
                    || (table..table_end).contains(&t)
                {
                    break;
                }
                targets.push(t);
            }
            (!targets.is_empty()).then_some(JumpTable { site, table, targets, index })
        })
    }

    /// The fixed address a register was loaded from shortly before `site`:
    /// a pointer-sized load from a constant base or the global pointer.
    fn loaded_from(&self, site: u32, reg: Reg) -> Option<u32> {
        let psize = self.arch.pointer_size();
        for k in 1..=16 {
            let at = self.back(site, k)?;
            let i = self.decode(at)?;
            if !i.writes.contains(&reg) {
                continue;
            }
            let m = i.mem.filter(|m| !m.store && u32::from(m.size) == psize)?;
            return Some(self.const_before(at, 0, m.base, 0)?.wrapping_add(m.offset as u32));
        }
        None
    }

    /// Finds the interface tables - stores of a function's address to a
    /// fixed word, and function tables in the image - and names the callee
    /// of each call through a register loaded from one, when the slot only
    /// ever holds one function.
    fn resolve_slots(&mut self) {
        let mut slots: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        let psize = self.arch.pointer_size();
        for &pc in self.owner.keys() {
            let Some(i) = self.decode(pc) else { continue };
            let Some(m) = i.mem.filter(|m| m.store && u32::from(m.size) == psize) else { continue };
            let Some(&value_reg) = i.reads.iter().find(|r| **r != m.base) else { continue };
            let Some(value) = self.value_before(pc, value_reg) else { continue };
            if !self.funcs.contains_key(&value) {
                continue;
            }
            if let Some(base) = self.value_before(pc, m.base) {
                slots.entry(base.wrapping_add(m.offset as u32)).or_default().insert(value);
            }
        }
        let mut indirect = BTreeMap::new();
        for addrs in self.reach.values() {
            for &pc in addrs {
                let Some(i) = self.decode(pc) else { continue };
                if i.flow != (Flow::Call { target: None }) {
                    continue;
                }
                let Some(&reg) = i.reads.first() else { continue };
                // A misaligned word or one below the image is a stale value
                // from another path, not a slot.
                if let Some(slot) = self.loaded_from(pc, reg)
                    && slot.is_multiple_of(psize)
                    && slot >= self.image.base
                {
                    indirect.insert(pc, slot);
                }
            }
        }
        // A slot in the image holds its initial word until a store says
        // otherwise: a function table the linker filled.
        let endian = self.arch.endian();
        for &slot in indirect.values() {
            if let Some(w) = self.image.read(slot, psize, endian)
                && self.funcs.contains_key(&w)
            {
                slots.entry(slot).or_default().insert(w);
            }
        }
        for (&site, slot) in &indirect {
            if let Some(targets) = slots.get(slot)
                && targets.len() == 1
            {
                let t = *targets.iter().next().unwrap();
                self.callers.entry(t).or_default().insert(site);
                if let Some(owner) = self.owner.get(&site).copied()
                    && let Some(f) = self.funcs.get_mut(&owner)
                {
                    f.calls.insert(t);
                }
            }
        }
        self.slots = slots;
        self.indirect = indirect;
    }

    /// Every address each owned instruction refers to: calls and jumps, the
    /// memory loads and stores through a known base, and addresses built in
    /// a register that land in the image.
    fn collect_refs(&mut self) {
        let mut refs: BTreeMap<u32, BTreeSet<(u32, Ref)>> = BTreeMap::new();
        for &pc in self.owner.keys() {
            let Some(i) = self.decode(pc) else { continue };
            match i.flow {
                Flow::Call { target: Some(t) } => {
                    refs.entry(t).or_default().insert((pc, Ref::Call));
                }
                Flow::Jump { target: Some(t) } | Flow::Branch { target: t, .. } => {
                    refs.entry(t).or_default().insert((pc, Ref::Jump));
                }
                _ => {}
            }
            if let Some(m) = i.mem
                && let Some(base) = self.value_before(pc, m.base)
            {
                let kind = if m.store { Ref::Write } else { Ref::Read };
                refs.entry(base.wrapping_add(m.offset as u32)).or_default().insert((pc, kind));
            } else if i.mem.is_none()
                && i.flow == Flow::Next
                && i.imm().is_some()
                && let [w] = i.writes[..]
                && let Some(v) = self.value_before(pc + i.size as u32, w)
                && self.image.contains(v)
                && i.reads.iter().any(|r| self.value_before(pc, *r) != Some(0))
            {
                // A constant built into a register from another that points
                // into the image (the second half of `lui`/`addiu`, a `$gp`
                // offset): a string, a table, a callback. The first half alone
                // is not the address.
                refs.entry(v).or_default().insert((pc, Ref::Addr));
            }
        }
        self.refs = refs;
    }

    /// The instructions the function starting at `start` owns, in address
    /// order (empty when no function starts there).
    pub fn insns_of(&self, start: u32) -> Vec<u32> {
        self.reach
            .get(&start)
            .map(|v| v.iter().copied().filter(|a| self.owner.get(a) == Some(&start)).collect())
            .unwrap_or_default()
    }

    /// The function that owns `addr`.
    pub fn func_of(&self, addr: u32) -> Option<&Func> {
        self.owner.get(&addr).and_then(|s| self.funcs.get(s))
    }

    /// Words in the code regions, outside data symbols, that no walk
    /// reached and are not zero.
    pub fn unowned_words(&self) -> u32 {
        let endian = self.arch.endian();
        let step = self.arch.align();
        let mut n = 0;
        for &(start, end) in &self.code {
            let mut pc = start;
            while pc < end {
                if !self.owner.contains_key(&pc) && self.image.word(pc, endian) != Some(0) && !self.in_data(pc) {
                    n += 1;
                }
                pc += step;
            }
        }
        n
    }

    /// The strings the code takes the address of (at least `min` printable
    /// characters, never loaded or stored as a word), by address, with the
    /// sites that refer to each.
    pub fn strings(&self, min: usize) -> BTreeMap<u32, (String, BTreeSet<u32>)> {
        self.refs
            .iter()
            .filter(|(a, sites)| {
                !self.owner.contains_key(a)
                    && sites.iter().any(|(_, how)| *how == Ref::Addr)
                    && !sites.iter().any(|(_, how)| matches!(how, Ref::Read | Ref::Write))
            })
            .filter_map(|(&a, sites)| {
                let s = self.image.string(a, min)?;
                Some((a, (s, sites.iter().map(|(site, _)| *site).collect())))
            })
            .collect()
    }
}

/// The global pointer's value as the start-up code leaves it: the last
/// write to it in the straight line from `entry` to the first call or jump
/// (within 256 instructions), evaluated back through whatever registers built
/// it (MIPS crt0: `lui gp, hi; addiu gp, gp, lo`, or `move gp, a0` after
/// building `a0`). None for an architecture with no global pointer, or when
/// that code does not set it to a constant.
pub fn find_gp<A: Arch + ?Sized>(arch: &A, image: Image<'_>, entry: u32) -> Option<u32> {
    let gp = arch.global_pointer()?;
    let decode = |at: u32| image.slice(at, image.end().saturating_sub(at) as usize).map(|b| arch.decode(b, at));
    // The straight line, in order.
    let mut line: Vec<Insn> = Vec::new();
    let mut at = entry;
    for _ in 0..256 {
        let Some(insn) = decode(at).filter(|i| i.is_valid()) else { break };
        let stop = matches!(insn.flow, Flow::Call { .. } | Flow::Jump { .. } | Flow::Return | Flow::Invalid);
        let Some(next) = at.checked_add(insn.size.max(1) as u32) else { break };
        line.push(insn);
        if stop {
            break;
        }
        at = next;
    }
    fn value<A: Arch + ?Sized>(arch: &A, line: &[Insn], k: usize, reg: Reg, depth: u32) -> Option<u32> {
        let insn = &line[k];
        if !insn.writes.contains(&reg) || depth > 8 {
            return None;
        }
        arch.evaluate(insn, &|r| {
            let j = line[..k].iter().rposition(|i| i.writes.contains(&r))?;
            value(arch, line, j, r, depth + 1)
        })
    }
    let last = line.iter().rposition(|i| i.writes.contains(&gp))?;
    value(arch, &line, last, gp, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mips::Mips;

    fn image(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// hwtr's walks_calls_and_switches: main calls f; f dispatches a two-case
    /// switch through a jump table.
    #[test]
    fn walks_calls_and_switches() {
        let words: [u32; 26] = [
            0x0c00_4008, // 80010000 main: jal f
            0,           //          nop
            0x03e0_0008, //          jr ra
            0,
            0,
            0,
            0,
            0,
            0x2c82_0002, // 80010020 f: sltiu v0, a0, 2
            0x3c01_8001, //          lui at, 0x8001
            0x0004_1880, //          sll v1, a0, 2
            0x0023_0821, //          addu at, at, v1
            0x8c23_0060, //          lw v1, 0x60(at)
            0,
            0x0060_0008, // 80010038 jr v1
            0,
            0x03e0_0008, // 80010040 case 0: jr ra
            0x2402_0001, //          li v0, 1
            0x03e0_0008, // 80010048 case 1: jr ra
            0x2402_0002, //          li v0, 2
            0,
            0,
            0,
            0,
            0x8001_0040, // 80010060 the table
            0x8001_0048,
        ];
        let bytes = image(&words);
        let arch = Mips::r3000();
        let img = Image { base: 0x8001_0000, bytes: &bytes };
        let p = Program::analyze(&arch, img, 0x8001_0000, None, &Symbols::new());
        assert_eq!(p.funcs.keys().copied().collect::<Vec<_>>(), [0x8001_0000, 0x8001_0020]);
        let jt = &p.jump_tables[&0x8001_0038];
        assert_eq!(jt.table, 0x8001_0060);
        assert_eq!(jt.targets, [0x8001_0040, 0x8001_0048]);
        assert_eq!(p.funcs[&0x8001_0020].end, 0x8001_0050);
        assert_eq!(p.code_end, 0x8001_0050);
        assert_eq!(p.callers[&0x8001_0020].iter().copied().collect::<Vec<_>>(), [0x8001_0000]);
        assert_eq!(p.funcs[&0x8001_0020].found, Found::Call);
        assert_eq!(p.unowned_words(), 0);
    }

    /// `$gp` from crt0, a `$gp`-relative load, a string built by
    /// `lui`/`addiu` and passed as an argument, a tail call to a function
    /// the symbols name (walked after the caller), and an interface slot
    /// naming the target of a `jalr`.
    #[test]
    fn data_refs_strings_tail_calls_and_slots() {
        let words: [u32; 32] = [
            0x3c1c_8002, // 80010000 entry: lui gp, 0x8002
            0x279c_8000, //          addiu gp, gp, -0x8000   (gp = 0x80018000)
            0x8f82_0010, //          lw v0, 0x10(gp)         (reads 0x80018010)
            0x3c04_8001, //          lui a0, 0x8001
            0x0c00_4010, //          jal g                   (0x80010040)
            0x2484_0070, //          addiu a0, a0, 0x70      (the string, in the delay slot)
            0x3c01_8001, //          lui at, 0x8001
            0x2402_0000, //          li v0, 0 (placeholder overwritten below)
            0x3c03_8001, //          lui v1, 0x8001
            0x2463_0060, //          addiu v1, v1, 0x60      (h)
            0xac23_007c, //          sw v1, 0x7c(at)         (slot 0x8001007c = h)
            0x8c39_007c, //          lw t9, 0x7c(at)
            0x0320_f809, //          jalr t9
            0,
            0x0800_4018, //          j h                     (a tail call into h)
            0,
            0x03e0_0008, // 80010040 g: jr ra
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0x03e0_0008, // 80010060 h: jr ra
            0,
            0,
            0,
            u32::from_le_bytes(*b"hell"), // 80010070 "hello"
            u32::from_le_bytes([b'o', 0, 0, 0]),
            0,
            0,
        ];
        let bytes = image(&words);
        let arch = Mips::r3000();
        let img = Image { base: 0x8001_0000, bytes: &bytes };
        let gp = find_gp(&arch, img, 0x8001_0000);
        assert_eq!(gp, Some(0x8001_8000));
        let mut syms = Symbols::new();
        syms.mark(crate::Symbol { kind: Kind::Func, ..crate::Symbol::new("h", 0x8001_0060) });
        let p = Program::analyze(&arch, img, 0x8001_0000, gp, &syms);
        assert!(p.funcs.contains_key(&0x8001_0040) && p.funcs.contains_key(&0x8001_0060));
        assert_eq!(p.arg_at_call(0x8001_0010, Reg::gpr(4)), Some(0x8001_0070), "a0 includes the delay slot");
        assert!(p.refs[&0x8001_8010].contains(&(0x8001_0008, Ref::Read)), "the gp-relative load");
        assert!(p.refs[&0x8001_007c].contains(&(0x8001_0028, Ref::Write)));
        let strings = p.strings(4);
        assert_eq!(strings[&0x8001_0070].0, "hello");
        assert_eq!(p.slots[&0x8001_007c].iter().copied().collect::<Vec<_>>(), [0x8001_0060]);
        assert_eq!(p.indirect[&0x8001_0030], 0x8001_007c);
        assert!(p.callers[&0x8001_0060].contains(&0x8001_0030), "the jalr resolved through the slot");
        assert!(p.funcs[&0x8001_0000].tail_calls.contains(&0x8001_0060));
    }

    /// What the PS2's Metrowerks layout taught: a call-shaped word in data
    /// far from the code seeds nothing, and dead code after a branch inside
    /// a sized `func` symbol is that function's, not a new one.
    #[test]
    fn heuristics_stay_in_code_regions_and_out_of_sized_functions() {
        let base = 0x0010_0000u32;
        let mut words = vec![0u32; 0x5_0000 / 4];
        words[0] = 0x1000_0003; // b base+0x10
        words[2] = 0x2402_0001; // li v0, 1 (dead)
        words[3] = 0x2403_0002; // li v1, 2 (dead)
        words[4] = 0x03e0_0008; // jr ra
        words[0x10] = 0x03e0_0008; // base+0x40: jr ra, reached by nothing real
        words[0x4_8000 / 4] = 0x0c00_0000 | ((base + 0x40) >> 2); // data: "jal base+0x40"
        let bytes = image(&words);
        let arch = Mips::r5900();
        let img = Image { base, bytes: &bytes };
        let mut syms = Symbols::new();
        syms.mark(crate::Symbol { kind: Kind::Func, size: Some(0x18), ..crate::Symbol::new("main", base) });
        let p = Program::analyze(&arch, img, base, None, &syms);
        assert_eq!(p.funcs.keys().copied().collect::<Vec<_>>(), [base]);
        assert_eq!(p.code, [(base, base + 0x18)]);
        assert!(p.in_code(base + 0x8) && !p.in_code(base + 0x4_8000));

        // Without the size, the dead code is a gap seed, as hwtr finds it.
        let mut bare = Symbols::new();
        bare.mark(crate::Symbol { kind: Kind::Func, ..crate::Symbol::new("main", base) });
        let p = Program::analyze(&arch, img, base, None, &bare);
        assert_eq!(p.funcs.get(&(base + 8)).map(|f| f.found), Some(Found::Gap));
        assert!(!p.funcs.contains_key(&(base + 0x40)));
    }

    /// MIPS with its fixed size hidden, as a variable-length backend (x86)
    /// would be: the analysis must step back through decoded instructions
    /// and get the same answers.
    struct Unsized(Mips);

    impl Arch for Unsized {
        fn name(&self) -> &'static str {
            self.0.name()
        }
        fn endian(&self) -> Endian {
            self.0.endian()
        }
        fn align(&self) -> u32 {
            self.0.align()
        }
        fn decode(&self, bytes: &[u8], addr: u32) -> Insn {
            self.0.decode(bytes, addr)
        }
        fn reg_name(&self, reg: Reg) -> String {
            self.0.reg_name(reg)
        }
        fn render(&self, insn: &Insn, label: &dyn Fn(u32) -> Option<String>) -> String {
            self.0.render(insn, label)
        }
        fn lift(&self, insn: &Insn) -> Option<crate::ir::Lifted> {
            self.0.lift(insn)
        }
        fn call_conv(&self) -> Option<crate::insn::CallConv> {
            self.0.call_conv()
        }
    }

    #[test]
    fn a_variable_length_backend_gets_the_same_answers() {
        let programs: [(&[u32], u32); 2] = [
            (
                &[
                    0x0c00_4008,
                    0,
                    0x03e0_0008,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0x2c82_0002,
                    0x3c01_8001,
                    0x0004_1880,
                    0x0023_0821,
                    0x8c23_0060,
                    0,
                    0x0060_0008,
                    0,
                    0x03e0_0008,
                    0x2402_0001,
                    0x03e0_0008,
                    0x2402_0002,
                    0,
                    0,
                    0,
                    0,
                    0x8001_0040,
                    0x8001_0048,
                ],
                0x8001_0000,
            ),
            (
                &[
                    0x3c1c_8002,
                    0x279c_8000,
                    0x8f82_0010,
                    0x3c04_8001,
                    0x0c00_4010,
                    0x2484_0070,
                    0x3c01_8001,
                    0,
                    0x3c03_8001,
                    0x2463_0060,
                    0xac23_007c,
                    0x8c39_007c,
                    0x0320_f809,
                    0,
                    0x0800_4018,
                    0,
                    0x03e0_0008,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0x03e0_0008,
                    0,
                    0,
                    0,
                ],
                0x8001_0000,
            ),
        ];
        for (words, base) in programs {
            let bytes = image(words);
            let img = Image { base, bytes: &bytes };
            let fixed = Mips::r3000();
            let var = Unsized(Mips::r3000());
            assert_eq!(find_gp(&fixed, img, base), find_gp(&var, img, base));
            let gp = find_gp(&fixed, img, base);
            let a = Program::analyze(&fixed, img, base, gp, &Symbols::new());
            let b = Program::analyze(&var, img, base, gp, &Symbols::new());
            assert_eq!(a.funcs, b.funcs);
            assert_eq!(a.jump_tables, b.jump_tables);
            assert_eq!(a.refs, b.refs);
            assert_eq!(a.slots, b.slots);
            assert!(b.starts.len() > 4, "the variable-length path was taken");
        }
    }

    #[test]
    fn symbols_marked_func_are_certain_starts() {
        // Two functions nothing calls; the symbol makes the second certain.
        let words = [0x03e0_0008u32, 0, 0x03e0_0008, 0];
        let bytes = image(&words);
        let arch = Mips::r3000();
        let img = Image { base: 0x1000, bytes: &bytes };
        let mut s = Symbols::new();
        s.mark(crate::Symbol { kind: Kind::Func, ..crate::Symbol::new("lonely", 0x1008) });
        let p = Program::analyze(&arch, img, 0x1000, None, &s);
        assert_eq!(p.funcs[&0x1008].found, Found::Symbol);
    }
}
