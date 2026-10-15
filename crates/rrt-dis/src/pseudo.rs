//! A small decompiler: an analysed function as C-like pseudo-code, one basic
//! block at a time. Ported from rugrats' `decomp.rs` onto the lifting IR
//! ([`crate::ir`]), so it works for any backend that lifts its instructions
//! ([`Arch::lift`]) and describes its calling convention
//! ([`Arch::call_conv`]).
//!
//! Each block is executed symbolically: registers hold expression trees, so a
//! run of instructions folds into the expressions it computes. Stores, calls
//! and branches become statements. A register liveness pass decides what is
//! written: a value is shown only where it is used later, and named when it
//! is used more than once. Control flow stays as labels and gotos.
//!
//! Folded idioms: addresses built over several instructions (MIPS
//! `lui`/`addiu`) into names or strings; the stack frame, so stack slots read
//! as `local_NN` and the prologue's saves of callee-saved registers and the
//! epilogue's restores disappear; global-pointer slots into globals; GCC's
//! 64-bit fixed-point multiply into `fx(a * b >> n)`; and GCC's
//! divide-by-zero guards around an aborting trap (dropped). A call shows as
//! many arguments as the callee reads, found by the same liveness pass run
//! on the callee.
//!
//! An instruction the backend does not lift prints as a comment, and the
//! registers it writes read as themselves afterwards.
//!
//! It is a reading aid: a port's code is written by hand from it and checked
//! against the original.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::analysis::Program;
use crate::insn::{Arch, CallConv, Flow, Insn, Reg, RegFile};
use crate::ir::{BinOp, Effect, Expr, Lifted, UnOp};
use crate::listing::Listing;

/// Pseudo-code for the functions of one analysed program.
pub struct Pseudo<'p, 'a, A: Arch + ?Sized> {
    p: &'p Program<'a, A>,
    names: Box<dyn Fn(u32) -> Option<String> + 'p>,
    cc: CallConv,
    arity: RefCell<BTreeMap<u32, usize>>,
}

impl<'p, 'a, A: Arch + ?Sized> Pseudo<'p, 'a, A> {
    /// Pseudo-code naming addresses as `listing` does, and showing the
    /// strings the code refers to as literals.
    pub fn new(listing: &'p Listing<'p, 'a, A>) -> Self {
        let p = listing.program();
        let strings = p.strings(3);
        Pseudo::with_names(p, move |a| {
            if let Some((s, _)) = strings.get(&a) {
                let s: String = s.chars().take(40).collect();
                return Some(format!("{s:?}"));
            }
            listing.label(a)
        })
    }

    /// Pseudo-code naming addresses with `names` (None prints the address).
    pub fn with_names(p: &'p Program<'a, A>, names: impl Fn(u32) -> Option<String> + 'p) -> Self {
        Pseudo { p, names: Box::new(names), cc: p.arch.call_conv().unwrap_or_default(), arity: RefCell::default() }
    }

    /// How many argument registers the function at `addr` reads, from
    /// liveness at its entry; all of them for an address that is no function.
    pub fn arity(&self, addr: u32) -> usize {
        if let Some(&n) = self.arity.borrow().get(&addr) {
            return n;
        }
        let all = self.cc.args.len();
        if !self.p.funcs.contains_key(&addr) {
            return all;
        }
        // Recursion reads as all of them until it is known.
        self.arity.borrow_mut().insert(addr, all);
        // A function that owns no instructions (all of them another's) has
        // no blocks: it reads as taking all of them.
        let n = Cfg::new(self, addr)
            .and_then(|f| f.live_in.first().cloned())
            .map_or(all, |entry| self.cc.args.iter().rposition(|r| entry.contains(r)).map_or(0, |k| k + 1));
        self.arity.borrow_mut().insert(addr, n);
        n
    }

    /// Pseudo-code for the function starting at `start`; None when no
    /// function starts there.
    pub fn function(&self, start: u32) -> Option<String> {
        let f = self.p.funcs.get(&start)?;
        let cfg = Cfg::new(self, start)?;
        let sp = self.cc.stack_pointer;
        // The frame: the stack pointer lowered by a constant early on.
        let frame = cfg.addrs.iter().take(4).find_map(|a| match cfg.lifted(*a)?.effects.as_slice() {
            [Effect::Set(r, Expr::Bin(BinOp::Add, b, c))]
                if Some(*r) == sp && Some(b.as_ref()) == sp.map(Expr::Reg).as_ref() =>
            {
                match c.as_ref() {
                    Expr::Const(c) if *c < 0 => Some(-c),
                    _ => None,
                }
            }
            _ => None,
        });
        let frame = frame.unwrap_or(0);
        let mut out = String::new();
        let args: Vec<String> = self.cc.args[..self.arity(start)].iter().map(|r| self.p.arch.reg_name(*r)).collect();
        let _ = writeln!(
            out,
            "// {}({}) {start:08x}..{:08x}, {} instructions, frame {frame}",
            self.func_name(start),
            args.join(", "),
            f.end,
            f.insns
        );
        let mut spills: BTreeMap<i64, Reg> = BTreeMap::new();
        for k in 0..cfg.blocks.len() {
            if cfg.traps.contains(&cfg.blocks[k].start) {
                continue;
            }
            let mut after = BTreeMap::new();
            cfg.live_through(self, k, Some(&mut after));
            let mut e = Emitter {
                ps: self,
                cfg: &cfg,
                regs: BTreeMap::new(),
                out: &mut out,
                spills: &mut spills,
                after,
                uses: BTreeMap::new(),
                entry: k == 0,
                inflight: Vec::new(),
                marks: Vec::new(),
            };
            if k == 0
                && frame != 0
                && let Some(sp) = sp
            {
                // At entry sp is the frame plus its size, so after the
                // prologue's adjustment sp-relative addresses are frame
                // offsets.
                e.regs.insert(sp, Expr::bin(BinOp::Add, Expr::Reg(sp), Expr::Const(frame)));
            }
            e.block(k);
        }
        Some(out)
    }

    /// A callee's name: its symbol or generated name, unless that is a
    /// local label or a string (a call into the middle of a function).
    fn func_name(&self, a: u32) -> String {
        match (self.names)(a) {
            Some(n) if !n.starts_with(['.', '"']) && !n.contains(char::is_whitespace) => n,
            _ => format!("fn_{a:08x}"),
        }
    }

    /// A goto label: a symbol's name when it is an identifier, else `L_`.
    fn label(&self, a: u32) -> String {
        match (self.names)(a) {
            Some(n) if is_ident(&n) && !self.p.funcs.contains_key(&a) => n,
            _ => format!("L_{a:08x}"),
        }
    }

    fn rname(&self, r: Reg) -> String {
        self.p.arch.var_name(r)
    }

    /// A value: named when it is an address in the image (a function, a
    /// string, a table), else a number. Addresses outside the image (I/O,
    /// the scratchpad) are named where they are loaded or stored, not as
    /// values: a constant there is as likely an offset.
    fn constant(&self, v: i64) -> String {
        let u = v as u32;
        if !(-4096..=4096).contains(&v)
            && self.p.image.contains(u)
            && let Some(n) = (self.names)(u)
        {
            return n;
        }
        if !(-4096..=4096).contains(&v) { format!("0x{u:x}") } else { v.to_string() }
    }

