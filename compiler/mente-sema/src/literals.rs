//! The integer type an expression of literals alone takes (`core-calculus.md`, item 5).
//!
//! A literal without a suffix has no type of its own. Arithmetic on literals is exact and needs
//! none, but `~`, `&`, `|`, `^` and a shift work on bits in one type, so under one of them an
//! expression of literals alone needs the type its literals take. [`taken`] gives it, for each
//! expression of a function whose type is that of literals alone:
//!
//! - the type of the smallest expression around it that has one: through arithmetic, the bit
//!   operations, a shift's left operand, unary `-` and `~`, and the branches of an `if` and the
//!   arms of a `match`, whose values share a type;
//! - where none has, the type of the place that expression is narrowed into, a binding's, an
//!   assigned place's, a parameter's, a result's, a field's or an element's, through the array
//!   literals around it;
//! - and `i64` where there is no such place: an operand of a comparison, an index, a shift
//!   amount, a scrutinee, a discarded value, a binding that states no type.
//!
//! The type checker records the place each expression is narrowed into ([`Typing::places`]);
//! the rest is read from the tree. The static ranges (`wide.rs`) and the lowering read this
//! table. The reference interpreter does not: it follows the same rule as it evaluates, so
//! that the comparison of the paths checks one against the other.

use mente_syntax::tree::{
    ArmBody, BinaryOp, BlockId, Body, Else, ExprId, ExprKind, FnSyntax, Payload, StmtId, StmtKind,
    Tree, UnaryOp,
};

use crate::check::Typing;
use crate::types::{Int, IntTy, Ty, I64};

/// Where the literals of an expression of literals alone take their type from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum From {
    /// The other operands: an expression around it has an integer type.
    Operands,
    /// The place the value is narrowed into.
    Place,
    /// Neither: the type is `i64`.
    Alone,
}

/// The type the literals of an expression of literals alone take, and where from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taken {
    pub ty: IntTy,
    pub from: From,
}

/// For each expression of `f`, by its index, the type its literals take when its own type is
/// that of literals alone, and `None` for an expression of any other type.
pub fn taken(f: &FnSyntax, typing: &Typing) -> Vec<Option<Taken>> {
    let mut w = Walk {
        tree: &f.tree,
        typing,
        out: vec![None; f.tree.exprs.len()],
    };
    for c in &f.contracts {
        w.expr(c.expr, None);
    }
    match f.body {
        Some(Body::Expr(e)) => w.expr(e, None),
        Some(Body::Block(b)) => w.block(b, None),
        None => {}
    }
    w.out
}

/// The integer type a type names: its one type, or the type that holds a mix.
fn int_ty(t: &Ty) -> Option<IntTy> {
    match t {
        Ty::Int(Int::One(t)) | Ty::Int(Int::Mixed(Some(t))) => Some(*t),
        Ty::Int(Int::Literal | Int::Mixed(None))
        | Ty::Bool
        | Ty::Array(..)
        | Ty::Struct(_)
        | Ty::Enum(_)
        | Ty::Unit
        | Ty::Any => None,
    }
}

/// The type the elements of an array of type `own` take: its element type where that names
/// one, and otherwise the element type of `here`, the type its own literals take.
fn element(own: &Ty, here: Option<&(Ty, From)>) -> Option<(Ty, From)> {
    let of = |t: &Ty| match t {
        Ty::Array(_, elem) => Some((**elem).clone()),
        Ty::Int(_) | Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => None,
    };
    match of(own) {
        // The other elements give the type, as the other operands of an operation do.
        Some(elem) if elem.settled() == elem && elem != Ty::Any => Some((elem, From::Operands)),
        Some(_) | None => here.and_then(|(t, from)| of(t).map(|elem| (elem, *from))),
    }
}

struct Walk<'a> {
    tree: &'a Tree,
    typing: &'a Typing,
    out: Vec<Option<Taken>>,
}

