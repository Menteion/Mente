//! The integer type of each expression of a function, as the interpreter derives it on its own
//! (`spec/core-calculus.md`, item 5).
//!
//! The checker gives every expression a type, and MIR and native code read it, among them the
//! type of an expression of operators, which its literals take. The interpreter reads from the
//! checker only the types of the leaves of such an expression, the names, calls, fields and
//! literals among its operands, a suffixed literal's included, and derives the type of every
//! operator, branch, arm and indexed element over them here, with a join of its own taken from
//! the ranges of the types; so a slip in the checker's join, or in which operators make one
//! expression, shows as a difference when the paths are compared.

use mente_sema::types::{Int as IntType, IntTy, Ty, INT_TYPES};
use mente_sema::CheckedFn;
use mente_syntax::tree::{
    ArmBody, BinaryOp, BlockId, Else, ExprId, ExprKind, StmtKind, Tree, UnaryOp,
};

/// The integer type of an expression as derived here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Literals alone, which have no type of their own; and an expression of no type (item 6),
    /// which never gives a value and so gives its operator no type either.
    Literal,
    /// Integers of a type, an array's through its elements: the smallest standard type that
    /// holds the range of every operand's, where one does.
    Typed(Option<IntTy>),
    /// No integer: a `bool`, a struct, an enum or unit.
    Other,
}

impl Kind {
    /// The type an expression of this kind gives the literals among its operands.
    pub(crate) fn ty(self) -> Option<IntTy> {
        match self {
            Kind::Typed(t) => t,
            Kind::Literal | Kind::Other => None,
        }
    }

    /// The kind of a leaf of static type `t`, an array's through its elements.
    pub(crate) fn of(t: &Ty) -> Kind {
        match t {
            Ty::Int(IntType::Literal) | Ty::Any => Kind::Literal,
            Ty::Int(IntType::One(t)) => Kind::Typed(Some(*t)),
            Ty::Int(IntType::Mixed(t)) => Kind::Typed(*t),
            Ty::Array(_, elem) => Kind::of(elem),
            Ty::Bool | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit => Kind::Other,
        }
    }

    /// The kind of an operator over operands of kinds `self` and `other`: literals take the
    /// other operand's; two types make the smallest standard type that holds both.
    pub(crate) fn join(self, other: Kind) -> Kind {
        match (self, other) {
            (Kind::Literal, k) | (k, Kind::Literal) => k,
            (Kind::Typed(Some(a)), Kind::Typed(Some(b))) => Kind::Typed(holding(a, b)),
            (Kind::Typed(None), Kind::Typed(_)) | (Kind::Typed(Some(_)), Kind::Typed(None)) => {
                Kind::Typed(None)
            }
            (Kind::Other, Kind::Typed(_) | Kind::Other) | (Kind::Typed(_), Kind::Other) => {
                Kind::Other
            }
        }
    }
}

/// The smallest standard type whose range holds both `a`'s and `b`'s, `a` or `b` itself where
/// it does, if one does.
fn holding(a: IntTy, b: IntTy) -> Option<IntTy> {
    let holds = |t: IntTy, u: IntTy| t.max() >= u.max() && t.min_magnitude() >= u.min_magnitude();
    [a, b]
        .into_iter()
        .chain(INT_TYPES)
        .find(|&t| holds(t, a) && holds(t, b))
}

/// The kind of every expression of `f`, by its id.
pub(crate) fn kinds(f: &CheckedFn) -> Vec<Kind> {
    let tree = &f.syntax.tree;
    let exprs = &f.typing.exprs;
    let mut out = vec![None; exprs.len()];
    for i in 0..exprs.len() {
        kind(tree, exprs, ExprId(i as u32), &mut out);
    }
    out.into_iter().map(|k| k.unwrap_or(Kind::Other)).collect()
}