    /// Where a load or store goes, as text.
    fn place(&self, addr: &Expr, size: u8, signed: bool) -> String {
        let t = ty(size, signed);
        let tag = if t == "s32" { String::new() } else { format!(" /*{t}*/") };
        let sp = self.cc.stack_pointer;
        let gp = self.cc.global_pointer;
        if let Expr::Bin(BinOp::Add, a, b) = addr
            && let Expr::Const(off) = b.as_ref()
        {
            if let Expr::Reg(r) = a.as_ref() {
                if Some(*r) == sp {
                    return format!("local_{off:x}{tag}");
                }
                if Some(*r) == gp
                    && let Some(gpv) = self.p.gp
                {
                    let at = gpv.wrapping_add(*off as u32);
                    return format!("{}{tag}", (self.names)(at).unwrap_or(format!("g_{at:08x}")));
                }
            }
            // An address in the image plus a register: a table indexed by
            // a byte offset.
            let at = *off as u32;
            if !(-4096..=4096).contains(off) && self.p.image.contains(at) {
                let table = (self.names)(at).unwrap_or(format!("0x{at:08x}"));
                return format!("*({t}*)({table} + {}){tag}", self.expr(a));
            }
            let sign = if *off < 0 { "-" } else { "" };
            return format!("{}->{sign}0x{:x}{tag}", self.expr(a), off.unsigned_abs());
        }
        if let Expr::Const(c) = addr {
            let at = *c as u32;
            return format!("{}{tag}", (self.names)(at).unwrap_or(format!("*({t}*)0x{at:08x}")));
        }
        if let Expr::Reg(r) = addr
            && Some(*r) == sp
        {
            return format!("local_0{tag}");
        }
        if let Expr::Reg(_) | Expr::Var(_) = addr {
            return format!("{}->0x0{tag}", self.expr(addr));
        }
        // base[index], scaled by the element size.
        if let Expr::Bin(BinOp::Add, a, b) = addr
            && let Expr::Bin(BinOp::Shl, i, n) = a.as_ref()
            && let Expr::Const(n) = n.as_ref()
            && (0..8).contains(n)
            && 1 << n == i64::from(size)
        {
            return format!("{}[{}]{tag}", self.expr(b), self.expr(i));
        }
        format!("*({t}*)({})", self.expr(addr))
    }

    fn expr(&self, e: &Expr) -> String {
        match e {
            Expr::Reg(r) => self.rname(*r),
            Expr::Const(v) => self.constant(*v),
            Expr::Bin(BinOp::Add, a, b) if matches!(a.as_ref(), Expr::Reg(r) if Some(*r) == self.cc.stack_pointer) => {
                match b.as_ref() {
                    Expr::Const(off) => format!("&local_{off:x}"),
                    _ => format!("({} + {})", self.expr(a), self.expr(b)),
                }
            }
            Expr::Bin(op @ BinOp::MulLo { .. }, a, b) => {
                format!("lo({} {} {})", self.expr(a), op.symbol(), self.expr(b))
            }
            Expr::Bin(op @ BinOp::MulHi { .. }, a, b) => {
                format!("hi({} {} {})", self.expr(a), op.symbol(), self.expr(b))
            }
            Expr::Bin(op, a, b) if op.is_function() => format!("{}({}, {})", op.symbol(), self.expr(a), self.expr(b)),
            Expr::Bin(op, a, b) => format!("({} {} {})", self.expr(a), op.symbol(), self.expr(b)),
            Expr::Un(UnOp::Neg | UnOp::FNeg, a) => format!("-{}", self.expr(a)),
            Expr::Un(UnOp::Not, a) => format!("~{}", self.expr(a)),
            Expr::Un(UnOp::FAbs, a) => format!("fabs({})", self.expr(a)),
            Expr::Un(UnOp::FSqrt, a) => format!("sqrt({})", self.expr(a)),
            Expr::Un(UnOp::IntToFloat, a) => format!("(float){}", self.expr(a)),
            Expr::Un(UnOp::FloatToInt, a) => format!("(s32){}", self.expr(a)),
            Expr::Load { size, signed, addr } => self.place(addr, *size, *signed),
            Expr::MulShr(a, b, n) => format!("fx({} * {} >> {n})", self.expr(a), self.expr(b)),
            Expr::Cond(c, a, b) => format!("({} ? {} : {})", self.cond(c), self.expr(a), self.expr(b)),
            Expr::Var(n) => n.clone(),
        }
    }

    /// A condition, without its outer parentheses.
    fn cond(&self, e: &Expr) -> String {
        match e {
            Expr::Bin(op, a, b) if !matches!(op, BinOp::MulLo { .. } | BinOp::MulHi { .. }) && !op.is_function() => {
                format!("{} {} {}", self.expr(a), op.symbol(), self.expr(b))
            }
            _ => self.expr(e),
        }
    }
}

/// Whether pseudo-code folds a register's values into expressions: the
/// general, special (hi/lo) and flag registers. A coprocessor's are written
/// as statements where they change.
fn folds(r: Reg) -> bool {
    matches!(r.file, RegFile::Gpr | RegFile::Special | RegFile::Flags)
}