impl Walk<'_> {
    /// Walks `e`. `around` is the type the expression around it gives its literals, where it
    /// gives one, with where that type comes from.
    fn expr(&mut self, e: ExprId, around: Option<&(Ty, From)>) {
        let tree = self.tree;
        let own = &self.typing.exprs[e.0 as usize];
        // The place the value is narrowed into, where it is narrowed into one; else what the
        // expression around it gives.
        let landing: Option<(Ty, From)> = self.typing.places[e.0 as usize]
            .clone()
            .map(|t| (t, From::Place))
            .or(around.cloned());
        // What this expression gives its own operands: its type where it has one, since they
        // are each other's "other operands", and else what it was given.
        let here: Option<(Ty, From)> = match int_ty(own) {
            Some(t) => Some((Ty::int(t), From::Operands)),
            None => landing,
        };
        match own {
            Ty::Int(Int::Literal) => {
                let given = here
                    .as_ref()
                    .and_then(|(t, from)| Some((int_ty(t)?, *from)));
                let (ty, from) = given.unwrap_or((I64, From::Alone));
                self.out[e.0 as usize] = Some(Taken { ty, from });
            }
            Ty::Int(Int::One(_) | Int::Mixed(_))
            | Ty::Bool
            | Ty::Array(..)
            | Ty::Struct(_)
            | Ty::Enum(_)
            | Ty::Unit
            | Ty::Any => {}
        }
        let here = here.as_ref();
        match &tree.expr(e).kind {
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Error => {}
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Not => self.expr(*operand, None),
                UnaryOp::Neg | UnaryOp::BitNot => self.expr(*operand, here),
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
                    self.expr(*lhs, here);
                    self.expr(*rhs, here);
                }
                // A shift's amount is a count, of no type but its own.
                BinaryOp::Shl | BinaryOp::Shr => {
                    self.expr(*lhs, here);
                    self.expr(*rhs, None);
                }
                // A comparison compares exact values, across types: it gives none.
                BinaryOp::And
                | BinaryOp::Or
                | BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {
                    self.expr(*lhs, None);
                    self.expr(*rhs, None);
                }
            },
            ExprKind::Index { base, index } => {
                self.expr(*base, None);
                self.expr(*index, None);
            }
            ExprKind::Field { base, .. } => self.expr(*base, None),
            // Each argument and each field's value is narrowed into a place of its own.
            ExprKind::Call { args, .. } => {
                for a in args {
                    self.expr(a.value, None);
                }
            }
            ExprKind::Construct { fields, .. } => {
                for f in fields {
                    self.expr(f.value, None);
                }
            }
            ExprKind::Variant { payload, .. } => match payload {
                Payload::None => {}
                Payload::Named(fields) => {
                    for f in fields {
                        self.expr(f.value, None);
                    }
                }
                Payload::Positional(args) => {
                    for a in args {
                        self.expr(a.value, None);
                    }
                }
            },
            ExprKind::Match { scrutinee, arms } => {
                self.expr(*scrutinee, None);
                for arm in arms {
                    match arm.body {
                        ArmBody::Expr(x) => self.expr(x, here),
                        ArmBody::Block(b) => self.block(b, here),
                    }
                }
            }
            ExprKind::Array(items) => {
                let elem = element(own, here);
                for &i in items {
                    self.expr(i, elem.as_ref());
                }
            }
            ExprKind::Repeat { value, .. } => {
                let elem = element(own, here);
                self.expr(*value, elem.as_ref());
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.expr(*cond, None);
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
    fn block(&mut self, b: BlockId, around: Option<&(Ty, From)>) {
        let stmts = &self.tree.block(b).stmts;
        for (i, &s) in stmts.iter().enumerate() {
            let last = i + 1 == stmts.len();
            self.stmt(s, if last { around } else { None });
        }
    }

    fn stmt(&mut self, s: StmtId, around: Option<&(Ty, From)>) {
        let tree = self.tree;
        match &tree.stmt(s).kind {
            StmtKind::Binding { init, .. } => self.expr(*init, None),
            StmtKind::Assign { place, value, .. } => {
                self.expr(*place, None);
                self.expr(*value, None);
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                self.expr(*cond, None);
                for &i in invariants.iter().chain(decreases) {
                    self.expr(i, None);
                }
                self.block(*body, None);
            }
            StmtKind::Guard { cond, otherwise } => {
                self.expr(*cond, None);
                self.block(*otherwise, None);
            }
            StmtKind::Return(value) => {
                if let Some(e) = value {
                    self.expr(*e, None);
                }
            }
            StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Discard(e) => self.expr(*e, None),
            // The value of a block is its last statement's.
            StmtKind::Expr(e) => self.expr(*e, around),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wide;

    /// The index, in the last function of `full`, of the first expression written `text`.
    fn at(full: &str, text: &str) -> (crate::CheckedModule, usize) {
        let m = crate::check_program_without_proofs(full).unwrap_or_else(|d| panic!("{:?}", d));
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

    /// The other operands come first, wherever the value goes: an expression around the
    /// literals that has a type gives it, through arithmetic, bit operations, a shift's left
    /// operand, unary `-` and `~`, the branches of an `if`, the arms of a `match` and the
    /// other elements of an array literal.
    #[test]
    fn test_literals_take_the_type_of_the_other_operands_first() {
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

    /// Where no expression around them has a type, the literals take the type of the place
    /// the value is narrowed into: a binding that states its type, an assigned place, the
    /// place of a compound assignment, a parameter, a field, a result, and the elements of
    /// an array any of these holds.
    #[test]
    fn test_literals_take_the_type_of_the_place_next() {
        let decls =
            "struct P {\n  m: u8\n}\nenum E {\n  V(y: u8)\n  W\n}\nfn mask(m: u8) -> u8 = m\n";
        for (src, text, ty) in [
            ("pub fn f() -> u8 {\n  val m: u8 = ~3\n  m\n}", "~3", "u8"),
            (
                "pub fn f() -> u8 {\n  val m: u8 = 1 + ~3\n  m\n}",
                "~3",
                "u8",
            ),
            (
                "pub fn f() -> u16 {\n  var m: u16 = 0\n  m = ~0x0F\n  m\n}",
                "~0x0F",
                "u16",
            ),
            (
                "pub fn f() -> u16 {\n  var m: u16 = 0\n  m &= ~1\n  m\n}",
                "~1",
                "u16",
            ),
            (
                "pub fn f() -> u16 {\n  var xs: [2]u16 = [0; 2]\n  xs[1] = 1 << 15\n  xs[0]\n}",
                "1 << 15",
                "u16",
            ),
            ("pub fn f() -> u8 = mask(~3)", "~3", "u8"),
            ("pub fn f() -> u8 = P(m: ~3).m", "~3", "u8"),
            ("pub fn f() -> E = E.V(y: ~3)", "~3", "u8"),
            ("pub fn f() -> u8 = ~3", "~3", "u8"),
            ("pub fn f() -> u8 {\n  ~3\n}", "~3", "u8"),
            (
                "pub fn f(c: bool) -> u8 {\n  if c {\n    return ~3\n  }\n  0\n}",
                "~3",
                "u8",
            ),
            ("pub fn f() -> u64 = 1 << 63", "1", "u64"),
            (
                "pub fn f() -> u64 = 1 | 9223372036854775808",
                "1 | 9223372036854775808",
                "u64",
            ),
            (
                "pub fn f(c: bool) -> u8 {\n  val m: u8 = if c { ~3 } else { 4 }\n  m\n}",
                "~3",
                "u8",
            ),
            (
                "pub fn f(c: bool) -> u8 {\n  val m: u8 = match c {\n    true => ~3\n    false => 4\n  }\n  m\n}",
                "~3",
                "u8",
            ),
            ("pub fn f() -> [2]u8 = [~3, 1]", "~3", "u8"),
            ("pub fn f() -> [2]u8 = [~3; 2]", "~3", "u8"),
            ("pub fn f() -> [1][2]u8 = [[~3, 1]]", "~3", "u8"),
            (
                "pub fn f() -> [1][2]u8 {\n  val xs: [1][2]u8 = [[~3; 2]; 1]\n  xs\n}",
                "~3",
                "u8",
            ),
            (
                "pub fn f(c: bool) -> [1]u8 = if c { [~3] } else { [1] }",
                "~3",
                "u8",
            ),
        ] {
            let src = format!("{}{}", decls, src);
            assert_eq!(
                of(&src, text),
                Some((ty, From::Place)),
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
            ("pub fn f(a: u8) -> bool = a == ~3", "~3"),
            ("pub fn f(a: u8) -> bool = ~3 < a", "~3"),
            ("pub fn f(xs: [8]u8) -> u8 = xs[~3 & 7]", "~3 & 7"),
            ("pub fn f(a: u8) -> u8 = a << (~3 & 1)", "~3 & 1"),
            ("pub fn f() -> u8 = match ~3 {\n  0 => 1\n  _ => 2\n}", "~3"),
            ("pub fn f() -> u8 {\n  _ = ~3\n  0\n}", "~3"),
            ("pub fn f(a: u8) -> u8\n  requires a < ~3\n= a", "~3"),
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

    /// The static range of a bit operation over literals alone is that of the type its
    /// literals take, so that an operation beside it computes in a machine type that holds
    /// its value (`E0222`), and an array literal that holds it has a range to keep it in.
    #[test]
    fn test_a_bit_operation_on_literals_has_the_range_of_the_type_they_take() {
        let range = |src: &str, text: &str| {
            let full = format!("module m\n{}\n", src);
            let (m, at) = at(&full, text);
            let f = m.functions.last().and_then(|f| f.as_ref())?;
            wide::intervals(&f.syntax, &f.resolution, &f.typing)[at].map(|i| (i.lo, i.hi))
        };
        let u8_ = Some(((false, 0), (false, 255)));
        assert_eq!(range("pub fn f(a: u8) -> u16 = a + ~3", "~3"), u8_);
        assert_eq!(
            range("pub fn f(a: u8) -> u16 = a + ~3", "a + ~3"),
            Some(((false, 0), (false, 510)))
        );
        assert_eq!(range("pub fn f() -> [2]u8 = [~3, 1]", "~3"), u8_);
        assert_eq!(range("pub fn f() -> u8 = 1 << 7", "1 << 7"), u8_);
        assert_eq!(range("pub fn f() -> u8 = 3 & 5", "3 & 5"), u8_);
        assert_eq!(
            range("pub fn f() -> i64 {\n  val m = ~3\n  m\n}", "~3"),
            Some(((true, 1 << 63), (false, (1 << 63) - 1)))
        );
    }
}
