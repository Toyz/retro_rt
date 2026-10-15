//! What instructions compute, in a form shared by every instruction set: a
//! backend lifts each instruction ([`Arch::lift`]) into [`Effect`]s on
//! registers and memory over [`Expr`] trees, plus the condition of a branch
//! and the address of a jump or call through a register.
//!
//! The analysis evaluates lifted instructions to follow constants
//! ([`Arch::evaluate`]'s default), and [`crate::pseudo`] folds them into
//! pseudo-code. Values are 32-bit: constants wrap as `i32`, which is what the
//! consoles' address arithmetic does; a 64-bit operation is lifted on its low
//! 32 bits.
//!
//! [`Arch::lift`]: crate::Arch::lift
//! [`Arch::evaluate`]: crate::Arch::evaluate

use std::rc::Rc;

use crate::insn::Reg;

/// A two-operand operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// `a + b`, wrapping.
    Add,
    /// `a - b`, wrapping.
    Sub,
    /// The low 32 bits of `a * b`.
    Mul,
    /// `a & b`.
    And,
    /// `a | b`.
    Or,
    /// `a ^ b`.
    Xor,
    /// `a << b`.
    Shl,
    /// `a >> b`, logical (zeros in).
    Shr,
    /// `a >> b`, arithmetic (the sign in).
    Sar,
    /// 1 when `a == b`, else 0.
    Eq,
    /// 1 when `a != b`.
    Ne,
    /// 1 when `a < b`, signed.
    Lt,
    /// 1 when `a <= b`, signed.
    Le,
    /// 1 when `a > b`, signed.
    Gt,
    /// 1 when `a >= b`, signed.
    Ge,
    /// 1 when `a < b`, unsigned.
    Ltu,
    /// The low word of the 64-bit product (MIPS `lo` after `mult`).
    MulLo {
        /// A signed multiply.
        signed: bool,
    },
    /// The high word of the 64-bit product (MIPS `hi` after `mult`).
    MulHi {
        /// A signed multiply.
        signed: bool,
    },
    /// The quotient.
    Div {
        /// A signed divide.
        signed: bool,
    },
    /// The remainder.
    Rem {
        /// A signed divide.
        signed: bool,
    },
    /// Single-precision `a + b` (values are the floats' bits).
    FAdd,
    /// Single-precision `a - b`.
    FSub,
    /// Single-precision `a * b`.
    FMul,
    /// Single-precision `a / b`.
    FDiv,
    /// The larger float.
    FMax,
    /// The smaller float.
    FMin,
    /// 1 when the floats are equal.
    FEq,
    /// 1 when float `a < b`.
    FLt,
    /// 1 when float `a <= b`.
    FLe,
}