/// The kind of `e`: an operator's from its operands', a leaf's from its static type.
fn kind(tree: &Tree, exprs: &[Ty], e: ExprId, out: &mut [Option<Kind>]) -> Kind {
    if let Some(k) = out[e.0 as usize] {
        return k;
    }
    let mut of = |x: ExprId| kind(tree, exprs, x, out);
    let k = match &tree.expr(e).kind {
        ExprKind::Unary { op, operand } => match op {
            UnaryOp::Neg | UnaryOp::BitNot => of(*operand),
            UnaryOp::Not => Kind::Other,
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
                let l = of(*lhs);
                l.join(of(*rhs))
            }
            // A shift's amount is a count, which gives the shift no type.
            BinaryOp::Shl | BinaryOp::Shr => of(*lhs),
            BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => Kind::Other,
        },
        // The elements of an array literal make its element type, each the others' operand.
        ExprKind::Array(items) => items.iter().fold(Kind::Literal, |k, &i| k.join(of(i))),
        ExprKind::Repeat { value, .. } => of(*value),
        // An element read from an array has the array's element kind.
        ExprKind::Index { base, .. } => of(*base),
        // The branches and the arms give each other their kinds, a branch that ends in no value
        // none; an `if` without `else` gives no integer.
        ExprKind::If {
            then, otherwise, ..
        } => {
            let first = tail(tree, *then).map_or(Kind::Literal, &mut of);
            match otherwise {
                Some(Else::Block(b)) => first.join(tail(tree, *b).map_or(Kind::Literal, &mut of)),
                Some(Else::If(i)) => first.join(of(*i)),
                None => Kind::Other,
            }
        }
        ExprKind::Match { arms, .. } => arms.iter().fold(Kind::Literal, |k, arm| {
            let body = match arm.body {
                ArmBody::Expr(x) => Some(x),
                ArmBody::Block(b) => tail(tree, b),
            };
            k.join(body.map_or(Kind::Literal, &mut of))
        }),
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Field { .. }
        | ExprKind::Call { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Error => Kind::of(&exprs[e.0 as usize]),
    };
    out[e.0 as usize] = Some(k);
    k
}

/// The expression whose value block `b` gives, its last statement's; none where it ends in no
/// value.
fn tail(tree: &Tree, b: BlockId) -> Option<ExprId> {
    let s = *tree.block(b).stmts.last()?;
    match &tree.stmt(s).kind {
        StmtKind::Expr(x) => Some(*x),
        StmtKind::Binding { .. }
        | StmtKind::Assign { .. }
        | StmtKind::While { .. }
        | StmtKind::Guard { .. }
        | StmtKind::Return(_)
        | StmtKind::Break
        | StmtKind::Continue
        | StmtKind::Discard(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str) -> IntTy {
        IntTy::named(name).expect("a standard type")
    }

    #[test]
    fn test_the_interpreter_joins_types_by_their_ranges() {
        let typed = |n: &str| Kind::Typed(Some(t(n)));
        let cases = [
            (typed("u8"), typed("u8"), typed("u8")),
            (typed("u8"), typed("u32"), typed("u32")),
            (typed("u32"), typed("u8"), typed("u32")),
            (typed("u8"), typed("i8"), typed("i16")),
            (typed("i32"), typed("u32"), typed("i64")),
            (typed("u64"), typed("i64"), typed("i128")),
            (typed("usize"), typed("u8"), typed("usize")),
            (typed("u8"), typed("usize"), typed("usize")),
            (typed("u128"), typed("i8"), Kind::Typed(None)),
            (Kind::Typed(None), typed("u8"), Kind::Typed(None)),
            (Kind::Literal, typed("u16"), typed("u16")),
            (typed("u16"), Kind::Literal, typed("u16")),
            (Kind::Literal, Kind::Literal, Kind::Literal),
            (Kind::Other, typed("u8"), Kind::Other),
        ];
        for (a, b, joined) in cases {
            assert_eq!(a.join(b), joined, "{:?} with {:?}", a, b);
        }
    }
}
