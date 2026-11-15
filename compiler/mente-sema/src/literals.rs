//! The integer type an expression of literals alone takes (`core-calculus.md`, item 5;
//! [`literal-type-from-operands`]).
//!
//! A literal without a suffix has no type of its own. Arithmetic on literals is exact and needs
//! none, but `~`, `&`, `|`, `^` and a shift work on bits in one type, so under one of them an
//! expression of literals alone needs the type its literals take. Types flow one way, up from
//! the operands: [`taken`] gives each expression whose type is that of literals alone the type
//! of the smallest expression around it that has one, through arithmetic, the bit operations, a
//! shift's left operand, unary `-` and `~`, the branches of an `if`, the arms of a `match` and
//! the elements of an array literal, whose values make that expression's type, and the other
//! operand of a comparison; and `i64` where none has. The place a value is narrowed into never
//! gives a literal its type. [`value`] computes an expression of literals alone in those types,
//! for the check of a compared literal (`E0233`).
//!
//! The static ranges (`wide.rs`), Tier 1 and the lowering read this table. The reference
//! interpreter does not: it derives the same type from the expressions around the literals as
//! it evaluates, so that the comparison of the paths checks one against the other.
//!
//! [`literal-type-from-operands`]: ../../docs/design-review.md#literal-type-from-operands

use mente_syntax::tree::{
    ArmBody, AssignOp, BinaryOp, BlockId, Body, Else, ExprId, ExprKind, FnSyntax, Payload, StmtId,
    StmtKind, Tree, UnaryOp,
};

use crate::check::Typing;
use crate::types::{Int, IntTy, Ty, I64};

/// Where the literals of an expression of literals alone take their type from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum From {
    /// The other operands: an expression around it has an integer type.
    Operands,
    /// None: the type is `i64`.
    Alone,
    /// None, though the operands around it have types: no standard type holds them together,
    /// a `u128` and an `i8` say. The literal's value is exact where it does not depend on a
    /// type, and `E0212` where it does, under `~`, a bit operation or a shift; `ty` is then
    /// `i64` only as a placeholder, which no execution path reads (`core-calculus.md`, item 5).
    Unheld,
}

/// What the expression around a literal gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Around {
    /// No type: literals alone.
    Nothing,
    /// The type of the operands around it.
    Type(IntTy),
    /// Operands of types no standard type holds together.
    Unheld,
}

/// What an expression of type `t` gives the literals among its operands, an array's through its
/// elements: its type, or that no type holds its integers; none for literals alone or a type of
/// no integers.
fn around(t: &Ty) -> Option<Around> {
    match t {
        Ty::Int(Int::One(t)) | Ty::Int(Int::Mixed(Some(t))) => Some(Around::Type(*t)),
        Ty::Int(Int::Mixed(None)) => Some(Around::Unheld),
        Ty::Array(_, elem) => around(elem),
        Ty::Int(Int::Literal) | Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => None,
    }
}

/// The type the literals of an expression of literals alone take, and where from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taken {
    pub ty: IntTy,
    pub from: From,
}

/// The literal table of a function: for each expression, by its index, the type its literals
/// take where its own type is that of literals alone (`literals`), or an array of them at any
/// depth, as an array literal, a repeat, or a branch or an element read that gives one
/// (`arrays`); `None` for an expression of any other type.
pub struct Table {
    pub literals: Vec<Option<Taken>>,
    pub arrays: Vec<Option<Taken>>,
}

/// The literal table of `f`.
pub fn taken(f: &FnSyntax, typing: &Typing) -> Table {
    let mut w = Walk {
        tree: &f.tree,
        typing,
        out: vec![None; f.tree.exprs.len()],
        arrays: vec![None; f.tree.exprs.len()],
    };
    for c in &f.contracts {
        w.expr(c.expr, Around::Nothing);
    }
    match f.body {
        Some(Body::Expr(e)) => w.expr(e, Around::Nothing),
        Some(Body::Block(b)) => w.block(b, Around::Nothing),
        None => {}
    }
    Table {
        literals: w.out,
        arrays: w.arrays,
    }
}

/// Whether `t` is the type of literals alone, or an array of them at any depth.
fn of_literals(t: &Ty) -> bool {
    match t {
        Ty::Int(Int::Literal) => true,
        Ty::Array(_, e) => of_literals(e),
        Ty::Int(Int::One(_) | Int::Mixed(_))
        | Ty::Bool
        | Ty::Struct(_)
        | Ty::Enum(_)
        | Ty::Unit
        | Ty::Any => false,
    }
}

/// What an operation on operands of types `a` and `b` gives its literals, as [`around`] says:
/// the type of them all where both are integers, else `a`'s.
fn joined(a: &Ty, b: &Ty) -> Around {
    let int = |t: &Ty| match t {
        Ty::Int(i) => Some(*i),
        Ty::Bool | Ty::Array(..) | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => None,
    };
    let t = match int(a).zip(int(b)) {
        Some((x, y)) => around(&Ty::Int(x.join(y))),
        None => around(a),
    };
    t.unwrap_or(Around::Nothing)
}

/// An exact integer: whether it is negative, and its magnitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exact {
    pub negative: bool,
    pub magnitude: u128,
}

impl Exact {
    pub fn new(negative: bool, magnitude: u128) -> Exact {
        Exact {
            negative: negative && magnitude != 0,
            magnitude,
        }
    }

    pub(crate) fn negated(self) -> Exact {
        Exact::new(!self.negative, self.magnitude)
    }

    fn plus(self, o: Exact) -> Option<Exact> {
        if self.negative == o.negative {
            return Some(Exact::new(
                self.negative,
                self.magnitude.checked_add(o.magnitude)?,
            ));
        }
        Some(if self.magnitude >= o.magnitude {
            Exact::new(self.negative, self.magnitude - o.magnitude)
        } else {
            Exact::new(o.negative, o.magnitude - self.magnitude)
        })
    }

    fn times(self, o: Exact) -> Option<Exact> {
        Some(Exact::new(
            self.negative != o.negative,
            self.magnitude.checked_mul(o.magnitude)?,
        ))
    }

    /// Whether the value is one of type `t`.
    pub fn fits(self, t: IntTy) -> bool {
        if self.negative {
            self.magnitude <= t.min_magnitude()
        } else {
            self.magnitude <= t.max()
        }
    }

    /// The value's two's-complement bits in type `t`, which holds it.
    fn bits(self, t: IntTy) -> u128 {
        let raw = if self.negative {
            self.magnitude.wrapping_neg()
        } else {
            self.magnitude
        };
        raw & mask(t)
    }