impl BinOp {
    /// How pseudo-code writes it: C's operator, `u` marking an unsigned
    /// form where C would need a cast (`u>>`, `u<`, `/u`).
    pub fn symbol(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            And => "&",
            Or => "|",
            Xor => "^",
            Shl => "<<",
            Shr => "u>>",
            Sar => ">>",
            Eq => "==",
            Ne => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            Ltu => "u<",
            MulLo { signed: true } | MulHi { signed: true } => "*",
            MulLo { signed: false } | MulHi { signed: false } => "*u",
            Div { signed: true } => "/",
            Div { signed: false } => "/u",
            Rem { signed: true } => "%",
            Rem { signed: false } => "%u",
            FAdd => "+",
            FSub => "-",
            FMul => "*",
            FDiv => "/",
            FMax => "fmax",
            FMin => "fmin",
            FEq => "==",
            FLt => "<",
            FLe => "<=",
        }
    }

    /// Whether pseudo-code writes it as a function, `name(a, b)`.
    pub fn is_function(self) -> bool {
        matches!(self, BinOp::FMax | BinOp::FMin)
    }

    /// The operation on 32-bit values; None for a divide by zero.
    pub fn apply(self, a: u32, b: u32) -> Option<u32> {
        use BinOp::*;
        let (sa, sb) = (a as i32, b as i32);
        Some(match self {
            Add => a.wrapping_add(b),
            Sub => a.wrapping_sub(b),
            Mul | MulLo { .. } => a.wrapping_mul(b),
            And => a & b,
            Or => a | b,
            Xor => a ^ b,
            Shl => a.wrapping_shl(b & 31),
            Shr => a.wrapping_shr(b & 31),
            Sar => sa.wrapping_shr(b & 31) as u32,
            Eq => (a == b).into(),
            Ne => (a != b).into(),
            Lt => (sa < sb).into(),
            Le => (sa <= sb).into(),
            Gt => (sa > sb).into(),
            Ge => (sa >= sb).into(),
            Ltu => (a < b).into(),
            MulHi { signed: true } => ((i64::from(sa) * i64::from(sb)) >> 32) as u32,
            MulHi { signed: false } => ((u64::from(a) * u64::from(b)) >> 32) as u32,
            Div { signed: true } => sa.checked_div(sb)? as u32,
            Div { signed: false } => a.checked_div(b)?,
            Rem { signed: true } => sa.checked_rem(sb)? as u32,
            Rem { signed: false } => a.checked_rem(b)?,
            FAdd | FSub | FMul | FDiv | FMax | FMin => {
                let (x, y) = (f32::from_bits(a), f32::from_bits(b));
                match self {
                    FAdd => x + y,
                    FSub => x - y,
                    FMul => x * y,
                    FDiv => x / y,
                    FMax => x.max(y),
                    _ => x.min(y),
                }
                .to_bits()
            }
            FEq => (f32::from_bits(a) == f32::from_bits(b)).into(),
            FLt => (f32::from_bits(a) < f32::from_bits(b)).into(),
            FLe => (f32::from_bits(a) <= f32::from_bits(b)).into(),
        })
    }
}

/// A one-operand operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    /// `-a`.
    Neg,
    /// `~a`.
    Not,
    /// Float `-a`.
    FNeg,
    /// Float `|a|`.
    FAbs,
    /// Float square root.
    FSqrt,
    /// A signed integer converted to a float.
    IntToFloat,
    /// A float converted to a signed integer, truncating.
    FloatToInt,
}

impl UnOp {
    /// The operation on 32-bit values (floats as their bits).
    pub fn apply(self, a: u32) -> u32 {
        let f = f32::from_bits(a);
        match self {
            UnOp::Neg => a.wrapping_neg(),
            UnOp::Not => !a,
            UnOp::FNeg => (-f).to_bits(),
            UnOp::FAbs => f.abs().to_bits(),
            UnOp::FSqrt => f.sqrt().to_bits(),
            UnOp::IntToFloat => (a as i32 as f32).to_bits(),
            UnOp::FloatToInt => f as i32 as u32,
        }
    }
}

/// A value, as a tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// A register's value before the instruction (in a lifted instruction),
    /// or as last written out (in pseudo-code).
    Reg(Reg),
    /// A constant.
    Const(i64),
    /// A two-operand operation.
    Bin(BinOp, Rc<Expr>, Rc<Expr>),
    /// A one-operand operation.
    Un(UnOp, Rc<Expr>),
    /// A load of `size` bytes from an address, sign-extended when `signed`.
    Load {
        /// Bytes read.
        size: u8,
        /// Sign-extended rather than zero-extended.
        signed: bool,
        /// The address.
        addr: Rc<Expr>,
    },
    /// `(a * b) >> n` in 64 bits: GCC's fixed-point multiply, recognised
    /// from its `lo`/`hi` halves by [`Expr::simplify`].
    MulShr(Rc<Expr>, Rc<Expr>, u32),
    /// `c ? a : b`: a conditional move (MIPS `movz`/`movn`).
    Cond(Rc<Expr>, Rc<Expr>, Rc<Expr>),
    /// A named temporary (pseudo-code's own).
    Var(String),
}

