//! What every backend must satisfy, checked on random bytes: a new
//! [`Arch`] calls [`check`] from its tests (as `Mips` does in
//! `tests/conformance.rs`) before anything else uses it.
//!
//! The checks are the analysis's and the decompiler's assumptions:
//!
//! - **Decoding.** It never panics and is deterministic. The address is kept.
//!   A valid instruction takes between 1 and 16 bytes, no more than it was
//!   given, and an invalid one has no delay slot.
//! - **Control flow.** A delay slot only follows a branch, jump, call or
//!   return, and every target is aligned.
//! - **Rendering.** Text is one non-empty line, and every register an
//!   instruction reads or writes has a name.
//! - **Lifting agrees with decoding.** Every register a lift reads is in
//!   [`Insn::reads`] and every one it sets is in [`Insn::writes`]. A load or
//!   store in the lift matches [`Insn::mem`]. A condition only comes with a
//!   branch or call, and a target only with a jump, call or return through a
//!   register.
//! - **Evaluation and the calling convention.** [`Arch::evaluate`] never
//!   panics. The calling convention names its registers and repeats none
//!   of its arguments.

use crate::insn::{Arch, Flow, Insn};
use crate::ir::{Effect, Expr, Lifted};

/// A rule a backend broke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// The bytes decoded.
    pub bytes: Vec<u8>,
    /// Where.
    pub addr: u32,
    /// What went wrong.
    pub what: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex: Vec<String> = self.bytes.iter().map(|b| format!("{b:02x}")).collect();
        write!(f, "{} at 0x{:08x}: {}", hex.join(" "), self.addr, self.what)
    }
}

/// xorshift64, so a failure reproduces from the seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Decodes `rounds` random byte strings with `arch` (seeded by `seed`, at
/// addresses at 0, near the top of the address space and between) and
/// returns every rule broken; empty when the backend conforms. A panic
/// inside the backend is not caught: it fails the calling test.
pub fn check(arch: &dyn Arch, rounds: u32, seed: u64) -> Vec<Failure> {
    let mut rng = Rng(seed | 1);
    let mut out = Vec::new();
    let align = arch.align().max(1);
    if let Some(cc) = arch.call_conv() {
        let named = cc
            .args
            .iter()
            .chain(cc.results)
            .chain(cc.saved)
            .chain(cc.clobbered)
            .chain(cc.stack_pointer.iter())
            .chain(cc.global_pointer.iter())
            .chain(cc.return_address.iter());
        for r in named {
            if arch.reg_name(*r).is_empty() {
                out.push(Failure { bytes: vec![], addr: 0, what: format!("call_conv register {r:?} has no name") });
            }
        }
        for (k, a) in cc.args.iter().enumerate() {
            if cc.args[..k].contains(a) {
                out.push(Failure { bytes: vec![], addr: 0, what: format!("call_conv repeats argument {a:?}") });
            }
        }
    }
    for round in 0..rounds {
        let len = (rng.next() % 17) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        // Mostly whole instructions with common low fields; sometimes raw.
        if round % 4 == 0 && len >= 4 {
            bytes[0] &= 0xf0;
        }
        let addr = match round % 5 {
            0 => 0,
            1 => u32::MAX - (rng.next() as u32 % 0x100),
            _ => rng.next() as u32,
        } / align
            * align;
        let mut fail = |what: String| out.push(Failure { bytes: bytes.clone(), addr, what });
        let insn = arch.decode(&bytes, addr);
        if arch.decode(&bytes, addr) != insn {
            fail("decoding the same bytes twice differs".into());
        }
        if insn.addr != addr {
            fail(format!("addr is 0x{:08x}", insn.addr));
        }
        if insn.is_valid() {
            if insn.size == 0 || insn.size as usize > bytes.len() || insn.size > 16 {
                fail(format!("valid with size {} from {} bytes", insn.size, bytes.len()));
            }
            check_flow(&insn, align, &mut fail);
        } else if insn.delay_slot {
            fail("invalid with a delay slot".into());
        }
        let text = arch.render(&insn, &|_| None);
        if text.trim().is_empty() || text.contains('\n') {
            fail(format!("render gives {text:?}"));
        }
        for r in insn.reads.iter().chain(&insn.writes) {
            if arch.reg_name(*r).is_empty() {
                fail(format!("register {r:?} has no name"));
            }
        }
        if let Some(l) = arch.lift(&insn) {
            check_lift(&insn, &l, &mut fail);
        }
        let k = rng.next();
        let _ = arch.evaluate(&insn, &|r| Some((k >> (r.index % 32)) as u32));
        let _ = arch.switch_bound(&insn);
    }
    out
}

fn check_flow(insn: &Insn, align: u32, fail: &mut impl FnMut(String)) {
    let transfer = matches!(insn.flow, Flow::Branch { .. } | Flow::Jump { .. } | Flow::Call { .. } | Flow::Return);
    if insn.delay_slot && !transfer {
        fail(format!("delay slot on {:?}", insn.flow));
    }
    if let Some(t) = insn.flow.target()
        && !t.is_multiple_of(align)
    {
        fail(format!("target 0x{t:08x} not aligned to {align}"));
    }
}

fn check_lift(insn: &Insn, l: &Lifted, fail: &mut impl FnMut(String)) {
    for r in l.reads() {
        if !insn.reads.contains(&r) {
            fail(format!("{} lift reads {r:?}, not in reads {:?}", insn.mnemonic, insn.reads));
        }
    }
    for r in l.writes() {
        if !insn.writes.contains(&r) {
            fail(format!("{} lift sets {r:?}, not in writes {:?}", insn.mnemonic, insn.writes));
        }
    }
    let loads = l.effects.iter().any(|e| matches!(e, Effect::Set(_, x) if x.has_load()));
    let stores: Vec<u8> = l
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Store { size, .. } => Some(*size),
            _ => None,
        })
        .collect();
    match insn.mem {
        Some(m) if m.store && stores != [m.size] => {
            fail(format!("{} stores {} bytes but the lift stores {stores:?}", insn.mnemonic, m.size))
        }
        // A load into a hard-wired register is lost: nothing to lift.
        Some(m) if !m.store && !loads && !insn.writes.is_empty() => {
            fail(format!("{} loads but the lift does not", insn.mnemonic))
        }
        None if loads || !stores.is_empty() => {
            fail(format!("{} has no mem but the lift touches memory", insn.mnemonic))
        }
        _ => {}
    }
    for e in &l.effects {
        if let Effect::Set(_, Expr::Load { size, .. }) = e
            && let Some(m) = insn.mem
            && *size != m.size
        {
            fail(format!("{} loads {} bytes but the lift loads {size}", insn.mnemonic, m.size));
        }
    }
    if l.cond.is_some() && !matches!(insn.flow, Flow::Branch { .. } | Flow::Call { .. }) {
        fail(format!("{} has a condition but flow {:?}", insn.mnemonic, insn.flow));
    }
    if l.target.is_some()
        && !matches!(insn.flow, Flow::Jump { target: None } | Flow::Call { target: None } | Flow::Return)
    {
        fail(format!("{} has a target expression but flow {:?}", insn.mnemonic, insn.flow));
    }
}