    /// The value of type `t` whose bits are `bits`.
    fn of_bits(bits: u128, t: IntTy) -> Exact {
        let bits = bits & mask(t);
        let sign = 1u128 << (t.bits - 1);
        if t.signed && bits & sign != 0 {
            Exact::new(true, (bits ^ mask(t)) + 1)
        } else {
            Exact::new(false, bits)
        }
    }
}

/// Whether `e`, of a function whose expressions have the types `exprs`, is an operator of one
/// expression with its operands, whose literals take the type of every operand of that
/// expression that has one (`core-calculus.md`, item 5): arithmetic, unary `-`, and `~`, a bit
/// operation or a shift of literals, which have no type of their own; and a literal. A bit
/// operation with an operand of a type, or a shift whose shifted operand has one, works in that
/// type, so the expression of its literals ends there; so does it at a comparison, a call's
/// argument, a narrowing point, a branch's value and an index.
fn joins(tree: &Tree, exprs: &[Ty], e: ExprId) -> bool {
    let untyped = exprs[e.0 as usize] == Ty::Int(Int::Literal);
    match &tree.expr(e).kind {
        ExprKind::Int { .. } => true,
        ExprKind::Unary { op, .. } => match op {
            UnaryOp::Neg => true,
            UnaryOp::BitNot => untyped,
            UnaryOp::Not => false,
        },
        ExprKind::Binary { op, .. } => match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => true,
            BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::Shl
            | BinaryOp::Shr => untyped,
            BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => false,
        },
        ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Index { .. }
        | ExprKind::Field { .. }
        | ExprKind::Call { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Match { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::If { .. }
        | ExprKind::Error => false,
    }
}

/// The bits of type `t`.
fn mask(t: IntTy) -> u128 {
    u128::MAX >> (128 - t.bits)
}

/// The exact value of `e`, an expression of literals alone of function `f`: arithmetic exact,
/// and `~`, `&`, `|`, `^` and a shift in the type their literals take (`literals`, as
/// [`taken`] gives it). None where an operation traps, an operand outside the type it is
/// narrowed into, a zero divisor or a shift amount out of range, where the value leaves 128 bits,
/// and for an expression of any other kind.
pub fn value(tree: &Tree, literals: &[Option<Taken>], e: ExprId) -> Option<Exact> {
    let ty = || literals.get(e.0 as usize).copied().flatten().map(|t| t.ty);
    let of = |x: ExprId| value(tree, literals, x);
    match &tree.expr(e).kind {
        ExprKind::Int {
            value,
            suffix: None,
        } => Some(Exact::new(false, *value)),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => Some(of(*operand)?.negated()),
        ExprKind::Unary {
            op: UnaryOp::BitNot,
            operand,
        } => complement(of(*operand)?, ty()?),
        ExprKind::Binary { op, lhs, rhs } => apply(*op, of(*lhs)?, of(*rhs)?, ty()),
        ExprKind::Int {
            suffix: Some(_), ..
        }
        | ExprKind::Unary {
            op: UnaryOp::Not, ..
        }
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Index { .. }
        | ExprKind::Field { .. }
        | ExprKind::Call { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Match { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::If { .. }
        | ExprKind::Error => None,
    }
}

/// Why an operation over literals traps in the type it works in: an operand outside that type,
/// which it is narrowed into first; a shift by an amount not below the type's width; a shift
/// whose value leaves the type; or a zero divisor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    Outside(Exact),
    ShiftAmount(Exact),
    Shifted(Option<Exact>),
    ZeroDivisor,
}

/// Why `e`, an operation of a function whose operands are expressions of literals alone of one
/// value each, traps whenever it runs, in `t`, the type its literals take: `~`, `&`, `|`, `^`, a
/// shift, a division or a remainder. None where it does not trap, where an operand has no one
/// value, an operand of its own that traps included, and for an expression of any other kind;
/// so of an expression of literals alone, only the innermost operation that traps is named.
pub fn trap(tree: &Tree, literals: &[Option<Taken>], e: ExprId, t: IntTy) -> Option<Trap> {
    let of = |x: ExprId| value(tree, literals, x);
    let outside = |x: Exact| (!x.fits(t)).then_some(Trap::Outside(x));
    match &tree.expr(e).kind {
        ExprKind::Unary { op, operand } => match op {
            UnaryOp::BitNot => outside(of(*operand)?),
            UnaryOp::Neg | UnaryOp::Not => None,
        },
        ExprKind::Binary { op, lhs, rhs } => {
            let (a, b) = (of(*lhs)?, of(*rhs)?);
            match op {
                BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                    outside(a).or_else(|| outside(b))
                }
                BinaryOp::Shl | BinaryOp::Shr => {
                    let width = Exact::new(false, u128::from(t.bits));
                    let amount = b.negative || b.magnitude >= width.magnitude;
                    if amount {
                        return Some(Trap::ShiftAmount(b));
                    }
                    outside(a).or_else(|| {
                        let shifted = apply(*op, a, b, Some(t));
                        let k = 1u128 << b.magnitude;
                        let exact = a.times(Exact::new(false, k));
                        (*op == BinaryOp::Shl && shifted.is_none()).then_some(Trap::Shifted(exact))
                    })
                }
                BinaryOp::Div | BinaryOp::Rem => (b.magnitude == 0).then_some(Trap::ZeroDivisor),
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::And
                | BinaryOp::Or
                | BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => None,
            }
        }
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Index { .. }
        | ExprKind::Field { .. }
        | ExprKind::Call { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Match { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::If { .. }
        | ExprKind::Error => None,
    }
}

/// The most values [`values`] keeps of one expression.
const MOST_VALUES: usize = 64;

/// The values `e`, an expression of literals alone of a function, may have, each exact, as
/// [`value`] computes one: one for each branch of an `if`, each arm of a `match` and each
/// element of an array literal it indexes, in each combination where they are operands. None
/// where an operation traps, a value leaves 128 bits, there are more than [`MOST_VALUES`], and
/// for an expression of any other kind.
pub fn values(tree: &Tree, literals: &[Option<Taken>], e: ExprId) -> Option<Vec<Exact>> {
    let ty = literals.get(e.0 as usize).copied().flatten().map(|t| t.ty);
    let of = |x: ExprId| values(tree, literals, x);
    let mut out: Vec<Exact> = Vec::new();
    let mut add = |v: Exact| {
        if !out.contains(&v) {
            out.push(v);
        }
    };
    match &tree.expr(e).kind {
        ExprKind::Int { value, suffix } => match suffix {
            None => add(Exact::new(false, *value)),
            Some(_) => return None,
        },
        ExprKind::Unary { op, operand } => match op {
            UnaryOp::Neg => of(*operand)?.into_iter().for_each(|x| add(x.negated())),
            UnaryOp::BitNot => {
                for x in of(*operand)? {
                    add(complement(x, ty?)?);
                }
            }
            UnaryOp::Not => return None,
        },
        ExprKind::Binary { op, lhs, rhs } => {
            let (a, b) = (of(*lhs)?, of(*rhs)?);
            if a.len() * b.len() > MOST_VALUES {
                return None;
            }
            for x in &a {
                for y in &b {
                    add(apply(*op, *x, *y, ty)?);
                }
            }
        }
        ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::Index { .. } => {
            for alt in alternatives(tree, e) {
                if alt == e {
                    return None;
                }
                of(alt)?.into_iter().for_each(&mut add);
            }
        }
        ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Field { .. }
        | ExprKind::Call { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::Error => return None,
    }
    (out.len() <= MOST_VALUES).then_some(out)
}

/// The expressions whose value `e` takes: each branch's of an `if`, each arm's of a `match` and
/// each element's of an array literal it indexes, at every level of indexing and through those
/// nested in them, and `e` itself
/// for an expression of any other kind. A branch that ends in no value, by `return` or a trap,
/// gives none.
pub fn alternatives(tree: &Tree, e: ExprId) -> Vec<ExprId> {
    let mut out = Vec::new();
    collect_alternatives(tree, e, &mut out);
    out
}

fn collect_alternatives(tree: &Tree, e: ExprId, out: &mut Vec<ExprId>) {
    let tail = |b: BlockId, out: &mut Vec<ExprId>| {
        let Some(&s) = tree.block(b).stmts.last() else {
            return;
        };
        match &tree.stmt(s).kind {
            StmtKind::Expr(x) => collect_alternatives(tree, *x, out),
            StmtKind::Binding { .. }
            | StmtKind::Assign { .. }
            | StmtKind::While { .. }
            | StmtKind::Guard { .. }
            | StmtKind::Return(_)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Discard(_) => {}
        }
    };
    match &tree.expr(e).kind {
        ExprKind::If {
            then, otherwise, ..
        } => {
            tail(*then, out);
            match otherwise {
                Some(Else::Block(b)) => tail(*b, out),
                Some(Else::If(i)) => collect_alternatives(tree, *i, out),
                None => {}
            }
        }
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                match arm.body {
                    ArmBody::Expr(x) => collect_alternatives(tree, x, out),
                    ArmBody::Block(b) => tail(b, out),
                }
            }
        }
        // The arrays the base may be, through its own indexing, branches and arms, give their
        // elements: `[[300, 1], [1, 1]][k][j]` may be each of the four.
        ExprKind::Index { base, .. } => {
            let mut arrays = Vec::new();
            collect_alternatives(tree, *base, &mut arrays);
            for b in arrays {
                match &tree.expr(b).kind {
                    ExprKind::Array(items) => {
                        for &i in items {
                            collect_alternatives(tree, i, out);
                        }
                    }
                    ExprKind::Repeat { value, .. } => collect_alternatives(tree, *value, out),
                    ExprKind::Int { .. }
                    | ExprKind::Bool(_)
                    | ExprKind::Name(_)
                    | ExprKind::Todo(_)
                    | ExprKind::Unreachable(_)
                    | ExprKind::Unary { .. }
                    | ExprKind::Binary { .. }
                    | ExprKind::Index { .. }
                    | ExprKind::Field { .. }
                    | ExprKind::Call { .. }
                    | ExprKind::Construct { .. }
                    | ExprKind::Variant { .. }
                    | ExprKind::Match { .. }
                    | ExprKind::If { .. }
                    // A base that is no array literal, a branch that traps, is an alternative
                    // of its own, of no value known, as such a branch of an `if` is.
                    | ExprKind::Error => out.push(b),
                }
            }
        }
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Unary { .. }
        | ExprKind::Binary { .. }
        | ExprKind::Field { .. }
        | ExprKind::Call { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::Error => out.push(e),
    }
}

/// `~x` in type `t`: none where `x` lies outside it, since it is narrowed into it first.
fn complement(x: Exact, t: IntTy) -> Option<Exact> {
    Some(Exact::of_bits(!x.filter_fits(t)?.bits(t), t))
}

/// `a op b` for the arithmetic, bit and shift operators: arithmetic exact, a bit operation and
/// a shift in `t`, the type their literals take. None for any other operator, where an operation
/// traps and where the value leaves 128 bits.
pub(crate) fn apply(op: BinaryOp, a: Exact, b: Exact, t: Option<IntTy>) -> Option<Exact> {
    match op {
        BinaryOp::Add => a.plus(b),
        BinaryOp::Sub => a.plus(b.negated()),
        BinaryOp::Mul => a.times(b),
        // Truncated toward zero, the remainder with the dividend's sign.
        BinaryOp::Div => Some(Exact::new(
            a.negative != b.negative,
            a.magnitude.checked_div(b.magnitude)?,
        )),
        BinaryOp::Rem => Some(Exact::new(
            a.negative,
            a.magnitude.checked_rem(b.magnitude)?,
        )),
        BinaryOp::BitAnd => in_type(t?, a, b, |x, y| x & y),
        BinaryOp::BitOr => in_type(t?, a, b, |x, y| x | y),
        BinaryOp::BitXor => in_type(t?, a, b, |x, y| x ^ y),
        BinaryOp::Shl | BinaryOp::Shr => {
            let t = t?;
            let k = u32::try_from(b.magnitude)
                .ok()
                .filter(|k| !b.negative && *k < t.bits)?;
            let x = a.filter_fits(t)?;
            if op == BinaryOp::Shl {
                x.times(Exact::new(false, 1u128 << k)).filter(|v| v.fits(t))
            } else {
                // Rounded toward negative infinity.
                let q = x.magnitude >> k;
                let exact = q << k == x.magnitude;
                Some(Exact::new(x.negative, q + u128::from(x.negative && !exact)))
            }
        }
        BinaryOp::And
        | BinaryOp::Or
        | BinaryOp::Eq
        | BinaryOp::Ne
        | BinaryOp::Lt
        | BinaryOp::Le
        | BinaryOp::Gt
        | BinaryOp::Ge => None,
    }
}

/// A bit operation of `a` and `b` in type `t`: none where either lies outside it, since each is
/// narrowed into it first.
fn in_type(t: IntTy, a: Exact, b: Exact, op: fn(u128, u128) -> u128) -> Option<Exact> {
    let (x, y) = (a.filter_fits(t)?.bits(t), b.filter_fits(t)?.bits(t));
    Some(Exact::of_bits(op(x, y), t))
}

impl Exact {
    /// The value, when type `t` holds it.
    fn filter_fits(self, t: IntTy) -> Option<Exact> {
        self.fits(t).then_some(self)
    }
}

struct Walk<'a> {
    tree: &'a Tree,
    typing: &'a Typing,
    out: Vec<Option<Taken>>,
    arrays: Vec<Option<Taken>>,
}

impl Walk<'_> {
    /// Walks `e`. `around` is the integer type the expression around it gives its literals,
    /// where it gives one.
    fn expr(&mut self, e: ExprId, around: Around) {
        self.walk(e, around, false);
    }

    /// Walks `e`, an operand of an operator of one expression where `member`: its literals
    /// then take `around`, the type of every operand of that expression that has one
    /// ([`joins`]), not the type of `e` alone, so that the order of the operands never
    /// changes a literal's value (`core-calculus.md`, item 5).
    fn walk(&mut self, e: ExprId, around: Around, member: bool) {
        let tree = self.tree;
        let own = &self.typing.exprs[e.0 as usize];
        // What this expression gives the literals among its operands: within one expression the
        // type of all its operands, which the expression around it was given; elsewhere its own
        // type where it has one, since they are each other's other operands, and else what it
        // was given.
        let joined = joins(tree, &self.typing.exprs, e);
        let here = if member && joined {
            around
        } else {
            self::around(own).unwrap_or(around)
        };
        let taken = match here {
            Around::Type(ty) => Taken {
                ty,
                from: From::Operands,
            },
            Around::Nothing => Taken {
                ty: I64,
                from: From::Alone,
            },
            Around::Unheld => Taken {
                ty: I64,
                from: From::Unheld,
            },
        };
        // An array of literals gives its elements what it is given, at every depth, so its
        // literals take that type, whichever element they stand in.
        match own {
            Ty::Int(Int::Literal) => self.out[e.0 as usize] = Some(taken),
            Ty::Array(..) if of_literals(own) => self.arrays[e.0 as usize] = Some(taken),
            Ty::Int(Int::One(_) | Int::Mixed(_))
            | Ty::Bool
            | Ty::Array(..)
            | Ty::Struct(_)
            | Ty::Enum(_)
            | Ty::Unit
            | Ty::Any => {}
        }
        match &tree.expr(e).kind {
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Error => {}
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Not => self.expr(*operand, Around::Nothing),
                UnaryOp::Neg | UnaryOp::BitNot => self.walk(*operand, here, joined),
            },
            ExprKind::Binary { op, lhs, rhs } => match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Rem
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor => {
                    self.walk(*lhs, here, joined);
                    self.walk(*rhs, here, joined);
                }
                // A shift's amount is a count, of no type but its own.
                BinaryOp::Shl | BinaryOp::Shr => {
                    self.walk(*lhs, here, joined);
                    self.expr(*rhs, Around::Nothing);
                }
                // A comparison gives each operand the other's type.
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {
                    let ty = |x: &ExprId| {
                        self::around(&self.typing.exprs[x.0 as usize]).unwrap_or(Around::Nothing)
                    };
                    let (left, right) = (ty(rhs), ty(lhs));
                    self.expr(*lhs, left);
                    self.expr(*rhs, right);
                }
                BinaryOp::And | BinaryOp::Or => {
                    self.expr(*lhs, Around::Nothing);
                    self.expr(*rhs, Around::Nothing);
                }
            },
            // An array literal indexed gives its element's value, so its elements take what the
            // indexing is given, as an `if`'s branches do; the index takes none.
            ExprKind::Index { base, index } => {
                self.expr(*base, here);
                self.expr(*index, Around::Nothing);
            }
            ExprKind::Field { base, .. } => self.expr(*base, Around::Nothing),
            // An argument and a field's value are each narrowed into a place of their own,
            // which gives their literals no type.
            ExprKind::Call { args, .. } => {
                for a in args {
                    self.expr(a.value, Around::Nothing);
                }
            }
            ExprKind::Construct { fields, .. } => {
                for f in fields {
                    self.expr(f.value, Around::Nothing);
                }
            }
            ExprKind::Variant { payload, .. } => match payload {
                Payload::None => {}
                Payload::Named(fields) => {
                    for f in fields {
                        self.expr(f.value, Around::Nothing);
                    }
                }
                Payload::Positional(args) => {
                    for a in args {
                        self.expr(a.value, Around::Nothing);
                    }
                }
            },
            ExprKind::Match { scrutinee, arms } => {
                self.expr(*scrutinee, Around::Nothing);
                for arm in arms {
                    match arm.body {
                        ArmBody::Expr(x) => self.expr(x, here),
                        ArmBody::Block(b) => self.block(b, here),
                    }
                }
            }
            // The elements make the literal's type, so each is the others' operand.
            ExprKind::Array(items) => {
                for &i in items {
                    self.expr(i, here);
                }
            }
            ExprKind::Repeat { value, .. } => self.expr(*value, here),
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.expr(*cond, Around::Nothing);
                self.block(*then, here);
                match otherwise {
                    Some(Else::Block(b)) => self.block(*b, here),
                    Some(Else::If(i)) => self.expr(*i, here),
                    None => {}
                }
            }
        }
    }

    /// Walks a block whose value, its last statement's when that is an expression, takes
    /// `around`.
    fn block(&mut self, b: BlockId, around: Around) {
        let stmts = &self.tree.block(b).stmts;
        for (i, &s) in stmts.iter().enumerate() {
            let last = i + 1 == stmts.len();
            self.stmt(s, if last { around } else { Around::Nothing });
        }
    }

    fn stmt(&mut self, s: StmtId, around: Around) {
        let tree = self.tree;
        match &tree.stmt(s).kind {
            StmtKind::Binding { init, .. } => self.expr(*init, Around::Nothing),
            StmtKind::Assign { place, op, value } => {
                self.expr(*place, Around::Nothing);
                // `x op= e` is `x = x op e` (item 6): under arithmetic `e`'s operands are
                // operands of that expression with `x`, whose literals take the type of them
                // all; a bit operation works in `x`'s type, which its literal operand takes;
                // and a shift's amount takes none.
                let ty = |x: ExprId| &self.typing.exprs[x.0 as usize];
                let (around, member) = match op {
                    AssignOp::Compound(
                        BinaryOp::Add
                        | BinaryOp::Sub
                        | BinaryOp::Mul
                        | BinaryOp::Div
                        | BinaryOp::Rem,
                    ) => (joined(ty(*place), ty(*value)), true),
                    AssignOp::Compound(BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor) => {
                        (self::around(ty(*place)).unwrap_or(Around::Nothing), false)
                    }
                    AssignOp::Compound(
                        BinaryOp::Shl
                        | BinaryOp::Shr
                        | BinaryOp::And
                        | BinaryOp::Or
                        | BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge,
                    )
                    | AssignOp::Set => (Around::Nothing, false),
                };
                self.walk(*value, around, member);
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                self.expr(*cond, Around::Nothing);
                for &i in invariants.iter().chain(decreases) {
                    self.expr(i, Around::Nothing);
                }
                self.block(*body, Around::Nothing);
            }
            StmtKind::Guard { cond, otherwise } => {
                self.expr(*cond, Around::Nothing);
                self.block(*otherwise, Around::Nothing);
            }
            StmtKind::Return(value) => {
                if let Some(e) = value {
                    self.expr(*e, Around::Nothing);
                }
            }
            StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Discard(e) => self.expr(*e, Around::Nothing),
            // The value of a block is its last statement's.
            StmtKind::Expr(e) => self.expr(*e, around),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wide;

    /// The functions of a module, typed though the checker reports an error in them, as it does
    /// an operation over literals that traps (`E0234`), whose values these tests read.
    struct Typed {
        functions: Vec<Option<crate::CheckedFn>>,
    }

    /// The index, in the last function of `full`, of the first expression written `text`.
    fn at(full: &str, text: &str) -> (Typed, usize) {
        let module = mente_syntax::parse_module(full).unwrap_or_else(|d| panic!("{:?}", d));
        let m = Typed {
            functions: crate::bodies(full, &module, false).functions,
        };
        let f = m
            .functions
            .last()
            .and_then(|f| f.as_ref())
            .expect("a function");
        let tree = &f.syntax.tree;
        let at = (0..tree.exprs.len())
            .find(|&i| {
                let s = tree.exprs[i].span;
                &full[s.start..s.end] == text
            })
            .unwrap_or_else(|| panic!("no `{}` in {}", text, full));
        (m, at)
    }

    /// The type the literals of the first expression written `text` take in the last function
    /// of `src`, and where from; `None` when that expression has a type of its own.
    fn of(src: &str, text: &str) -> Option<(&'static str, From)> {
        let full = format!("module m\n{}\n", src);
        let (m, at) = at(&full, text);
        let f = m.functions.last().and_then(|f| f.as_ref())?;
        f.typing.literals[at].map(|t| (t.ty.name, t.from))
    }

    /// The other operands give the type, wherever the value goes: an expression around the
    /// literals that has a type gives it, through arithmetic, bit operations, a shift's left
    /// operand, unary `-` and `~`, the branches of an `if`, the arms of a `match` and the
    /// other elements of an array literal; and `x op= e` is `x = x op e`, so `x` gives `e`
    /// its type.
    #[test]
    fn test_literals_take_the_type_of_the_other_operands() {
        for (src, text, ty) in [
            ("pub fn f(a: u8) -> u8 = a & ~3", "~3", "u8"),
            ("pub fn f(a: u8) -> u8 = a & ~3", "3", "u8"),
            ("pub fn f(a: u8) -> u16 = a & ~3", "~3", "u8"),
            ("pub fn f(a: u8) -> u16 = a + ~3", "~3", "u8"),
            ("pub fn f(a: u8) -> u16 = (1 + ~3) + a", "~3", "u8"),
            ("pub fn f(a: u8) -> u16 = (1 + ~3) + a", "1 + ~3", "u8"),
            ("pub fn f(a: u8) -> u16 = a - -(~3)", "~3", "u8"),
            ("pub fn f(a: u8) -> u16 = a | (1 << 7)", "1 << 7", "u8"),
            ("pub fn f(a: u8) -> u16 = a | (1 << 7)", "1", "u8"),
            ("pub fn f(a: u8, b: u16) -> u32 = a + b + ~3", "~3", "u16"),
            (
                "pub fn f(a: u8, c: bool) -> u16 = if c { ~3 } else { a }",
                "~3",
                "u8",
            ),
            (
                "pub fn f(a: u8, c: bool) -> u16 = if c { a } else if c { 1 } else { ~3 }",
                "~3",
                "u8",
            ),
            (
                "pub fn f(a: u8) -> u16 = match a {\n  0 => ~3\n  _ => a\n}",
                "~3",
                "u8",
            ),
            (
                "pub fn f(a: u8) -> u16 = match a {\n  0 => {\n    ~3\n  }\n  _ => a\n}",
                "~3",
                "u8",
            ),
            ("pub fn f(a: u8) -> [2]u16 = [~3, a]", "~3", "u8"),
            ("pub fn f(a: u8) -> [2][1]u16 = [[~3], [a]]", "~3", "u8"),
            ("pub fn f(a: i128) -> i128 = a ^ ~0", "~0", "i128"),
            (
                "pub fn f() -> u16 {\n  var m: u16 = 0\n  m &= ~1\n  m\n}",
                "~1",
                "u16",
            ),
            (
                "pub fn f(xs: [2]u8) -> u8 {\n  var ys = xs\n  ys[0] ^= ~0\n  ys[0]\n}",
                "~0",
                "u8",
            ),
            (
                "pub fn f(c: bool, xs: [1]u8) -> [1]u8 = if c { [~3] } else { xs }",
                "~3",
                "u8",
            ),
        ] {
            assert_eq!(
                of(src, text),
                Some((ty, From::Operands)),
                "`{}` in {}",
                text,
                src
            );
        }
    }

    /// The place a value is narrowed into never gives a literal its type: a binding that
    /// states its type, an assigned place, a parameter, a field, a result, and the elements
    /// of an array any of these holds. Where no expression around the literals has a type,
    /// they are an `i64`, and the value is narrowed into the place as any value is. A
    /// compound assignment's place is the other operand of its right side, but not of a
    /// shift's amount.
    #[test]
    fn test_a_place_never_gives_a_literal_its_type() {
        let decls =
            "struct P {\n  m: u8\n}\nenum E {\n  V(y: u8)\n  W\n}\nfn mask(m: u8) -> u8 = m\n";
        for (src, text) in [
            ("pub fn f() -> u8 {\n  val m: u8 = ~3\n  m\n}", "~3"),
            ("pub fn f() -> u8 {\n  val m: u8 = 1 + ~3\n  m\n}", "~3"),
            ("pub fn f() -> u16 {\n  var m: u16 = 0\n  m = ~0x0F\n  m\n}", "~0x0F"),
            (
                "pub fn f() -> u16 {\n  var xs: [2]u16 = [0; 2]\n  xs[1] = 1 << 15\n  xs[0]\n}",
                "1 << 15",
            ),
            ("pub fn f() -> u16 {\n  var m: u16 = 1\n  m <<= ~0 & 3\n  m\n}", "~0 & 3"),
            ("pub fn f() -> u8 = mask(~3)", "~3"),
            ("pub fn f() -> u8 = P(m: ~3).m", "~3"),
            ("pub fn f() -> E = E.V(y: ~3)", "~3"),
            ("pub fn f() -> u8 = ~3", "~3"),
            ("pub fn f() -> u8 {\n  ~3\n}", "~3"),
            ("pub fn f(c: bool) -> u8 {\n  if c {\n    return ~3\n  }\n  0\n}", "~3"),
            ("pub fn f() -> u64 = 1 << 63", "1"),
            ("pub fn f() -> u64 = 1 | 9223372036854775808", "1 | 9223372036854775808"),
            (
                "pub fn f(c: bool) -> u8 {\n  val m: u8 = if c { ~3 } else { 4 }\n  m\n}",
                "~3",
            ),
            (
                "pub fn f(c: bool) -> u8 {\n  val m: u8 = match c {\n    true => ~3\n    false => 4\n  }\n  m\n}",
                "~3",
            ),
            ("pub fn f() -> [2]u8 = [~3, 1]", "~3"),
            ("pub fn f() -> [2]u8 = [~3; 2]", "~3"),
            ("pub fn f() -> [1][2]u8 = [[~3, 1]]", "~3"),
            (
                "pub fn f() -> [1][2]u8 {\n  val xs: [1][2]u8 = [[~3; 2]; 1]\n  xs\n}",
                "~3",
            ),
            ("pub fn f(c: bool) -> [1]u8 = if c { [~3] } else { [1] }", "~3"),
        ] {
            let src = format!("{}{}", decls, src);
            assert_eq!(
                of(&src, text),
                Some(("i64", From::Alone)),
                "`{}` in {}",
                text,
                src
            );
        }
    }

    /// Where there is neither, an expression of literals alone is an `i64`: a binding that
    /// states no type, an operand of a comparison, an index, a shift amount, a scrutinee, a
    /// discarded value and a contract clause. And an expression with a type of its own takes
    /// none.
    #[test]
    fn test_literals_alone_are_i64_and_a_typed_expression_takes_no_type() {
        for (src, text) in [
            ("pub fn f() -> i64 {\n  val m = ~3\n  m\n}", "~3"),
            ("pub fn f() -> i64 {\n  val xs = [~3, 1]\n  xs[0]\n}", "~3"),
            ("pub fn f(xs: [8]u8) -> u8 = xs[~3 & 7]", "~3 & 7"),
            ("pub fn f(a: u8) -> u8 = a << (~3 & 1)", "~3 & 1"),
            ("pub fn f() -> u8 = match ~3 {\n  0 => 1\n  _ => 2\n}", "~3"),
            ("pub fn f() -> u8 {\n  _ = ~3\n  0\n}", "~3"),
            (
                "pub fn f(c: bool) -> u8 {\n  if c {\n    _ = ~3\n  }\n  0\n}",
                "~3",
            ),
        ] {
            assert_eq!(
                of(src, text),
                Some(("i64", From::Alone)),
                "`{}` in {}",
                text,
                src
            );
        }
        for (src, text) in [
            ("pub fn f(a: u8) -> u8 = a & ~3", "a"),
            ("pub fn f(a: u8) -> u8 = a & ~3", "a & ~3"),
            ("pub fn f(a: u8) -> bool = a == ~3", "a == ~3"),
            ("pub fn f() -> u8 = ~3_u8", "~3_u8"),
            ("pub fn f() -> [2]u8 = [~3, 1]", "[~3, 1]"),
        ] {
            assert_eq!(of(src, text), None, "`{}` in {}", text, src);
        }
    }

    /// A literal takes the type of every operand that has one of the arithmetic or bit
    /// expression it stands in, whatever their order: `~3` beside a `u8` and a `u32` is a `u32`
    /// in each spelling, and beside a `u8` and an `i8` an `i16`. The expression ends at a bit
    /// operation with an operand of a type, at a comparison's side, at a call's argument and at
    /// a branch's value, which take their own; a branch of literals alone takes what the `if`
    /// is given.
    #[test]
    fn test_a_literal_takes_the_type_of_every_operand_of_its_expression() {
        let two = "pub fn f(a: u8, b: u32, c: bool, k: usize[0..2]) -> u64";
        for (src, text, ty, from) in [
            (format!("{} = a + ~3 + b", two), "~3", "u32", From::Operands),
            (format!("{} = a + b + ~3", two), "~3", "u32", From::Operands),
            (format!("{} = ~3 + a + b", two), "~3", "u32", From::Operands),
            (
                format!("{} = a * (b - ~3)", two),
                "~3",
                "u32",
                From::Operands,
            ),
            (
                "pub fn f(a: u8, c: i8) -> i16 = a + ~3 + c".to_string(),
                "~3",
                "i16",
                From::Operands,
            ),
            (
                format!("{} = a + (~3 & 7) + b", two),
                "~3 & 7",
                "u32",
                From::Operands,
            ),
            (
                format!("{} = (a & ~3) + b", two),
                "~3",
                "u8",
                From::Operands,
            ),
            (
                format!("{} = a + (if c {{ ~3 }} else {{ 0 }}) + b", two),
                "~3",
                "u32",
                From::Operands,
            ),
            (
                format!("{} = a + (if c {{ a + ~3 }} else {{ 0 }}) + b", two),
                "~3",
                "u8",
                From::Operands,
            ),
            (
                format!("{} = a + [~3, 1][k] + b", two),
                "~3",
                "u32",
                From::Operands,
            ),
            (
                "pub fn f(a: u8, b: u32) -> bool = a + ~3 == b".to_string(),
                "~3",
                "u8",
                From::Operands,
            ),
            (
                "fn id(x: i64) -> i64 = x\npub fn f(a: u8, b: u32) -> i64 = a + id(~3) + b"
                    .to_string(),
                "~3",
                "i64",
                From::Alone,
            ),
        ] {
            assert_eq!(of(&src, text), Some((ty, from)), "`{}` in {}", text, src);
        }
        // Operands no standard type holds together give their literals none.
        for (src, text) in [
            ("pub fn f(a: u128, c: i8) -> i128 = a + ~3 + c", "~3"),
            ("pub fn f(a: u128, c: i8) -> bool = a + c == ~3", "~3"),
            ("pub fn f(a: u128, c: i8) -> bool = a + c == 300", "300"),
        ] {
            assert_eq!(
                of(src, text).map(|t| t.1),
                Some(From::Unheld),
                "`{}` in {}",
                text,
                src
            );
        }
    }

    /// `x op= e` is `x = x op e`, for every compound operator: under arithmetic `e`'s literals
    /// take the type of `x` and of every operand of `e` that has one, wider than `x`'s too; a
    /// bit operation of `e` with an operand of a type works in that type; a compound bit
    /// operation gives its literal operand `x`'s type; and a shift's amount takes none.
    #[test]
    fn test_a_compound_assignment_gives_its_value_the_type_of_its_place_and_operands() {
        let body = |params: &str, place: &str, stmt: &str| {
            format!(
                "pub fn f({}) -> {} {{\n  var m = m0\n  {}\n  m\n}}",
                params, place, stmt
            )
        };
        let narrow = "m0: u32, a: u8";
        for (src, text, ty, from) in [
            (
                body(narrow, "u32", "m += a + ~3"),
                "~3",
                "u32",
                From::Operands,
            ),
            (
                body(narrow, "u32", "m -= a - ~3"),
                "~3",
                "u32",
                From::Operands,
            ),
            (
                body(narrow, "u32", "m *= a * ~3"),
                "~3",
                "u32",
                From::Operands,
            ),
            (
                body(narrow, "u32", "m /= a + ~3"),
                "~3",
                "u32",
                From::Operands,
            ),
            (
                body(narrow, "u32", "m %= a + ~3"),
                "~3",
                "u32",
                From::Operands,
            ),
            (body(narrow, "u32", "m += ~3"), "~3", "u32", From::Operands),
            (
                body("m0: u32, a: u8, b: u64", "u64", "m += a + ~3 + b"),
                "~3",
                "u64",
                From::Operands,
            ),
            (
                body(narrow, "u32", "m += (a & ~3) + 1"),
                "~3",
                "u8",
                From::Operands,
            ),
            (body("m0: u8", "u8", "m &= ~3"), "~3", "u8", From::Operands),
            (body("m0: u8", "u8", "m |= ~3"), "~3", "u8", From::Operands),
            (body("m0: u8", "u8", "m ^= ~3"), "~3", "u8", From::Operands),
            (
                body("m0: u8", "u8", "m <<= ~0 & 3"),
                "~0 & 3",
                "i64",
                From::Alone,
            ),
            (
                body("m0: u8", "u8", "m >>= ~0 & 3"),
                "~0 & 3",
                "i64",
                From::Alone,
            ),
        ] {
            assert_eq!(of(&src, text), Some((ty, from)), "`{}` in {}", text, src);
        }
    }

    /// A comparison gives each operand's literals the other operand's type, on either side,
    /// through the expression the literals stand in; two
    /// expressions of literals alone compared are each an `i64`, and so are the literals of a
    /// comparison's operand that a shift's amount or an index holds.
    #[test]
    fn test_a_compared_literal_takes_the_type_of_the_other_operand() {
        for (src, text, ty, from) in [
            (
                "pub fn f(a: u8) -> bool = a == ~3",
                "~3",
                "u8",
                From::Operands,
            ),
            (
                "pub fn f(a: u8) -> bool = ~3 != a",
                "~3",
                "u8",
                From::Operands,
            ),
            (
                "pub fn f(a: u8) -> bool = 252 < a",
                "252",
                "u8",
                From::Operands,
            ),
            (
                "pub fn f(a: u8) -> bool = a <= 3 + 4",
                "3 + 4",
                "u8",
                From::Operands,
            ),
            (
                "pub fn f(a: u8) -> bool = a + 1 > 255",
                "255",
                "u8",
                From::Operands,
            ),
            (
                "pub fn f(a: i16) -> bool = a >= -1",
                "-1",
                "i16",
                From::Operands,
            ),
            (
                "pub fn f(a: u8) -> u8\n  requires a < ~3\n= a",
                "~3",
                "u8",
                From::Operands,
            ),
            ("pub fn f() -> bool = ~3 == -4", "~3", "i64", From::Alone),
            ("pub fn f() -> bool = ~3 == -4", "-4", "i64", From::Alone),
            (
                "pub fn f(a: u8) -> bool = a == 1 << (~3 & 1)",
                "~3 & 1",
                "i64",
                From::Alone,
            ),
            (
                "pub fn f(a: u8, b: bool) -> bool = b == (a < 3)",
                "3",
                "u8",
                From::Operands,
            ),
            // An array literal indexed gives its elements what the indexing is given.
            (
                "pub fn f(a: u8, k: usize[0..2]) -> bool = a == [~3, 1][k]",
                "~3",
                "u8",
                From::Operands,
            ),
            (
                "pub fn f(a: u8, c: bool) -> bool = a == (if c { ~3 } else { 1 })",
                "~3",
                "u8",
                From::Operands,
            ),
        ] {
            assert_eq!(of(src, text), Some((ty, from)), "`{}` in {}", text, src);
        }
        // An operand of a type takes none from the other, a literal or not.
        for (src, text) in [
            ("pub fn f(a: u8) -> bool = a == ~3", "a"),
            ("pub fn f(a: u8, b: u32) -> bool = a == b + 1", "b + 1"),
        ] {
            assert_eq!(of(src, text), None, "`{}` in {}", text, src);
        }
    }

    /// The values an expression of literals alone may take: each branch's, arm's and indexed
    /// element's, in each combination under an operator, in the type their literals take, here
    /// a `u8`'s from the operand of the addition they stand in; none where an operand has a
    /// type of its own.
    #[test]
    fn test_the_values_of_literals_alone_are_each_branchs_arms_and_elements() {
        let values_of = |expr: &str| -> Option<Vec<(bool, u128)>> {
            let full = format!(
                "module m\npub fn f(a: u8, c: bool, k: usize[0..2]) -> u8 {{\n  _ = a + ({})\n  a\n}}\n",
                expr
            );
            let (m, at) = at(&full, expr);
            let f = m.functions.last().and_then(|f| f.as_ref())?;
            let mut out: Vec<(bool, u128)> =
                values(&f.syntax.tree, &f.typing.literals, ExprId(at as u32))?
                    .into_iter()
                    .map(|v| (v.negative, v.magnitude))
                    .collect();
            out.sort();
            Some(out)
        };
        for (expr, expected) in [
            (
                "if c { 300 } else { 1 }",
                Some(vec![(false, 1), (false, 300)]),
            ),
            (
                "(if c { ~3 } else { 1 }) + 2",
                Some(vec![(false, 3), (false, 254)]),
            ),
            (
                "if c { 1 } else if k == 0 { -2 } else { 3 * 4 }",
                Some(vec![(false, 1), (false, 12), (true, 2)]),
            ),
            (
                "[~3, 1, 7][k]",
                Some(vec![(false, 1), (false, 7), (false, 252)]),
            ),
            ("[~0; 2][k]", Some(vec![(false, 255)])),
            (
                "match a {\n    0 => 300\n    1 => {\n      ~3\n    }\n    _ => 2\n  }",
                Some(vec![(false, 2), (false, 252), (false, 300)]),
            ),
            ("if c { a } else { 1 }", None),
            // A branch that traps gives no value known, whether it is the value or the array
            // it is indexed from.
            ("if c { 1 } else { todo(\"x\") }", None),
            ("(if c { [1] } else { todo(\"x\") })[0]", None),
            // At most `MOST_VALUES` combinations are tried: eight by eight are, eight by nine
            // are not, however few their sums.
            (
                "[0, 1, 2, 3, 4, 5, 6, 7][k] + [0, 1, 2, 3, 4, 5, 6, 7][k]",
                Some((0..=14).map(|v| (false, v)).collect()),
            ),
            (
                "[0, 1, 2, 3, 4, 5, 6, 7][k] + [0, 1, 2, 3, 4, 5, 6, 7, 8][k]",
                None,
            ),
        ] {
            assert_eq!(values_of(expr), expected, "`{}`", expr);
        }
    }

    /// The exact value of an expression of literals alone: arithmetic exact, a bit operation
    /// and a shift in the type their literals take, here a `u8`'s or an `i8`'s from the operand
    /// beside them; none where an operation would trap.
    #[test]
    fn test_the_value_of_literals_alone_is_computed_in_the_type_they_take() {
        let value_in = |ty: &str, text: &str| -> Option<(bool, u128)> {
            let full = format!(
                "module m\npub fn f(a: {}) -> {} {{\n  _ = a + ({})\n  a\n}}\n",
                ty, ty, text
            );
            let (m, at) = at(&full, text);
            let f = m.functions.last().and_then(|f| f.as_ref())?;
            value(&f.syntax.tree, &f.typing.literals, ExprId(at as u32))
                .map(|v| (v.negative, v.magnitude))
        };
        let pos = |v: u128| Some((false, v));
        let neg = |v: u128| Some((true, v));
        for (ty, text, expected) in [
            ("u8", "7", pos(7)),
            ("u8", "-7", neg(7)),
            ("u8", "-0", pos(0)),
            ("u8", "3 + 4", pos(7)),
            ("u8", "3 - 4", neg(1)),
            ("u8", "-3 - 4", neg(7)),
            ("u8", "-3 + 4", pos(1)),
            ("u8", "4 - 4", pos(0)),
            ("u8", "20 * 15", pos(300)),
            ("u8", "-20 * 15", neg(300)),
            ("u8", "-20 * -15", pos(300)),
            ("u8", "7 / 2", pos(3)),
            ("u8", "-7 / 2", neg(3)),
            ("u8", "7 / -2", neg(3)),
            ("u8", "7 / 0", None),
            ("u8", "-7 % 2", neg(1)),
            ("u8", "7 % -2", pos(1)),
            ("u8", "7 % 0", None),
            ("u8", "~3", pos(252)),
            ("u8", "~0", pos(255)),
            ("u8", "~256", None),
            ("u8", "~-1", None),
            ("i8", "~3", neg(4)),
            ("i8", "~-128", pos(127)),
            ("i8", "~128", None),
            ("u8", "12 & 10", pos(8)),
            ("u8", "12 | 10", pos(14)),
            ("u8", "12 ^ 10", pos(6)),
            ("u8", "256 & 1", None),
            ("u8", "1 | 256", None),
            ("i8", "-1 & 5", pos(5)),
            ("i8", "-1 ^ 5", neg(6)),
            ("i8", "-128 | 0", neg(128)),
            ("u8", "1 << 7", pos(128)),
            ("u8", "1 << 8", None),
            ("u8", "2 << 7", None),
            ("u8", "256 << 0", None),
            ("u8", "1 << -1", None),
            ("u8", "128 >> 7", pos(1)),
            ("i8", "-7 >> 1", neg(4)),
            ("i8", "-8 >> 1", neg(4)),
            ("i8", "7 >> 1", pos(3)),
            ("i8", "-128 >> 7", neg(1)),
            ("i8", "-1 >> 8", None),
            ("u128", "~0", pos(u128::MAX)),
            ("i128", "~0", neg(1)),
            (
                "i128",
                "-170141183460469231731687303715884105728 | 0",
                neg(1 << 127),
            ),
        ] {
            assert_eq!(value_in(ty, text), expected, "`{}` beside {}", text, ty);
        }
        // A suffix, a name and a comparison are no literals alone.
        let full = "module m\npub fn f(a: u8) -> bool = a == 3_u8\n";
        let (m, at3) = at(full, "3_u8");
        let f = m.functions.last().and_then(|f| f.as_ref()).unwrap();
        assert_eq!(
            value(&f.syntax.tree, &f.typing.literals, ExprId(at3 as u32)),
            None
        );
        let (_, a) = at(full, "a");
        assert_eq!(
            value(&f.syntax.tree, &f.typing.literals, ExprId(a as u32)),
            None
        );
        let (_, all) = at(full, "a == 3_u8");
        assert_eq!(
            value(&f.syntax.tree, &f.typing.literals, ExprId(all as u32)),
            None
        );
    }

    /// The static range of a bit operation over literals alone is its value, computed in the
    /// type its literals take, as a literal's is, and the hull of its values where it may have
    /// more than one; so an operation beside it computes in a machine type that holds its value
    /// (`E0222`), and an array literal that holds it has a range to keep it in.
    #[test]
    fn test_a_bit_operation_on_literals_has_its_value_or_the_range_of_their_type() {
        let range = |src: &str, text: &str| {
            let full = format!("module m\n{}\n", src);
            let (m, at) = at(&full, text);
            let f = m.functions.last().and_then(|f| f.as_ref())?;
            wide::intervals(&f.syntax, &f.resolution, &f.typing)[at].map(|i| (i.lo, i.hi))
        };
        let point =
            |negative: bool, magnitude: u128| Some(((negative, magnitude), (negative, magnitude)));
        assert_eq!(
            range("pub fn f(a: u8) -> u16 = a + ~3", "~3"),
            point(false, 252)
        );
        assert_eq!(
            range("pub fn f(a: u8) -> u16 = a + ~3", "a + ~3"),
            Some(((false, 252), (false, 507)))
        );
        assert_eq!(
            range("pub fn f(a: u8) -> [2]u8 = [~3, a]", "~3"),
            point(false, 252)
        );
        // A place gives none: these are `i64`s narrowed where they land.
        assert_eq!(range("pub fn f() -> [2]u8 = [~3, 1]", "~3"), point(true, 4));
        assert_eq!(
            range("pub fn f() -> u8 = 1 << 7", "1 << 7"),
            point(false, 128)
        );
        assert_eq!(range("pub fn f() -> u8 = 3 & 5", "3 & 5"), point(false, 1));
        assert_eq!(
            range("pub fn f() -> i64 {\n  val m = ~3\n  m\n}", "~3"),
            point(true, 4)
        );
        // More than one value: their hull.
        assert_eq!(
            range(
                "pub fn f(a: u8, k: usize[0..2]) -> u16 = a + [~3, 1][k]",
                "[~3, 1][k]"
            ),
            Some(((false, 1), (false, 252)))
        );
    }
}