impl Expr {
    /// `a op b`.
    pub fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
        Expr::Bin(op, Rc::new(a), Rc::new(b))
    }

    /// `op a`.
    pub fn un(op: UnOp, a: Expr) -> Expr {
        Expr::Un(op, Rc::new(a))
    }

    /// A load of `size` bytes from `addr`.
    pub fn load(size: u8, signed: bool, addr: Expr) -> Expr {
        Expr::Load { size, signed, addr: Rc::new(addr) }
    }

    /// The value when every register it reads is `known`; None when one is
    /// not, or for a load or a temporary.
    pub fn eval(&self, known: &dyn Fn(Reg) -> Option<u32>) -> Option<u32> {
        match self {
            Expr::Reg(r) => known(*r),
            Expr::Const(v) => Some(*v as u32),
            Expr::Bin(op, a, b) => op.apply(a.eval(known)?, b.eval(known)?),
            Expr::Un(op, a) => Some(op.apply(a.eval(known)?)),
            Expr::MulShr(a, b, n) => {
                Some(((i64::from(a.eval(known)? as i32) * i64::from(b.eval(known)? as i32)) >> n) as u32)
            }
            Expr::Cond(c, a, b) => {
                if c.eval(known)? != 0 {
                    a.eval(known)
                } else {
                    b.eval(known)
                }
            }
            Expr::Load { .. } | Expr::Var(_) => None,
        }
    }

    /// The expression with every register replaced by what `value` gives
    /// for it.
    pub fn subst(&self, value: &dyn Fn(Reg) -> Expr) -> Expr {
        let f = |x: &Rc<Expr>| Rc::new(x.subst(value));
        match self {
            Expr::Reg(r) => value(*r),
            Expr::Bin(op, a, b) => Expr::Bin(*op, f(a), f(b)),
            Expr::Un(op, a) => Expr::Un(*op, f(a)),
            Expr::Load { size, signed, addr } => Expr::Load { size: *size, signed: *signed, addr: f(addr) },
            Expr::MulShr(a, b, n) => Expr::MulShr(f(a), f(b), *n),
            Expr::Cond(c, a, b) => Expr::Cond(f(c), f(a), f(b)),
            Expr::Const(_) | Expr::Var(_) => self.clone(),
        }
    }

    /// The expression with every copy of `from` replaced by `to`.
    pub fn replace(&self, from: &Expr, to: &Expr) -> Expr {
        if self == from {
            return to.clone();
        }
        let f = |x: &Rc<Expr>| Rc::new(x.replace(from, to));
        match self {
            Expr::Bin(op, a, b) => Expr::Bin(*op, f(a), f(b)),
            Expr::Un(op, a) => Expr::Un(*op, f(a)),
            Expr::Load { size, signed, addr } => Expr::Load { size: *size, signed: *signed, addr: f(addr) },
            Expr::MulShr(a, b, n) => Expr::MulShr(f(a), f(b), *n),
            Expr::Cond(c, a, b) => Expr::Cond(f(c), f(a), f(b)),
            _ => self.clone(),
        }
    }

    /// Whether it reads any of `regs`.
    pub fn mentions(&self, regs: &[Reg]) -> bool {
        match self {
            Expr::Reg(r) => regs.contains(r),
            Expr::Bin(_, a, b) | Expr::MulShr(a, b, _) => a.mentions(regs) || b.mentions(regs),
            Expr::Un(_, a) | Expr::Load { addr: a, .. } => a.mentions(regs),
            Expr::Cond(c, a, b) => c.mentions(regs) || a.mentions(regs) || b.mentions(regs),
            Expr::Const(_) | Expr::Var(_) => false,
        }
    }

    /// Whether it reads memory.
    pub fn has_load(&self) -> bool {
        match self {
            Expr::Load { .. } => true,
            Expr::Bin(_, a, b) | Expr::MulShr(a, b, _) => a.has_load() || b.has_load(),
            Expr::Un(_, a) => a.has_load(),
            Expr::Cond(c, a, b) => c.has_load() || a.has_load() || b.has_load(),
            _ => false,
        }
    }

    /// Nodes in the tree.
    pub fn size(&self) -> usize {
        match self {
            Expr::Bin(_, a, b) | Expr::MulShr(a, b, _) => 1 + a.size() + b.size(),
            Expr::Un(_, a) | Expr::Load { addr: a, .. } => 1 + a.size(),
            Expr::Cond(c, a, b) => 1 + c.size() + a.size() + b.size(),
            _ => 1,
        }
    }

    /// [`Expr::simplify`] applied bottom-up, through the whole tree.
    pub fn simplify_all(self) -> Expr {
        let f = |x: &Rc<Expr>| Rc::new(x.as_ref().clone().simplify_all());
        let e = match &self {
            Expr::Bin(op, a, b) => Expr::Bin(*op, f(a), f(b)),
            Expr::Un(op, a) => Expr::Un(*op, f(a)),
            Expr::Load { size, signed, addr } => Expr::Load { size: *size, signed: *signed, addr: f(addr) },
            Expr::MulShr(a, b, n) => Expr::MulShr(f(a), f(b), *n),
            Expr::Cond(c, a, b) => Expr::Cond(f(c), f(a), f(b)),
            _ => self,
        };
        e.simplify()
    }

    /// Folds constants and recognises idioms at the top of the tree: `x + 0`,
    /// `(x + a) + b`, constant arithmetic, and `(lo u>> n) | (hi << 32-n)`
    /// of one signed product as [`Expr::MulShr`].
    pub fn simplify(self) -> Expr {
        use BinOp::*;
        use Expr::*;
        let Bin(op, a, b) = &self else { return self };
        match (*op, a.as_ref(), b.as_ref()) {
            (_, Const(x), Const(y)) if !matches!(op, Div { .. } | Rem { .. }) || *y != 0 => {
                match op.apply(*x as u32, *y as u32) {
                    Some(v) => Const(i64::from(v as i32)),
                    None => self,
                }
            }
            (Add | Or | Xor | Sub | Shl | Shr | Sar, x, Const(0)) => x.clone(),
            // Constants to the right of a commutative operation, so that
            // offsets meet and fold: (c + x) + d is (x + c) + d is x + (c+d).
            (Add | Mul | And | Or | Xor, Const(_), x) if !matches!(x, Const(_)) => {
                Expr::bin(*op, x.clone(), a.as_ref().clone()).simplify()
            }
            (Add, Bin(Add, x, c1), Const(c2)) => match c1.as_ref() {
                Const(c1) => {
                    let c = i64::from((*c1 as i32).wrapping_add(*c2 as i32));
                    if c == 0 { x.as_ref().clone() } else { Expr::bin(Add, x.as_ref().clone(), Const(c)) }
                }
                _ => self,
            },
            (Or, Bin(Shr, l, n1), Bin(Shl, h, n2)) => match (l.as_ref(), h.as_ref(), n1.as_ref(), n2.as_ref()) {
                (Bin(MulLo { signed: true }, a1, b1), Bin(MulHi { signed: true }, a2, b2), Const(n), Const(m))
                    if a1 == a2 && b1 == b2 && n + m == 32 =>
                {
                    MulShr(a1.clone(), b1.clone(), *n as u32)
                }
                _ => self,
            },
            _ => self,
        }
    }
}