fn is_ident(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn ty(size: u8, signed: bool) -> String {
    match (size, signed) {
        (1, true) => "s8".into(),
        (1, false) => "u8".into(),
        (2, true) => "s16".into(),
        (2, false) => "u16".into(),
        (4, _) => "s32".into(),
        (8, _) => "s64".into(),
        (n, _) => format!("u{}", u32::from(n) * 8),
    }
}

/// The index a switch dispatches on: in the table load's address, the term
/// scaled by the entry size (`x << 2` for 4-byte entries).
fn switch_index(e: &Expr, psize: u32) -> Option<Expr> {
    fn find(e: &Expr, n: i64) -> Option<Expr> {
        match e {
            Expr::Bin(BinOp::Shl, x, k) if matches!(k.as_ref(), Expr::Const(k) if *k == n) => Some(x.as_ref().clone()),
            Expr::Bin(BinOp::Add, a, b) => find(a, n).or_else(|| find(b, n)),
            _ => None,
        }
    }
    let Expr::Load { addr, .. } = e else { return None };
    find(addr, i64::from(psize.trailing_zeros()))
}

/// An address as base and constant offset.
fn split(a: &Expr) -> (Expr, i64) {
    match a {
        Expr::Bin(BinOp::Add, x, c) => match c.as_ref() {
            Expr::Const(o) => (x.as_ref().clone(), *o),
            _ => (a.clone(), 0),
        },
        Expr::Const(o) => (Expr::Const(0), *o),
        _ => (a.clone(), 0),
    }
}

/// Whether a load in `e` may read what a store to `store` writes. Stack
/// slots are taken not to alias anything but stack slots, and the same base
/// at offsets four or more apart not to overlap.
fn aliases(e: &Expr, store: &Expr, sp: Option<Reg>) -> bool {
    match e {
        Expr::Load { addr, .. } => {
            if aliases(addr, store, sp) {
                return true;
            }
            let ((b1, o1), (b2, o2)) = (split(addr), split(store));
            let is_sp = |b: &Expr| matches!(b, Expr::Reg(r) if Some(*r) == sp);
            if is_sp(&b1) != is_sp(&b2) {
                return false;
            }
            if b1 == b2 {
                return (o1 - o2).abs() < 4;
            }
            true
        }
        Expr::Bin(_, a, b) | Expr::MulShr(a, b, _) => aliases(a, store, sp) || aliases(b, store, sp),
        Expr::Cond(c, a, b) => aliases(c, store, sp) || aliases(a, store, sp) || aliases(b, store, sp),
        Expr::Un(_, a) => aliases(a, store, sp),
        _ => false,
    }
}

struct Block {
    start: u32,
    end: u32,
    succ: Vec<u32>,
}

/// A function's blocks and register liveness.
struct Cfg {
    addrs: Vec<u32>,
    set: BTreeSet<u32>,
    insns: BTreeMap<u32, (Insn, Option<Lifted>)>,
    blocks: Vec<Block>,
    index: BTreeMap<u32, usize>,
    leaders: BTreeSet<u32>,
    /// Addresses that get a label: branch and jump targets.
    targets: BTreeSet<u32>,
    /// Branches that only guard an aborting trap.
    guards: BTreeSet<u32>,
    /// Blocks that are only an aborting trap.
    traps: BTreeSet<u32>,
    live_in: Vec<BTreeSet<Reg>>,
}

fn transfers(flow: Flow) -> bool {
    matches!(flow, Flow::Branch { .. } | Flow::Jump { .. } | Flow::Call { .. } | Flow::Return)
}

impl Cfg {
    fn new<A: Arch + ?Sized>(ps: &Pseudo<'_, '_, A>, start: u32) -> Option<Cfg> {
        let p = ps.p;
        p.funcs.get(&start)?;
        let addrs = p.insns_of(start);
        let set: BTreeSet<u32> = addrs.iter().copied().collect();
        let insns: BTreeMap<u32, (Insn, Option<Lifted>)> = addrs
            .iter()
            .filter_map(|&a| {
                let i = p.insn(a)?;
                let l = p.arch.lift(&i);
                Some((a, (i, l)))
            })
            .collect();
        let mut cfg = Cfg {
            addrs,
            set,
            insns,
            blocks: Vec::new(),
            index: BTreeMap::new(),
            leaders: [start].into(),
            targets: [start].into(),
            guards: BTreeSet::new(),
            traps: BTreeSet::new(),
            live_in: Vec::new(),
        };
        for a in cfg.addrs.clone() {
            let Some(flow) = cfg.insn(a).map(|i| i.flow) else { continue };
            if transfers(flow) {
                if let Some(t) = flow.target()
                    && cfg.set.contains(&t)
                    && !matches!(flow, Flow::Call { .. })
                {
                    cfg.leaders.insert(t);
                    cfg.targets.insert(t);
                }
                // Calls return into the same block.
                if !matches!(flow, Flow::Call { .. }) {
                    let after = cfg.after_transfer(a);
                    cfg.leaders.insert(after);
                }
            }
            if cfg.aborts(a) {
                let next = cfg.next(a);
                cfg.leaders.insert(a);
                cfg.leaders.insert(next);
            }
            if let Some(jt) = p.jump_tables.get(&a) {
                cfg.leaders.extend(jt.targets.iter().copied());
                cfg.targets.extend(jt.targets.iter().copied());
            }
        }
        let mut cur: Option<u32> = None;
        let mut prev_end = 0;
        for &a in &cfg.addrs {
            if let Some(s) = cur
                && (cfg.leaders.contains(&a) || a != prev_end)
            {
                cfg.blocks.push(Block { start: s, end: prev_end, succ: vec![] });
                cur = None;
            }
            if cur.is_none() {
                cur = Some(a);
            }
            prev_end = cfg.next(a);
        }
        if let Some(s) = cur {
            cfg.blocks.push(Block { start: s, end: prev_end, succ: vec![] });
        }
        // Traps, and the branches that skip them: a conditional branch just
        // before a trap block, to just after it.
        cfg.traps =
            cfg.blocks.iter().filter(|b| cfg.next(b.start) == b.end && cfg.aborts(b.start)).map(|b| b.start).collect();
        for &t in &cfg.traps.clone() {
            let k = cfg.addrs.partition_point(|a| *a < t);
            for &at in cfg.addrs[k.saturating_sub(8)..k].iter() {
                if let Some(i) = cfg.insn(at)
                    && matches!(i.flow, Flow::Branch { target, .. } if target == cfg.next(t))
                {
                    cfg.guards.insert(at);
                }
            }
        }
        for k in 0..cfg.blocks.len() {
            let fall = cfg.blocks[k].end;
            let succ = if cfg.traps.contains(&cfg.blocks[k].start) {
                vec![fall]
            } else {
                let last = cfg.order(k).last().map(|(pc, _)| *pc).unwrap_or(cfg.blocks[k].start);
                match cfg.insn(last).map(|i| i.flow) {
                    Some(Flow::Return | Flow::Halt | Flow::Invalid) | None => vec![],
                    Some(Flow::Jump { target: None }) => {
                        p.jump_tables.get(&last).map_or(vec![], |jt| jt.targets.clone())
                    }
                    Some(Flow::Jump { target: Some(t) }) => {
                        if cfg.set.contains(&t) {
                            vec![t]
                        } else {
                            vec![]
                        }
                    }
                    Some(Flow::Branch { target, .. }) => vec![target, fall],
                    Some(Flow::Call { .. } | Flow::Next | Flow::Trap) => vec![fall],
                }
            };
            cfg.blocks[k].succ = succ;
        }
        // The guards' own targets are labels only through other branches.
        for &g in &cfg.guards {
            let Some(t) = cfg.insn(g).and_then(|i| i.flow.target()) else { continue };
            let other = cfg.addrs.iter().any(|&a| {
                !cfg.guards.contains(&a)
                    && cfg.insn(a).is_some_and(|i| {
                        transfers(i.flow) && !matches!(i.flow, Flow::Call { .. }) && i.flow.target() == Some(t)
                    })
            });
            if !other {
                cfg.targets.remove(&t);
            }
        }
        cfg.index = cfg.blocks.iter().enumerate().map(|(k, b)| (b.start, k)).collect();
        cfg.live_in = vec![BTreeSet::new(); cfg.blocks.len()];
        // Liveness converges in a few passes; the cap only guards against a
        // malformed graph.
        for _ in 0..64 {
            let mut changed = false;
            for k in (0..cfg.blocks.len()).rev() {
                let inn = cfg.live_through(ps, k, None);
                if inn != cfg.live_in[k] {
                    cfg.live_in[k] = inn;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Some(cfg)
    }

    fn insn(&self, a: u32) -> Option<&Insn> {
        self.insns.get(&a).map(|(i, _)| i)
    }

    fn lifted(&self, a: u32) -> Option<&Lifted> {
        self.insns.get(&a).and_then(|(_, l)| l.as_ref())
    }

    /// The address after the instruction at `a`.
    fn next(&self, a: u32) -> u32 {
        a.wrapping_add(self.insn(a).map_or(1, |i| u32::from(i.size.max(1))))
    }

    /// The address after a transfer at `a` and its delay slot.
    fn after_transfer(&self, a: u32) -> u32 {
        let n = self.next(a);
        if self.insn(a).is_some_and(|i| i.delay_slot) { self.next(n) } else { n }
    }

    fn aborts(&self, a: u32) -> bool {
        self.lifted(a).is_some_and(|l| l.effects.contains(&Effect::Abort))
    }

    fn live_out(&self, k: usize) -> BTreeSet<Reg> {
        self.blocks[k]
            .succ
            .iter()
            .filter_map(|s| self.index.get(s))
            .flat_map(|&j| self.live_in[j].iter().copied())
            .collect()
    }

    /// The block's instructions in execution order, as (address, whether it
    /// is a transfer whose delay slot follows in the block).
    fn order(&self, k: usize) -> Vec<(u32, bool)> {
        let b = &self.blocks[k];
        let mut v = Vec::new();
        let mut pc = b.start;
        while pc < b.end && self.set.contains(&pc) {
            let next = self.next(pc);
            let branch = self.insn(pc).is_some_and(|i| i.delay_slot) && next < b.end;
            v.push((pc, branch));
            pc = if branch { self.next(next) } else { next };
        }
        v
    }

    /// Liveness back through block `k`: its live-in, and if asked, what is
    /// live after each instruction. A branch reads its operands before its
    /// delay slot runs, and a call or return reads its arguments or results
    /// after it.
    fn live_through<A: Arch + ?Sized>(
        &self,
        ps: &Pseudo<'_, '_, A>,
        k: usize,
        mut after: Option<&mut BTreeMap<u32, BTreeSet<Reg>>>,
    ) -> BTreeSet<Reg> {
        let mut live = self.live_out(k);
        let mut note = |pc: u32, live: &BTreeSet<Reg>| {
            if let Some(after) = after.as_deref_mut() {
                after.insert(pc, live.clone());
            }
        };
        for (pc, branch) in self.order(k).into_iter().rev() {
            note(pc, &live);
            let (early, late, defs) = self.uses_defs(ps, pc);
            for r in defs {
                live.remove(&r);
            }
            live.extend(late);
            if branch {
                let d = self.next(pc);
                note(d, &live);
                let (u, l, defs) = self.uses_defs(ps, d);
                for r in defs {
                    live.remove(&r);
                }
                live.extend(u);
                live.extend(l);
            }
            live.extend(early);
        }
        live
    }

    /// The registers the instruction at `pc` reads, before and after its
    /// delay slot, and writes.
    fn uses_defs<A: Arch + ?Sized>(&self, ps: &Pseudo<'_, '_, A>, pc: u32) -> (Vec<Reg>, Vec<Reg>, Vec<Reg>) {
        let Some(i) = self.insn(pc) else { return (vec![], vec![], vec![]) };
        if self.guards.contains(&pc) {
            return (vec![], vec![], vec![]);
        }
        let cc = &ps.cc;
        let args = |n: usize| cc.args[..n.min(cc.args.len())].to_vec();
        let mut defs = i.writes.clone();
        let late = match i.flow {
            Flow::Return => cc.results.iter().take(1).copied().collect(),
            Flow::Jump { target: None } if ps.p.jump_tables.contains_key(&pc) => vec![],
            Flow::Jump { target: None } | Flow::Call { target: None } => args(cc.args.len()),
            Flow::Call { target: Some(t) } => args(ps.arity(t)),
            Flow::Jump { target: Some(t) } if !self.set.contains(&t) => args(ps.arity(t)),
            _ => vec![],
        };
        if matches!(i.flow, Flow::Call { .. }) {
            defs.extend(cc.clobbered);
        }
        let early = if i.flow == Flow::Return { vec![] } else { i.reads.clone() };
        (early, late, defs)
    }
}

struct Emitter<'e, 'p, 'a, A: Arch + ?Sized> {
    ps: &'e Pseudo<'p, 'a, A>,
    cfg: &'e Cfg,
    regs: BTreeMap<Reg, Expr>,
    out: &'e mut String,
    spills: &'e mut BTreeMap<i64, Reg>,
    /// What is live after each instruction of the block.
    after: BTreeMap<u32, BTreeSet<Reg>>,
    /// For each instruction that writes a register, how often the value is
    /// read before it is overwritten.
    uses: BTreeMap<(u32, Reg), usize>,
    /// Whether this is the function's first block.
    entry: bool,
    /// Values computed but not yet stored anywhere, kept correct while
    /// registers are written out.
    inflight: Vec<Expr>,
    /// Marks standing for copies of a value being written out.
    marks: Vec<(Expr, Expr)>,
}

impl<A: Arch + ?Sized> Emitter<'_, '_, '_, A> {
    fn get(&self, r: Reg) -> Expr {
        self.regs.get(&r).cloned().unwrap_or(Expr::Reg(r))
    }

    /// `e` (in registers as they were before an instruction) in terms of
    /// the pending values.
    fn resolve(&self, e: &Expr) -> Expr {
        e.subst(&|r| self.get(r)).simplify_all()
    }

    fn line(&mut self, s: String) {
        let _ = writeln!(self.out, "    {s}");
    }

    fn is_frame(&self, r: Reg, e: &Expr) -> bool {
        let sp = self.ps.cc.stack_pointer;
        Some(r) == sp
            && matches!(e, Expr::Bin(BinOp::Add, a, b) if matches!((a.as_ref(), b.as_ref()), (Expr::Reg(x), Expr::Const(_)) if Some(*x) == sp))
    }

    /// The stack slot an address is, if it is one.
    fn slot(&self, a: &Expr) -> Option<i64> {
        match a {
            Expr::Bin(BinOp::Add, x, c) if matches!(x.as_ref(), Expr::Reg(r) if Some(*r) == self.ps.cc.stack_pointer) => {
                match c.as_ref() {
                    Expr::Const(o) => Some(*o),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Writes register `r`'s pending value out. Other pending values that
    /// still read the old `r` are written first; copies of the value inside
    /// other pending or in-flight values come to read `r` instead; and an
    /// in-flight value that reads the old `r` keeps it in a temporary.
    fn materialize(&mut self, r: Reg, busy: &mut Vec<Reg>) {
        let Some(p) = self.regs.get(&r).cloned() else { return };
        if p == Expr::Reg(r) || self.is_frame(r, &p) {
            return;
        }
        // Copies of the value, marked until `r` holds it.
        let mark = Expr::Var(format!("\u{0}{}", self.ps.rname(r)));
        let share = p.size() > 2;
        if share {
            for (q, x) in self.regs.iter_mut() {
                if *q != r {
                    *x = x.replace(&p, &mark);
                }
            }
            for x in self.inflight.iter_mut() {
                *x = x.replace(&p, &mark);
            }
            self.marks.push((mark.clone(), p.clone()));
        }
        busy.push(r);
        let readers: Vec<Reg> =
            self.regs.iter().filter(|(q, x)| **q != r && x.mentions(&[r])).map(|(q, _)| *q).collect();
        for q in readers {
            if busy.contains(&q) {
                // A cycle: the old value goes to a temporary.
                self.keep_old(r);
            } else {
                self.materialize(q, busy);
            }
        }
        busy.pop();
        if self.inflight.iter().any(|x| x.mentions(&[r])) {
            self.keep_old(r);
        }
        let e = self.regs[&r].clone();
        let text = format!("{} = {};", self.ps.rname(r), self.show(&e));
        self.line(text);
        self.regs.insert(r, Expr::Reg(r));
        if share {
            self.marks.retain(|(m, _)| *m != mark);
            for x in self.regs.values_mut() {
                *x = x.replace(&mark, &Expr::Reg(r));
            }
            for x in self.inflight.iter_mut() {
                *x = x.replace(&mark, &Expr::Reg(r));
            }
        }
    }

    /// Saves `r` in `old_r` and makes every pending and in-flight read of
    /// `r` read that.
    fn keep_old(&mut self, r: Reg) {
        let name = self.ps.rname(r);
        let tmp = Expr::Var(format!("old_{name}"));
        self.line(format!("old_{name} = {name};"));
        for x in self.regs.values_mut().chain(self.inflight.iter_mut()) {
            *x = x.replace(&Expr::Reg(r), &tmp);
        }
    }

    /// An expression with marked copies spelled out.
    fn unmark(&self, e: &Expr) -> Expr {
        let mut e = e.clone();
        for (m, p) in self.marks.iter().rev() {
            e = e.replace(m, p);
        }
        // A copy spelled out may fold with what surrounds it.
        if self.marks.is_empty() { e } else { e.simplify_all() }
    }

    fn show(&self, e: &Expr) -> String {
        self.ps.expr(&self.unmark(e))
    }

    /// Writes out `regs` (those still pending) while `values` are in flight:
    /// built from the registers as they are, and kept correct as registers
    /// are written. Returns the values.
    fn materialize_with(&mut self, regs: &[Reg], values: Vec<Expr>) -> Vec<Expr> {
        let base = self.inflight.len();
        self.inflight.extend(values);
        for &r in regs {
            self.materialize(r, &mut vec![]);
        }
        self.inflight.split_off(base)
    }

    fn set(&mut self, pc: u32, r: Reg, e: Expr) {
        // Nothing pending may read the old value once it is replaced: what
        // is still needed is written out, the rest forgotten.
        let live = self.after.get(&pc).cloned().unwrap_or_default();
        let readers: Vec<Reg> =
            self.regs.iter().filter(|(q, x)| **q != r && x.mentions(&[r])).map(|(q, _)| *q).collect();
        let (keep, drop): (Vec<Reg>, Vec<Reg>) = readers.into_iter().partition(|q| live.contains(q));
        for q in drop {
            self.regs.remove(&q);
        }
        let e = self.materialize_with(&keep, vec![e]).pop().unwrap().simplify();
        let named = self.uses.get(&(pc, r)).is_some_and(|&n| n >= 2) && e.size() >= 6;
        self.regs.insert(r, e);
        if named {
            self.materialize(r, &mut vec![]);
        }
    }

    /// Writes back registers (those `keep` says) whose value is an
    /// expression, and forgets them. The writes are one parallel assignment,
    /// so they are ordered to read each register before it is overwritten,
    /// with a temporary to break a cycle. `cond`, an expression in the values
    /// from before the writes, comes back rewritten in terms of after them.
    fn flush(&mut self, keep: impl Fn(Reg) -> bool, cond: Option<Expr>) -> Option<Expr> {
        let regs = std::mem::take(&mut self.regs);
        let mut pending: Vec<(Reg, Expr)> =
            regs.into_iter().filter(|(r, e)| *e != Expr::Reg(*r) && keep(*r) && !self.is_frame(*r, e)).collect();
        let cond = cond.map(|c| {
            let mut by_size = pending.clone();
            by_size.sort_by_key(|(_, e)| std::cmp::Reverse(e.size()));
            // Parts of the condition a write computes read as that register.
            let mut c2 = c.clone();
            for (r, e) in by_size.iter().filter(|(_, e)| e.size() > 1) {
                c2 = c2.replace(e, &Expr::Var(self.ps.rname(*r)));
            }
            let targets: Vec<Reg> = pending.iter().map(|(r, _)| *r).collect();
            if c2.mentions(&targets) {
                // It still reads a register the writes change: test it first.
                let text = format!("cond = {};", self.ps.cond(&c));
                self.line(text);
                return Expr::Var("cond".into());
            }
            c2
        });
        while !pending.is_empty() {
            let free = (0..pending.len())
                .find(|&k| !pending.iter().enumerate().any(|(j, (_, e))| j != k && e.mentions(&[pending[k].0])));
            let Some(k) = free else {
                let r = pending[0].0;
                let name = self.ps.rname(r);
                let tmp = Expr::Var(format!("old_{name}"));
                self.line(format!("old_{name} = {name};"));
                for (_, e) in pending.iter_mut().skip(1) {
                    *e = e.replace(&Expr::Reg(r), &tmp);
                }
                continue;
            };
            let (r, e) = pending.remove(k);
            let text = format!("{} = {};", self.ps.rname(r), self.ps.expr(&e));
            self.line(text);
        }
        cond
    }

    /// Forgets pending values nothing reads any more.
    fn prune(&mut self, live: &BTreeSet<Reg>) {
        let sp = self.ps.cc.stack_pointer;
        self.regs.retain(|r, e| live.contains(r) || Some(*r) == sp || *e == Expr::Reg(*r));
    }

    fn block(&mut self, k: usize) {
        let b = &self.cfg.blocks[k];
        let start = b.start;
        if self.cfg.targets.contains(&start) {
            let _ = writeln!(self.out, "{}:", self.ps.label(start));
        }
        // How often each written value is read.
        let live_out = self.cfg.live_out(k);
        let mut open: BTreeMap<Reg, u32> = BTreeMap::new();
        let mut count = |open: &mut BTreeMap<Reg, u32>, uses: &[Reg], defs: &[(u32, Reg)]| {
            for r in uses {
                if let Some(&at) = open.get(r) {
                    *self.uses.entry((at, *r)).or_default() += 1;
                }
            }
            for &(pc, r) in defs {
                open.insert(r, pc);
                self.uses.insert((pc, r), 0);
            }
        };
        let order = self.cfg.order(k);
        for &(pc, branch) in &order {
            let (early, late, defs) = self.cfg.uses_defs(self.ps, pc);
            count(&mut open, &early, &[]);
            if branch {
                let d = self.cfg.next(pc);
                let (u, l, dd) = self.cfg.uses_defs(self.ps, d);
                let dd: Vec<_> = dd.into_iter().map(|r| (d, r)).collect();
                count(&mut open, &[u, l].concat(), &dd);
            }
            let defs: Vec<_> = defs.into_iter().map(|r| (pc, r)).collect();
            count(&mut open, &late, &defs);
        }
        for (r, at) in open {
            if live_out.contains(&r) {
                *self.uses.entry((at, r)).or_default() += 1;
            }
        }
        for (pc, branch) in order {
            let Some(i) = self.cfg.insn(pc) else { continue };
            if transfers(i.flow) {
                let delay = branch.then(|| self.cfg.next(pc));
                self.branch(pc, delay);
                continue;
            }
            self.step(pc);
            if let Some(live) = self.after.get(&pc).cloned() {
                self.prune(&live);
            }
        }
        self.flush(|r| live_out.contains(&r), None);
    }

    /// Applies the effects of one instruction, all reading the registers as
    /// they were before it.
    fn apply(&mut self, pc: u32, effects: &[Effect]) {
        let mut pending: Vec<Effect> = effects
            .iter()
            .map(|e| match e {
                Effect::Set(r, x) => Effect::Set(*r, self.resolve(x)),
                Effect::Store { size, addr, value } => {
                    Effect::Store { size: *size, addr: self.resolve(addr), value: self.resolve(value) }
                }
                other => other.clone(),
            })
            .collect();
        pending.reverse();
        while let Some(eff) = pending.pop() {
            // The later effects' values ride in flight while this one is
            // applied.
            let base = self.inflight.len();
            for e in pending.iter().rev() {
                match e {
                    Effect::Set(_, x) => self.inflight.push(x.clone()),
                    Effect::Store { addr, value, .. } => self.inflight.extend([addr.clone(), value.clone()]),
                    _ => {}
                }
            }
            match eff {
                Effect::Set(r, e) => self.assign(pc, r, e),
                Effect::Store { size, addr, value } => self.store(pc, size, addr, value),
                Effect::Opaque(text) => {
                    let insn = self.cfg.insn(pc).cloned();
                    self.opaque(pc, insn.as_ref(), format!("{text};"));
                }
                Effect::Abort => self.line("trap();".into()),
            }
            let mut back = self.inflight.split_off(base).into_iter();
            for e in pending.iter_mut().rev() {
                match e {
                    Effect::Set(_, x) => *x = back.next().unwrap(),
                    Effect::Store { addr, value, .. } => {
                        *addr = back.next().unwrap();
                        *value = back.next().unwrap();
                    }
                    _ => {}
                }
            }
        }
    }

    fn assign(&mut self, pc: u32, r: Reg, e: Expr) {
        // A coprocessor's or device's register: written where it happens, as
        // a statement, rather than folded.
        if !folds(r) {
            let readers: Vec<Reg> =
                self.regs.iter().filter(|(q, x)| **q != r && x.mentions(&[r])).map(|(q, _)| *q).collect();
            let live = self.after.get(&pc).cloned().unwrap_or_default();
            let (keep, drop): (Vec<Reg>, Vec<Reg>) = readers.into_iter().partition(|q| live.contains(q));
            for q in drop {
                self.regs.remove(&q);
            }
            let e = self.materialize_with(&keep, vec![e]).pop().unwrap();
            let text = format!("{} = {};", self.ps.rname(r), self.show(&e));
            self.line(text);
            self.regs.insert(r, Expr::Reg(r));
            return;
        }
        // An epilogue restoring a saved register from its slot.
        if let Expr::Load { addr, .. } = &e
            && let Some(o) = self.slot(addr)
            && self.spills.get(&o) == Some(&r)
        {
            self.regs.insert(r, Expr::Reg(r));
            return;
        }
        self.set(pc, r, e);
    }

    fn store(&mut self, pc: u32, size: u8, addr: Expr, value: Expr) {
        // A prologue saving a register it will restore: in the entry block,
        // the register still as it came in.
        if let (Some(o), Expr::Reg(r)) = (self.slot(&addr), &value)
            && self.entry
            && !self.regs.contains_key(r)
            && self.ps.cc.saved.contains(r)
            && size >= 4
        {
            self.spills.insert(o, *r);
            return;
        }
        // Values read from what the store may change, and still needed, are
        // written out first.
        let live = self.after.get(&pc).cloned().unwrap_or_default();
        let sp = self.ps.cc.stack_pointer;
        let hit: Vec<Reg> = self.regs.iter().filter(|(_, e)| aliases(e, &addr, sp)).map(|(r, _)| *r).collect();
        let (keep, drop): (Vec<Reg>, Vec<Reg>) = hit.into_iter().partition(|r| live.contains(r));
        for r in drop {
            self.regs.remove(&r);
        }
        let v = self.materialize_with(&keep, vec![addr, value]);
        let text = format!("{} = {};", self.ps.place(&self.unmark(&v[0]), size, false), self.show(&v[1]));
        self.line(text);
    }

    /// A statement the IR does not model. Pending values that read a
    /// register it overwrites are written out first (or forgotten, when
    /// nothing reads them later), as are the registers it reads; after it,
    /// the registers it writes read as themselves.
    fn opaque(&mut self, pc: u32, insn: Option<&Insn>, text: String) {
        let (reads, writes) = insn.map_or((vec![], vec![]), |i| (i.reads.clone(), i.writes.clone()));
        let live = self.after.get(&pc).cloned().unwrap_or_default();
        let readers: Vec<Reg> =
            self.regs.iter().filter(|(q, x)| !writes.contains(q) && x.mentions(&writes)).map(|(q, _)| *q).collect();
        for q in readers {
            if live.contains(&q) {
                self.materialize(q, &mut vec![]);
            } else {
                self.regs.remove(&q);
            }
        }
        for r in reads {
            if self.regs.get(&r).is_some_and(|e| *e != Expr::Reg(r)) {
                self.materialize(r, &mut vec![]);
            }
        }
        self.line(text);
        for w in writes {
            self.regs.insert(w, Expr::Reg(w));
        }
    }

    fn step(&mut self, pc: u32) {
        let Some(i) = self.cfg.insn(pc).cloned() else { return };
        match self.cfg.lifted(pc).cloned() {
            Some(l) => self.apply(pc, &l.effects),
            None => {
                let text = format!("/* {} */", self.ps.p.arch.render(&i, &|_| None));
                self.opaque(pc, Some(&i), text);
            }
        }
    }

    fn call_args(&self, n: usize) -> String {
        self.ps.cc.args[..n.min(self.ps.cc.args.len())]
            .iter()
            .map(|r| self.ps.expr(&self.get(*r)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn branch(&mut self, pc: u32, delay: Option<u32>) {
        let Some(i) = self.cfg.insn(pc).cloned() else { return };
        let l = self.cfg.lifted(pc).cloned().unwrap_or_default();
        // The condition and target read the registers before the delay slot.
        let cond = l.cond.as_ref().map(|c| self.resolve(c));
        let s = l.target.as_ref().map(|t| self.resolve(t)).unwrap_or(Expr::Const(0));
        let mut v = self.materialize_with(&[], vec![s, cond.clone().unwrap_or(Expr::Const(0))]);
        if !l.effects.is_empty() {
            self.inflight.append(&mut v);
            self.apply(pc, &l.effects);
            v = self.inflight.split_off(self.inflight.len() - 2);
        }
        if let Some(d) = delay {
            self.inflight.append(&mut v);
            self.step(d);
            v = self.inflight.split_off(self.inflight.len() - 2);
        }
        let cond = cond.map(|_| v.pop().unwrap());
        let s = v.swap_remove(0);
        // What is live once the branch is taken or not.
        let live_end = self.after.get(&pc).cloned().unwrap_or_default();
        if self.cfg.guards.contains(&pc) {
            self.prune(&live_end);
            return;
        }
        let cc = self.ps.cc;
        let result = cc.results.first().copied();
        match i.flow {
            Flow::Call { target } => {
                // Values still needed after the call are written out first:
                // the call may change memory and the registers it clobbers.
                let live: Vec<Reg> = self
                    .regs
                    .iter()
                    .filter(|(r, e)| {
                        !cc.clobbered.contains(r) && live_end.contains(r) && (e.has_load() || e.mentions(cc.clobbered))
                    })
                    .map(|(r, _)| *r)
                    .collect();
                let mut v = self.materialize_with(&live, vec![s, cond.clone().unwrap_or(Expr::Const(0))]);
                let cond = cond.map(|_| v.pop().unwrap());
                let s = v.swap_remove(0);
                // Through a register holding a constant: a direct call.
                let target = target.or(match self.unmark(&s) {
                    Expr::Const(c) => Some(c as u32),
                    _ => None,
                });
                let (name, n, var) = match target {
                    None => (format!("(*{})", self.show(&s)), cc.args.len(), format!("r_{pc:x}")),
                    Some(a) => {
                        let name = self.ps.func_name(a);
                        let var = if is_ident(&name) { format!("r_{name}") } else { format!("r_{a:x}") };
                        (name, self.ps.arity(a), var)
                    }
                };
                let call = format!("{name}({})", self.call_args(n));
                let used = result.is_some_and(|r| live_end.contains(&r));
                match (&cond, result) {
                    (Some(c), Some(r)) => self.line(format!("if ({}) {} = {call};", self.ps.cond(c), self.ps.rname(r))),
                    (Some(c), None) => self.line(format!("if ({}) {call};", self.ps.cond(c))),
                    (None, _) if used => self.line(format!("{var} = {call};")),
                    (None, _) => self.line(format!("{call};")),
                }
                for r in cc.clobbered {
                    self.regs.remove(r);
                }
                if used
                    && cond.is_none()
                    && let Some(r) = result
                {
                    self.regs.insert(r, Expr::Var(var));
                }
                self.prune(&live_end);
            }
            Flow::Return => {
                let v = result.map(|r| self.get(r));
                if let Some(r) = result {
                    self.regs.remove(&r);
                }
                let rest: Vec<Reg> = cc.results.iter().skip(1).copied().collect();
                self.flush(|r| rest.contains(&r), None);
                let text = match v {
                    Some(v) => format!("return {};", self.ps.expr(&v)),
                    None => "return;".into(),
                };
                self.line(text);
            }
            Flow::Jump { target: None } => {
                // A switch shows its index, as of the jump.
                let table = self.ps.p.jump_tables.get(&pc);
                let s = match table.and_then(|jt| jt.index) {
                    Some(x) => self.get(x),
                    None => s,
                };
                let s = self.flush(|r| live_end.contains(&r), Some(s)).unwrap();
                match self.ps.p.jump_tables.get(&pc) {
                    Some(jt) => {
                        let index = match jt.index {
                            Some(_) => s.clone(),
                            None => switch_index(&s, self.ps.p.arch.pointer_size()).unwrap_or(s.clone()),
                        };
                        let text = format!("switch ({}) {{  // table 0x{:08x}", self.ps.cond(&index), jt.table);
                        self.line(text);
                        for (k, t) in jt.targets.iter().enumerate() {
                            let text = format!("    case {k}: goto {};", self.ps.label(*t));
                            self.line(text);
                        }
                        self.line("}".into());
                    }
                    None => match s {
                        // Through a register holding a constant: a tail call.
                        Expr::Const(c) => {
                            let t = c as u32;
                            let text = format!(
                                "return {}({}); // tail call",
                                self.ps.func_name(t),
                                self.call_args(self.ps.arity(t))
                            );
                            self.line(text);
                        }
                        _ => {
                            let text = format!("goto *{};", self.ps.expr(&s));
                            self.line(text);
                        }
                    },
                }
            }
            Flow::Jump { target: Some(t) } => {
                if self.cfg.leaders.contains(&t) {
                    self.flush(|r| live_end.contains(&r), None);
                    let text = format!("goto {};", self.ps.label(t));
                    self.line(text);
                } else {
                    let name = self.ps.func_name(t);
                    let args = self.call_args(self.ps.arity(t));
                    self.flush(|_| false, None);
                    self.line(format!("return {name}({args}); // tail call"));
                }
            }
            _ => {
                let cond = self.flush(|r| live_end.contains(&r), cond);
                let t = i.flow.target().unwrap_or(0);
                let label = self.ps.label(t);
                match cond {
                    // A condition that folded to a constant: always or never.
                    Some(Expr::Const(0)) => {}
                    Some(Expr::Const(_)) | None => self.line(format!("goto {label};")),
                    Some(c) => {
                        let text = format!("if ({}) goto {label};", self.ps.cond(&c));
                        self.line(text)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{Image, find_gp};
    use crate::mips::Mips;
    use crate::{Kind, Symbol, Symbols};

    const BASE: u32 = 0x8001_0000;

    fn r(op: u32, rs: u32, rt: u32, rd: u32, sa: u32, funct: u32) -> u32 {
        (op << 26) | (rs << 21) | (rt << 16) | (rd << 11) | (sa << 6) | funct
    }
    fn i(op: u32, rs: u32, rt: u32, imm: i32) -> u32 {
        (op << 26) | (rs << 21) | (rt << 16) | (imm as u32 & 0xffff)
    }
    fn jal(t: u32) -> u32 {
        (3 << 26) | ((t >> 2) & 0x03ff_ffff)
    }
    const JR_RA: u32 = 0x03e0_0008;
    const NOP: u32 = 0;
    const SP: u32 = 29;
    const RA: u32 = 31;

    /// Pseudo-code for every function of `words` at BASE, entry BASE,
    /// named by `syms`.
    fn pseudo(words: &[u32], syms: &Symbols) -> String {
        pseudo_on(Mips::r3000(), words, syms)
    }

    fn pseudo_on(arch: Mips, words: &[u32], syms: &Symbols) -> String {
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let image = Image { base: BASE, bytes: &bytes };
        let p = Program::analyze(&arch, image, BASE, find_gp(&arch, image, BASE), syms);
        let l = Listing::new(&p, syms);
        let ps = Pseudo::new(&l);
        p.funcs.keys().map(|&f| ps.function(f).unwrap()).collect()
    }

    fn body(text: &str) -> Vec<&str> {
        text.lines().filter(|l| l.starts_with("    ")).map(str::trim).collect()
    }

    #[test]
    fn a_frame_a_string_argument_and_the_callees_arity() {
        let mut w = vec![
            i(9, SP, SP, -24),     // addiu sp, sp, -24
            i(43, SP, RA, 20),     // sw ra, 20(sp)
            i(43, SP, 16, 16),     // sw s0, 16(sp)
            r(0, 5, 0, 16, 0, 33), // move s0, a1
            i(15, 0, 4, 0x8001),   // lui a0, 0x8001
            jal(BASE + 0x40),      // jal puts
            i(9, 4, 4, 0x60),      // addiu a0, a0, 0x60 (the string)
            r(0, 16, 2, 2, 0, 33), // addu v0, s0, v0
            i(35, SP, RA, 20),     // lw ra, 20(sp)
            i(35, SP, 16, 16),     // lw s0, 16(sp)
            JR_RA,
            i(9, SP, SP, 24), // addiu sp, sp, 24
        ];
        w.resize(16, 0);
        // puts(a0): reads a0 only.
        w.extend([r(0, 4, 0, 2, 0, 33), JR_RA, NOP]); // 0x40: move v0, a0; jr ra
        w.resize(24, 0);
        w.extend([u32::from_le_bytes(*b"hi t"), u32::from_le_bytes(*b"here"), 0]);
        let mut syms = Symbols::new();
        syms.mark(Symbol { kind: Kind::Func, ..Symbol::new("puts", BASE + 0x40) });
        let text = pseudo(&w, &syms);
        assert!(text.contains("// func_80010000(a0, a1) 80010000..80010030, 12 instructions, frame 24"), "{text}");
        assert!(text.contains("// puts(a0)"), "arity from the callee's liveness: {text}");
        let main = &text[..text.find("// puts").unwrap()];
        assert_eq!(
            body(main),
            ["s0 = a1;", "r_puts = puts(\"hi there\");", "return (s0 + r_puts);"],
            "the prologue's saves and the epilogue's restores are gone; s0 is written before the call clobbers a1: {main}"
        );
    }

    #[test]
    fn the_fixed_point_multiply_and_a_guarded_divide() {
        let w = vec![
            r(0, 4, 5, 0, 0, 24), // mult a0, a1
            r(0, 0, 0, 2, 0, 18), // mflo v0
            r(0, 0, 0, 3, 0, 16), // mfhi v1
            r(0, 0, 2, 2, 12, 2), // srl v0, v0, 12
            r(0, 0, 3, 3, 20, 0), // sll v1, v1, 20
            r(0, 2, 3, 2, 0, 37), // or v0, v0, v1
            r(0, 2, 6, 0, 0, 26), // div v0, a2
            i(5, 6, 0, 2),        // bnez a2, +2
            NOP,
            0x0007_000d,          // break 7
            r(0, 0, 0, 2, 0, 18), // mflo v0
            JR_RA,
            NOP,
        ];
        let text = pseudo(&w, &Symbols::new());
        // The guard and its break are gone; lo crosses the block they ended.
        assert_eq!(body(&text), ["lo = (fx(a0 * a1 >> 12) / a2);", "return lo;"], "{text}");
    }

    #[test]
    fn a_switch_through_a_jump_table() {
        let mut w = vec![
            i(11, 4, 1, 3),       // sltiu at, a0, 3
            i(4, 1, 0, 9),        // beqz at, default (0x2c)
            r(0, 0, 4, 2, 2, 0),  // sll v0, a0, 2
            i(15, 0, 1, 0x8001),  // lui at, 0x8001
            r(0, 1, 2, 1, 0, 33), // addu at, at, v0
            i(35, 1, 2, 0x40),    // lw v0, 0x40(at)
            NOP,
            r(0, 2, 0, 0, 0, 8), // jr v0
            NOP,
            JR_RA,          // 0x24: case 0
            i(9, 0, 2, 10), //   li v0, 10
            JR_RA,          // 0x2c: default
            i(9, 0, 2, -1), //   li v0, -1
        ];
        w.resize(16, 0);
        w.extend([BASE + 0x24, BASE + 0x24, BASE + 0x2c]);
        let text = pseudo(&w, &Symbols::new());
        assert!(text.contains("switch (a0) {  // table 0x80010040"), "the index, not the load: {text}");
        assert!(text.contains("case 2: goto L_8001002c;"), "{text}");
        assert!(text.contains("L_80010024:\n    return 10;"), "{text}");
    }

    /// Rugrats' 0x80023a8c: a value read from a GTE register before a
    /// command overwrites it is written out before the command, not after.
    #[test]
    fn a_value_read_from_the_gte_is_kept_across_the_next_command() {
        let mtc2 = |rt: u32, rd: u32| 0x4880_0000 | (rt << 16) | (rd << 11);
        let mfc2 = |rt: u32, rd: u32| 0x4800_0000 | (rt << 16) | (rd << 11);
        const NCLIP: u32 = 0x4b40_0006;
        let w = vec![
            mtc2(4, 12), // mtc2 a0, sxy0
            NCLIP,
            NOP,
            mfc2(8, 24), // mfc2 t0, mac0
            NOP,
            r(0, 0, 8, 3, 0, 35), // negu v1, t0
            mtc2(5, 12),          // mtc2 a1, sxy0
            NCLIP,
            i(10, 3, 2, 2), // slti v0, v1, 2
            JR_RA,
            NOP,
        ];
        let text = pseudo(&w, &Symbols::new());
        let b = body(&text);
        let v1 = b.iter().position(|l| *l == "v1 = -gte.mac0;").expect(&text);
        let second = b.iter().rposition(|l| l.starts_with("gte_nclip")).expect(&text);
        assert!(v1 < second, "{text}");
        assert_eq!(b.last(), Some(&"return (v1 < 2);"), "{text}");
    }

    #[test]
    fn unlifted_instructions_are_comments_and_their_results_unknown() {
        let w = vec![
            r(0, 0, 4, 2, 0, 0x3c), // dsll32 v0, a0, 0 (64-bit: not lifted)
            JR_RA,
            NOP,
        ];
        let text = pseudo_on(Mips::r5900(), &w, &Symbols::new());
        assert_eq!(body(&text), ["/* dsll32  v0, a0, 0 */", "return v0;"], "{text}");
    }

    #[test]
    fn the_ees_floats_and_conditional_moves() {
        let w = vec![
            0x4482_0000,            // mtc1 v0, f0
            0x4484_0800,            // mtc1 a0, f1
            0x4601_0080,            // add.s f2, f0, f1
            0x4680_10a0,            // cvt.s.w f2, f2
            0x4402_1000,            // mfc1 v0, f2
            r(0, 5, 6, 2, 0, 0x0a), // movz v0, a1, a2
            JR_RA,
            NOP,
        ];
        let text = pseudo_on(Mips::r5900(), &w, &Symbols::new());
        assert_eq!(
            body(&text),
            ["f0 = v0;", "f1 = a0;", "f2 = (f0 + f1);", "f2 = (float)f2;", "return (a2 == 0 ? a1 : f2);"],
            "{text}"
        );
    }

    #[test]
    fn a_call_through_a_constant_and_a_call_that_does_not_return() {
        let w = vec![
            i(9, 0, 10, 0xa0),     // li t2, 0xa0
            r(0, 10, 0, RA, 0, 9), // jalr t2
            i(9, 0, 9, 0x44),      // li t1, 0x44
            jal(BASE + 0x20),      // jal exit
            NOP,
            0x0000_040d, // break 0x10: does not return
            JR_RA,       // never reached from here
            NOP,
            JR_RA, // 0x20: exit
            NOP,
        ];
        let mut syms = Symbols::new();
        syms.mark(Symbol { kind: Kind::Func, ..Symbol::new("exit", BASE + 0x20) });
        syms.mark(Symbol { kind: Kind::Func, ..Symbol::new("bios_a0", 0xa0) });
        let text = pseudo(&w, &syms);
        assert!(
            text.contains("    bios_a0(a0, a1, a2, a3);\n    exit();"),
            "a constant target is a direct call: {text}"
        );
        assert!(text.contains("// func_80010000(a0, a1, a2, a3) 80010000..80010018"), "the walk ends at break: {text}");
    }
}