/// What a lifted instruction does, besides control flow. Every expression
/// reads the registers as they were before the instruction: the effects of
/// one instruction happen together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// A register gets a value.
    Set(Reg, Expr),
    /// `size` bytes of `value` are stored at `addr`.
    Store {
        /// Bytes written.
        size: u8,
        /// The address.
        addr: Expr,
        /// The value.
        value: Expr,
    },
    /// Something the IR does not model (a GTE command, a system call),
    /// printed as this text. Its register effects are the instruction's
    /// `reads` and `writes`.
    Opaque(String),
    /// The program stops here (MIPS `break`): code that only guards against
    /// it, such as GCC's divide-by-zero checks, is not shown.
    Abort,
}

/// One lifted instruction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lifted {
    /// What it does to registers and memory, together.
    pub effects: Vec<Effect>,
    /// For a conditional branch (or a conditional call), when it is taken.
    pub cond: Option<Expr>,
    /// For a jump or call through a register, the address.
    pub target: Option<Expr>,
}

impl Lifted {
    /// An instruction with these effects and no control flow of its own.
    pub fn effects(effects: Vec<Effect>) -> Lifted {
        Lifted { effects, ..Lifted::default() }
    }

    /// Every register the lifted instruction reads.
    pub fn reads(&self) -> Vec<Reg> {
        fn walk(e: &Expr, out: &mut Vec<Reg>) {
            match e {
                Expr::Reg(r) if !out.contains(r) => out.push(*r),
                Expr::Bin(_, a, b) | Expr::MulShr(a, b, _) => {
                    walk(a, out);
                    walk(b, out);
                }
                Expr::Un(_, a) | Expr::Load { addr: a, .. } => walk(a, out),
                Expr::Cond(c, a, b) => {
                    walk(c, out);
                    walk(a, out);
                    walk(b, out);
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for e in &self.effects {
            match e {
                Effect::Set(_, x) => walk(x, &mut out),
                Effect::Store { addr, value, .. } => {
                    walk(addr, &mut out);
                    walk(value, &mut out);
                }
                Effect::Opaque(_) | Effect::Abort => {}
            }
        }
        for x in self.cond.iter().chain(&self.target) {
            walk(x, &mut out);
        }
        out
    }

    /// Every register the lifted instruction sets.
    pub fn writes(&self) -> Vec<Reg> {
        self.effects
            .iter()
            .filter_map(|e| match e {
                Effect::Set(r, _) => Some(*r),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(i: u8) -> Expr {
        Expr::Reg(Reg::gpr(i))
    }

    #[test]
    fn evaluation_wraps_at_32_bits_and_refuses_unknowns() {
        let e = Expr::bin(BinOp::Add, Expr::Const(0x8001_0000), Expr::Const(-0x8000));
        assert_eq!(e.eval(&|_| None), Some(0x8000_8000));
        assert_eq!(Expr::bin(BinOp::Sar, Expr::Const(-8), Expr::Const(1)).eval(&|_| None), Some(-4i32 as u32));
        assert_eq!(Expr::bin(BinOp::Shr, Expr::Const(-8), Expr::Const(1)).eval(&|_| None), Some(0x7fff_fffc));
        assert_eq!(Expr::bin(BinOp::Ltu, Expr::Const(-1), Expr::Const(1)).eval(&|_| None), Some(0));
        assert_eq!(Expr::bin(BinOp::Div { signed: true }, Expr::Const(1), Expr::Const(0)).eval(&|_| None), None);
        assert_eq!(r(4).eval(&|_| None), None);
        assert_eq!(r(4).eval(&|_| Some(7)), Some(7));
        assert_eq!(Expr::load(4, true, Expr::Const(0)).eval(&|_| Some(1)), None);
    }

    #[test]
    fn simplify_folds_constants_offsets_and_the_fixed_point_multiply() {
        use BinOp::*;
        assert_eq!(
            Expr::bin(Add, Expr::Const(0x8001_0000), Expr::Const(0x10)).simplify(),
            Expr::Const(0x8001_0010u32 as i32 as i64)
        );
        assert_eq!(Expr::bin(Add, r(4), Expr::Const(0)).simplify(), r(4));
        let nested = Expr::bin(Add, Expr::bin(Add, r(4), Expr::Const(8)), Expr::Const(-8)).simplify();
        assert_eq!(nested, r(4));
        let (a, b) = (r(4), r(5));
        let lo = Expr::bin(MulLo { signed: true }, a.clone(), b.clone());
        let hi = Expr::bin(MulHi { signed: true }, a.clone(), b.clone());
        let fx = Expr::bin(Or, Expr::bin(Shr, lo, Expr::Const(12)), Expr::bin(Shl, hi, Expr::Const(20))).simplify();
        assert_eq!(fx, Expr::MulShr(Rc::new(a), Rc::new(b), 12));
        assert_eq!(fx.eval(&|r| Some(if r.index == 4 { 0x2000 } else { 0x3000 })), Some(0x6000));
    }

    #[test]
    fn lifted_reports_what_it_reads_and_writes() {
        let l = Lifted {
            effects: vec![
                Effect::Set(Reg::gpr(2), Expr::bin(BinOp::Add, r(4), r(5))),
                Effect::Store { size: 4, addr: r(29), value: r(6) },
            ],
            cond: Some(r(7)),
            target: None,
        };
        assert_eq!(l.reads(), [Reg::gpr(4), Reg::gpr(5), Reg::gpr(29), Reg::gpr(6), Reg::gpr(7)]);
        assert_eq!(l.writes(), [Reg::gpr(2)]);
    }
}
