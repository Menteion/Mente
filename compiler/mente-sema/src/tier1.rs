//! Tier 1 for one function of M1 (`spec/core-calculus.md`, item 8;
//! `spec/ghost-contracts-and-totality.md`, items 2 and 4), ported from the M0b spike
//! (`spikes/tier1-latency/src/tier1.rs`) onto the typed tree.
//!
//! [`check`] runs an abstract interpreter over one function whose names and types check: an
//! octagon over its integer bindings, plus a variable for each call argument and result and for
//! some intermediate values. Facts come from the bindings' types and refinements, the function's
//! `requires`, conditions, assignments, the callees' `ensures` and refined results, and loop
//! heads, where Houdini keeps the inductive candidates from the loop's own text. Each obligation
//! is recorded as proven or not. One Tier 1 must prove and cannot is an error that names a
//! valuation Tier 1 cannot rule out, never an input that fails ([`tier1-counterexample`]), with a
//! `guard` as its suggested fix; one that may stay a run-time check is only recorded.
//!
//! [`check_with_queries`] also writes each obligation Tier 1 discharges as an SMT-LIB 2 query, for
//! the SMT cross-check in CI ([`smt`]).
//!
//! [`tier1-counterexample`]: ../../docs/design-review.md#tier1-counterexample

use std::collections::BTreeSet;

use mente_syntax::diagnostics::{line_col, Applicability, Diagnostic, Edit, Fix};
use mente_syntax::tree::{
    Arg, Arm, ArmBody, AssignOp, BinaryOp, BlockId, Body, Bound, ContractKind, Else, ExprId,
    ExprKind, FnSyntax, PatternKind, Payload, Refinement, StmtId, StmtKind, Tree, TypeId, TypeKind,
    UnaryOp,
};
use mente_syntax::{Attr, Span};

use crate::check::Typing;
use crate::names::{BindingKind, Resolution};
use crate::octagon::{Exact, Octagon, Term, BIG, INF, RELATIONAL_LIMIT};
use crate::types::{Int, IntTy, Ty};

pub mod smt;

use smt::{Goal, Path};

/// How a function routes its clauses outside the Tier-1 fragment: `@checked` makes them
/// run-time checks, `@prove` trusted axioms until `mn prove` runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Routing {
    pub checked: bool,
    pub prove: bool,
}

impl Routing {
    pub fn of(attrs: &[Attr]) -> Routing {
        Routing {
            checked: attrs.iter().any(|a| a.name == "checked"),
            prove: attrs.iter().any(|a| a.name == "prove"),
        }
    }
}

/// What Tier 1 reads of a function it calls: its header, never its body.
#[derive(Clone, Copy, Debug)]
pub struct Callee<'a> {
    pub syntax: &'a FnSyntax,
}

/// The function Tier 1 checks, with its names and types.
#[derive(Clone, Copy, Debug)]
pub struct Function<'a> {
    pub syntax: &'a FnSyntax,
    pub resolution: &'a Resolution,
    pub typing: &'a Typing,
    pub routing: Routing,
    /// The module's structs, whose fields' refinements Tier 1 proves and takes as facts.
    pub structs: &'a crate::structs::Structs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObligationKind {
    /// A value entering a refined type: must be proven.
    Refinement,
    /// A value entering a plain integer type: may stay a run-time check.
    Range,
    /// An index inside its array: may stay a run-time check.
    Index,
    /// A divisor that is not zero: may stay a run-time check.
    Divisor,
    /// A shift amount within its operand's width: may stay a run-time check.
    Shift,
    /// A callee's `requires` at a call: must be proven.
    Requires,
    /// An `ensures` at a return: must be proven.
    Ensures,
    /// A written loop `invariant`: must be proven.
    Invariant,
    /// A `decreases` measure, never negative and smaller at each round or recursive call:
    /// must be proven.
    Decreases,
}

impl ObligationKind {
    /// The kind as one word, as `mn smt` names a query: `index`, `requires`.
    pub fn name(self) -> &'static str {
        match self {
            ObligationKind::Refinement => "refinement",
            ObligationKind::Range => "range",
            ObligationKind::Index => "index",
            ObligationKind::Divisor => "divisor",
            ObligationKind::Shift => "shift",
            ObligationKind::Requires => "requires",
            ObligationKind::Ensures => "ensures",
            ObligationKind::Invariant => "invariant",
            ObligationKind::Decreases => "decreases",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    pub kind: ObligationKind,
    pub span: Span,
    pub must: bool,
    pub proven: bool,
    /// What a run-time check checks against, as `mn view --checks` prints it: `into `u8``,
    /// `into an array of 4`, `below 8`; empty when there is nothing to say.
    pub detail: String,
    /// What tells it apart from other obligations of its kind at its place; the lowering leaves
    /// out a check only where every obligation of its kind, place and tag is proven.
    pub tag: Tag,
}

/// What tells apart obligations of one kind at one place, so that the lowering leaves out of
/// MIR exactly the checks Tier 1 proves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    /// Nothing more than the kind and the place.
    None,
    /// A narrowing: the type the value enters.
    Into(&'static str),
    /// A `requires`, `ensures` or `invariant` clause, or a `decreases` measure that is never
    /// negative: the clause's own span. Each conjunct of an invariant carries its clause's.
    Clause(Span),
    /// The `decreases` measures together, smaller than at the previous round or on entry.
    Smaller,
    /// A `mut` place's refinement, with what the callee left in it.
    Back,
}

/// Every obligation of a function, proven or not, the errors for those it must prove, and the
/// clauses it trusts: those of an `@prove` function outside the Tier-1 fragment, each with its
/// place and its text, assumed until `mn prove` runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub obligations: Vec<Obligation>,
    pub diagnostics: Vec<Diagnostic>,
    pub axioms: Vec<(Span, String)>,
}

/// What a caller's check reads of a function: its parameters' and result's types with their
/// refinements, and its contract clauses. A session checks a caller again when it changes.
pub fn header(f: &FnSyntax) -> String {
    let t = &f.tree;
    let params: Vec<String> = f.params.iter().map(|p| t.render_type(p.ty)).collect();
    let clauses: Vec<String> = f.contracts.iter().map(|c| t.render_expr(c.expr)).collect();
    format!(
        "({}) {} ({})",
        params.join(" "),
        f.result.map_or(String::new(), |r| t.render_type(r)),
        clauses.join(" ")
    )
}

// ---------------------------------------------------------------------------------------------
// The Tier-1 fragment
// ---------------------------------------------------------------------------------------------

/// A linear form by the text of its variables (`x`, `xs.len`), for classifying a clause.
#[derive(Clone, Debug, Default)]
struct Form {
    vars: Vec<(String, i128)>,
    constant: i128,
}

impl Form {
    fn add(mut self, other: Form, sign: i128) -> Form {
        for (name, k) in other.vars {
            match self.vars.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 += sign * k,
                None => self.vars.push((name, sign * k)),
            }
        }
        self.constant = self
            .constant
            .saturating_add(other.constant.saturating_mul(sign));
        self
    }

    fn scaled(mut self, k: i128) -> Form {
        for v in &mut self.vars {
            v.1 = v.1.saturating_mul(k);
        }
        self.constant = self.constant.saturating_mul(k);
        self
    }
}

/// What the classifier knows of a name: whether it is an integer binding, and the length of a
/// fixed-size array, whose `.len` is the constant `N` of its type `[N]T`.
pub struct Names<'a> {
    pub is_int: &'a dyn Fn(&str) -> bool,
    /// The type of a field path of a binding, `s.a.b` as `s` and `["a", "b"]`, when it has one.
    pub path_ty: &'a dyn Fn(&str, &[String]) -> Option<Ty>,
}

/// The type of field path `names` of a value of type `t`, through structs only.
fn path_ty(structs: &crate::structs::Structs, t: &Ty, names: &[String]) -> Option<Ty> {
    let Some((first, rest)) = names.split_first() else {
        return Some(t.clone());
    };
    let Ty::Struct(s) = t else {
        return None;
    };
    let (_, field) = structs.get(s)?.field(first)?;
    path_ty(structs, &field.ty, rest)
}

fn form(tree: &Tree, e: ExprId, names: &Names) -> Result<Form, String> {
    let var = |name: String| Form {
        vars: vec![(name, 1)],
        constant: 0,
    };
    match &tree.expr(e).kind {
        ExprKind::Int { value, .. } => Ok(Form {
            vars: Vec::new(),
            constant: i128::try_from(*value).unwrap_or(i128::MAX),
        }),
        ExprKind::Name(n) if (names.is_int)(n) => Ok(var(n.clone())),
        // A chain of fields of a binding: an integer field path is a variable, and an array's
        // `.len`, of a binding or of a field path, the constant length of its type.
        ExprKind::Field { .. } => {
            let mut path = Vec::new();
            let mut at = e;
            while let ExprKind::Field { base, name } = &tree.expr(at).kind {
                path.insert(0, name.clone());
                at = *base;
            }
            let ExprKind::Name(root) = &tree.expr(at).kind else {
                return Err("a field of a value that is not a binding".to_string());
            };
            if let Some((last, prefix)) = path.split_last() {
                // The array a path names, the binding itself when it is one.
                let len = match (names.path_ty)(root, prefix) {
                    Some(Ty::Array(n, _)) => Some(n),
                    _ => None,
                };
                if let Some(len) = len.filter(|_| last == "len") {
                    return Ok(Form {
                        vars: Vec::new(),
                        constant: i128::try_from(len).unwrap_or(i128::MAX),
                    });
                }
            }
            match (names.path_ty)(root, &path) {
                Some(Ty::Int(_)) => Ok(var(format!("{}.{}", root, path.join(".")))),
                _ => Err("a field that is not an integer".to_string()),
            }
        }
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => Ok(form(tree, *operand, names)?.scaled(-1)),
        ExprKind::Binary {
            op: op @ (BinaryOp::Add | BinaryOp::Sub),
            lhs,
            rhs,
        } => {
            let sign = if *op == BinaryOp::Add { 1 } else { -1 };
            Ok(form(tree, *lhs, names)?.add(form(tree, *rhs, names)?, sign))
        }
        ExprKind::Binary {
            op: BinaryOp::Mul,
            lhs,
            rhs,
        } => {
            let (a, b) = (form(tree, *lhs, names)?, form(tree, *rhs, names)?);
            match (a.vars.is_empty(), b.vars.is_empty()) {
                (false, false) => Err("a product of variables".to_string()),
                (true, _) => Ok(b.scaled(a.constant)),
                (false, true) => Ok(a.scaled(b.constant)),
            }
        }
        ExprKind::Binary { op, .. } => Err(format!("`{}`", op.symbol())),
        ExprKind::Name(_) => Err("a value that is not an integer".to_string()),
        ExprKind::Call { .. } => Err("a call".to_string()),
        ExprKind::Index { .. } => Err("an array element".to_string()),
        ExprKind::Bool(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Unary { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Match { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::If { .. }
        | ExprKind::Error => Err("an expression that is not linear arithmetic".to_string()),
    }
}

/// Why a contract clause lies outside the Tier-1 fragment, or `None` when it is a conjunction
/// of comparisons `±x ±y <op> c` (`ghost-contracts-and-totality.md`, item 2). `names` tells
/// which names are integer bindings and how long each fixed-size array is.
pub fn outside_fragment(tree: &Tree, e: ExprId, names: &Names) -> Option<String> {
    match &tree.expr(e).kind {
        ExprKind::Binary {
            op: BinaryOp::And,
            lhs,
            rhs,
        } => outside_fragment(tree, *lhs, names).or_else(|| outside_fragment(tree, *rhs, names)),
        ExprKind::Binary {
            op: BinaryOp::Eq | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge,
            lhs,
            rhs,
        } => {
            let diff = match (form(tree, *lhs, names), form(tree, *rhs, names)) {
                (Ok(a), Ok(b)) => a.add(b, -1),
                (Err(why), _) | (_, Err(why)) => return Some(why),
            };
            let vars: Vec<i128> = diff.vars.iter().map(|v| v.1).filter(|k| *k != 0).collect();
            if vars.len() > 2 {
                Some("more than two variables".to_string())
            } else if vars.iter().any(|k| k.abs() != 1) {
                Some("a coefficient other than 1 or -1".to_string())
            } else {
                None
            }
        }
        ExprKind::Binary { op, .. } => Some(format!("`{}`", op.symbol())),
        ExprKind::Unary {
            op: UnaryOp::Not, ..
        } => Some("`not`".to_string()),
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Unary { .. }
        | ExprKind::Index { .. }
        | ExprKind::Field { .. }
        | ExprKind::Call { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::Match { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::If { .. }
        | ExprKind::Error => Some("a value that is not a comparison".to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// Abstract values
// ---------------------------------------------------------------------------------------------

/// A linear form `terms + constant`, with at most two terms.
type Lin = (Vec<Term>, i128);

#[derive(Clone, Debug, PartialEq)]
enum Val {
    /// Exactly `terms + constant`.
    Lin(Vec<Term>, i128),
    /// `base + q`, where `q` is `num / div` rounded toward zero, `div >= 1`: the facts of
    /// division by a positive constant (`core-calculus.md`, item 8).
    Quot { base: Lin, num: Lin, div: i128 },
    /// Somewhere in `[lo, hi]`.
    Itv(i128, i128),
    /// An array whose every element lies in the value.
    Arr(Box<Val>),
    /// A `bool`, no value, or a value Tier 1 does not follow.
    Other,
}

const TOP: Val = Val::Itv(-INF, INF);

fn var_val(v: usize) -> Val {
    Val::Lin(vec![Term::pos(v)], 0)
}

fn lin(terms: Vec<Term>, c: i128) -> Val {
    if c.abs() >= BIG {
        TOP
    } else {
        Val::Lin(terms, c)
    }
}

fn int_literal(value: u128) -> Val {
    i128::try_from(value).map_or(TOP, |v| lin(Vec::new(), v))
}

fn sat_add(a: i128, b: i128) -> i128 {
    if a >= INF || b >= INF {
        INF
    } else if a <= -INF || b <= -INF {
        -INF
    } else {
        (a + b).clamp(-INF, INF)
    }
}

fn neg_terms(terms: &[Term]) -> Vec<Term> {
    terms.iter().map(|t| t.negated()).collect()
}

/// The sum of two linear forms' terms; `None` when it has more than two terms or a coefficient
/// other than 1 or -1, which octagons cannot hold.
fn lin_add(a: &[Term], b: &[Term]) -> Option<Vec<Term>> {
    let mut out: Vec<Term> = Vec::new();
    for t in a.iter().chain(b) {
        match out.iter().position(|u| u.var == t.var) {
            Some(k) if out[k].neg != t.neg => {
                out.remove(k);
            }
            Some(_) => return None,
            None => out.push(*t),
        }
    }
    (out.len() <= 2).then_some(out)
}

fn negate(v: &Val) -> Val {
    match v {
        Val::Lin(t, c) => Val::Lin(neg_terms(t), -c),
        Val::Quot { base, num, div } => Val::Quot {
            base: (neg_terms(&base.0), -base.1),
            num: (neg_terms(&num.0), -num.1),
            div: *div,
        },
        Val::Itv(lo, hi) => Val::Itv(-hi, -lo),
        other @ (Val::Arr(_) | Val::Other) => other.clone(),
    }
}

fn as_const(v: &Val) -> Option<i128> {
    match v {
        Val::Lin(t, c) if t.is_empty() => Some(*c),
        Val::Lin(..) | Val::Quot { .. } | Val::Itv(..) | Val::Arr(_) | Val::Other => None,
    }
}

/// `a / d` rounded toward zero, for `d >= 1`, with the unbounded ends kept.
fn trunc_div(a: i128, d: i128) -> i128 {
    if a.abs() >= INF {
        a
    } else {
        a / d
    }
}

/// `a - a / d`, which grows with `a`: what division by `d` leaves of `a`.
fn rest(a: i128, d: i128) -> i128 {
    if a.abs() >= INF {
        a
    } else {
        a - a / d
    }
}

fn lin_interval(st: &Octagon, terms: &[Term], c: i128) -> (i128, i128) {
    let hi = sat_add(st.upper(terms), c);
    let lo = sat_add(-st.upper(&neg_terms(terms)), c);
    (lo, hi)
}

/// The bounds of a value in a state.
fn interval(st: &Octagon, v: &Val) -> (i128, i128) {
    match v {
        Val::Lin(t, c) => lin_interval(st, t, *c),
        Val::Quot { base, num, div } => {
            let (blo, bhi) = lin_interval(st, &base.0, base.1);
            let (nlo, nhi) = lin_interval(st, &num.0, num.1);
            let mut lo = sat_add(blo, trunc_div(nlo, *div));
            let mut hi = sat_add(bhi, trunc_div(nhi, *div));
            if let Some(sum) = lin_add(&base.0, &num.0) {
                let (slo, shi) = lin_interval(st, &sum, base.1 + num.1);
                lo = lo.max(sat_add(slo, -rest(nhi, *div)));
                hi = hi.min(sat_add(shi, -rest(nlo, *div)));
            }
            (lo, hi)
        }
        Val::Itv(lo, hi) => (*lo, *hi),
        Val::Arr(_) | Val::Other => (-INF, INF),
    }
}

/// A value with no reference to a variable: an interval, an array of them, or `Other`.
fn summary_of(st: &Octagon, v: &Val) -> Val {
    match v {
        Val::Arr(_) | Val::Other => v.clone(),
        Val::Lin(..) | Val::Quot { .. } | Val::Itv(..) => {
            let (lo, hi) = interval(st, v);
            Val::Itv(lo, hi)
        }
    }
}

fn join_summary(a: &Val, b: &Val) -> Val {
    match (a, b) {
        (Val::Itv(a, b), Val::Itv(c, d)) => Val::Itv(*a.min(c), *b.max(d)),
        (Val::Arr(x), Val::Arr(y)) => Val::Arr(Box::new(join_summary(x, y))),
        _ => Val::Other,
    }
}

fn add_vals(st: &Octagon, l: &Val, r: &Val) -> Val {
    match (l, r) {
        (Val::Lin(a, c), Val::Lin(b, d)) => {
            if let Some(t) = lin_add(a, b) {
                return lin(t, c + d);
            }
        }
        (Val::Quot { base, num, div }, Val::Lin(b, d))
        | (Val::Lin(b, d), Val::Quot { base, num, div }) => {
            if let (Some(t), true) = (lin_add(&base.0, b), (base.1 + d).abs() < BIG) {
                return Val::Quot {
                    base: (t, base.1 + d),
                    num: num.clone(),
                    div: *div,
                };
            }
        }
        _ => {}
    }
    let ((a, b), (c, d)) = (interval(st, l), interval(st, r));
    Val::Itv(sat_add(a, c), sat_add(b, d))
}

/// A product of two bounds, the unbounded ends kept: a sentinel times a number of either sign
/// saturates past it, and is clamped back.
fn mul_bound(a: i128, b: i128) -> i128 {
    a.saturating_mul(b).clamp(-INF, INF)
}

fn hull([p, q, r, s]: [i128; 4]) -> Val {
    Val::Itv(p.min(q).min(r).min(s), p.max(q).max(r).max(s))
}

fn mul_vals(st: &Octagon, l: &Val, r: &Val) -> Val {
    // A factor of 1 or -1 keeps the other one's relations; any other goes by its bounds.
    for (k, v) in [(as_const(r), l), (as_const(l), r)] {
        match k {
            Some(1) => return v.clone(),
            Some(-1) => return negate(v),
            _ => {}
        }
    }
    let ((a, b), (c, d)) = (interval(st, l), interval(st, r));
    hull([
        mul_bound(a, c),
        mul_bound(a, d),
        mul_bound(b, c),
        mul_bound(b, d),
    ])
}

fn div_vals(st: &Octagon, l: &Val, r: &Val) -> Val {
    match as_const(r) {
        Some(c) if c > 0 => {
            if let Val::Lin(t, k) = l {
                return Val::Quot {
                    base: (Vec::new(), 0),
                    num: (t.clone(), *k),
                    div: c,
                };
            }
            let (lo, hi) = interval(st, l);
            return Val::Itv(trunc_div(lo, c), trunc_div(hi, c));
        }
        _ => {}
    }
    let ((a, b), (c, d)) = (interval(st, l), interval(st, r));
    // A divisor of no value, after a narrowing that always traps, gives a quotient of none.
    if c > d {
        return Val::Itv(c, d);
    }
    if c > 0 {
        // A positive divisor keeps the dividend's sign; the quotient's ends come from the
        // dividend's ends over the divisor's.
        let lo = if a.abs() >= INF {
            a
        } else {
            (a / c).min(a / d)
        };
        let hi = if b.abs() >= INF {
            b
        } else {
            (b / c).max(b / d)
        };
        Val::Itv(lo, hi)
    } else if d < 0 {
        negate(&div_vals(st, l, &negate(r)))
    } else {
        // A divisor that may be zero is not zero where the division runs, so the quotient's
        // magnitude is at most the dividend's.
        let m = a.abs().max(b.abs());
        Val::Itv(-m, m)
    }
}

/// The bounds of `x % y`: the sign of `x`, and a magnitude below `|y|` and at most `|x|`.
/// A zero divisor traps, so `|y|` bounds the magnitude even when `y` may be zero.
fn rem_interval(st: &Octagon, l: &Val, r: &Val) -> (i128, i128) {
    let ((a, b), (c, d)) = (interval(st, l), interval(st, r));
    let m = a.abs().max(b.abs());
    let bound = (c.abs().max(d.abs()) - 1).min(m);
    (
        if a >= 0 { 0 } else { -bound },
        if b <= 0 { 0 } else { bound },
    )
}

/// The range of an integer type; a 128-bit one reaches beyond the domain, so it is unbounded.
/// The range of an integer type in the domain. The 128-bit types reach past the domain's bound,
/// so their upper end is unbounded, and so is `i128`'s lower end; `u128` keeps its lower end,
/// 0, without which a negative value would be proven to fit it.
fn int_range(t: IntTy) -> (i128, i128) {
    if t.bits == 128 {
        return (if t.signed { -INF } else { 0 }, INF);
    }
    (-(t.min_magnitude() as i128), t.max() as i128)
}

fn int_of(t: &Ty) -> Option<IntTy> {
    match t {
        Ty::Int(Int::One(t)) => Some(*t),
        Ty::Int(Int::Literal | Int::Mixed(_))
        | Ty::Bool
        | Ty::Array(..)
        | Ty::Struct(_)
        | Ty::Enum(_)
        | Ty::Unit
        | Ty::Any => None,
    }
}

fn clamp_to(v: Val, t: Option<IntTy>, st: &Octagon) -> Val {
    match t {
        Some(t) => {
            let ((lo, hi), (tlo, thi)) = (interval(st, &v), int_range(t));
            Val::Itv(lo.max(tlo), hi.min(thi))
        }
        None => v,
    }
}

/// The smallest `2^k - 1` at or above `v`.
fn ones(v: i128) -> i128 {
    let mut m = 0;
    while m < v && m < INF {
        m = 2 * m + 1;
    }
    m
}

fn shift_vals(st: &Octagon, op: BinaryOp, l: &Val, r: &Val, t: Option<IntTy>) -> Val {
    let (lo, hi) = interval(st, l);
    let k = as_const(r).filter(|k| (0..=120).contains(k));
    let v = match (op, k) {
        (BinaryOp::Shl, Some(k)) => Val::Itv(mul_bound(lo, 1 << k), mul_bound(hi, 1 << k)),
        (BinaryOp::Shr, Some(k)) => Val::Itv(
            if lo <= -INF { lo } else { lo >> k },
            if hi >= INF { hi } else { hi >> k },
        ),
        (BinaryOp::Shr, None) => Val::Itv(lo.min(0), if hi >= 0 { hi } else { -1 }),
        _ => TOP,
    };
    clamp_to(v, t, st)
}

fn bits_vals(st: &Octagon, op: BinaryOp, l: &Val, r: &Val, t: Option<IntTy>) -> Val {
    let ((a, b), (c, d)) = (interval(st, l), interval(st, r));
    let v = match op {
        BinaryOp::BitAnd if a >= 0 && c >= 0 => Val::Itv(0, b.min(d)),
        BinaryOp::BitAnd if a >= 0 => Val::Itv(0, b),
        BinaryOp::BitAnd if c >= 0 => Val::Itv(0, d),
        BinaryOp::BitOr | BinaryOp::BitXor if a >= 0 && c >= 0 => Val::Itv(0, ones(b.max(d))),
        BinaryOp::Or
        | BinaryOp::And
        | BinaryOp::Eq
        | BinaryOp::Ne
        | BinaryOp::Lt
        | BinaryOp::Le
        | BinaryOp::Gt
        | BinaryOp::Ge
        | BinaryOp::BitOr
        | BinaryOp::BitXor
        | BinaryOp::BitAnd
        | BinaryOp::Shl
        | BinaryOp::Shr
        | BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::Rem => TOP,
    };
    clamp_to(v, t, st)
}

/// `~x`: `max - x` in an unsigned type, `-x - 1` in a signed one.
fn bit_not(v: &Val, t: Option<IntTy>) -> Val {
    match (v, t) {
        (Val::Lin(terms, c), Some(t)) => {
            let top = if t.signed { -1 } else { int_range(t).1 };
            lin(neg_terms(terms), sat_add(top, -c))
        }
        _ => TOP,
    }
}

// ---------------------------------------------------------------------------------------------
// Shapes: what a declared type asks of a value
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Refine {
    lo: Val,
    hi: Val,
    inclusive: bool,
    /// The bounds as a `guard` writes them, and the type as written.
    lo_text: String,
    hi_text: String,
    text: String,
    /// The integer type and refinement a `var` can be declared with to meet this one, when
    /// both bounds are literals.
    declared: Option<(String, Refinement)>,
    /// Each bound, exactly, when it is a literal: one beyond the octagon's reach, such as the
    /// end of `u128[0..340282366920938463463374607431768211455]`, is checked against it.
    lo_lit: Option<Exact>,
    hi_lit: Option<Exact>,
}

/// A refinement's bound, exactly, when it is a literal.
fn bound_exact(b: &Bound) -> Option<Exact> {
    match b {
        Bound::Int { negative, value } => Some(Exact::new(*negative, *value)),
        Bound::Name(_) | Bound::Len(_) => None,
    }
}

/// A literal bound beyond the octagon's reach, which only an exact comparison checks.
fn far(b: Option<Exact>) -> Option<Exact> {
    b.filter(|c| c.is_far())
}

/// The integer type `name` refined by `r`, when both bounds are literals: a `var` declared
/// with it holds exactly the values the refinement admits, wherever the refinement was written.
fn literal_refinement(name: &str, r: &Refinement) -> Option<(String, Refinement)> {
    let literal = |b: &Bound| matches!(b, Bound::Int { .. });
    (literal(&r.lo) && literal(&r.hi)).then(|| (name.to_string(), r.clone()))
}

/// An integer type with a refinement, as a declaration writes it: `usize[0..=16]`.
fn refined_type_text(name: &str, r: &Refinement) -> String {
    format!(
        "{}[{}..{}{}]",
        name,
        bound_text(&r.lo),
        if r.inclusive { "=" } else { "" },
        bound_text(&r.hi)
    )
}

#[derive(Clone, Debug)]
enum Shape {
    Int {
        int: Option<IntTy>,
        refine: Option<Box<Refine>>,
    },
    Array(u128, Box<Shape>),
    /// A struct of the module, whose fields have the shapes its declaration gives them.
    Struct(String),
    Other,
}

/// A step from a place's root binding to the place: an index, or a field.
#[derive(Clone, Debug)]
enum Step {
    Index(ExprId),
    Field(String),
}

impl Shape {
    fn plain(t: &Ty) -> Shape {
        match t {
            Ty::Int(_) => Shape::Int {
                int: int_of(t),
                refine: None,
            },
            Ty::Array(n, e) => Shape::Array(*n, Box::new(Shape::plain(e))),
            Ty::Struct(name) => Shape::Struct(name.clone()),
            Ty::Bool | Ty::Enum(_) | Ty::Unit | Ty::Any => Shape::Other,
        }
    }

    /// The shape with its plain ranges dropped, its refinements kept.
    fn refinements(&self) -> Shape {
        match self {
            Shape::Int { refine, .. } => Shape::Int {
                int: None,
                refine: refine.clone(),
            },
            Shape::Array(n, e) => Shape::Array(*n, Box::new(e.refinements())),
            // A struct's fields hold their refinements wherever its value goes.
            Shape::Struct(_) | Shape::Other => Shape::Other,
        }
    }

    fn element(&self) -> Shape {
        match self {
            Shape::Array(_, e) => (**e).clone(),
            Shape::Int { .. } | Shape::Struct(_) | Shape::Other => Shape::Other,
        }
    }
}

fn bound_text(b: &Bound) -> String {
    match b {
        Bound::Int { negative, value } => format!("{}{}", if *negative { "-" } else { "" }, value),
        Bound::Name(n) => n.clone(),
        Bound::Len(n) => format!("{}.len", n),
    }
}

/// Whether the interval Tier 1 gives `v` in `st` lies within the integer type `t`, both its
/// ends bounded.
fn within(st: &Octagon, v: &Val, t: IntTy) -> bool {
    let (lo, hi) = interval(st, v);
    let (tlo, thi) = int_range(t);
    tlo <= lo && hi <= thi && -INF < lo && hi < INF
}

/// The range of the integer type `t` as a refinement, both bounds literals: what a `match`
/// narrows its scrutinee into.
fn type_refine(t: IntTy) -> Refine {
    let lo = Bound::Int {
        negative: t.signed,
        value: t.min_magnitude(),
    };
    let hi = Bound::Int {
        negative: false,
        value: t.max(),
    };
    Refine {
        lo: bound_int(t.signed, t.min_magnitude()),
        hi: bound_int(false, t.max()),
        inclusive: true,
        lo_text: bound_text(&lo),
        hi_text: bound_text(&hi),
        text: t.name.to_string(),
        declared: None,
        lo_lit: bound_exact(&lo),
        hi_lit: bound_exact(&hi),
    }
}

fn bound_int(negative: bool, value: u128) -> Val {
    let v = int_literal(value);
    if negative {
        negate(&v)
    } else {
        v
    }
}

// ---------------------------------------------------------------------------------------------
// Conditions
// ---------------------------------------------------------------------------------------------

/// A comparison, the relation a condition states between two values: the six operators that
/// compare, and no other, so that a condition never holds an operator it cannot narrow by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rel {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Rel {
    /// The comparison `op` is, none for an operator that does not compare.
    fn of(op: BinaryOp) -> Option<Rel> {
        match op {
            BinaryOp::Eq => Some(Rel::Eq),
            BinaryOp::Ne => Some(Rel::Ne),
            BinaryOp::Lt => Some(Rel::Lt),
            BinaryOp::Le => Some(Rel::Le),
            BinaryOp::Gt => Some(Rel::Gt),
            BinaryOp::Ge => Some(Rel::Ge),
            BinaryOp::Or
            | BinaryOp::And
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitAnd
            | BinaryOp::Shl
            | BinaryOp::Shr
            | BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Rem => None,
        }
    }

    /// The operator that writes it.
    fn op(self) -> BinaryOp {
        match self {
            Rel::Eq => BinaryOp::Eq,
            Rel::Ne => BinaryOp::Ne,
            Rel::Lt => BinaryOp::Lt,
            Rel::Le => BinaryOp::Le,
            Rel::Gt => BinaryOp::Gt,
            Rel::Ge => BinaryOp::Ge,
        }
    }

    fn symbol(self) -> &'static str {
        self.op().symbol()
    }
}

/// A condition whose operands are evaluated.
#[derive(Clone, Debug)]
enum Cond {
    Const(bool),
    Cmp(Rel, Box<Val>, Box<Val>),
    /// A value compared with a constant beyond the octagon's reach, kept exactly; boxed, so
    /// that a condition stays small in the frames of nested expressions.
    Far(Rel, Box<(Val, Exact)>),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    Not(Box<Cond>),
    Unknown,
}

fn cmp(op: Rel, l: Val, r: Val) -> Cond {
    Cond::Cmp(op, Box::new(l), Box::new(r))
}

/// `a < b` in lexicographic order, for two measures of one length: the last pair compares
/// with `false` after it.
fn lex_less(a: &[Val], b: &[Val]) -> Cond {
    match (a, b) {
        ([x, xs @ ..], [y, ys @ ..]) => Cond::Or(
            Box::new(cmp(Rel::Lt, x.clone(), y.clone())),
            Box::new(Cond::And(
                Box::new(cmp(Rel::Eq, x.clone(), y.clone())),
                Box::new(lex_less(xs, ys)),
            )),
        ),
        _ => Cond::Const(false),
    }
}

/// The values of a measure as a message writes them: `3`, or `(3, 0)` for several.
fn tuple(values: &[Exact]) -> String {
    let parts: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    match parts.as_slice() {
        [one] => one.clone(),
        _ => format!("({})", parts.join(", ")),
    }
}

/// The comparison with its operands swapped: `a < b` is `b > a`.
fn mirror(op: Rel) -> Rel {
    match op {
        Rel::Lt => Rel::Gt,
        Rel::Le => Rel::Ge,
        Rel::Gt => Rel::Lt,
        Rel::Ge => Rel::Le,
        Rel::Eq => Rel::Eq,
        Rel::Ne => Rel::Ne,
    }
}

/// The comparison that holds exactly where `op` fails.
fn flip(op: Rel) -> Rel {
    match op {
        Rel::Lt => Rel::Ge,
        Rel::Le => Rel::Gt,
        Rel::Gt => Rel::Le,
        Rel::Ge => Rel::Lt,
        Rel::Eq => Rel::Ne,
        Rel::Ne => Rel::Eq,
    }
}

/// Forgets a call site's variables, before and after the call.
fn forget_site(st: &mut Octagon, site: &CallSite) {
    let paths = site.paths.iter().flatten().map(|p| p.var);
    for v in site
        .ins
        .iter()
        .chain(&site.outs)
        .flatten()
        .copied()
        .chain(paths)
    {
        st.forget(v);
    }
}

/// Whether `v` reads `var`.
fn mentions(v: &Val, var: usize) -> bool {
    let has = |t: &[Term]| t.iter().any(|t| t.var == var);
    match v {
        Val::Lin(t, _) => has(t),
        Val::Quot { base, num, .. } => has(&base.0) || has(&num.0),
        Val::Itv(..) | Val::Arr(_) | Val::Other => false,
    }
}

/// Whether condition `c` reads `var`.
fn cond_mentions(c: &Cond, var: usize) -> bool {
    match c {
        Cond::Cmp(_, l, r) => mentions(l, var) || mentions(r, var),
        Cond::Far(_, far) => mentions(&far.0, var),
        Cond::Not(a) => cond_mentions(a, var),
        Cond::And(a, b) | Cond::Or(a, b) => cond_mentions(a, var) || cond_mentions(b, var),
        Cond::Const(_) | Cond::Unknown => false,
    }
}

/// After `var = terms + c`: the bounds beyond the octagon's reach that a single term carries,
/// shifted by `c` and negated with the term. A sum of more terms carries none, which is sound.
fn copy_far(st: &mut Octagon, var: usize, terms: &[Term], c: i128) {
    let [t] = terms else {
        return;
    };
    let (lo, hi) = st.far_bounds(t.var);
    if lo.is_none() && hi.is_none() {
        return;
    }
    let (lo, hi) = if t.neg {
        (hi.map(Exact::negated), lo.map(Exact::negated))
    } else {
        (lo, hi)
    };
    st.add_exact(var, lo.and_then(|x| x.plus(c)), hi.and_then(|x| x.plus(c)));
}

/// Narrows `st` to the points where `t + k op c` holds, `c` a constant beyond the octagon's
/// reach, as an exact bound of `t`'s variable.
fn constrain_far(st: &mut Octagon, op: Rel, t: Term, k: i128, c: Exact) {
    // `±x + k op c` is `±x op c - k`, and `-x op b` is `x op' -b`.
    let Some(b) = c.plus(k.checked_neg().unwrap_or(i128::MAX)) else {
        return;
    };
    let (op, b) = if t.neg {
        (mirror(op), b.negated())
    } else {
        (op, b)
    };
    let bound = match op {
        Rel::Lt => b.plus(-1),
        Rel::Gt => b.plus(1),
        Rel::Eq | Rel::Ne | Rel::Le | Rel::Ge => Some(b),
    };
    // No value lies beyond the exact integers, which hold every value of a 128-bit type: a
    // strict comparison past them holds nowhere.
    let Some(bound) = bound else {
        st.set_empty();
        return;
    };
    let (lower, upper) = match op {
        Rel::Lt | Rel::Le => (None, Some(bound)),
        Rel::Gt | Rel::Ge => (Some(bound), None),
        Rel::Eq => (Some(bound), Some(bound)),
        // A value other than one beyond the octagon's reach: no bound narrows to it.
        Rel::Ne => (None, None),
    };
    st.add_exact(t.var, lower, upper);
}

/// `x - (terms + c)` lies in `[lo, hi]`: exactly when `terms` has at most one term, else as the
/// octagon facts the sum allows.
fn constrain_offset(st: &mut Octagon, x: usize, terms: &[Term], c: i128, lo: i128, hi: i128) {
    let xt = Term::pos(x);
    let (low, high) = (sat_add(c, lo), sat_add(c, hi));
    match terms {
        [t] => {
            st.add_lin(&[xt, t.negated()], high);
            st.add_lin(&[xt.negated(), *t], -low);
        }
        // No term, or two: each term against `x` by the other's bounds, then `x`'s own.
        _ => {
            for (k, t) in terms.iter().enumerate() {
                let (olo, ohi) = lin_interval(st, &[terms[1 - k]], 0);
                st.add_lin(&[xt, t.negated()], sat_add(high, ohi));
                st.add_lin(&[xt.negated(), *t], -sat_add(low, olo));
            }
            let (slo, shi) = lin_interval(st, terms, 0);
            st.add_lin(&[xt], sat_add(high, shi));
            st.add_lin(&[xt.negated()], -sat_add(low, slo));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Printing clauses and values as Mente writes them
// ---------------------------------------------------------------------------------------------

/// How tightly an expression binds (`grammar.md`, item 7): an operand of looser binding than
/// its place is put in parentheses.
fn prec(kind: &ExprKind) -> u8 {
    match kind {
        ExprKind::If { .. } | ExprKind::Match { .. } => 0,
        ExprKind::Binary { op, .. } => match op {
            BinaryOp::Or => 1,
            BinaryOp::And => 2,
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => 4,
            BinaryOp::BitOr => 5,
            BinaryOp::BitXor => 6,
            BinaryOp::BitAnd => 7,
            BinaryOp::Shl | BinaryOp::Shr => 8,
            BinaryOp::Add | BinaryOp::Sub => 9,
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 10,
        },
        ExprKind::Unary {
            op: UnaryOp::Not, ..
        } => 3,
        ExprKind::Unary { .. } => 11,
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
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::Error => 12,
    }
}

/// The level of a comparison's operand: anything that binds more tightly than a comparison.
const OPERAND: u8 = 5;

fn paren((text, p): (String, u8), min: u8) -> String {
    if p < min {
        format!("({})", text)
    } else {
        text
    }
}

type Subst<'s> = &'s dyn Fn(ExprId, &str) -> Option<(String, u8)>;

fn no_subst(_: ExprId, _: &str) -> Option<(String, u8)> {
    None
}

/// An expression in Mente's syntax, with names replaced where `subst` says.
fn print(tree: &Tree, e: ExprId, subst: Subst) -> (String, u8) {
    let kind = &tree.expr(e).kind;
    let p = prec(kind);
    let at = |e: ExprId, min: u8| paren(print(tree, e, subst), min);
    let text = match kind {
        ExprKind::Int { value, suffix } => match suffix {
            Some(s) => format!("{}_{}", value, s),
            None => value.to_string(),
        },
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::Name(n) => return subst(e, n).unwrap_or((n.clone(), p)),
        ExprKind::Unary { op, operand } => match op {
            UnaryOp::Not => format!("not {}", at(*operand, 3)),
            UnaryOp::Neg => format!("-{}", at(*operand, 12)),
            UnaryOp::BitNot => format!("~{}", at(*operand, 12)),
        },
        ExprKind::Binary { op, lhs, rhs } => {
            let left = if p == 4 { p + 1 } else { p };
            format!("{} {} {}", at(*lhs, left), op.symbol(), at(*rhs, p + 1))
        }
        ExprKind::Index { base, index } => format!("{}[{}]", at(*base, 12), at(*index, 0)),
        ExprKind::Field { base, name } => match subst(e, "") {
            // A field path may be replaced whole.
            Some(t) => return t,
            None => format!("{}.{}", at(*base, 12), name),
        },
        ExprKind::Call { callee, args } => {
            let args: Vec<String> = args
                .iter()
                .map(|a| format!("{}{}", if a.is_mut { "mut " } else { "" }, at(a.value, 0)))
                .collect();
            format!("{}({})", callee, args.join(", "))
        }
        ExprKind::Construct { name, fields } => {
            let fields: Vec<String> = fields
                .iter()
                .map(|f| format!("{}: {}", f.name, at(f.value, 0)))
                .collect();
            format!("{}({})", name, fields.join(", "))
        }
        ExprKind::Array(items) => {
            let items: Vec<String> = items.iter().map(|i| at(*i, 0)).collect();
            format!("[{}]", items.join(", "))
        }
        ExprKind::Repeat { value, count } => format!("[{}; {}]", at(*value, 0), count),
        ExprKind::Todo(r) => format!("todo({:?})", r),
        ExprKind::Unreachable(r) => format!("unreachable({:?})", r),
        ExprKind::If { cond, .. } => format!("if {} {{ ... }}", at(*cond, 0)),
        ExprKind::Match { scrutinee, .. } => format!("match {} {{ ... }}", at(*scrutinee, 0)),
        ExprKind::Variant {
            enum_name,
            name,
            payload,
            ..
        } => match payload {
            Payload::None => format!("{}.{}", enum_name, name),
            Payload::Named(fields) => {
                let fields: Vec<String> = fields
                    .iter()
                    .map(|f| format!("{}: {}", f.name, at(f.value, 0)))
                    .collect();
                format!("{}.{}({})", enum_name, name, fields.join(", "))
            }
            Payload::Positional(args) => {
                let args: Vec<String> = args
                    .iter()
                    .map(|a| format!("{}{}", if a.is_mut { "mut " } else { "" }, at(a.value, 0)))
                    .collect();
                format!("{}.{}({})", enum_name, name, args.join(", "))
            }
        },
        ExprKind::Error => "_".to_string(),
    };
    (text, p)
}

/// The one-line `guard` a fix inserts: a function with a result leaves by `todo`, so that its
/// author decides what it does where the condition is false; one without returns.
fn guard_line(has_result: bool, cond: &str) -> String {
    let leave = if has_result {
        format!(
            "todo(\"{} is false here\")",
            cond.replace('\\', "\\\\").replace('"', "\\\"")
        )
    } else {
        "return".to_string()
    };
    format!("guard {} else {{ {} }}", cond, leave)
}

// ---------------------------------------------------------------------------------------------
// The analysis of one function
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct CallSite {
    callee: usize,
    ins: Vec<Option<usize>>,
    outs: Vec<Option<usize>>,
    result: Option<usize>,
    /// For each parameter of a struct type, a variable for each of its scalar field paths the
    /// callee's clauses read, holding the argument's at the call.
    paths: Vec<Vec<FieldPath>>,
}

/// A scalar field path Tier 1 follows: `s.a.b` of a binding `s` of a struct type, as its
/// field names, not through an array (`core-calculus.md`, item 13), with its variable and its
/// declared shape.
#[derive(Clone, Debug)]
struct FieldPath {
    names: Vec<String>,
    var: usize,
    shape: Shape,
}

/// The variables of a function, counted as they are given out.
struct Counter(usize);

impl Counter {
    /// A new variable, when one is wanted.
    fn alloc(&mut self, wanted: bool) -> Option<usize> {
        self.0 += usize::from(wanted);
        wanted.then(|| self.0 - 1)
    }
}

/// The variables of a function besides its bindings' and their paths' (`Fx::rest`).
struct Rest {
    sites: Vec<Site>,
    temps: [usize; 2],
    scratch: usize,
    probe: usize,
    measures: Vec<Vec<(usize, usize)>>,
    entry_measures: Vec<(usize, usize)>,
}

/// A scalar field path, as its field names, and a value of it.
type PathVal = (Vec<String>, Val);

/// The scalar field paths of a value of struct `name`, not through an array, each with its
/// declared shape: an integer field, and each scalar path of a struct field, in the order of
/// the declaration. `None` when there are more than `room`, which each one listed takes one of:
/// structs nested level by level can have a number of them that doubles with each level, and
/// none is listed past the room.
fn scalar_paths(
    structs: &crate::structs::Structs,
    name: &str,
    room: &mut usize,
) -> Option<Vec<(Vec<String>, Shape)>> {
    let def = structs.get(name)?;
    let mut out = Vec::new();
    for f in &def.fields {
        match Fx::declared_shape(def.tree(), f.decl) {
            shape @ Shape::Int { .. } => {
                *room = room.checked_sub(1)?;
                out.push((vec![f.name.clone()], shape));
            }
            Shape::Struct(inner) => {
                for (mut names, shape) in scalar_paths(structs, &inner, room)? {
                    names.insert(0, f.name.clone());
                    out.push((names, shape));
                }
            }
            Shape::Array(..) | Shape::Other => {}
        }
    }
    Some(out)
}

/// The declared shape of scalar field path `names` of a value of struct `name`.
fn path_shape(structs: &crate::structs::Structs, name: &str, names: &[String]) -> Option<Shape> {
    let (first, rest) = names.split_first()?;
    let def = structs.get(name)?;
    let f = def.fields.iter().find(|f| f.name == *first)?;
    match (Fx::declared_shape(def.tree(), f.decl), rest) {
        (shape @ Shape::Int { .. }, []) => Some(shape),
        (Shape::Struct(inner), _) => path_shape(structs, &inner, rest),
        _ => None,
    }
}

/// The binding and field names of `e` when it is a chain of fields of a binding, not through
/// an index: `s.a.b` is `s` and `["a", "b"]`, and `s` alone is `s` and none.
fn field_chain(tree: &Tree, resolution: &Resolution, e: ExprId) -> Option<(usize, Vec<String>)> {
    match &tree.expr(e).kind {
        ExprKind::Field { base, name } => {
            let (root, mut names) = field_chain(tree, resolution, *base)?;
            names.push(name.clone());
            Some((root, names))
        }
        ExprKind::Name(_) => resolution.names[e.0 as usize].map(|b| (b.0 as usize, Vec::new())),
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Unary { .. }
        | ExprKind::Binary { .. }
        | ExprKind::Index { .. }
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

/// The name a chain of fields `e` starts from, and its fields: `s.a.b` is `s` and `["a", "b"]`,
/// and `s` alone is `s` and none.
fn chain_of(tree: &Tree, e: ExprId) -> Option<(String, Vec<String>)> {
    match &tree.expr(e).kind {
        ExprKind::Field { base, name } => {
            let (root, mut names) = chain_of(tree, *base)?;
            names.push(name.clone());
            Some((root, names))
        }
        ExprKind::Name(n) => Some((n.clone(), Vec::new())),
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Unary { .. }
        | ExprKind::Binary { .. }
        | ExprKind::Index { .. }
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

/// Whether Tier 1 follows every value `e`, a clause in the Tier-1 fragment, reads: `read`
/// tells it for a name, or a chain of fields of one, and a literal reads nothing.
fn fragment_followed(tree: &Tree, e: ExprId, read: &dyn Fn(ExprId) -> Option<bool>) -> bool {
    if let Some(followed) = read(e) {
        return followed;
    }
    match &tree.expr(e).kind {
        ExprKind::Unary { operand, .. } => fragment_followed(tree, *operand, read),
        ExprKind::Binary { lhs, rhs, .. } => {
            fragment_followed(tree, *lhs, read) && fragment_followed(tree, *rhs, read)
        }
        ExprKind::Int { .. } | ExprKind::Bool(_) => true,
        ExprKind::Name(_)
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

/// The field paths of parameter `param` that `e`, a clause of its function, reads.
fn clause_paths(tree: &Tree, e: ExprId, param: &str, out: &mut Vec<Vec<String>>) {
    let mut names = Vec::new();
    let mut at = e;
    while let ExprKind::Field { base, name } = &tree.expr(at).kind {
        names.insert(0, name.clone());
        at = *base;
    }
    if !names.is_empty() {
        if matches!(&tree.expr(at).kind, ExprKind::Name(n) if n == param) {
            out.push(names);
        }
        return;
    }
    // A clause Tier 1 reads lies in the fragment: comparisons, `and`, `not`, sums and negations.
    match &tree.expr(e).kind {
        ExprKind::Unary { operand, .. } => clause_paths(tree, *operand, param, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            clause_paths(tree, *lhs, param, out);
            clause_paths(tree, *rhs, param, out);
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
        | ExprKind::Error => {}
    }
}

#[derive(Clone, Debug)]
enum Site {
    None,
    Call(CallSite),
    /// The value of an `if` or of a remainder.
    Value(usize),
}

#[derive(Default)]
struct Frame {
    breaks: Vec<Octagon>,
    continues: Vec<Octagon>,
}

/// The facts at the head of an enclosing loop, for an obligation inside it.
struct Head {
    span: Span,
    kept: Vec<String>,
    dropped: Vec<String>,
}

/// Where an obligation's `guard` goes.
#[derive(Clone, Copy, Debug)]
enum Anchor {
    /// On its own line before the statement.
    Before(StmtId),
    /// After the block's last statement.
    After(BlockId),
    /// At the start of an expression body, turned into a block.
    Body(ExprId),
    /// At the start of a `match` arm written as an expression, turned into a block.
    Arm(ExprId),
    Nowhere,
}

/// A candidate fact at a loop head: an expression of the loop's text, or the non-strict form
/// of a strict comparison in its condition ([`loop-condition-candidates`]).
///
/// [`loop-condition-candidates`]: ../../docs/design-review.md#loop-condition-candidates
#[derive(Clone, Copy, Debug)]
enum Cand {
    Expr(ExprId),
    Weak(ExprId),
}

struct Fx<'a> {
    syntax: &'a FnSyntax,
    tree: &'a Tree,
    resolution: &'a Resolution,
    typing: &'a Typing,
    routing: Routing,
    structs: &'a crate::structs::Structs,
    callees: &'a [Option<Callee<'a>>],
    src: &'a str,
    /// The octagon variable of each integer binding.
    vars: Vec<Option<usize>>,
    /// The scalar field paths Tier 1 follows of each binding of a struct type.
    paths: Vec<Vec<FieldPath>>,
    /// The scalar field paths of each struct literal last evaluated, and their values then.
    built: Vec<Option<Vec<PathVal>>>,
    /// The variables of each expression that needs some.
    sites: Vec<Site>,
    result: Option<usize>,
    result_shape: Option<Shape>,
    /// Scratch variables: two for comparisons, one for self-referential assignments, one for
    /// the value a diagnostic names.
    temps: [usize; 2],
    scratch: usize,
    probe: usize,
    count: usize,
    report: Report,
    recording: bool,
    loops: Vec<Frame>,
    /// The bindings in scope, in declaration order, for valuations.
    scope: Vec<usize>,
    /// The innermost statement being checked, before which a `guard` goes.
    stmt: Option<StmtId>,
    heads: Vec<Head>,
    /// For each `while` statement, a variable for each of its `decreases` measures at the
    /// start of a round, and one for its value at the end.
    measures: Vec<Vec<(usize, usize)>>,
    /// For each of the function's `decreases` clauses, which together form one measure, a
    /// variable for its value on entry, against which a recursive call's is compared, and one
    /// for its value at the call, for a diagnostic.
    entry_measures: Vec<(usize, usize)>,
    /// The measures of the enclosing loops' rounds, each followed by how much it has changed.
    tracks: Vec<Track>,
    /// The `E0502`s a refined `var` might fix.
    retries: Vec<Retry>,
    /// When the SMT cross-check asks for them, the query of each obligation discharged so far.
    queries: Option<Vec<smt::Query>>,
    /// Each loop analysed outside the recording pass, with its entry state and its exit state,
    /// which depends on nothing else there: without it a loop would be analysed again for every
    /// round of every loop around it, in time exponential in their nesting.
    /// The loop whose header, its condition and measures at the head of a round, is being
    /// read.
    head_of: Option<StmtId>,
    loop_memo: Vec<(StmtId, Octagon, Octagon)>,
    /// The passes over a loop's body, for the test that keeps them linear in nesting.
    passes: usize,
    /// The `match` arm written as an expression being checked, before which a `guard` goes
    /// once the arm is made a block, when no statement inside it is the innermost.
    arm: Option<ExprId>,
    /// Whether the value being narrowed is an `if`'s or a `match`'s, which a `guard` that repeats it
    /// would give a variable of its own and tell Tier 1 nothing of.
    unfollowed: bool,
    /// The declared shape of each pattern binding: its payload field's, in its enum's tree.
    pattern_shapes: Vec<Option<Shape>>,
}

/// An `E0502` that declaring a `var` refined might fix: the diagnostic's index in the report,
/// the type and refinement the `var` would be declared with, and the binding the value reads,
/// when it is one.
struct Retry {
    diagnostic: usize,
    declared: (String, Refinement),
    read: Option<usize>,
}

/// A loop measure linear in bindings: `delta` follows its change since the round's start,
/// moved at each assignment to a binding in `terms` by the interval of that binding's change,
/// so that a decrease along every path survives the joins of the body.
struct Track {
    delta: usize,
    terms: Vec<Term>,
}

/// Checks one function, whose callees are `callees` in module order (`None` for a function
/// without a body). `src` is the text its spans point into, for the fixes.
pub fn check(f: &Function, callees: &[Option<Callee>], src: &str) -> Report {
    checked(f, callees, src, false).0
}

/// [`check`], with each obligation Tier 1 discharges written as an SMT-LIB 2 query for the SMT
/// cross-check ([`smt`]). Only `mn smt` asks for them, and [`check`] spends nothing on them.
pub fn check_with_queries(
    f: &Function,
    callees: &[Option<Callee>],
    src: &str,
) -> (Report, Vec<smt::Query>) {
    checked(f, callees, src, true)
}

fn checked(
    f: &Function,
    callees: &[Option<Callee>],
    src: &str,
    queries: bool,
) -> (Report, Vec<smt::Query>) {
    let (mut report, retries, queries) = run(f, callees, src, queries);
    for r in &retries {
        if let Some(fix) = refined_var_fix(f, callees, src, &report.diagnostics, r) {
            report.diagnostics[r.diagnostic].fixes.insert(0, fix);
        }
    }
    report.diagnostics.sort_by_key(|d| d.primary.start);
    (report, queries)
}

/// One pass of Tier 1 over a function, its diagnostics in the order it found them, and the
/// query of each obligation it discharged when `queries`.
fn run(
    f: &Function,
    callees: &[Option<Callee>],
    src: &str,
    queries: bool,
) -> (Report, Vec<Retry>, Vec<smt::Query>) {
    let mut fx = Fx::new(f, callees, src);
    if queries {
        fx.queries = Some(Vec::new());
    }
    fx.route();
    let mut st = Octagon::new(fx.count);
    fx.entry(&mut st);
    fx.body(&mut st);
    (fx.report, fx.retries, fx.queries.unwrap_or_default())
}

/// The fix that declares a `var` refined, offered before the `guard` for the `E0502` of `r`, when
/// the value is a binding's name: Tier 1 checks the function again with each candidate declared
/// so, in the order of declaration, and offers the first after which that obligation holds and no
/// other fails. The candidates are that binding and the bindings named with it in a comparison,
/// each a `var` of the refinement's integer type declared without a refinement. A refinement of
/// any other `var` gives Tier 1 no fact about the value, and trying it would only slow the
/// re-check of an edit. It runs only once an obligation has failed, so code that proves pays
/// nothing for it.
fn refined_var_fix(
    f: &Function,
    callees: &[Option<Callee>],
    src: &str,
    before: &[Diagnostic],
    r: &Retry,
) -> Option<Fix> {
    let (name, refinement) = &r.declared;
    let key = |d: &Diagnostic| (d.code, d.primary.start, d.primary.end);
    let failed = key(&before[r.diagnostic]);
    let bindings = &f.resolution.bindings;
    let tree = &f.syntax.tree;
    for b in compared_with(tree, f.resolution, r.read?) {
        let Some(t) = bindings[b]
            .ty
            .filter(|_| bindings[b].kind == BindingKind::Var)
        else {
            continue;
        };
        let TypeKind::Int {
            name: declared,
            refinement: None,
        } = &tree.ty(t).kind
        else {
            continue;
        };
        if declared != name {
            continue;
        }
        let mut syntax = f.syntax.clone();
        syntax.tree.types[t.0 as usize].kind = TypeKind::Int {
            name: name.clone(),
            refinement: Some(refinement.clone()),
        };
        let g = Function {
            syntax: &syntax,
            ..*f
        };
        let after = run(&g, callees, src, false).0.diagnostics;
        let fixed = after.iter().all(|d| key(d) != failed);
        let nothing_new = after
            .iter()
            .all(|d| before.iter().any(|o| key(o) == key(d)));
        if fixed && nothing_new {
            let text = refined_type_text(name, refinement);
            return Some(Fix {
                message: format!("declare `{}` as `{}`", bindings[b].name, text),
                applicability: Applicability::MachineApplicable,
                edits: vec![Edit {
                    span: tree.ty(t).span,
                    replacement: text,
                }],
            });
        }
    }
    None
}

/// Binding `b` and the bindings named with it in a comparison of the function, by index in the
/// order of declaration. A name counts where arithmetic reaches it, as in `lo + 1 < hi`, and not
/// inside an index: `a[i] > a[best]` compares elements, and says nothing of `i` and `best`.
fn compared_with(tree: &Tree, resolution: &Resolution, b: usize) -> BTreeSet<usize> {
    let mut out = BTreeSet::from([b]);
    for (i, e) in tree.exprs.iter().enumerate() {
        let comparison = matches!(
            e.kind,
            ExprKind::Binary {
                op: BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge,
                ..
            }
        );
        if comparison {
            let names = named(tree, resolution, ExprId(i as u32));
            if names.contains(&b) {
                out.extend(names);
            }
        }
    }
    out
}

/// The bindings `e` names where arithmetic reaches them, by index.
fn named(tree: &Tree, resolution: &Resolution, e: ExprId) -> Vec<usize> {
    let mut out: Vec<usize> = resolution.names[e.0 as usize]
        .iter()
        .map(|id| id.0 as usize)
        .collect();
    match &tree.expr(e).kind {
        ExprKind::Binary { lhs, rhs, .. } => {
            out.extend(named(tree, resolution, *lhs));
            out.extend(named(tree, resolution, *rhs));
        }
        ExprKind::Unary { operand, .. } => out.extend(named(tree, resolution, *operand)),
        // A name is itself; arithmetic does not reach inside the others: an index or a field
        // reads an element, and a call, an aggregate, an `if` or a `match` gives a value of its
        // own.
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Call { .. }
        | ExprKind::Index { .. }
        | ExprKind::Field { .. }
        | ExprKind::Array(_)
        | ExprKind::Repeat { .. }
        | ExprKind::Construct { .. }
        | ExprKind::Variant { .. }
        | ExprKind::If { .. }
        | ExprKind::Match { .. }
        | ExprKind::Error => {}
    }
    out
}

/// The declared shape of each binding a `match` pattern makes, by binding: its payload field's,
/// in its enum's tree, refinements included; none for every other binding.
fn pattern_shapes(f: &Function) -> Vec<Option<Shape>> {
    let mut out = vec![None; f.resolution.bindings.len()];
    for (m, e) in f.syntax.tree.exprs.iter().enumerate() {
        let ExprKind::Match { arms, .. } = &e.kind else {
            continue;
        };
        for (k, arm) in arms.iter().enumerate() {
            let PatternKind::Variant {
                enum_name,
                name,
                bindings: Some(bs),
                ..
            } = &arm.pattern.kind
            else {
                continue;
            };
            let Some(def) = f.structs.get_enum(enum_name) else {
                continue;
            };
            let Some((_, v)) = def.variant(name) else {
                continue;
            };
            for (i, b) in bs.iter().enumerate() {
                let field = match &b.field {
                    Some((n, _)) => v.field(n).map(|(_, fd)| fd),
                    None => v.fields.get(i),
                };
                let (Some(fd), Some(id)) =
                    (field, f.resolution.patterns.get(&(ExprId(m as u32), k, i)))
                else {
                    continue;
                };
                out[id.0 as usize] = Some(Fx::declared_shape(def.tree(), fd.decl));
            }
        }
    }
    out
}

fn is_int_type(tree: &Tree, t: TypeId) -> bool {
    matches!(tree.ty(t).kind, TypeKind::Int { .. })
}

impl<'a> Fx<'a> {
    fn new(f: &Function<'a>, callees: &'a [Option<Callee<'a>>], src: &'a str) -> Fx<'a> {
        let tree = &f.syntax.tree;
        let mut n = Counter(0);
        let result = n.alloc(f.syntax.result.is_some_and(|t| is_int_type(tree, t)));
        let vars = f
            .resolution
            .bindings
            .iter()
            .zip(&f.typing.bindings)
            .map(|(b, t)| match b.kind {
                BindingKind::Result => result,
                BindingKind::Param { .. }
                | BindingKind::Val
                | BindingKind::Var
                | BindingKind::Pattern => n.alloc(matches!(t, Ty::Int(_))),
            })
            .collect();
        // The rest of the function's variables, a field read's included, come first. A
        // binding's paths are followed only where they fit in what is left of the cap, so that
        // with them the function stays within it; past it, they are not, and its fields read
        // as their types and refinements, as elements do.
        let Rest {
            sites,
            temps,
            scratch,
            probe,
            measures,
            entry_measures,
        } = Fx::rest(f, callees, &mut n);
        let mut room = RELATIONAL_LIMIT.saturating_sub(n.0);
        let paths: Vec<Vec<FieldPath>> = f
            .typing
            .bindings
            .iter()
            .map(|t| {
                let Ty::Struct(s) = t else {
                    return Vec::new();
                };
                let mut left = room;
                let Some(listed) = scalar_paths(f.structs, s, &mut left) else {
                    return Vec::new();
                };
                room = left;
                listed
                    .into_iter()
                    .map(|(names, shape)| FieldPath {
                        names,
                        var: n.alloc(true).unwrap_or(0),
                        shape,
                    })
                    .collect()
            })
            .collect();
        let mut fx = Fx {
            syntax: f.syntax,
            tree,
            resolution: f.resolution,
            typing: f.typing,
            routing: f.routing,
            structs: f.structs,
            callees,
            src,
            vars,
            paths,
            built: vec![None; tree.exprs.len()],
            sites,
            result,
            result_shape: None,
            temps,
            scratch,
            probe,
            count: n.0,
            report: Report::default(),
            recording: true,
            loops: Vec::new(),
            scope: Vec::new(),
            stmt: None,
            heads: Vec::new(),
            measures,
            entry_measures,
            tracks: Vec::new(),
            retries: Vec::new(),
            queries: None,
            head_of: None,
            loop_memo: Vec::new(),
            passes: 0,
            arm: None,
            unfollowed: false,
            pattern_shapes: pattern_shapes(f),
        };
        let shape = f.syntax.result.map(|t| fx.own_shape(t));
        fx.result_shape = shape;
        fx
    }

    /// The variables of a function besides its bindings' and their paths': each call site's,
    /// each value site's, a field read's included, which a path it follows leaves unused,
    /// the temporaries, the scratch and the probe, and each measure's.
    fn rest(f: &Function<'a>, callees: &'a [Option<Callee<'a>>], n: &mut Counter) -> Rest {
        let tree = &f.syntax.tree;
        let sites = tree
            .exprs
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let int = matches!(f.typing.exprs[i], Ty::Int(_));
                match &e.kind {
                    ExprKind::Call { .. } => {
                        let callee = f.resolution.calls[i];
                        match callee.and_then(|c| callees.get(c).copied().flatten()) {
                            Some(c) => {
                                let ct = &c.syntax.tree;
                                let params = &c.syntax.params;
                                Site::Call(CallSite {
                                    callee: callee.unwrap_or(0),
                                    ins: params
                                        .iter()
                                        .map(|p| n.alloc(is_int_type(ct, p.ty)))
                                        .collect(),
                                    outs: params
                                        .iter()
                                        .map(|p| n.alloc(p.is_mut && is_int_type(ct, p.ty)))
                                        .collect(),
                                    result: n
                                        .alloc(c.syntax.result.is_some_and(|t| is_int_type(ct, t))),
                                    paths: params
                                        .iter()
                                        .map(|p| {
                                            let Ty::Struct(s) = Ty::of(ct, p.ty) else {
                                                return Vec::new();
                                            };
                                            let mut read = Vec::new();
                                            for clause in &c.syntax.contracts {
                                                clause_paths(ct, clause.expr, &p.name, &mut read);
                                            }
                                            read.sort();
                                            read.dedup();
                                            read.into_iter()
                                                .filter_map(|names| {
                                                    let shape = path_shape(f.structs, &s, &names)?;
                                                    Some(FieldPath {
                                                        names,
                                                        var: n.alloc(true).unwrap_or(0),
                                                        shape,
                                                    })
                                                })
                                                .collect()
                                        })
                                        .collect(),
                                })
                            }
                            None => Site::None,
                        }
                    }
                    ExprKind::If { .. }
                    | ExprKind::Match { .. }
                    | ExprKind::Index { .. }
                    | ExprKind::Field { .. }
                    | ExprKind::Binary {
                        op: BinaryOp::Rem, ..
                    } if int => Site::Value(n.alloc(true).unwrap_or(0)),
                    // A literal beyond the octagon's reach is a variable whose exact bounds
                    // hold its value.
                    ExprKind::Int { value, .. } if Exact::new(false, *value).is_far() => {
                        Site::Value(n.alloc(true).unwrap_or(0))
                    }
                    ExprKind::Int { .. }
                    | ExprKind::Bool(_)
                    | ExprKind::Name(_)
                    | ExprKind::Todo(_)
                    | ExprKind::Unreachable(_)
                    | ExprKind::Unary { .. }
                    | ExprKind::Binary { .. }
                    | ExprKind::Index { .. }
                    | ExprKind::Field { .. }
                    | ExprKind::Construct { .. }
                    | ExprKind::Variant { .. }
                    | ExprKind::Match { .. }
                    | ExprKind::Array(_)
                    | ExprKind::Repeat { .. }
                    | ExprKind::If { .. }
                    | ExprKind::Error => Site::None,
                }
            })
            .collect();
        let temps = [n.alloc(true).unwrap_or(0), n.alloc(true).unwrap_or(0)];
        let scratch = n.alloc(true).unwrap_or(0);
        let probe = n.alloc(true).unwrap_or(0);
        let measures = tree
            .stmts
            .iter()
            .map(|s| match &s.kind {
                StmtKind::While { decreases, .. } => decreases
                    .iter()
                    .map(|_| (n.alloc(true).unwrap_or(0), n.alloc(true).unwrap_or(0)))
                    .collect(),
                StmtKind::Binding { .. }
                | StmtKind::Assign { .. }
                | StmtKind::Guard { .. }
                | StmtKind::Return(_)
                | StmtKind::Break
                | StmtKind::Continue
                | StmtKind::Discard(_)
                | StmtKind::Expr(_) => Vec::new(),
            })
            .collect();
        let entry_measures = f
            .syntax
            .contracts
            .iter()
            .filter(|c| c.kind == ContractKind::Decreases)
            .map(|_| (n.alloc(true).unwrap_or(0), n.alloc(true).unwrap_or(0)))
            .collect();
        Rest {
            sites,
            temps,
            scratch,
            probe,
            measures,
            entry_measures,
        }
    }

    // -- names, types and texts --------------------------------------------------------------

    /// The first binding named `name`, `result` aside: M1 has no shadowing.
    fn named(&self, name: &str) -> Option<usize> {
        self.resolution
            .bindings
            .iter()
            .position(|b| b.name == name && b.kind != BindingKind::Result)
    }

    fn own_bound(&self, b: &Bound) -> Val {
        match b {
            Bound::Int { negative, value } => bound_int(*negative, *value),
            Bound::Name(n) => self
                .named(n)
                .and_then(|i| self.vars[i])
                .map_or(TOP, var_val),
            Bound::Len(n) => match self.named(n).map(|i| &self.typing.bindings[i]) {
                Some(Ty::Array(len, _)) => int_literal(*len),
                _ => TOP,
            },
        }
    }

    fn own_shape(&self, t: TypeId) -> Shape {
        let tree = self.tree;
        match &tree.ty(t).kind {
            TypeKind::Int { name, refinement } => Shape::Int {
                int: IntTy::named(name),
                refine: refinement.as_ref().map(|r| {
                    Box::new(Refine {
                        lo: self.own_bound(&r.lo),
                        hi: self.own_bound(&r.hi),
                        inclusive: r.inclusive,
                        lo_text: bound_text(&r.lo),
                        hi_text: bound_text(&r.hi),
                        text: tree.render_type(t),
                        declared: literal_refinement(name, r),
                        lo_lit: bound_exact(&r.lo),
                        hi_lit: bound_exact(&r.hi),
                    })
                }),
            },
            TypeKind::Array { len, elem } => Shape::Array(*len, Box::new(self.own_shape(*elem))),
            TypeKind::Named(name) => Shape::Struct(name.clone()),
            TypeKind::Bool | TypeKind::Enum(_) | TypeKind::Error => Shape::Other,
        }
    }

    fn binding_shape(&self, b: usize) -> Shape {
        if let Some(shape) = self.pattern_shapes.get(b).cloned().flatten() {
            return shape;
        }
        match self.resolution.bindings[b].ty {
            Some(t) => self.own_shape(t),
            None => Shape::plain(&self.typing.bindings[b]),
        }
    }

    /// The shape of a callee's type at a call: its refinements' bounds name the call's
    /// arguments.
    fn callee_shape(&self, c: &Callee, site: &CallSite, args: &[Arg], t: TypeId) -> Shape {
        let tree = &c.syntax.tree;
        let param = |n: &str| c.syntax.params.iter().position(|p| p.name == n);
        let len = |k: usize| match &tree.ty(c.syntax.params[k].ty).kind {
            TypeKind::Array { len, .. } => Some(*len),
            TypeKind::Int { .. }
            | TypeKind::Bool
            | TypeKind::Named(_)
            | TypeKind::Enum(_)
            | TypeKind::Error => None,
        };
        let bound = |b: &Bound| match b {
            Bound::Int { negative, value } => bound_int(*negative, *value),
            Bound::Name(n) => param(n).and_then(|k| site.ins[k]).map_or(TOP, var_val),
            Bound::Len(n) => param(n).and_then(len).map_or(TOP, int_literal),
        };
        let text = |b: &Bound| match b {
            Bound::Name(n) => param(n)
                .and_then(|k| args.get(k))
                .map_or(n.clone(), |a| paren(self.own_text(a.value), OPERAND)),
            Bound::Len(n) => param(n)
                .and_then(len)
                .map_or(bound_text(b), |l| l.to_string()),
            Bound::Int { .. } => bound_text(b),
        };
        match &tree.ty(t).kind {
            TypeKind::Int { name, refinement } => Shape::Int {
                int: IntTy::named(name),
                refine: refinement.as_ref().map(|r| {
                    Box::new(Refine {
                        lo: bound(&r.lo),
                        hi: bound(&r.hi),
                        inclusive: r.inclusive,
                        lo_text: text(&r.lo),
                        hi_text: text(&r.hi),
                        text: tree.render_type(t),
                        declared: literal_refinement(name, r),
                        lo_lit: bound_exact(&r.lo),
                        hi_lit: bound_exact(&r.hi),
                    })
                }),
            },
            TypeKind::Array { len, elem } => {
                Shape::Array(*len, Box::new(self.callee_shape(c, site, args, *elem)))
            }
            // A struct tells a call's checks nothing more: its fields hold their refinements.
            TypeKind::Bool | TypeKind::Named(_) | TypeKind::Enum(_) | TypeKind::Error => {
                Shape::Other
            }
        }
    }

    /// The shape of field `field` of struct `name`, as its declaration gives it: a field's
    /// refinement has literal bounds.
    fn field_shape(&self, name: &str, field: &str) -> Shape {
        let Some(def) = self.structs.get(name) else {
            return Shape::Other;
        };
        let Some((_, f)) = def.field(field) else {
            return Shape::Other;
        };
        Self::declared_shape(def.tree(), f.decl)
    }

    /// The shape a type of a struct's declaration names, in its tree.
    fn declared_shape(tree: &Tree, t: TypeId) -> Shape {
        match &tree.ty(t).kind {
            TypeKind::Int { name, refinement } => Shape::Int {
                int: IntTy::named(name),
                refine: refinement.as_ref().map(|r| {
                    let bound = |b: &Bound| match b {
                        Bound::Int { negative, value } => bound_int(*negative, *value),
                        Bound::Name(_) | Bound::Len(_) => TOP,
                    };
                    Box::new(Refine {
                        lo: bound(&r.lo),
                        hi: bound(&r.hi),
                        inclusive: r.inclusive,
                        lo_text: bound_text(&r.lo),
                        hi_text: bound_text(&r.hi),
                        text: tree.render_type(t),
                        declared: literal_refinement(name, r),
                        lo_lit: bound_exact(&r.lo),
                        hi_lit: bound_exact(&r.hi),
                    })
                }),
            },
            TypeKind::Array { len, elem } => {
                Shape::Array(*len, Box::new(Self::declared_shape(tree, *elem)))
            }
            TypeKind::Named(name) => Shape::Struct(name.clone()),
            TypeKind::Bool | TypeKind::Enum(_) | TypeKind::Error => Shape::Other,
        }
    }

    /// The shape of the place a root binding and its steps name.
    fn place_shape(&self, root: Option<usize>, steps: &[Step]) -> Shape {
        let mut shape = root.map_or(Shape::Other, |b| self.binding_shape(b));
        for step in steps {
            shape = match (step, &shape) {
                (Step::Index(_), _) => shape.element(),
                (Step::Field(f), Shape::Struct(name)) => self.field_shape(name, f),
                (Step::Field(_), _) => Shape::Other,
            };
        }
        shape
    }

    /// An expression of this function as written, with how tightly it binds.
    /// Whether Tier 1 follows the value of `e`, so that a `guard` on it narrows what a later
    /// use reads: a literal, an integer binding, a field path it follows, an array's `.len`,
    /// and their sums, differences and negations.
    fn followed(&self, e: ExprId) -> bool {
        match &self.tree.expr(e).kind {
            ExprKind::Int { .. } => true,
            ExprKind::Name(_) => self.resolution.names[e.0 as usize]
                .is_some_and(|b| self.vars[b.0 as usize].is_some()),
            ExprKind::Field { base, name } => {
                self.path_var(e).is_some()
                    || (name == "len"
                        && matches!(self.typing.exprs[base.0 as usize], Ty::Array(..)))
            }
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => self.followed(*operand),
            ExprKind::Binary {
                op: BinaryOp::Add | BinaryOp::Sub,
                lhs,
                rhs,
            } => self.followed(*lhs) && self.followed(*rhs),
            ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Index { .. }
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

    /// The caller's text, at a call with `args`, for `e`, a parameter of `callee` or a field
    /// path of one in its clause: the argument's text, or, for a field path, the field's value
    /// in a struct literal or the path of the argument.
    fn arg_text(&self, callee: &FnSyntax, args: &[Arg], e: ExprId) -> Option<(String, u8)> {
        self.arg_reading(callee, args, e).map(|(text, _)| text)
    }

    /// Whether Tier 1 follows every value `e`, a clause of `callee` in the Tier-1 fragment,
    /// reads at a call with `args`, so that a `guard` written in the caller's terms narrows
    /// what the clause reads.
    fn clause_followed(&self, callee: &FnSyntax, args: &[Arg], e: ExprId) -> bool {
        let tree = &callee.tree;
        let read = |e: ExprId| {
            let (root, names) = chain_of(tree, e)?;
            let k = callee.params.iter().position(|p| p.name == *root)?;
            Some(self.reading(args.get(k)?.value, &names).1)
        };
        fragment_followed(tree, e, &read)
    }

    /// [`Self::arg_text`], and whether Tier 1 follows the value that text reads.
    fn arg_reading(
        &self,
        callee: &FnSyntax,
        args: &[Arg],
        e: ExprId,
    ) -> Option<((String, u8), bool)> {
        let (root, names) = chain_of(&callee.tree, e)?;
        let k = callee.params.iter().position(|p| p.name == *root)?;
        Some(self.reading(args.get(k)?.value, &names))
    }

    /// The text, in this function, of field path `names` of `at`, an expression of it, and
    /// whether Tier 1 follows the value that text reads: a struct literal's field is its value,
    /// and the rest of the path is read from `at` as it is.
    fn reading(&self, mut at: ExprId, names: &[String]) -> ((String, u8), bool) {
        for (i, name) in names.iter().enumerate() {
            if let ExprKind::Construct { fields, .. } = &self.tree.expr(at).kind {
                if let Some(f) = fields.iter().find(|f| f.name == *name) {
                    at = f.value;
                    continue;
                }
            }
            let rest = &names[i..];
            let text = format!("{}.{}", paren(self.own_text(at), 12), rest.join("."));
            // A path Tier 1 follows, or an array's only field, `.len`, which is a constant.
            let followed = field_chain(self.tree, self.resolution, at).is_some_and(|(r, pre)| {
                let mut full = pre;
                full.extend(rest.iter().cloned());
                self.paths[r].iter().any(|p| p.names == full)
            }) || matches!(self.typing.exprs[at.0 as usize], Ty::Array(..));
            return ((text, 12), followed);
        }
        (self.own_text(at), self.followed(at))
    }

    /// For a field path of `result` in this function's clause, or `result` itself: its text
    /// for a returned `value`, and whether Tier 1 follows what that text reads.
    fn result_reading(&self, e: ExprId, value: ExprId) -> Option<((String, u8), bool)> {
        let (b, names) = field_chain(self.tree, self.resolution, e)?;
        (self.resolution.bindings[b].kind == BindingKind::Result)
            .then(|| self.reading(value, &names))
    }

    /// The variable of `e` when it is a scalar field path Tier 1 follows.
    fn path_var(&self, e: ExprId) -> Option<usize> {
        let (root, names) = field_chain(self.tree, self.resolution, e)?;
        let p = self.paths.get(root)?.iter().find(|p| p.names == names)?;
        Some(p.var)
    }

    /// The paths of binding `b` under `prefix`, each with its names after the prefix.
    fn paths_under(&self, b: usize, prefix: &[String]) -> Vec<(Vec<String>, FieldPath)> {
        self.paths.get(b).map_or(Vec::new(), |ps| {
            ps.iter()
                .filter(|p| p.names.starts_with(prefix))
                .map(|p| (p.names[prefix.len()..].to_vec(), p.clone()))
                .collect()
        })
    }

    /// The variables a binding's value is held in: its own and its field paths'.
    fn vars_of(&self, b: usize) -> Vec<usize> {
        let paths = self.paths.get(b).into_iter().flatten().map(|p| p.var);
        self.vars
            .get(b)
            .copied()
            .flatten()
            .into_iter()
            .chain(paths)
            .collect()
    }

    /// The scalar field paths of `e`, a value of a struct type already evaluated, with their
    /// values: a struct literal's as it built them, and a followed place's variables.
    fn struct_vals(&self, e: ExprId) -> Vec<(Vec<String>, Val)> {
        if let (ExprKind::Construct { .. }, Some(built)) =
            (&self.tree.expr(e).kind, &self.built[e.0 as usize])
        {
            return built.clone();
        }
        // Any other value's paths are left out: where they land, each takes its declared
        // bounds.
        field_chain(self.tree, self.resolution, e).map_or(Vec::new(), |(root, prefix)| {
            self.paths_under(root, &prefix)
                .into_iter()
                .map(|(rel, p)| (rel, var_val(p.var)))
                .collect()
        })
    }

    /// Binding `b`'s paths under `prefix` take the value of `e`, a struct already evaluated,
    /// all at once: a value that reads another of them reads it as it was.
    fn land(&mut self, st: &mut Octagon, b: usize, prefix: &[String], e: ExprId) {
        let vals = self.struct_vals(e);
        let mut dests: Vec<(FieldPath, Val)> = self
            .paths_under(b, prefix)
            .into_iter()
            .map(|(rel, p)| {
                let v = match vals.iter().find(|(n, _)| *n == rel) {
                    Some((_, v)) => v.clone(),
                    None => self.summary(st, &p.shape),
                };
                (p, v)
            })
            .collect();
        for i in 0..dests.len() {
            let var = dests[i].0.var;
            for later in dests.iter_mut().skip(i + 1) {
                if mentions(&later.1, var) {
                    later.1 = summary_of(st, &later.1);
                }
            }
            let (p, v) = dests[i].clone();
            self.track(st, p.var, &v);
            self.assign(st, p.var, &v);
            self.facts(st, p.var, &p.shape);
        }
    }

    /// Binding `b`'s paths under `prefix` keep only their types and refinements: after a call
    /// that takes them `mut`, and at the head of a loop that changes them.
    fn havoc_paths(&self, st: &mut Octagon, b: usize, prefix: &[String]) {
        for (_, p) in self.paths_under(b, prefix) {
            self.track(st, p.var, &TOP);
            st.forget(p.var);
            self.facts(st, p.var, &p.shape);
        }
    }

    /// The variable, at call `site`, of a callee's clause's field path `e` of one of its
    /// parameters.
    fn callee_path_var(
        &self,
        c: &Callee,
        site: &CallSite,
        e: ExprId,
        after: bool,
    ) -> Option<usize> {
        let tree = &c.syntax.tree;
        let mut names = Vec::new();
        let mut at = e;
        while let ExprKind::Field { base, name } = &tree.expr(at).kind {
            names.insert(0, name.clone());
            at = *base;
        }
        let ExprKind::Name(n) = &tree.expr(at).kind else {
            return None;
        };
        let k = c.syntax.params.iter().position(|p| p.name == *n)?;
        if after && c.syntax.params[k].is_mut {
            return None;
        }
        let p = site.paths.get(k)?.iter().find(|p| p.names == names)?;
        Some(p.var)
    }

    /// The value of `e` exactly, when it is an integer literal, negated or not.
    fn literal(&self, e: ExprId) -> Option<Exact> {
        match &self.tree.expr(e).kind {
            ExprKind::Int { value, .. } => Some(Exact::new(false, *value)),
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => self.literal(*operand).map(Exact::negated),
            ExprKind::Bool(_)
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
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => None,
        }
    }

    fn own_text(&self, e: ExprId) -> (String, u8) {
        let expr = self.tree.expr(e);
        match self.src.get(expr.span.start..expr.span.end) {
            Some(text) => (text.to_string(), prec(&expr.kind)),
            None => print(self.tree, e, &no_subst),
        }
    }

    fn own_int(&self, name: &str) -> bool {
        self.resolution
            .bindings
            .iter()
            .zip(&self.typing.bindings)
            .any(|(b, t)| b.name == name && matches!(t, Ty::Int(_)))
    }

    fn outside(&self, e: ExprId) -> Option<String> {
        let path = |n: &str, names: &[String]| {
            let (_, t) = self
                .resolution
                .bindings
                .iter()
                .zip(&self.typing.bindings)
                .find(|(b, _)| b.name == n)?;
            path_ty(self.structs, t, names)
        };
        let names = Names {
            is_int: &|n| self.own_int(n),
            path_ty: &path,
        };
        outside_fragment(self.tree, e, &names)
    }

    fn callee_outside(&self, c: &Callee, e: ExprId) -> Option<String> {
        let s = c.syntax;
        // Only `requires` clauses are classified at a call, where `result` is not visible.
        let is_int = |n: &str| {
            s.params
                .iter()
                .any(|p| p.name == n && is_int_type(&s.tree, p.ty))
        };
        let path = |n: &str, names: &[String]| {
            let p = s.params.iter().find(|p| p.name == n)?;
            path_ty(self.structs, &Ty::of(&s.tree, p.ty), names)
        };
        let names = Names {
            is_int: &is_int,
            path_ty: &path,
        };
        outside_fragment(&s.tree, e, &names)
    }

    fn span(&self, e: ExprId) -> Span {
        self.tree.expr(e).span
    }

    // -- diagnostics -------------------------------------------------------------------------

    /// Records an obligation, proven or not. `paths` gives the states Tier 1 decided it on, each
    /// with the obligation as it stands there, computing again what the decision computed where
    /// it must. It is called only here, and only for an obligation proven while the SMT
    /// cross-check asks for queries, so that each query is built from the very states of the
    /// decision and nothing is spent on it otherwise.
    fn record(
        &mut self,
        kind: ObligationKind,
        span: Span,
        must: bool,
        proven: bool,
        tag: Tag,
        paths: impl FnOnce(&mut Self) -> Vec<Path>,
    ) {
        let o = Obligation {
            kind,
            span,
            must,
            proven,
            detail: String::new(),
            tag,
        };
        self.record_check(o, paths);
    }

    fn record_check(&mut self, o: Obligation, paths: impl FnOnce(&mut Self) -> Vec<Path>) {
        if !self.recording {
            return;
        }
        if o.proven && self.queries.is_some() {
            let paths = self.quietly(paths);
            let query = smt::Query {
                obligation: self.report.obligations.len(),
                text: smt::query(self, &o, paths),
            };
            if let Some(queries) = &mut self.queries {
                queries.push(query);
            }
        }
        self.report.obligations.push(o);
    }

    /// Runs `f` with nothing recorded: what it evaluates again records no obligation and emits
    /// no diagnostic.
    fn quietly<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let was = self.recording;
        self.recording = false;
        let out = f(self);
        self.recording = was;
        out
    }

    fn anchor(&self) -> Anchor {
        match (self.stmt, self.syntax.body) {
            (Some(s), _) => Anchor::Before(s),
            (None, _) if self.arm.is_some() => self.arm.map_or(Anchor::Nowhere, Anchor::Arm),
            (None, Some(Body::Expr(e))) => Anchor::Body(e),
            _ => Anchor::Nowhere,
        }
    }

    /// A point of a failing state over the bindings in scope and `named`, as text, then the
    /// values the same point gives `extra`.
    fn valuation(
        &self,
        st: &Octagon,
        named: &[(String, usize)],
        extra: &[usize],
    ) -> (String, Vec<Exact>) {
        let mut names = Vec::new();
        let mut vars = Vec::new();
        for &b in &self.scope {
            let name = &self.resolution.bindings[b].name;
            if let Some(v) = self.vars[b] {
                names.push(name.clone());
                vars.push(v);
            }
            for p in &self.paths[b] {
                names.push(format!("{}.{}", name, p.names.join(".")));
                vars.push(p.var);
            }
        }
        for (n, v) in named {
            names.push(n.clone());
            vars.push(*v);
        }
        vars.extend(extra);
        let picked = st.pick(&vars).unwrap_or_default();
        let text: Vec<String> = names
            .iter()
            .zip(&picked)
            .map(|(n, v)| format!("{} = {}", n, v))
            .collect();
        let values = picked.iter().skip(names.len()).copied().collect();
        (text.join(", "), values)
    }

    /// A `guard` stating clause `cond` where Tier 1 follows every value it reads, so that the
    /// guard narrows what the clause reads; otherwise none, and the message says why.
    fn usable_guard(
        cond: String,
        followed: bool,
        at: &str,
        message: String,
    ) -> (Option<String>, String) {
        if followed {
            return (Some(cond), message);
        }
        let why = format!(
            "{}; {} the clause is `{}`, which reads a value Tier 1 does not follow, so no `guard` it can use states it",
            message, at, cond
        );
        (None, why)
    }

    /// `Tier 1 cannot rule out ...`: `with` follows a valuation, `without` stands for one.
    fn cannot_rule_out(valuation: &str, with: &str, without: &str) -> String {
        if valuation.is_empty() {
            format!("Tier 1 cannot rule out that {}", without)
        } else {
            format!("Tier 1 cannot rule out `{}`, {}", valuation, with)
        }
    }

    fn point(&self, byte: usize) -> Span {
        let (line, col) = line_col(self.src, byte);
        Span {
            start: byte,
            end: byte,
            line: line as u32,
            col: col as u32,
        }
    }

    fn line_start(&self, byte: usize) -> usize {
        self.src
            .get(..byte)
            .and_then(|s| s.rfind('\n'))
            .map_or(0, |i| i + 1)
    }

    fn indent(&self, line_start: usize) -> String {
        let rest = self.src.get(line_start..).unwrap_or("");
        rest.chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    /// The edit that inserts `line`, a `guard`, at `anchor`.
    fn guard_edit(&self, anchor: Anchor, line: &str) -> Option<Edit> {
        match anchor {
            Anchor::Before(s) => {
                let span = self.tree.stmt(s).span;
                let ls = self.line_start(span.start);
                let lead = self.src.get(ls..span.start)?;
                Some(if lead.trim().is_empty() {
                    Edit {
                        span: self.point(ls),
                        replacement: format!("{}{}\n", lead, line),
                    }
                } else {
                    Edit {
                        span: self.point(span.start),
                        replacement: format!("{}\n{}", line, self.indent(ls)),
                    }
                })
            }
            Anchor::After(b) => {
                let block = self.tree.block(b);
                Some(match block.stmts.last() {
                    Some(&s) => {
                        let span = self.tree.stmt(s).span;
                        Edit {
                            span: self.point(span.end),
                            replacement: format!(
                                "\n{}{}",
                                self.indent(self.line_start(span.start)),
                                line
                            ),
                        }
                    }
                    None => Edit {
                        span: self.point(block.span.start + 1),
                        replacement: format!(
                            "\n{}  {}",
                            self.indent(self.line_start(block.span.start)),
                            line
                        ),
                    },
                })
            }
            Anchor::Body(e) => {
                let span = self.span(e);
                let eq = self
                    .src
                    .get(..span.start)?
                    .trim_end()
                    .strip_suffix('=')?
                    .len();
                let ind = self.indent(self.line_start(self.syntax.name_span.start));
                let value = self.src.get(span.start..span.end)?;
                Some(Edit {
                    span: Span {
                        end: span.end,
                        ..self.point(eq)
                    },
                    replacement: format!(
                        "{{\n{ind}  {line}\n{ind}  {value}\n{ind}}}",
                        ind = ind,
                        line = line,
                        value = value
                    ),
                })
            }
            Anchor::Arm(x) => {
                let span = self.span(x);
                let ind = self.indent(self.line_start(span.start));
                let value = self.src.get(span.start..span.end)?;
                Some(Edit {
                    span,
                    replacement: format!(
                        "{{\n{ind}  {line}\n{ind}  {value}\n{ind}}}",
                        ind = ind,
                        line = line,
                        value = value
                    ),
                })
            }
            Anchor::Nowhere => None,
        }
    }

    fn emit(
        &mut self,
        code: &'static str,
        span: Span,
        message: String,
        guard: Option<String>,
        anchor: Anchor,
    ) {
        if !self.recording {
            return;
        }
        // A `guard` before a loop narrows only the state the loop is entered with: for what its
        // header reads at the head of a round, one that reads a binding the loop changes says
        // nothing of a later round, so none is offered.
        let (guard, message) = match (guard, anchor) {
            (Some(g), Anchor::Before(s)) if self.head_of == Some(s) && self.names_changed(s, &g) => (
                None,
                format!(
                    "{}; this loop changes what it reads, so a `guard` before the loop would not hold at a later round, and none is offered",
                    message
                ),
            ),
            (g, _) => (g, message),
        };
        let mut d = Diagnostic::error(code, span, message);
        if let Some(h) = self.heads.last() {
            let list = |v: &[String]| {
                v.iter()
                    .map(|c| format!("`{}`", c))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let kept = if h.kept.is_empty() {
                "no candidate fact".to_string()
            } else {
                list(&h.kept)
            };
            let dropped = if h.dropped.is_empty() {
                String::new()
            } else {
                format!(
                    ", and dropped {}, which it cannot show inductive",
                    list(&h.dropped)
                )
            };
            d = d.with_related(
                h.span,
                format!("at the head of this loop Tier 1 holds {}{}", kept, dropped),
            );
        }
        if let Some(cond) = guard {
            let line = guard_line(self.syntax.result.is_some(), &cond);
            if let Some(edit) = self.guard_edit(anchor, &line) {
                d.fixes.push(Fix {
                    message: format!("insert `{}`", line),
                    applicability: Applicability::Suggestion,
                    edits: vec![edit],
                });
            }
        }
        self.report.diagnostics.push(d);
    }

    /// Whether `text`, a condition in this function's terms, names a binding loop `s` changes.
    fn names_changed(&self, s: StmtId, text: &str) -> bool {
        let mut changed = Vec::new();
        self.changed_stmt(s, &mut changed);
        mente_syntax::tokenize(text).iter().any(|t| {
            matches!(&t.kind, mente_syntax::TokenKind::Ident(n)
                if self.named(n).is_some_and(|b| changed.contains(&b)))
        })
    }

    /// `E0503` for each clause outside the Tier-1 fragment, unless the function routes them;
    /// with `@prove`, each is a trusted clause.
    fn route(&mut self) {
        let mut clauses = Vec::new();
        for c in &self.syntax.contracts {
            match c.kind {
                ContractKind::Requires => clauses.push(("requires", c.expr)),
                ContractKind::Ensures => clauses.push(("ensures", c.expr)),
                ContractKind::Decreases => {}
            }
        }
        for s in &self.tree.stmts {
            if let StmtKind::While { invariants, .. } = &s.kind {
                clauses.extend(invariants.iter().map(|i| ("invariant", *i)));
            }
        }
        let name = &self.syntax.name;
        let ls = self.line_start(self.syntax.name_span.start);
        let at = self.point(ls);
        let ind = self.indent(ls);
        for (kw, e) in clauses {
            let Some(why) = self.outside(e) else {
                continue;
            };
            let clause = print(self.tree, e, &no_subst).0;
            if self.routing.prove {
                let of = if kw == "invariant" { "in" } else { "of" };
                let text = format!("{} {} {} {}", kw, clause, of, name);
                self.report.axioms.push((self.span(e), text));
                continue;
            }
            if self.routing.checked {
                continue;
            }
            let fix = |attr: &str, effect: &str| Fix {
                message: format!("mark `{}` `@{}`: {}", name, attr, effect),
                applicability: Applicability::Suggestion,
                edits: vec![Edit {
                    span: at,
                    replacement: format!("{}@{}\n", ind, attr),
                }],
            };
            let mut d = Diagnostic::error(
                "E0503",
                self.span(e),
                format!(
                    "`{} {}` lies outside the Tier-1 fragment, since it uses {}; mark `{}` `@checked` to check it at run time, or `@prove` to make it a trusted axiom until `mn prove` runs",
                    kw, clause, why, name
                ),
            );
            d.fixes
                .push(fix("checked", "the clause becomes a run-time check"));
            d.fixes.push(fix(
                "prove",
                "the clause becomes a trusted axiom until `mn prove` runs",
            ));
            self.report.diagnostics.push(d);
        }
    }

    // -- the octagon operations that need scratch variables ----------------------------------

    /// Gives `var` the value `v`.
    fn assign(&self, st: &mut Octagon, var: usize, v: &Val) {
        let mentions = |t: &[Term]| t.iter().any(|t| t.var == var);
        let self_ref = match v {
            Val::Lin(t, _) => mentions(t),
            Val::Quot { base, num, .. } => mentions(&base.0) || mentions(&num.0),
            Val::Itv(..) | Val::Arr(_) | Val::Other => false,
        };
        if self_ref {
            let s = self.scratch;
            self.assign(st, s, v);
            st.forget(var);
            constrain_offset(st, var, &[Term::pos(s)], 0, 0, 0);
            copy_far(st, var, &[Term::pos(s)], 0);
            st.forget(s);
            return;
        }
        st.forget(var);
        match v {
            Val::Lin(t, c) => {
                constrain_offset(st, var, t, *c, 0, 0);
                copy_far(st, var, t, *c);
            }
            Val::Quot { base, num, div } => {
                let (nlo, nhi) = lin_interval(st, &num.0, num.1);
                constrain_offset(
                    st,
                    var,
                    &base.0,
                    base.1,
                    trunc_div(nlo, *div),
                    trunc_div(nhi, *div),
                );
                if let Some(sum) = lin_add(&base.0, &num.0) {
                    constrain_offset(
                        st,
                        var,
                        &sum,
                        base.1 + num.1,
                        -rest(nhi, *div),
                        -rest(nlo, *div),
                    );
                }
            }
            Val::Itv(lo, hi) => constrain_offset(st, var, &[], 0, *lo, *hi),
            Val::Arr(_) | Val::Other => {}
        }
    }

    /// A value as a linear form, put in `temp` when it is not one already.
    fn linear(&self, st: &mut Octagon, v: &Val, temp: usize) -> Option<Lin> {
        match v {
            Val::Lin(t, c) => Some((t.clone(), *c)),
            Val::Quot { .. } | Val::Itv(..) => {
                self.assign(st, temp, v);
                Some((vec![Term::pos(temp)], 0))
            }
            Val::Arr(_) | Val::Other => None,
        }
    }

    /// Adds `a - b <= k`.
    fn le(&self, st: &mut Octagon, a: &Val, b: &Val, k: i128) {
        let [t0, t1] = self.temps;
        if let (Some(x), Some(y)) = (self.linear(st, a, t0), self.linear(st, b, t1)) {
            let (terms, c) = match lin_add(&x.0, &neg_terms(&y.0)) {
                Some(t) => (t, x.1 - y.1),
                None => {
                    self.assign(st, t0, &Val::Lin(x.0, x.1));
                    self.assign(st, t1, &Val::Lin(y.0, y.1));
                    (vec![Term::pos(t0), Term::pos(t1).negated()], 0)
                }
            };
            st.add_lin(&terms, k - c);
        }
        st.forget(t0);
        st.forget(t1);
    }

    fn constrain(&self, st: &mut Octagon, op: Rel, l: &Val, r: &Val) {
        match op {
            Rel::Lt => self.le(st, l, r, -1),
            Rel::Le => self.le(st, l, r, 0),
            Rel::Gt => self.le(st, r, l, -1),
            Rel::Ge => self.le(st, r, l, 0),
            Rel::Eq => {
                self.le(st, l, r, 0);
                self.le(st, r, l, 0);
            }
            Rel::Ne => {
                let mut below = st.clone();
                self.le(&mut below, l, r, -1);
                self.le(st, r, l, -1);
                *st = below.join(st);
            }
        }
    }

    /// Narrows `st` to the points where `c` has the value `polarity`.
    fn assume(&self, st: &mut Octagon, c: &Cond, polarity: bool) {
        if st.is_empty() {
            return;
        }
        match c {
            Cond::Const(b) => {
                if *b != polarity {
                    st.set_empty();
                }
            }
            Cond::Not(a) => self.assume(st, a, !polarity),
            Cond::And(a, b) | Cond::Or(a, b) => {
                if matches!(c, Cond::And(..)) == polarity {
                    self.assume(st, a, polarity);
                    self.assume(st, b, polarity);
                } else {
                    let mut x = st.clone();
                    self.assume(&mut x, a, polarity);
                    self.assume(st, b, polarity);
                    *st = x.join(st);
                }
            }
            Cond::Cmp(op, l, r) => {
                let op = if polarity { *op } else { flip(*op) };
                self.constrain(st, op, l, r);
            }
            Cond::Far(op, far) => {
                let op = if polarity { *op } else { flip(*op) };
                // A value of one term is compared through its variable, any other through the
                // scratch variable, which takes the bounds the octagon gives it.
                let (t, k) = match &far.0 {
                    Val::Lin(terms, k) if terms.len() == 1 => (terms[0], *k),
                    v @ (Val::Lin(..)
                    | Val::Quot { .. }
                    | Val::Itv(..)
                    | Val::Arr(_)
                    | Val::Other) => {
                        self.assign(st, self.scratch, v);
                        (Term::pos(self.scratch), 0)
                    }
                };
                constrain_far(st, op, t, k, far.1);
                st.forget(self.scratch);
            }
            Cond::Unknown => {}
        }
    }

    /// The bounds of `v` in `st`, exactly, beyond the octagon's reach included: those the
    /// scratch variable takes from it. `None` for none.
    fn exact_range(&self, st: &Octagon, v: &Val) -> (Option<Exact>, Option<Exact>) {
        let mut s = st.clone();
        self.assign(&mut s, self.scratch, v);
        s.exact_bounds(self.scratch)
    }

    /// A state in which `c` fails, or `None` when it holds wherever `st` does.
    fn refute(&self, st: &Octagon, c: &Cond) -> Option<Octagon> {
        let mut s = st.clone();
        self.assume(&mut s, c, false);
        (!s.is_empty()).then_some(s)
    }

    /// Adds the facts a scalar shape gives `var`.
    fn facts(&self, st: &mut Octagon, var: usize, shape: &Shape) {
        if let Shape::Int { int, refine } = shape {
            if let Some(t) = int {
                let (lo, hi) = int_range(*t);
                constrain_offset(st, var, &[], 0, lo, hi);
                // The ends of a 128-bit type lie beyond the octagon's reach, and are kept
                // beside it.
                if t.bits == 128 {
                    let lower = t.signed.then(|| Exact::new(true, t.min_magnitude()));
                    st.add_exact(var, lower, Some(Exact::new(false, t.max())));
                }
            }
            if let Some(r) = refine {
                let v = var_val(var);
                let op = if r.inclusive { Rel::Le } else { Rel::Lt };
                match far(r.lo_lit) {
                    Some(c) => constrain_far(st, Rel::Ge, Term::pos(var), 0, c),
                    None => self.constrain(st, Rel::Ge, &v, &r.lo),
                }
                match far(r.hi_lit) {
                    Some(c) => constrain_far(st, op, Term::pos(var), 0, c),
                    None => self.constrain(st, op, &v, &r.hi),
                }
            }
        }
    }

    /// Each element of an array of this shape, as an interval.
    fn summary(&self, st: &Octagon, shape: &Shape) -> Val {
        match shape {
            Shape::Int { int, refine } => {
                // Without an integer type, the whole domain.
                let (mut lo, mut hi) = int.map_or(interval(st, &TOP), int_range);
                if let Some(r) = refine {
                    lo = lo.max(interval(st, &r.lo).0);
                    let top = interval(st, &r.hi).1;
                    hi = hi.min(if r.inclusive { top } else { sat_add(top, -1) });
                }
                Val::Itv(lo, hi)
            }
            Shape::Array(_, e) => Val::Arr(Box::new(self.summary(st, e))),
            Shape::Struct(_) | Shape::Other => Val::Other,
        }
    }

    // -- obligations -------------------------------------------------------------------------

    /// A value entering a plain integer type: a run-time check unless its bounds fit.
    /// Where the value always lies outside the type, when this point is reached: `at most -1`,
    /// `at least 256`.
    fn fits(&mut self, st: &Octagon, v: &Val, t: Option<IntTy>, span: Span) -> Option<String> {
        let (lo, hi) = interval(st, v);
        let range = t.map(int_range);
        let proven = t.is_some_and(|t| within(st, v, t));
        let detail = t.map_or("into its type".to_string(), |t| {
            format!("into `{}`", t.name)
        });
        let o = Obligation {
            kind: ObligationKind::Range,
            span,
            must: false,
            proven: st.is_empty() || proven,
            detail,
            tag: t.map_or(Tag::None, |t| Tag::Into(t.name)),
        };
        self.record_check(o, |_| vec![(st.clone(), Goal::Fits(v.clone(), t))]);
        // An empty interval is a value no path computes, after an operation that always traps.
        let (tlo, thi) = range.filter(|_| !st.is_empty() && lo <= hi)?;
        if hi < tlo {
            Some(format!("at most {}", hi))
        } else if lo > thi {
            Some(format!("at least {}", lo))
        } else {
            None
        }
    }

    /// `E0508`: a narrowing Tier 1 proves always traps where it is reached
    /// ([`strict-defaults`]).
    ///
    /// [`strict-defaults`]: ../../docs/design-review.md#strict-defaults
    fn always_traps(&mut self, side: &str, t: IntTy, span: Span, what: &str) {
        let message = format!(
            "Tier 1 proves {} is {} here, outside `{}` ({}), so this narrowing always traps",
            what,
            side,
            t.name,
            t.range_text()
        );
        self.emit("E0508", span, message, None, Anchor::Nowhere);
    }

    /// A value narrowed into a place of shape `shape`: its plain range may stay a run-time
    /// check, its refinement must be proven. `what` is the value as a `guard` writes it.
    fn narrow(
        &mut self,
        st: &Octagon,
        v: &Val,
        shape: &Shape,
        (span, tag): (Span, Tag),
        place: &str,
        what: Option<(String, u8)>,
    ) {
        match shape {
            Shape::Int { int, refine } => {
                if let Some(t) = int {
                    if let Some(side) = self.fits(st, v, Some(*t), span) {
                        // What follows a narrowing that always traps is never reached.
                        let shown = what.map_or("the value".to_string(), |w| format!("`{}`", w.0));
                        self.always_traps(&side, *t, span, &shown);
                        return;
                    }
                }
                if let Some(r) = refine {
                    self.refinement(st, v, r, (span, tag), place, what);
                }
            }
            Shape::Array(_, elem) => {
                let each = match v {
                    Val::Arr(s) => (**s).clone(),
                    Val::Lin(..) | Val::Quot { .. } | Val::Itv(..) | Val::Other => TOP,
                };
                let place = format!("each element of {}", place);
                self.narrow(st, &each, elem, (span, tag), &place, None);
            }
            // A value of a struct already holds its fields' refinements.
            Shape::Struct(_) | Shape::Other => {}
        }
    }

    fn refinement(
        &mut self,
        st: &Octagon,
        v: &Val,
        r: &Refine,
        (span, tag): (Span, Tag),
        place: &str,
        what: Option<(String, u8)>,
    ) {
        let high_op = if r.inclusive { Rel::Le } else { Rel::Lt };
        // A literal bound beyond the octagon's reach is compared exactly.
        let low = match far(r.lo_lit) {
            Some(c) => Cond::Far(Rel::Ge, Box::new((v.clone(), c))),
            None => cmp(Rel::Ge, v.clone(), r.lo.clone()),
        };
        let high = match far(r.hi_lit) {
            Some(c) => Cond::Far(high_op, Box::new((v.clone(), c))),
            None => cmp(high_op, v.clone(), r.hi.clone()),
        };
        let fail_low = self.refute(st, &low);
        let fail_high = self.refute(st, &high);
        let failing = fail_low.clone().or(fail_high.clone());
        self.record(
            ObligationKind::Refinement,
            span,
            true,
            failing.is_none(),
            tag,
            |_| {
                let within = Cond::And(Box::new(low), Box::new(high));
                vec![(st.clone(), Goal::Holds(within))]
            },
        );
        let (Some(mut s), true) = (failing, self.recording) else {
            return;
        };
        let outside = format!("outside `{}`, the type of {}", r.text, place);
        let shown = what
            .as_ref()
            .map_or("the value".to_string(), |w| format!("`{}`", w.0));
        // A binding in scope, or a field path of one, which a valuation names.
        let in_scope = |t: &Term| {
            self.scope.iter().any(|b| {
                self.vars[*b] == Some(t.var) || self.paths[*b].iter().any(|p| p.var == t.var)
            })
        };
        let named = what.as_ref().is_some_and(|w| {
            self.scope.iter().any(|b| {
                let name = &self.resolution.bindings[*b].name;
                *name == w.0
                    || self.paths[*b]
                        .iter()
                        .any(|p| format!("{}.{}", name, p.names.join(".")) == w.0)
            })
        });
        // A side whose bound lies beyond the octagon's reach fails at a value Tier 1 names
        // exactly: the nearest one outside the bound that the failing state allows.
        let far_side = match fail_low {
            Some(_) => far(r.lo_lit).map(|c| (false, c)),
            None => far(r.hi_lit).map(|c| (true, c)),
        };
        let message = if let Some((upper, c)) = far_side {
            let (lo, hi) = self.exact_range(&s, v);
            let value = if upper {
                let first = if r.inclusive {
                    c.plus(1).unwrap_or(c)
                } else {
                    c
                };
                lo.map_or(first, |l| l.max(first))
            } else {
                let last = c.plus(-1).unwrap_or(c);
                hi.map_or(last, |h| h.min(last))
            };
            match v {
                Val::Lin(t, 0) if named && t.len() == 1 && t.iter().all(in_scope) => format!(
                    "Tier 1 cannot rule out `{} = {}`, {}",
                    what.as_ref().map_or("", |w| w.0.as_str()),
                    value,
                    outside
                ),
                Val::Lin(..) | Val::Quot { .. } | Val::Itv(..) | Val::Arr(_) | Val::Other => {
                    format!(
                        "Tier 1 cannot rule out that {} is {}, {}",
                        shown, value, outside
                    )
                }
            }
        } else {
            // The probe holds the value, outside the refinement on the side that fails.
            let probe = var_val(self.probe);
            self.assign(&mut s, self.probe, v);
            let (op, bound) = match fail_low {
                Some(_) => (Rel::Ge, &r.lo),
                None => (high_op, &r.hi),
            };
            self.assume(&mut s, &cmp(op, probe, bound.clone()), false);
            let (valuation, values) = self.valuation(&s, &[], &[self.probe]);
            let value = values.first().copied();
            let core = format!(
                "{} is {}, {}",
                shown,
                value.unwrap_or(Exact::from_i128(0)),
                outside
            );
            // The valuation determines the value only when the value is linear in the bindings in
            // scope; otherwise Tier 1 knows only an interval for it, and names a point of that alone.
            match v {
                Val::Lin(t, _) if !t.is_empty() && t.iter().all(in_scope) => {
                    if named {
                        format!("Tier 1 cannot rule out `{}`, {}", valuation, outside)
                    } else {
                        format!("Tier 1 cannot rule out `{}`, where {}", valuation, core)
                    }
                }
                Val::Lin(..) | Val::Quot { .. } | Val::Itv(..) | Val::Arr(_) | Val::Other => {
                    format!("Tier 1 cannot rule out that {}", core)
                }
            }
        };
        let read = what.as_ref().and_then(|w| {
            self.scope
                .iter()
                .copied()
                .find(|b| self.resolution.bindings[*b].name == w.0)
        });
        // A value that passes a binding `mut` would change it again in a `guard` that repeats
        // it, so none is offered, and the message says why.
        let repeats = what.as_ref().is_some_and(|w| {
            mente_syntax::tokenize(&w.0)
                .iter()
                .any(|t| t.kind == mente_syntax::TokenKind::Mut)
        });
        let guard = what.map(|w| {
            let x = paren(w, OPERAND);
            let mut parts = Vec::new();
            if fail_low.is_some() {
                parts.push(format!("{} >= {}", x, r.lo_text));
            }
            if fail_high.is_some() {
                parts.push(format!("{} {} {}", x, high_op.symbol(), r.hi_text));
            }
            parts.join(" and ")
        });
        let (guard, message) = match guard {
            Some(_) if self.unfollowed => (
                None,
                format!(
                    "{}; a `guard` that repeats an `if` or a `match` tells Tier 1 nothing of this one, so none is offered: bind its value to a `val`, and `guard` that",
                    message
                ),
            ),
            Some(_) if repeats => (
                None,
                format!(
                    "{}; it passes a binding `mut`, which a `guard` repeating it would change again, so none is offered",
                    message
                ),
            ),
            _ => (guard, message),
        };
        let anchor = self.anchor();
        self.emit("E0502", span, message, guard, anchor);
        if let Some(declared) = r.declared.clone() {
            self.retries.push(Retry {
                diagnostic: self.report.diagnostics.len() - 1,
                declared,
                read,
            });
        }
    }

    fn index_check(&mut self, st: &Octagon, i: &Val, len: u128, span: Span) {
        let (lo, hi) = interval(st, i);
        let n = i128::try_from(len).unwrap_or(INF);
        let o = Obligation {
            kind: ObligationKind::Index,
            span,
            must: false,
            proven: st.is_empty() || (lo >= 0 && hi < n),
            detail: format!("into an array of {}", len),
            tag: Tag::None,
        };
        self.record_check(o, |_| vec![(st.clone(), Goal::Index(i.clone(), len))]);
    }

    /// The function's `ensures`, at a return of `v` or at the end of its body.
    fn ensures(
        &mut self,
        st: &Octagon,
        v: &Val,
        span: Span,
        value: Option<ExprId>,
        anchor: Anchor,
        at: &str,
    ) {
        if st.is_empty() {
            return;
        }
        let mut out = st.clone();
        let mut extra = Vec::new();
        if let (Some(r), Some(shape)) = (self.result, self.result_shape.clone()) {
            self.assign(&mut out, r, v);
            self.facts(&mut out, r, &shape);
            extra.push(("result".to_string(), r));
        }
        // A struct result's field paths, which its `ensures` may read, take the value's.
        let result = self
            .resolution
            .bindings
            .iter()
            .position(|b| b.kind == BindingKind::Result);
        if let (Some(rb), Some(e)) = (result, value) {
            if !self.paths[rb].is_empty() {
                self.land(&mut out, rb, &[], e);
                for p in self.paths[rb].clone() {
                    extra.push((format!("result.{}", p.names.join(".")), p.var));
                }
            }
        }
        let name = self.syntax.name.clone();
        let syntax = self.syntax;
        for c in &syntax.contracts {
            if c.kind != ContractKind::Ensures || self.outside(c.expr).is_some() {
                continue;
            }
            let mut s = out.clone();
            let k = self.cond(&mut s, c.expr);
            let failing = self.refute(&s, &k);
            let tag = Tag::Clause(self.span(c.expr));
            self.record(
                ObligationKind::Ensures,
                span,
                true,
                failing.is_none(),
                tag,
                |_| vec![(s, Goal::Holds(k))],
            );
            let Some(failing) = failing else {
                continue;
            };
            let clause = print(self.tree, c.expr, &no_subst).0;
            // `result`, and a field path of it, read the returned value.
            let read = |id: ExprId| value.and_then(|v| self.result_reading(id, v));
            let subst = |id: ExprId, _: &str| read(id).map(|r| r.0);
            let here = print(self.tree, c.expr, &subst).0;
            let followed = fragment_followed(self.tree, c.expr, &|id| {
                read(id)
                    .map(|r| r.1)
                    .or_else(|| chain_of(self.tree, id).map(|_| self.followed(id)))
            });
            let (valuation, _) = self.valuation(&failing, &extra, &[]);
            let what = format!("`ensures {}` of `{}` {}", clause, name, at);
            let message = Fx::cannot_rule_out(
                &valuation,
                &format!("which violates {}", what),
                &format!("{} fails", what),
            );
            let (guard, message) = Fx::usable_guard(here, followed, at, message);
            self.emit("E0505", span, message, guard, anchor);
        }
    }

    // -- the walk ------------------------------------------------------------------------------

    fn entry(&mut self, st: &mut Octagon) {
        for i in 0..self.syntax.params.len() {
            if let Some(v) = self.vars[i] {
                let shape = self.binding_shape(i);
                self.facts(st, v, &shape);
            }
            for p in self.paths[i].clone() {
                self.facts(st, p.var, &p.shape);
            }
            self.scope.push(i);
        }
        let syntax = self.syntax;
        for c in &syntax.contracts {
            if c.kind == ContractKind::Requires {
                let k = self.cond(st, c.expr);
                self.assume(st, &k, true);
            }
        }
        let measures = syntax
            .contracts
            .iter()
            .filter(|c| c.kind == ContractKind::Decreases);
        for (c, (m, _)) in measures.zip(self.entry_measures.clone()) {
            let v = self.eval(st, c.expr);
            self.assign(st, m, &v);
        }
    }

    fn body(&mut self, st: &mut Octagon) {
        let shape = self.result_shape.clone();
        let place = format!("the result of `{}`", self.syntax.name);
        match self.syntax.body {
            Some(Body::Expr(e)) => {
                let v = match &shape {
                    Some(s) => self.eval_into(st, e, s, &place),
                    None => self.eval(st, e),
                };
                self.ensures(st, &v, self.span(e), Some(e), Anchor::Body(e), "at its end");
            }
            Some(Body::Block(b)) => {
                let v = self.block(st, b, shape.as_ref());
                let last = self.tree.block(b).stmts.last().copied();
                let tail = last.and_then(|s| match self.tree.stmt(s).kind {
                    StmtKind::Expr(e) => Some(e),
                    StmtKind::Binding { .. }
                    | StmtKind::Assign { .. }
                    | StmtKind::While { .. }
                    | StmtKind::Guard { .. }
                    | StmtKind::Return(_)
                    | StmtKind::Break
                    | StmtKind::Continue
                    | StmtKind::Discard(_) => None,
                });
                match (&shape, tail, last) {
                    (Some(_), Some(e), Some(s)) => self.ensures(
                        st,
                        &v,
                        self.span(e),
                        Some(e),
                        Anchor::Before(s),
                        "at its end",
                    ),
                    (Some(_), _, _) => {}
                    (None, _, _) => {
                        let span = last.map_or(self.tree.block(b).span, |s| self.tree.stmt(s).span);
                        self.ensures(st, &v, span, None, Anchor::After(b), "at its end");
                    }
                }
            }
            None => {}
        }
    }

    fn block(&mut self, st: &mut Octagon, b: BlockId, tail: Option<&Shape>) -> Val {
        let mark = self.scope.len();
        let saved = self.stmt;
        let tree = self.tree;
        let stmts = &tree.block(b).stmts;
        let mut last = Val::Other;
        for (i, &s) in stmts.iter().enumerate() {
            if st.is_empty() {
                break;
            }
            self.stmt = Some(s);
            last = self.stmt_eval(st, s, tail.filter(|_| i + 1 == stmts.len()));
        }
        self.stmt = saved;
        self.scope.truncate(mark);
        last
    }

    fn stmt_eval(&mut self, st: &mut Octagon, s: StmtId, tail: Option<&Shape>) -> Val {
        let tree = self.tree;
        match &tree.stmt(s).kind {
            StmtKind::Expr(e) => match tail {
                Some(shape) => {
                    let place = format!("the result of `{}`", self.syntax.name);
                    return self.eval_into(st, *e, shape, &place);
                }
                None => return self.eval(st, *e),
            },
            StmtKind::Discard(e) => {
                self.eval(st, *e);
            }
            StmtKind::Binding { name, init, .. } => {
                if let Some(id) = self.resolution.stmts[s.0 as usize] {
                    let b = id.0 as usize;
                    let shape = self.binding_shape(b);
                    let v = self.eval_into(st, *init, &shape, &format!("`{}`", name));
                    if let Some(var) = self.vars[b] {
                        self.assign(st, var, &v);
                        self.facts(st, var, &shape);
                    }
                    if !self.paths[b].is_empty() {
                        self.land(st, b, &[], *init);
                    }
                    self.scope.push(b);
                }
            }
            StmtKind::Assign { place, op, value } => self.assign_stmt(st, *place, *op, *value),
            StmtKind::While {
                cond,
                invariants,
                body,
                ..
            } => self.while_loop(st, s, *cond, invariants, *body),
            StmtKind::Guard { cond, otherwise } => {
                let c = self.cond(st, *cond);
                let mut o = st.clone();
                self.assume(&mut o, &c, false);
                self.block(&mut o, *otherwise, None);
                self.assume(st, &c, true);
            }
            StmtKind::Return(value) => {
                let span = value.map_or(tree.stmt(s).span, |e| self.span(e));
                let place = format!("the result of `{}`", self.syntax.name);
                let v = match (value, self.result_shape.clone()) {
                    (Some(e), Some(shape)) => self.eval_into(st, *e, &shape, &place),
                    (Some(e), None) => self.eval(st, *e),
                    (None, _) => Val::Other,
                };
                self.ensures(st, &v, span, *value, Anchor::Before(s), "at this return");
                st.set_empty();
            }
            StmtKind::Break | StmtKind::Continue => {
                let snapshot = st.clone();
                if let Some(f) = self.loops.last_mut() {
                    if tree.stmt(s).kind == StmtKind::Break {
                        f.breaks.push(snapshot);
                    } else {
                        f.continues.push(snapshot);
                    }
                }
                st.set_empty();
            }
        }
        Val::Other
    }

    /// The root binding of a place and its steps, indexes and fields, outermost first.
    fn place_parts(&self, e: ExprId, steps: &mut Vec<Step>) -> Option<usize> {
        match &self.tree.expr(e).kind {
            ExprKind::Index { base, index } => {
                let root = self.place_parts(*base, steps);
                steps.push(Step::Index(*index));
                root
            }
            ExprKind::Field { base, name } => {
                let root = self.place_parts(*base, steps);
                steps.push(Step::Field(name.clone()));
                root
            }
            ExprKind::Name(_) => self.resolution.names[e.0 as usize].map(|b| b.0 as usize),
            // The checker passes as a place only a binding, an element, or a field of a struct
            // (`E0205`, `E0206`).
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
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

    fn assign_stmt(&mut self, st: &mut Octagon, place: ExprId, op: AssignOp, value: ExprId) {
        let mut steps = Vec::new();
        let root = self.place_parts(place, &mut steps);
        for (k, step) in steps.iter().enumerate() {
            if let Step::Index(i) = step {
                let iv = self.eval(st, *i);
                if let Shape::Array(n, _) = self.place_shape(root, &steps[..k]) {
                    self.index_check(st, &iv, n, self.span(*i));
                }
            }
        }
        let shape = self.place_shape(root, &steps);
        let scalar = root
            .filter(|_| steps.is_empty())
            .and_then(|b| self.vars[b])
            .or_else(|| self.path_var(place));
        let text = self.own_text(place);
        let place_name = format!("`{}`", text.0);
        let v = match op {
            AssignOp::Set => self.eval_into(st, value, &shape, &place_name),
            AssignOp::Compound(bop) => {
                // The place is read first, as the reference interpreter reads it.
                let mut current = scalar.map_or_else(|| self.summary(st, &shape), var_val);
                self.read_before(st, value, [&mut current]);
                let r = self.eval(st, value);
                let v = self.binary(st, value, bop, (place, &current), (value, &r));
                let p = prec(&ExprKind::Binary {
                    op: bop,
                    lhs: place,
                    rhs: value,
                });
                let what = format!(
                    "{} {} {}",
                    text.0,
                    bop.symbol(),
                    paren(self.own_text(value), p + 1)
                );
                self.narrow(
                    st,
                    &v,
                    &shape,
                    (self.span(value), Tag::None),
                    &place_name,
                    Some((what, p)),
                );
                v
            }
        };
        if let Some(var) = scalar {
            self.track(st, var, &v);
            self.assign(st, var, &v);
            self.facts(st, var, &shape);
        } else if let Some((r, prefix)) = field_chain(self.tree, self.resolution, place) {
            // A whole struct, or a struct field of one, takes the value's paths.
            if !self.paths_under(r, &prefix).is_empty() {
                self.land(st, r, &prefix, value);
            }
        }
    }

    /// Before `var` takes `v`: each tracked measure with `var` in it changes by what `var` does.
    fn track(&self, st: &mut Octagon, var: usize, v: &Val) {
        for t in &self.tracks {
            for term in t.terms.iter().filter(|term| term.var == var) {
                let diff = add_vals(st, v, &negate(&var_val(var)));
                let change = if term.neg { negate(&diff) } else { diff };
                let (lo, hi) = interval(st, &change);
                let s = self.scratch;
                st.forget(s);
                constrain_offset(st, s, &[Term::pos(t.delta)], 0, lo, hi);
                st.forget(t.delta);
                constrain_offset(st, t.delta, &[Term::pos(s)], 0, 0, 0);
                st.forget(s);
            }
        }
    }

    /// Evaluates `e` into a place of shape `shape`, element by element for an array literal.
    fn eval_into(&mut self, st: &mut Octagon, e: ExprId, shape: &Shape, place: &str) -> Val {
        match (&self.tree.expr(e).kind, shape) {
            (ExprKind::Array(items), Shape::Array(_, elem)) => {
                let place = format!("an element of {}", place);
                let mut each: Option<Val> = None;
                for &i in items {
                    let v = self.eval_into(st, i, elem, &place);
                    let s = summary_of(st, &v);
                    each = Some(each.map_or(s.clone(), |x| join_summary(&x, &s)));
                }
                Val::Arr(Box::new(each.unwrap_or(Val::Other)))
            }
            (ExprKind::Repeat { value, .. }, Shape::Array(_, elem)) => {
                let place = format!("an element of {}", place);
                let v = self.eval_into(st, *value, elem, &place);
                Val::Arr(Box::new(summary_of(st, &v)))
            }
            _ => {
                let v = self.eval(st, e);
                let what = self.own_text(e);
                // A `guard` that repeats an `if` or a `match` tells Tier 1 nothing of this one's
                // value, whose variable is given anew where it is evaluated.
                let control = self.tree.expr(e).kind.is_choice();
                let was = std::mem::replace(&mut self.unfollowed, control);
                self.narrow(st, &v, shape, (self.span(e), Tag::None), place, Some(what));
                self.unfollowed = was;
                v
            }
        }
    }

    /// An integer literal: a constant, or, beyond the octagon's reach, its variable, held at
    /// the value exactly. Apart from [`Self::eval`], whose frame every nested expression
    /// stacks up.
    fn int_val(&self, st: &mut Octagon, e: ExprId, value: u128) -> Val {
        match self.sites[e.0 as usize] {
            Site::Value(var) => {
                let exact = Exact::new(false, value);
                st.forget(var);
                st.add_exact(var, Some(exact), Some(exact));
                var_val(var)
            }
            Site::None | Site::Call(_) => int_literal(value),
        }
    }

    /// A struct literal: each value enters its field's type, in the order written, which is the
    /// declaration's, and the scalar paths it builds are kept for where it lands. Apart from
    /// [`Self::eval`], whose frame every nested expression stacks up.
    fn construct(
        &mut self,
        st: &mut Octagon,
        e: ExprId,
        name: &str,
        fields: &[mente_syntax::tree::FieldInit],
    ) -> Val {
        let mut built: Vec<(Vec<String>, Val)> = Vec::new();
        for f in fields {
            // A later field that changes a binding an earlier value reads leaves that
            // value as its bounds were.
            self.read_before(st, f.value, built.iter_mut().map(|(_, v)| v));
            let shape = self.field_shape(name, &f.name);
            let place = format!("field `{}` of `{}`", f.name, name);
            let v = self.eval_into(st, f.value, &shape, &place);
            match &shape {
                Shape::Int { .. } => built.push((vec![f.name.clone()], v)),
                Shape::Struct(_) => {
                    for (mut names, v) in self.struct_vals(f.value) {
                        names.insert(0, f.name.clone());
                        built.push((names, v));
                    }
                }
                Shape::Array(..) | Shape::Other => {}
            }
        }
        self.built[e.0 as usize] = Some(built);
        Val::Other
    }

    fn eval(&mut self, st: &mut Octagon, e: ExprId) -> Val {
        let (tree, typing) = (self.tree, self.typing);
        match &tree.expr(e).kind {
            ExprKind::Int { value, .. } => self.int_val(st, e, *value),
            ExprKind::Name(_) => match self.resolution.names[e.0 as usize] {
                Some(b) => {
                    let b = b.0 as usize;
                    match self.vars[b] {
                        Some(v) => var_val(v),
                        None => {
                            let shape = self.binding_shape(b);
                            self.summary(st, &shape)
                        }
                    }
                }
                None => Val::Other,
            },
            ExprKind::Todo(_) | ExprKind::Unreachable(_) => {
                st.set_empty();
                Val::Other
            }
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => {
                let v = self.eval(st, *operand);
                negate(&v)
            }
            ExprKind::Unary {
                op: UnaryOp::BitNot,
                operand,
            } => {
                let v = self.eval(st, *operand);
                bit_not(&v, int_of(&typing.exprs[operand.0 as usize]))
            }
            ExprKind::Unary { .. } => {
                self.cond(st, e);
                Val::Other
            }
            ExprKind::Binary { op, lhs, rhs } => match op {
                BinaryOp::And
                | BinaryOp::Or
                | BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {
                    self.cond(st, e);
                    Val::Other
                }
                BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitAnd
                | BinaryOp::Shl
                | BinaryOp::Shr
                | BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Rem => {
                    let mut l = self.eval(st, *lhs);
                    self.read_before(st, *rhs, [&mut l]);
                    let r = self.eval(st, *rhs);
                    self.binary(st, e, *op, (*lhs, &l), (*rhs, &r))
                }
            },
            ExprKind::Index { base, index } => {
                let b = self.eval(st, *base);
                let i = self.eval(st, *index);
                if let Ty::Array(n, _) = &typing.exprs[base.0 as usize] {
                    self.index_check(st, &i, *n, tree.expr(*index).span);
                }
                let each = match b {
                    Val::Arr(each) => *each,
                    Val::Lin(..) | Val::Quot { .. } | Val::Itv(..) | Val::Other => {
                        self.summary(st, &Shape::plain(&typing.exprs[e.0 as usize]))
                    }
                };
                // An element of a binding has its declared element type's refinement, whose
                // bounds may name other bindings.
                let Site::Value(var) = self.sites[e.0 as usize] else {
                    return each;
                };
                self.assign(st, var, &each);
                let mut steps = Vec::new();
                if let Some(root) = self.place_parts(e, &mut steps) {
                    let shape = self.place_shape(Some(root), &steps);
                    self.facts(st, var, &shape);
                }
                var_val(var)
            }
            ExprKind::Field { base, name } => {
                // A field path Tier 1 follows is its variable.
                if let Some(var) = self.path_var(e) {
                    return var_val(var);
                }
                self.eval(st, *base);
                match &typing.exprs[base.0 as usize] {
                    Ty::Array(n, _) => int_literal(*n),
                    // A field read is a value Tier 1 does not follow from a store: it lies in its
                    // type, and in its refinement, which every value of the struct holds.
                    Ty::Struct(s) => {
                        let shape = self.field_shape(s, name);
                        let each = self.summary(st, &shape);
                        let Site::Value(var) = self.sites[e.0 as usize] else {
                            return each;
                        };
                        self.assign(st, var, &each);
                        self.facts(st, var, &shape);
                        var_val(var)
                    }
                    Ty::Int(_) | Ty::Bool | Ty::Enum(_) | Ty::Unit | Ty::Any => TOP,
                }
            }
            ExprKind::Call { args, .. } => self.call(st, e, args),
            ExprKind::Construct { name, fields } => self.construct(st, e, name, fields),
            ExprKind::Variant {
                enum_name,
                name,
                payload,
                ..
            } => self.variant(st, enum_name, name, payload),
            ExprKind::Match { scrutinee, arms } => self.match_expr(st, e, *scrutinee, arms),
            ExprKind::Array(items) => {
                let mut each: Option<Val> = None;
                for &i in items {
                    let v = self.eval(st, i);
                    let s = summary_of(st, &v);
                    each = Some(each.map_or(s.clone(), |x| join_summary(&x, &s)));
                }
                Val::Arr(Box::new(each.unwrap_or(Val::Other)))
            }
            ExprKind::Repeat { value, .. } => {
                let v = self.eval(st, *value);
                Val::Arr(Box::new(summary_of(st, &v)))
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => self.if_expr(st, e, *cond, *then, *otherwise),
            ExprKind::Bool(_) | ExprKind::Error => Val::Other,
        }
    }

    fn binary(
        &mut self,
        st: &mut Octagon,
        e: ExprId,
        op: BinaryOp,
        (le, l): (ExprId, &Val),
        (re, r): (ExprId, &Val),
    ) -> Val {
        let lt = int_of(&self.typing.exprs[le.0 as usize]);
        let rt = int_of(&self.typing.exprs[re.0 as usize]);
        let (span, rspan) = (self.span(e), self.span(re));
        let v = match op {
            BinaryOp::Shl | BinaryOp::Shr => {
                let (lo, hi) = interval(st, r);
                let width = lt.map_or(0, |t| i128::from(t.bits));
                let o = Obligation {
                    kind: ObligationKind::Shift,
                    span: rspan,
                    must: false,
                    proven: st.is_empty() || (lo >= 0 && hi < width),
                    detail: lt.map_or(String::new(), |t| format!("below {}", t.bits)),
                    tag: Tag::None,
                };
                let bits = lt.map_or(0, |t| t.bits);
                self.record_check(o, |_| vec![(st.clone(), Goal::Shift(r.clone(), bits))]);
                let v = shift_vals(st, op, l, r, None);
                self.fits(st, &v, lt, span);
                return clamp_to(v, lt, st);
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                let t = lt.or(rt);
                self.fits(st, l, t, self.span(le));
                self.fits(st, r, t, rspan);
                return bits_vals(st, op, l, r, t);
            }
            BinaryOp::Div | BinaryOp::Rem => {
                let (lo, hi) = interval(st, r);
                self.record(
                    ObligationKind::Divisor,
                    rspan,
                    false,
                    st.is_empty() || lo > 0 || hi < 0,
                    Tag::None,
                    |_| vec![(st.clone(), Goal::NonZero(r.clone()))],
                );
                // A dividend smaller in magnitude than every divisor, one beyond the octagon's
                // reach included, has a quotient of 0 and is its own remainder.
                match (op, self.below_divisor(st, l, r)) {
                    (BinaryOp::Div, true) => lin(Vec::new(), 0),
                    (_, true) => l.clone(),
                    (BinaryOp::Div, false) => div_vals(st, l, r),
                    (_, false) => self.rem(st, e, l, r),
                }
            }
            BinaryOp::Sub => add_vals(st, l, &negate(r)),
            BinaryOp::Mul => mul_vals(st, l, r),
            BinaryOp::Add => add_vals(st, l, r),
            // `eval` gives comparisons and `and` and `or` arms of their own, so they never come
            // here; were one to, Tier 1 would know nothing of its value.
            BinaryOp::Or
            | BinaryOp::And
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => TOP,
        };
        // An operation on 128-bit operands is narrowed into their type (`core-calculus.md`,
        // item 4).
        match [lt, rt].into_iter().flatten().find(|t| t.bits == 128) {
            Some(t) => {
                self.fits(st, &v, Some(t), span);
                v
            }
            None => v,
        }
    }

    /// Whether every value of `l` is smaller in magnitude than every value of the divisor `r`,
    /// their bounds beyond the octagon's reach included.
    fn below_divisor(&self, st: &Octagon, l: &Val, r: &Val) -> bool {
        let zero = Exact::from_i128(0);
        let (rlo, rhi) = self.exact_range(st, r);
        // The divisor's least magnitude: 0 where it may be 0, or of either sign.
        let least = [rlo, rhi.map(Exact::negated)]
            .into_iter()
            .flatten()
            .fold(zero, Exact::max);
        match self.exact_range(st, l) {
            (Some(lo), Some(hi)) => hi < least && lo.negated() < least,
            _ => false,
        }
    }

    /// `x % y`, with the facts `core-calculus.md` item 8 gives it.
    fn rem(&mut self, st: &mut Octagon, e: ExprId, l: &Val, r: &Val) -> Val {
        let (lo, hi) = rem_interval(st, l, r);
        let Site::Value(var) = self.sites[e.0 as usize] else {
            return Val::Itv(lo, hi);
        };
        let ((llo, lhi), (rlo, rhi)) = (interval(st, l), interval(st, r));
        self.assign(st, var, &Val::Itv(lo, hi));
        let v = var_val(var);
        // A zero divisor traps, so one that is not negative is positive where this runs, and
        // one that is not positive is negative.
        if rlo >= 0 {
            self.le(st, &v, r, -1);
            self.le(st, &negate(r), &v, -1);
        }
        if rhi <= 0 {
            self.le(st, &v, &negate(r), -1);
            self.le(st, r, &v, -1);
        }
        if llo >= 0 {
            self.le(st, &v, l, 0);
        }
        if lhi <= 0 {
            self.le(st, l, &v, 0);
        }
        v
    }

    /// A variant built: each value of its payload enters its field's type, in the order
    /// written, which is the declaration's, as a struct's do; an enum's value is not followed.
    fn variant(&mut self, st: &mut Octagon, enum_name: &str, name: &str, payload: &Payload) -> Val {
        let fields: Vec<(ExprId, Option<String>)> = match payload {
            Payload::None => Vec::new(),
            Payload::Named(fields) => fields
                .iter()
                .map(|f| (f.value, Some(f.name.clone())))
                .collect(),
            // A payload built by position is an error, which the type checker reports first.
            Payload::Positional(args) => args.iter().map(|a| (a.value, None)).collect(),
        };
        let mut built: Vec<Val> = Vec::new();
        for (value, field) in fields {
            self.read_before(st, value, built.iter_mut());
            let decl = field.and_then(|f| {
                let def = self.structs.get_enum(enum_name)?;
                let (_, v) = def.variant(name)?;
                let (_, fd) = v.field(&f)?;
                Some((def, fd.decl, f))
            });
            let v = match decl {
                Some((def, t, f)) => {
                    let shape = Self::declared_shape(def.tree(), t);
                    let place = format!("field `{}` of `{}.{}`", f, enum_name, name);
                    self.eval_into(st, value, &shape, &place)
                }
                None => self.eval(st, value),
            };
            built.push(v);
        }
        Val::Other
    }

    /// A `match` (`core-calculus.md`, item 14): its scrutinee evaluated once, then each arm on
    /// the states its pattern selects, joined as an `if`'s branches are.
    fn match_expr(&mut self, st: &mut Octagon, e: ExprId, scrutinee: ExprId, arms: &[Arm]) -> Val {
        let ends = self.match_arms(st.clone(), e, scrutinee, arms);
        let v = match self.sites[e.0 as usize] {
            Site::Value(var) => {
                let mut out: Option<Octagon> = None;
                for (mut s, v) in ends {
                    self.assign(&mut s, var, &v);
                    out = Some(match out {
                        Some(o) => o.join(&s),
                        None => s,
                    });
                }
                *st = out.unwrap_or_else(|| {
                    let mut s = st.clone();
                    s.set_empty();
                    s
                });
                return var_val(var);
            }
            Site::None | Site::Call(_) => {
                let vals: Vec<Val> = ends
                    .iter()
                    .filter(|(s, _)| !s.is_empty())
                    .map(|(s, v)| summary_of(s, v))
                    .collect();
                let joined = vals
                    .iter()
                    .skip(1)
                    .fold(vals.first().cloned(), |a, b| a.map(|a| join_summary(&a, b)));
                joined.unwrap_or(Val::Other)
            }
        };
        let mut out: Option<Octagon> = None;
        for (s, _) in ends {
            out = Some(match out {
                Some(o) => o.join(&s),
                None => s,
            });
        }
        *st = out.unwrap_or_else(|| {
            let mut s = st.clone();
            s.set_empty();
            s
        });
        v
    }

    /// The state each arm of a `match` ends in, and its value: the scrutinee is evaluated once,
    /// in `st`; an arm's pattern then selects its states. A literal gives a scrutinee Tier 1
    /// follows its value, `true` and `false` decide a condition, `_` on an integer takes the
    /// range left past the literals before it at either end, and a variant's bindings take their
    /// fields' types and refinements.
    /// A `match`'s integer scrutinee is narrowed into its type, which its arms cover
    /// ([`match-scrutinee-narrowing`]): Tier 1 proves the value fits, or reports `E0502` with a
    /// `guard`, or `E0508` where it never fits; the arms then read it within the type. A place
    /// or a call's result already lies within its type, where it was stored or returned, and
    /// so does a value of one 128-bit type, checked at its operator.
    ///
    /// [`match-scrutinee-narrowing`]: ../../docs/design-review.md#match-scrutinee-narrowing
    fn scrutinee_fits(&mut self, st: &mut Octagon, scrutinee: ExprId, value: &Val) {
        let t = match &self.typing.exprs[scrutinee.0 as usize] {
            Ty::Int(Int::One(t)) | Ty::Int(Int::Mixed(Some(t))) => *t,
            Ty::Int(Int::Literal) => crate::types::I64,
            Ty::Int(Int::Mixed(None))
            | Ty::Bool
            | Ty::Array(..)
            | Ty::Struct(_)
            | Ty::Enum(_)
            | Ty::Unit
            | Ty::Any => return,
        };
        let span = self.span(scrutinee);
        let kind = &self.tree.expr(scrutinee).kind;
        // A value read from a place or given by a call already lies in its type.
        let stored = match kind {
            ExprKind::Name(_)
            | ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Call { .. } => true,
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => false,
        };
        // An operation on operands of one 128-bit type is checked at its operator against that
        // type (`core-calculus.md`, item 4), so its value, and an `if`'s or a `match`'s of that
        // type, already lies within it.
        let checked = match &self.typing.exprs[scrutinee.0 as usize] {
            Ty::Int(Int::One(t)) => t.bits == 128,
            Ty::Int(Int::Literal | Int::Mixed(_))
            | Ty::Bool
            | Ty::Array(..)
            | Ty::Struct(_)
            | Ty::Enum(_)
            | Ty::Unit
            | Ty::Any => false,
        };
        let settled = st.is_empty() || stored || checked;
        let what = self.own_text(scrutinee);
        if settled {
            // Nothing to prove.
        } else if let Some(side) = self.fits(st, value, Some(t), span) {
            self.always_traps(&side, t, span, &format!("`{}`", what.0));
        } else if !within(st, value, t) {
            // A `guard` that repeats an `if` or a `match` tells Tier 1 nothing of this one.
            let control = kind.is_choice();
            let was = std::mem::replace(&mut self.unfollowed, control);
            self.refinement(
                st,
                value,
                &type_refine(t),
                (span, Tag::Into(t.name)),
                "what the `match` reads",
                Some(what),
            );
            self.unfollowed = was;
        }
        // Past the narrowing, the value lies within the type: a bound beyond the octagon's
        // reach, a 128-bit type's, is compared exactly.
        let r = type_refine(t);
        for (op, bound, lit) in [(Rel::Ge, r.lo, r.lo_lit), (Rel::Le, r.hi, r.hi_lit)] {
            let c = match far(lit) {
                Some(c) => Cond::Far(op, Box::new((value.clone(), c))),
                None => cmp(op, value.clone(), bound),
            };
            self.assume(st, &c, true);
        }
    }

    fn match_arms(
        &mut self,
        mut st: Octagon,
        e: ExprId,
        scrutinee: ExprId,
        arms: &[Arm],
    ) -> Vec<(Octagon, Val)> {
        let is_bool = matches!(self.typing.exprs[scrutinee.0 as usize], Ty::Bool);
        let (value, cond) = if is_bool {
            (Val::Other, Some(self.cond(&mut st, scrutinee)))
        } else {
            (self.eval(&mut st, scrutinee), None)
        };
        self.scrutinee_fits(&mut st, scrutinee, &value);
        let mut literals: Vec<Exact> = Vec::new();
        let mut ends = Vec::new();
        for (k, arm) in arms.iter().enumerate() {
            let mut a = st.clone();
            let mark = self.scope.len();
            match &arm.pattern.kind {
                PatternKind::Bool(b) => {
                    if let Some(c) = &cond {
                        self.assume(&mut a, c, *b);
                    }
                }
                PatternKind::Int {
                    negative, value: n, ..
                } => {
                    let lit = Exact::new(*negative, *n);
                    literals.push(lit);
                    let c = if lit.is_far() {
                        Cond::Far(Rel::Eq, Box::new((value.clone(), lit)))
                    } else {
                        cmp(Rel::Eq, value.clone(), bound_int(*negative, *n))
                    };
                    self.assume(&mut a, &c, true);
                }
                PatternKind::Wildcard => {
                    // The values the literals before it take at either end of the scrutinee's
                    // range are left out of `_`'s.
                    // Each end a literal takes moves inward past the literals that follow it,
                    // whether or not the other end is bounded.
                    let (lo, hi) = interval(&a, &value);
                    let has = |x: i128| literals.contains(&Exact::from_i128(x));
                    if has(lo) {
                        let mut l = lo;
                        while has(l) {
                            l += 1;
                        }
                        self.assume(
                            &mut a,
                            &cmp(Rel::Ge, value.clone(), lin(Vec::new(), l)),
                            true,
                        );
                    }
                    if has(hi) {
                        let mut h = hi;
                        while has(h) {
                            h -= 1;
                        }
                        self.assume(
                            &mut a,
                            &cmp(Rel::Le, value.clone(), lin(Vec::new(), h)),
                            true,
                        );
                    }
                }
                PatternKind::Variant { bindings, .. } => {
                    for i in 0..bindings.as_ref().map_or(0, Vec::len) {
                        let Some(&id) = self.resolution.patterns.get(&(e, k, i)) else {
                            continue;
                        };
                        let b = id.0 as usize;
                        if let Some(var) = self.vars[b] {
                            let shape = self.binding_shape(b);
                            self.track(&mut a, var, &TOP);
                            a.forget(var);
                            self.facts(&mut a, var, &shape);
                        }
                        self.havoc_paths(&mut a, b, &[]);
                        self.scope.push(b);
                    }
                }
            }
            let v = if a.is_empty() {
                Val::Other
            } else {
                match arm.body {
                    ArmBody::Block(b) => self.block(&mut a, b, None),
                    ArmBody::Expr(x) => {
                        let (stmt, arm) = (self.stmt.take(), self.arm.replace(x));
                        let v = self.eval(&mut a, x);
                        self.stmt = stmt;
                        self.arm = arm;
                        v
                    }
                }
            };
            self.scope.truncate(mark);
            ends.push((a, v));
        }
        ends
    }

    fn if_expr(
        &mut self,
        st: &mut Octagon,
        e: ExprId,
        cond: ExprId,
        then: BlockId,
        otherwise: Option<Else>,
    ) -> Val {
        let c = self.cond(st, cond);
        let mut t = st.clone();
        self.assume(&mut t, &c, true);
        let tv = self.block(&mut t, then, None);
        let mut o = st.clone();
        self.assume(&mut o, &c, false);
        let ov = match otherwise {
            Some(Else::Block(b)) => self.block(&mut o, b, None),
            Some(Else::If(i)) => self.eval(&mut o, i),
            None => Val::Other,
        };
        let v = match self.sites[e.0 as usize] {
            Site::Value(var) => {
                self.assign(&mut t, var, &tv);
                self.assign(&mut o, var, &ov);
                var_val(var)
            }
            Site::None | Site::Call(_) => {
                let arms: Vec<Val> = [(&t, &tv), (&o, &ov)]
                    .into_iter()
                    .filter(|(s, _)| !s.is_empty())
                    .map(|(s, v)| summary_of(s, v))
                    .collect();
                match arms.as_slice() {
                    [a, b] => join_summary(a, b),
                    [a] => a.clone(),
                    _ => Val::Other,
                }
            }
        };
        *st = t.join(&o);
        v
    }

    /// A condition, its operands evaluated once; the right operand of `and` and `or` is
    /// evaluated where the left one has decided that it runs.
    fn cond(&mut self, st: &mut Octagon, e: ExprId) -> Cond {
        let tree = self.tree;
        match &tree.expr(e).kind {
            ExprKind::Bool(b) => Cond::Const(*b),
            ExprKind::Unary {
                op: UnaryOp::Not,
                operand,
            } => Cond::Not(Box::new(self.cond(st, *operand))),
            ExprKind::Binary {
                op: op @ (BinaryOp::And | BinaryOp::Or),
                lhs,
                rhs,
            } => {
                let and = *op == BinaryOp::And;
                let l = self.cond(st, *lhs);
                let mut s2 = st.clone();
                self.assume(&mut s2, &l, and);
                let r = self.cond(&mut s2, *rhs);
                let mut changed = Vec::new();
                self.changed_expr(*rhs, &mut changed);
                let mut l = l;
                if !changed.is_empty() {
                    // A right operand that changes bindings where it runs leaves them changed:
                    // the state is then where the left operand decided, or where the right one
                    // ended, and a left operand that reads what it changes is not known in it.
                    self.assume(st, &l, !and);
                    *st = st.join(&s2);
                    let touched: Vec<usize> =
                        changed.iter().flat_map(|&b| self.vars_of(b)).collect();
                    if touched.iter().any(|&x| cond_mentions(&l, x)) {
                        l = Cond::Unknown;
                    }
                }
                if and {
                    Cond::And(Box::new(l), Box::new(r))
                } else {
                    Cond::Or(Box::new(l), Box::new(r))
                }
            }
            // `and` and `or` have their arm above; an arithmetic or bit operation is a value.
            ExprKind::Binary { op, lhs, rhs } => match Rel::of(*op) {
                Some(op) => {
                    let mut l = self.eval(st, *lhs);
                    self.read_before(st, *rhs, [&mut l]);
                    let r = self.eval(st, *rhs);
                    match (far(self.literal(*lhs)), far(self.literal(*rhs))) {
                        (_, Some(c)) => Cond::Far(op, Box::new((l, c))),
                        (Some(c), None) => Cond::Far(mirror(op), Box::new((r, c))),
                        (None, None) => cmp(op, l, r),
                    }
                }
                None => {
                    self.eval(st, e);
                    Cond::Unknown
                }
            },
            ExprKind::Int { .. }
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Index { .. }
            | ExprKind::Field { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => {
                self.eval(st, e);
                Cond::Unknown
            }
        }
    }

    // -- calls -------------------------------------------------------------------------------

    /// A value in a callee's clause: its parameters are the call's variables, a `mut` one
    /// the value it leaves when `after`.
    fn callee_val(&self, st: &Octagon, c: &Callee, site: &CallSite, e: ExprId, after: bool) -> Val {
        let tree = &c.syntax.tree;
        let sub = |e: ExprId| self.callee_val(st, c, site, e, after);
        match &tree.expr(e).kind {
            ExprKind::Int { value, .. } => int_literal(*value),
            ExprKind::Name(n) if n == "result" => site.result.map_or(TOP, var_val),
            ExprKind::Name(n) => {
                let Some(k) = c.syntax.params.iter().position(|p| p.name == *n) else {
                    return TOP;
                };
                let var = if after && c.syntax.params[k].is_mut {
                    site.outs[k]
                } else {
                    site.ins[k]
                };
                var.map_or(TOP, var_val)
            }
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => negate(&sub(*operand)),
            ExprKind::Binary { op, lhs, rhs } => {
                let (l, r) = (sub(*lhs), sub(*rhs));
                match op {
                    BinaryOp::Add => add_vals(st, &l, &r),
                    BinaryOp::Sub => add_vals(st, &l, &negate(&r)),
                    BinaryOp::Mul => mul_vals(st, &l, &r),
                    BinaryOp::Div => div_vals(st, &l, &r),
                    BinaryOp::Rem => {
                        let (lo, hi) = rem_interval(st, &l, &r);
                        Val::Itv(lo, hi)
                    }
                    BinaryOp::Or
                    | BinaryOp::And
                    | BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::BitAnd
                    | BinaryOp::Shl
                    | BinaryOp::Shr => TOP,
                }
            }
            ExprKind::Field { .. } if self.callee_path_var(c, site, e, after).is_some() => {
                self.callee_path_var(c, site, e, after).map_or(TOP, var_val)
            }
            ExprKind::Field { base, .. } => match &tree.expr(*base).kind {
                ExprKind::Name(n) => c
                    .syntax
                    .params
                    .iter()
                    .find(|p| p.name == *n)
                    .and_then(|p| match &tree.ty(p.ty).kind {
                        TypeKind::Array { len, .. } => Some(int_literal(*len)),
                        TypeKind::Int { .. }
                        | TypeKind::Bool
                        | TypeKind::Named(_)
                        | TypeKind::Enum(_)
                        | TypeKind::Error => None,
                    })
                    .unwrap_or(TOP),
                ExprKind::Int { .. }
                | ExprKind::Bool(_)
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
                | ExprKind::Array(_)
                | ExprKind::Repeat { .. }
                | ExprKind::If { .. }
                | ExprKind::Error => TOP,
            },
            ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Index { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => TOP,
        }
    }

    fn callee_cond(
        &self,
        st: &Octagon,
        c: &Callee,
        site: &CallSite,
        e: ExprId,
        after: bool,
    ) -> Cond {
        let tree = &c.syntax.tree;
        let sub = |e: ExprId| Box::new(self.callee_cond(st, c, site, e, after));
        match &tree.expr(e).kind {
            ExprKind::Bool(b) => Cond::Const(*b),
            ExprKind::Unary {
                op: UnaryOp::Not,
                operand,
            } => Cond::Not(sub(*operand)),
            ExprKind::Binary {
                op: BinaryOp::And,
                lhs,
                rhs,
            } => Cond::And(sub(*lhs), sub(*rhs)),
            ExprKind::Binary {
                op: BinaryOp::Or,
                lhs,
                rhs,
            } => Cond::Or(sub(*lhs), sub(*rhs)),
            // `and` and `or` have their arms above; an arithmetic or bit operation is a value.
            ExprKind::Binary { op, lhs, rhs } => match Rel::of(*op) {
                Some(op) => cmp(
                    op,
                    self.callee_val(st, c, site, *lhs, after),
                    self.callee_val(st, c, site, *rhs, after),
                ),
                None => Cond::Unknown,
            },
            ExprKind::Int { .. }
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Unary { .. }
            | ExprKind::Index { .. }
            | ExprKind::Field { .. }
            | ExprKind::Call { .. }
            | ExprKind::Construct { .. }
            | ExprKind::Variant { .. }
            | ExprKind::Match { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => Cond::Unknown,
        }
    }

    /// A place a call may change through a `mut` parameter: a scalar binding loses its facts
    /// but its type's.
    fn clobber(&self, st: &mut Octagon, place: ExprId) {
        if let Some((root, prefix)) = field_chain(self.tree, self.resolution, place) {
            self.havoc_paths(st, root, &prefix);
        }
        if let ExprKind::Name(_) = self.tree.expr(place).kind {
            if let Some(b) = self.resolution.names[place.0 as usize] {
                let b = b.0 as usize;
                if let Some(v) = self.vars[b] {
                    self.track(st, v, &TOP);
                    st.forget(v);
                    let shape = self.binding_shape(b);
                    self.facts(st, v, &shape);
                }
            }
        }
    }

    /// At a call, the field paths of argument `k`, a struct, that the callee's clauses read, as
    /// they are when it is evaluated.
    fn arg_paths(&mut self, st: &mut Octagon, site: &CallSite, k: usize, arg: ExprId) {
        let Some(paths) = site.paths.get(k).filter(|ps| !ps.is_empty()) else {
            return;
        };
        let given = self.struct_vals(arg);
        for p in paths {
            let v = match given.iter().find(|(n, _)| *n == p.names) {
                Some((_, v)) => v.clone(),
                None => self.summary(st, &p.shape),
            };
            self.assign(st, p.var, &v);
            self.facts(st, p.var, &p.shape);
        }
    }

    /// After a call, a `mut` argument's place takes the parameter's final value `back`: an
    /// integer binding or field path its value, and a struct its paths' types and refinements.
    fn give_back(
        &mut self,
        st: &mut Octagon,
        (root, steps): (Option<usize>, &[Step]),
        arg: ExprId,
        back: &Val,
        shape: &Shape,
    ) {
        let scalar = root
            .filter(|_| steps.is_empty())
            .and_then(|b| self.vars[b])
            .or_else(|| self.path_var(arg));
        if let Some(var) = scalar {
            self.track(st, var, back);
            self.assign(st, var, back);
            self.facts(st, var, shape);
        } else if let Some((r, prefix)) = field_chain(self.tree, self.resolution, arg) {
            self.havoc_paths(st, r, &prefix);
        }
    }

    fn call(&mut self, st: &mut Octagon, e: ExprId, args: &[Arg]) -> Val {
        let known = match &self.sites[e.0 as usize] {
            Site::Call(site) => self
                .callees
                .get(site.callee)
                .copied()
                .flatten()
                .map(|c| (site.clone(), c)),
            Site::None | Site::Value(_) => None,
        };
        let Some((site, c)) = known else {
            // A function without a body: its `mut` arguments may change. The checker refuses
            // such a call (`E0224`) until M4's `extern` functions; this keeps Tier 1 sound then.
            for a in args {
                self.eval(st, a.value);
                if a.is_mut {
                    self.clobber(st, a.value);
                }
            }
            let typing = self.typing;
            return self.summary(st, &Shape::plain(&typing.exprs[e.0 as usize]));
        };
        let callee = c.syntax;
        let span = self.span(e);
        forget_site(st, &site);
        // Every argument is evaluated, left to right, its value kept as it is then, before any
        // enters its parameter's type, as the reference interpreter does: a narrowing inside a
        // later argument comes before an earlier argument's ...
        let mut vals = Vec::new();
        for (k, a) in args.iter().enumerate() {
            let v = self.eval(st, a.value);
            match site.ins.get(k).copied().flatten() {
                Some(var) => {
                    self.assign(st, var, &v);
                    vals.push(var_val(var));
                }
                None => vals.push(v),
            }
            self.arg_paths(st, &site, k, a.value);
        }
        // ... then each enters its parameter's plain type, left to right ...
        for (k, (a, p)) in args.iter().zip(&callee.params).enumerate() {
            let plain = Shape::plain(&Ty::of(&callee.tree, p.ty));
            self.narrow(
                st,
                &vals[k],
                &plain,
                (self.span(a.value), Tag::None),
                "",
                None,
            );
            if let Some(var) = site.ins[k] {
                self.facts(st, var, &plain);
            }
        }
        // ... then its refinement, once every argument is in.
        for (k, (a, p)) in args.iter().zip(&callee.params).enumerate() {
            let shape = self.callee_shape(&c, &site, args, p.ty).refinements();
            let place = format!("parameter `{}` of `{}`", p.name, callee.name);
            let what = self.own_text(a.value);
            let v = site.ins[k].map_or_else(|| vals[k].clone(), var_val);
            self.narrow(
                st,
                &v,
                &shape,
                (self.span(a.value), Tag::None),
                &place,
                Some(what),
            );
            if let Some(var) = site.ins[k] {
                self.facts(st, var, &shape);
            }
        }
        for clause in &callee.contracts {
            if clause.kind != ContractKind::Requires
                || self.callee_outside(&c, clause.expr).is_some()
            {
                continue;
            }
            let k = self.callee_cond(st, &c, &site, clause.expr, false);
            let failing = self.refute(st, &k);
            let tag = Tag::Clause(callee.tree.expr(clause.expr).span);
            self.record(
                ObligationKind::Requires,
                span,
                true,
                failing.is_none(),
                tag,
                |_| vec![(st.clone(), Goal::Holds(k))],
            );
            let Some(failing) = failing else {
                continue;
            };
            let extra = self.call_named(callee, &site);
            let (valuation, _) = self.valuation(&failing, &extra, &[]);
            let text = print(&callee.tree, clause.expr, &no_subst).0;
            let subst = |id: ExprId, _: &str| self.arg_text(callee, args, id);
            let at_call = print(&callee.tree, clause.expr, &subst).0;
            let what = format!("`requires {}` of `{}`", text, callee.name);
            let message = Fx::cannot_rule_out(
                &valuation,
                &format!("which violates {} at this call", what),
                &format!("this call violates {}", what),
            );
            let followed = self.clause_followed(callee, args, clause.expr);
            let (guard, message) = Fx::usable_guard(at_call, followed, "at this call", message);
            let anchor = self.anchor();
            self.emit("E0504", span, message, guard, anchor);
        }
        self.call_measures(st, &c, &site, args, span);
        // After the call: the result and the `mut` parameters' final values, then `ensures`.
        if let (Some(r), Some(t)) = (site.result, callee.result) {
            let shape = self.callee_shape(&c, &site, args, t);
            self.facts(st, r, &shape);
        }
        for (k, p) in callee.params.iter().enumerate() {
            if let Some(o) = site.outs[k] {
                let shape = self.callee_shape(&c, &site, args, p.ty);
                self.facts(st, o, &shape);
            }
        }
        for clause in &callee.contracts {
            if clause.kind == ContractKind::Ensures {
                let k = self.callee_cond(st, &c, &site, clause.expr, true);
                self.assume(st, &k, true);
            }
        }
        // A `mut` argument's place takes the parameter's final value back.
        for (k, (a, p)) in args.iter().zip(&callee.params).enumerate() {
            if !a.is_mut {
                continue;
            }
            let back = site.outs[k].map_or_else(
                || {
                    let shape = self.callee_shape(&c, &site, args, p.ty);
                    self.summary(st, &shape)
                },
                var_val,
            );
            let mut steps = Vec::new();
            let root = self.place_parts(a.value, &mut steps);
            let shape = self.place_shape(root, &steps);
            let place = format!("`{}`", self.own_text(a.value).0);
            self.narrow(
                st,
                &back,
                &shape.refinements(),
                (self.span(a.value), Tag::Back),
                &place,
                None,
            );
            self.give_back(st, (root, &steps), a.value, &back, &shape);
        }
        let value = match (site.result, callee.result) {
            (Some(r), _) => var_val(r),
            (None, Some(t)) => {
                let shape = self.callee_shape(&c, &site, args, t);
                self.summary(st, &shape)
            }
            (None, None) => Val::Other,
        };
        forget_site(st, &site);
        value
    }

    // -- loops -------------------------------------------------------------------------------

    fn conjuncts(&self, e: ExprId, out: &mut Vec<Cand>) {
        match &self.tree.expr(e).kind {
            ExprKind::Binary {
                op: BinaryOp::And,
                lhs,
                rhs,
            } => {
                self.conjuncts(*lhs, out);
                self.conjuncts(*rhs, out);
            }
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
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => out.push(Cand::Expr(e)),
        }
    }

    /// The conditions of the `guard`s in a loop's body, outside its inner loops.
    fn guards(&self, b: BlockId, out: &mut Vec<Cand>) {
        let tree = self.tree;
        for &s in &tree.block(b).stmts {
            match &tree.stmt(s).kind {
                StmtKind::Guard { cond, otherwise } => {
                    self.conjuncts(*cond, out);
                    self.guards(*otherwise, out);
                }
                StmtKind::Expr(e) => self.guards_in_if(*e, out),
                // A guard of an inner loop holds in its rounds only, and the other statements
                // hold none.
                StmtKind::Binding { .. }
                | StmtKind::Assign { .. }
                | StmtKind::While { .. }
                | StmtKind::Return(_)
                | StmtKind::Break
                | StmtKind::Continue
                | StmtKind::Discard(_) => {}
            }
        }
    }

    /// The conditions of the `guard`s in the blocks of an `if` or a `match` statement.
    fn guards_in_if(&self, e: ExprId, out: &mut Vec<Cand>) {
        match &self.tree.expr(e).kind {
            ExprKind::If {
                then, otherwise, ..
            } => {
                self.guards(*then, out);
                match otherwise {
                    Some(Else::Block(b)) => self.guards(*b, out),
                    Some(Else::If(i)) => self.guards_in_if(*i, out),
                    None => {}
                }
            }
            ExprKind::Match { arms, .. } => {
                for arm in arms {
                    match arm.body {
                        ArmBody::Block(b) => self.guards(b, out),
                        ArmBody::Expr(x) => self.guards_in_if(x, out),
                    }
                }
            }
            // No other expression holds a statement.
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
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::Error => {}
        }
    }

    /// The bindings a block declares, assigns or passes as `mut`.
    fn changed_block(&self, b: BlockId, out: &mut Vec<usize>) {
        for &s in &self.tree.block(b).stmts {
            self.changed_stmt(s, out);
        }
    }

    /// The bindings a statement declares, assigns or passes as `mut`: a loop's in its header,
    /// its condition, invariants and measures, which run at every round, as in its body.
    fn changed_stmt(&self, s: StmtId, out: &mut Vec<usize>) {
        match &self.tree.stmt(s).kind {
            StmtKind::Binding { init, .. } => {
                out.extend(self.resolution.stmts[s.0 as usize].map(|b| b.0 as usize));
                self.changed_expr(*init, out);
            }
            StmtKind::Assign { place, value, .. } => {
                out.extend(self.place_parts(*place, &mut Vec::new()));
                self.changed_expr(*place, out);
                self.changed_expr(*value, out);
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                for &e in std::iter::once(cond).chain(invariants).chain(decreases) {
                    self.changed_expr(e, out);
                }
                self.changed_block(*body, out);
            }
            StmtKind::Guard { cond, otherwise } => {
                self.changed_expr(*cond, out);
                self.changed_block(*otherwise, out);
            }
            StmtKind::Return(Some(e)) | StmtKind::Discard(e) | StmtKind::Expr(e) => {
                self.changed_expr(*e, out)
            }
            // Every other kind is named, so that a new one must say what it changes.
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
        }
    }

    fn changed_expr(&self, e: ExprId, out: &mut Vec<usize>) {
        match &self.tree.expr(e).kind {
            ExprKind::Call { args, .. } => {
                for a in args {
                    if a.is_mut {
                        out.extend(self.place_parts(a.value, &mut Vec::new()));
                    }
                    self.changed_expr(a.value, out);
                }
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.changed_expr(*cond, out);
                self.changed_block(*then, out);
                match otherwise {
                    Some(Else::Block(b)) => self.changed_block(*b, out),
                    Some(Else::If(i)) => self.changed_expr(*i, out),
                    None => {}
                }
            }
            ExprKind::Unary { operand, .. } => self.changed_expr(*operand, out),
            ExprKind::Binary { lhs, rhs, .. }
            | ExprKind::Index {
                base: lhs,
                index: rhs,
            } => {
                self.changed_expr(*lhs, out);
                self.changed_expr(*rhs, out);
            }
            ExprKind::Array(items) => {
                for &i in items {
                    self.changed_expr(i, out);
                }
            }
            ExprKind::Construct { fields, .. } => {
                for f in fields {
                    self.changed_expr(f.value, out);
                }
            }
            ExprKind::Variant { payload, .. } => match payload {
                Payload::None => {}
                Payload::Named(fields) => {
                    for f in fields {
                        self.changed_expr(f.value, out);
                    }
                }
                Payload::Positional(args) => {
                    for a in args {
                        if a.is_mut {
                            out.extend(self.place_parts(a.value, &mut Vec::new()));
                        }
                        self.changed_expr(a.value, out);
                    }
                }
            },
            // A `match` declares its patterns' bindings in its arms, as a block its bindings.
            ExprKind::Match { scrutinee, arms } => {
                self.changed_expr(*scrutinee, out);
                for (k, arm) in arms.iter().enumerate() {
                    let declared = match &arm.pattern.kind {
                        PatternKind::Variant {
                            bindings: Some(bs), ..
                        } => bs.len(),
                        PatternKind::Variant { bindings: None, .. }
                        | PatternKind::Int { .. }
                        | PatternKind::Bool(_)
                        | PatternKind::Wildcard => 0,
                    };
                    for i in 0..declared {
                        if let Some(id) = self.resolution.patterns.get(&(e, k, i)) {
                            out.push(id.0 as usize);
                        }
                    }
                    match arm.body {
                        ArmBody::Expr(x) => self.changed_expr(x, out),
                        ArmBody::Block(b) => self.changed_block(b, out),
                    }
                }
            }
            ExprKind::Field { base, .. } | ExprKind::Repeat { value: base, .. } => {
                self.changed_expr(*base, out)
            }
            // Every other kind is named, so that a new one must say what it changes.
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Error => {}
        }
    }

    /// Values read before `later` runs, kept as they are then: one that reads a binding
    /// `later` changes keeps the bounds it has now, as the reference interpreter has read it
    /// already.
    fn read_before<'v>(
        &self,
        st: &Octagon,
        later: ExprId,
        vals: impl IntoIterator<Item = &'v mut Val>,
    ) {
        let mut changed = Vec::new();
        self.changed_expr(later, &mut changed);
        let touched: Vec<usize> = changed.iter().flat_map(|&b| self.vars_of(b)).collect();
        for v in vals {
            if touched.iter().any(|&x| mentions(v, x)) {
                *v = summary_of(st, v);
            }
        }
    }

    fn cand(&mut self, st: &mut Octagon, c: Cand) -> Cond {
        match (
            c,
            &self
                .tree
                .expr(match c {
                    Cand::Expr(e) | Cand::Weak(e) => e,
                })
                .kind,
        ) {
            (Cand::Weak(_), ExprKind::Binary { op, lhs, rhs }) => {
                let weak = if *op == BinaryOp::Lt {
                    Rel::Le
                } else {
                    Rel::Ge
                };
                let l = self.eval(st, *lhs);
                let r = self.eval(st, *rhs);
                cmp(weak, l, r)
            }
            (Cand::Expr(e) | Cand::Weak(e), _) => self.cond(st, e),
        }
    }

    fn cand_text(&self, c: Cand) -> String {
        match (
            c,
            &self
                .tree
                .expr(match c {
                    Cand::Expr(e) | Cand::Weak(e) => e,
                })
                .kind,
        ) {
            (Cand::Weak(_), ExprKind::Binary { op, lhs, rhs }) => format!(
                "{} {} {}",
                paren(self.own_text(*lhs), OPERAND),
                if *op == BinaryOp::Lt { "<=" } else { ">=" },
                paren(self.own_text(*rhs), OPERAND)
            ),
            (Cand::Expr(e) | Cand::Weak(e), _) => self.own_text(e).0,
        }
    }

    fn refute_cand(&mut self, st: &Octagon, c: Cand) -> Option<Octagon> {
        let mut s = st.clone();
        let k = self.cand(&mut s, c);
        self.refute(&s, &k)
    }

    /// The state at a loop head: the entry state with every binding the loop changes
    /// forgotten but for its type, the surviving candidates, and the routed invariants.
    fn head(
        &mut self,
        entry: &Octagon,
        changed: &[usize],
        cands: &[Cand],
        alive: &[bool],
        axioms: &[ExprId],
    ) -> Octagon {
        let mut h = entry.clone();
        for &b in changed {
            if let Some(v) = self.vars[b] {
                self.track(&mut h, v, &TOP);
                h.forget(v);
                let shape = self.binding_shape(b);
                self.facts(&mut h, v, &shape);
            }
            self.havoc_paths(&mut h, b, &[]);
        }
        // A candidate is the loop's own text, which reports what it must where it runs, in the
        // round: here it only narrows the head.
        let was = self.recording;
        self.recording = false;
        for (c, ok) in cands.iter().zip(alive) {
            if *ok {
                let k = self.cand(&mut h, *c);
                self.assume(&mut h, &k, true);
            }
        }
        self.recording = was;
        for &a in axioms {
            let k = self.cond(&mut h, a);
            self.assume(&mut h, &k, true);
        }
        h
    }

    /// Runs the body once from the head: the state at the back edge and at the exit, where a
    /// `break` is one more exit.
    fn pass(
        &mut self,
        head: &Octagon,
        (s, cond, body): (StmtId, ExprId, BlockId),
        entry: &Octagon,
    ) -> (Octagon, Octagon) {
        self.passes += 1;
        let header = self.head_of.replace(s);
        let mut at = head.clone();
        let c = self.cond(&mut at, cond);
        let mut b = at.clone();
        self.assume(&mut b, &c, true);
        // The measures at the start of a round that runs: never negative, and followed. One
        // linear in bindings is tracked by its change; any other is kept, to compare with.
        let measures = self.loop_measures(s);
        let outer = self.tracks.len();
        let mut tracked = Vec::new();
        for &(e, start, _) in &measures {
            let v = self.eval(&mut b, e);
            self.measure_never_negative(&b, &v, e, (s, cond, body), entry);
            let terms = match &v {
                Val::Lin(t, _) if t.iter().all(|t| self.vars.contains(&Some(t.var))) => {
                    Some(t.clone())
                }
                Val::Lin(..) | Val::Quot { .. } | Val::Itv(..) | Val::Arr(_) | Val::Other => None,
            };
            tracked.push(terms.is_some());
            match terms {
                Some(terms) => {
                    self.assign(&mut b, start, &lin(Vec::new(), 0));
                    self.tracks.push(Track {
                        delta: start,
                        terms,
                    });
                }
                None => self.assign(&mut b, start, &v),
            }
        }
        self.head_of = header;
        self.loops.push(Frame::default());
        let mut ends = self.body_ends(b, body);
        self.tracks.truncate(outer);
        let frame = self.loops.pop().unwrap_or_default();
        ends.extend(frame.continues);
        self.measure_decreases(&ends, cond, &measures, &tracked);
        let back = ends
            .iter()
            .skip(1)
            .fold(ends[0].clone(), |acc, x| acc.join(x));
        self.assume(&mut at, &c, false);
        let exit = frame.breaks.iter().fold(at, |acc, x| acc.join(x));
        (back, exit)
    }

    /// A loop's `decreases` measures with the variables of each.
    fn loop_measures(&self, s: StmtId) -> Vec<(ExprId, usize, usize)> {
        match &self.tree.stmt(s).kind {
            StmtKind::While { decreases, .. } => decreases
                .iter()
                .zip(&self.measures[s.0 as usize])
                .map(|(e, (start, end))| (*e, *start, *end))
                .collect(),
            StmtKind::Binding { .. }
            | StmtKind::Assign { .. }
            | StmtKind::Guard { .. }
            | StmtKind::Return(_)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Discard(_)
            | StmtKind::Expr(_) => Vec::new(),
        }
    }

    /// A loop's measure at the head of a round that runs is never negative. The guard goes
    /// before the loop when the first round can break it, else at the end of the body.
    fn measure_never_negative(
        &mut self,
        b: &Octagon,
        v: &Val,
        e: ExprId,
        (s, cond, body): (StmtId, ExprId, BlockId),
        entry: &Octagon,
    ) {
        if !self.recording {
            return;
        }
        let never_negative = cmp(Rel::Ge, v.clone(), lin(Vec::new(), 0));
        let failing = self.refute(b, &never_negative);
        self.record(
            ObligationKind::Decreases,
            self.span(e),
            true,
            failing.is_none(),
            Tag::Clause(self.span(e)),
            |_| vec![(b.clone(), Goal::Holds(never_negative))],
        );
        let Some(failing) = failing else {
            return;
        };
        self.recording = false;
        let mut first = entry.clone();
        let c = self.cond(&mut first, cond);
        self.assume(&mut first, &c, true);
        let v0 = self.eval(&mut first, e);
        let on_entry = self.refute(&first, &cmp(Rel::Ge, v0, lin(Vec::new(), 0)));
        self.recording = true;
        let anchor = match on_entry {
            Some(_) => Anchor::Before(s),
            None => Anchor::After(body),
        };
        let text = self.own_text(e);
        let (valuation, _) = self.valuation(&failing, &[], &[]);
        let message = Fx::cannot_rule_out(
            &valuation,
            &format!(
                "at the head of a round of this loop, where its measure `{}` is negative: a `decreases` measure is never negative",
                text.0
            ),
            &format!(
                "this loop's measure `{}` is negative at the head of a round: a `decreases` measure is never negative",
                text.0
            ),
        );
        let (guard, message) = if self.followed(e) {
            (Some(format!("{} >= 0", paren(text, OPERAND))), message)
        } else {
            (
                None,
                format!(
                    "{}; it reads a value Tier 1 does not follow, so no `guard` it can use states it",
                    message
                ),
            )
        };
        self.emit("E0507", self.span(e), message, guard, anchor);
    }

    /// A round of a loop that the next round follows leaves its measures smaller, in
    /// lexicographic order, than at the round's start.
    /// Runs a loop's body, keeping apart the states in which a trailing `if` statement's
    /// branches end, so that a measure is checked on each path rather than on their join.
    fn body_ends(&mut self, mut st: Octagon, body: BlockId) -> Vec<Octagon> {
        let tree = self.tree;
        let stmts = &tree.block(body).stmts;
        let mark = self.scope.len();
        let saved = self.stmt;
        let mut ends = Vec::new();
        for (i, &s) in stmts.iter().enumerate() {
            if st.is_empty() {
                break;
            }
            self.stmt = Some(s);
            match &tree.stmt(s).kind {
                StmtKind::Expr(e) if i + 1 == stmts.len() => {
                    ends = self.if_ends(std::mem::replace(&mut st, Octagon::new(0)), *e);
                }
                StmtKind::Binding { .. }
                | StmtKind::Assign { .. }
                | StmtKind::While { .. }
                | StmtKind::Guard { .. }
                | StmtKind::Return(_)
                | StmtKind::Break
                | StmtKind::Continue
                | StmtKind::Discard(_)
                | StmtKind::Expr(_) => {
                    self.stmt_eval(&mut st, s, None);
                }
            }
        }
        self.stmt = saved;
        self.scope.truncate(mark);
        if ends.is_empty() {
            ends.push(st);
        }
        ends
    }

    /// The states an `if` statement's branches end in, one for each branch.
    fn if_ends(&mut self, mut st: Octagon, e: ExprId) -> Vec<Octagon> {
        // A `match` statement's arms end apart, as an `if`'s branches do.
        if let ExprKind::Match { scrutinee, arms } = &self.tree.expr(e).kind {
            return self
                .match_arms(st, e, *scrutinee, arms)
                .into_iter()
                .map(|(s, _)| s)
                .collect();
        }
        let ExprKind::If {
            cond,
            then,
            otherwise,
        } = &self.tree.expr(e).kind
        else {
            self.eval(&mut st, e);
            return vec![st];
        };
        let c = self.cond(&mut st, *cond);
        let mut t = st.clone();
        self.assume(&mut t, &c, true);
        self.block(&mut t, *then, None);
        let mut o = st;
        self.assume(&mut o, &c, false);
        let mut ends = vec![t];
        match otherwise {
            Some(Else::Block(b)) => {
                self.block(&mut o, *b, None);
                ends.push(o);
            }
            Some(Else::If(i)) => ends.extend(self.if_ends(o, *i)),
            None => ends.push(o),
        }
        ends
    }

    /// The first path of a round, among `ends`, on which the next round can start without the
    /// measures having got smaller, with the measures' values after and before it.
    fn round_fails(
        &mut self,
        end: &Octagon,
        cond: ExprId,
        measures: &[(ExprId, usize, usize)],
        tracked: &[bool],
    ) -> Option<(Octagon, Vec<Val>, Vec<Val>)> {
        let (next, after, before) = self.round(end, cond, measures, tracked);
        let failing = self.refute(&next, &lex_less(&after, &before))?;
        Some((failing, after, before))
    }

    /// The state in which the next round starts after the path of a round that ends in `end`,
    /// with the measures' values then and at the round's start.
    fn round(
        &mut self,
        end: &Octagon,
        cond: ExprId,
        measures: &[(ExprId, usize, usize)],
        tracked: &[bool],
    ) -> (Octagon, Vec<Val>, Vec<Val>) {
        let mut next = end.clone();
        let c = self.cond(&mut next, cond);
        self.assume(&mut next, &c, true);
        // A tracked measure has changed by its delta, which is below 0 when it decreased; any
        // other is evaluated again, to compare with its value at the round's start.
        let mut after = Vec::new();
        let mut before = Vec::new();
        for (m, t) in measures.iter().zip(tracked) {
            if *t {
                after.push(var_val(m.1));
                before.push(lin(Vec::new(), 0));
            } else {
                after.push(self.eval(&mut next, m.0));
                before.push(var_val(m.1));
            }
        }
        (next, after, before)
    }

    fn measure_decreases(
        &mut self,
        ends: &[Octagon],
        cond: ExprId,
        measures: &[(ExprId, usize, usize)],
        tracked: &[bool],
    ) {
        if !self.recording || measures.is_empty() {
            return;
        }
        self.recording = false;
        let found = ends
            .iter()
            .find_map(|end| self.round_fails(end, cond, measures, tracked));
        self.recording = true;
        let span = self.span(measures[0].0);
        // The query of measures that get smaller on every path holds each path's state.
        self.record(
            ObligationKind::Decreases,
            span,
            true,
            found.is_none(),
            Tag::Smaller,
            |fx| {
                ends.iter()
                    .map(|end| {
                        let (next, after, before) = fx.round(end, cond, measures, tracked);
                        (next, Goal::Holds(lex_less(&after, &before)))
                    })
                    .collect()
            },
        );
        let Some((mut failing, after, before)) = found else {
            return;
        };
        // What the round changed each measure by, read from the failing state.
        for ((a, z), m) in after.iter().zip(&before).zip(measures) {
            let change = add_vals(&failing, a, &negate(z));
            self.assign(&mut failing, m.2, &change);
        }
        let extra: Vec<usize> = measures.iter().map(|m| m.2).collect();
        let (valuation, values) = self.valuation(&failing, &[], &extra);
        let texts: Vec<String> = measures.iter().map(|m| self.own_text(m.0).0).collect();
        let (measure, by) = (texts.join(", "), tuple(&values));
        let message = Fx::cannot_rule_out(
            &valuation,
            &format!(
                "after a round of this loop, where its measure `{}` changes by {}: a `decreases` measure gets smaller at every round",
                measure, by
            ),
            &format!(
                "a round of this loop changes its measure `{}` by {}: a `decreases` measure gets smaller at every round",
                measure, by
            ),
        );
        self.emit("E0507", span, message, None, Anchor::Nowhere);
    }

    /// The bindings a call's valuation names: those in scope, or the callee's parameters when
    /// none is.
    fn call_named(&self, callee: &FnSyntax, site: &CallSite) -> Vec<(String, usize)> {
        if self.scope.iter().any(|b| self.vars[*b].is_some()) {
            return Vec::new();
        }
        callee
            .params
            .iter()
            .zip(&site.ins)
            .filter_map(|(p, v)| v.map(|v| (p.name.clone(), v)))
            .collect()
    }

    /// At a call, the callee's measure is never negative; at a recursive call, it is smaller
    /// than the caller's on entry.
    fn call_measures(
        &mut self,
        st: &Octagon,
        c: &Callee,
        site: &CallSite,
        args: &[Arg],
        span: Span,
    ) {
        let callee = c.syntax;
        let named = self.call_named(callee, site);
        let mut values = Vec::new();
        let mut texts = Vec::new();
        for clause in &callee.contracts {
            if clause.kind != ContractKind::Decreases {
                continue;
            }
            let v = self.callee_val(st, c, site, clause.expr, false);
            let text = print(&callee.tree, clause.expr, &no_subst).0;
            let what = format!("the measure `{}` of `{}`", text, callee.name);
            let never_negative = cmp(Rel::Ge, v.clone(), lin(Vec::new(), 0));
            let failing = self.refute(st, &never_negative);
            let tag = Tag::Clause(callee.tree.expr(clause.expr).span);
            self.record(
                ObligationKind::Decreases,
                span,
                true,
                failing.is_none(),
                tag,
                |_| vec![(st.clone(), Goal::Holds(never_negative))],
            );
            if let Some(failing) = failing {
                let (valuation, _) = self.valuation(&failing, &named, &[]);
                let message = Fx::cannot_rule_out(
                    &valuation,
                    &format!(
                        "where {} is negative at this call: a `decreases` measure is never negative",
                        what
                    ),
                    &format!(
                        "{} is negative at this call: a `decreases` measure is never negative",
                        what
                    ),
                );
                let subst = |id: ExprId, _: &str| self.arg_text(callee, args, id);
                let at_call = print(&callee.tree, clause.expr, &subst);
                let (guard, message) = if self.clause_followed(callee, args, clause.expr) {
                    (Some(format!("{} >= 0", paren(at_call, OPERAND))), message)
                } else {
                    (
                        None,
                        format!(
                            "{}; at this call it is `{}`, which reads a value Tier 1 does not follow, so no `guard` it can use states it",
                            message, at_call.0
                        ),
                    )
                };
                let anchor = self.anchor();
                self.emit("E0507", span, message, guard, anchor);
            }
            values.push(v);
            texts.push(text);
        }
        // At a direct recursive call, the clauses together, in lexicographic order, are smaller
        // than on the caller's entry.
        if !std::ptr::eq(callee, self.syntax) || values.is_empty() {
            return;
        }
        let entry: Vec<Val> = self.entry_measures.iter().map(|m| var_val(m.0)).collect();
        let smaller = lex_less(&values, &entry);
        let failing = self.refute(st, &smaller);
        self.record(
            ObligationKind::Decreases,
            span,
            true,
            failing.is_none(),
            Tag::Smaller,
            |_| vec![(st.clone(), Goal::Holds(smaller))],
        );
        let Some(mut failing) = failing else {
            return;
        };
        for (v, m) in values.iter().zip(&self.entry_measures) {
            self.assign(&mut failing, m.1, v);
        }
        let extra: Vec<usize> = self
            .entry_measures
            .iter()
            .map(|m| m.1)
            .chain(self.entry_measures.iter().map(|m| m.0))
            .collect();
        let (valuation, picked) = self.valuation(&failing, &named, &extra);
        let (to, from) = picked.split_at(values.len().min(picked.len()));
        let what = format!(
            "the measure `{}` of `{}` is {} at this recursive call and {} on entry",
            texts.join(", "),
            callee.name,
            tuple(to),
            tuple(from)
        );
        let message = Fx::cannot_rule_out(
            &valuation,
            &format!(
                "where {}: a `decreases` measure gets smaller at every recursive call",
                what
            ),
            &format!(
                "{}: a `decreases` measure gets smaller at every recursive call",
                what
            ),
        );
        self.emit("E0507", span, message, None, Anchor::Nowhere);
    }

    fn while_loop(
        &mut self,
        st: &mut Octagon,
        s: StmtId,
        cond: ExprId,
        invariants: &[ExprId],
        body: BlockId,
    ) {
        // Outside the recording pass nothing is recorded, so a loop met again with the same
        // entry state leaves the state it left before.
        if !self.recording {
            if let Some((_, _, exit)) = self.loop_memo.iter().find(|(k, e, _)| *k == s && e == st) {
                *st = exit.clone();
                return;
            }
        }
        let entry = (!self.recording).then(|| st.clone());
        let mut changed = Vec::new();
        self.changed_stmt(s, &mut changed);
        // Candidates come only from the loop's text: the written invariants first, then the
        // condition's conjuncts with the non-strict form of each strict one, then the guards.
        let mut cands = Vec::new();
        let mut axioms = Vec::new();
        // The written clause each conjunct of a written invariant comes from.
        let mut clause_of = Vec::new();
        for &i in invariants {
            if self.outside(i).is_some() {
                axioms.push(i);
            } else {
                self.conjuncts(i, &mut cands);
                clause_of.resize(cands.len(), self.span(i));
            }
        }
        let written = cands.len();
        let mut from_cond = Vec::new();
        self.conjuncts(cond, &mut from_cond);
        let tree = self.tree;
        let weak: Vec<Cand> = from_cond
            .iter()
            .filter_map(|c| match c {
                Cand::Expr(e) => match tree.expr(*e).kind {
                    ExprKind::Binary {
                        op: BinaryOp::Lt | BinaryOp::Gt,
                        ..
                    } => Some(Cand::Weak(*e)),
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
                    | ExprKind::Array(_)
                    | ExprKind::Repeat { .. }
                    | ExprKind::If { .. }
                    | ExprKind::Error => None,
                },
                Cand::Weak(_) => None,
            })
            .collect();
        cands.extend(from_cond);
        cands.extend(weak);
        self.guards(body, &mut cands);

        let was = self.recording;
        self.recording = false;
        let entry_ok: Vec<bool> = cands
            .iter()
            .map(|c| self.refute_cand(st, *c).is_none())
            .collect();
        let mut alive = entry_ok.clone();
        let mut back;
        loop {
            let head = self.head(st, &changed, &cands, &alive, &axioms);
            back = self.pass(&head, (s, cond, body), st).0;
            let mut dropped = false;
            for (k, c) in cands.iter().enumerate() {
                if alive[k] && self.refute_cand(&back, *c).is_some() {
                    alive[k] = false;
                    dropped = true;
                }
            }
            if !dropped {
                break;
            }
        }
        // The written invariants: true on entry and preserved, or an error.
        let mut failures = Vec::new();
        for k in 0..written {
            let failing = if !entry_ok[k] {
                self.refute_cand(st, cands[k]).map(|f| (f, true))
            } else if !alive[k] {
                self.refute_cand(&back, cands[k]).map(|f| (f, false))
            } else {
                None
            };
            failures.push(failing);
        }
        self.recording = was;
        for (k, failing) in failures.into_iter().enumerate() {
            let c = cands[k];
            let span = self.span(match c {
                Cand::Expr(e) | Cand::Weak(e) => e,
            });
            let tag = Tag::Clause(clause_of[k]);
            self.record(
                ObligationKind::Invariant,
                span,
                true,
                failing.is_none(),
                tag,
                |fx| fx.invariant_paths(st, &back, c),
            );
            let Some((state, on_entry)) = failing else {
                continue;
            };
            let text = self.cand_text(c);
            let (valuation, _) = self.valuation(&state, &[], &[]);
            let (with, without, anchor) = if on_entry {
                (
                    format!("on entry to this loop, which violates `invariant {}`", text),
                    format!("this loop is entered with `invariant {}` false", text),
                    Anchor::Before(s),
                )
            } else {
                (
                    format!(
                        "after a round of this loop's body, which violates `invariant {}`: the body does not preserve it",
                        text
                    ),
                    format!("a round of this loop's body leaves `invariant {}` false", text),
                    Anchor::After(body),
                )
            };
            let message = Fx::cannot_rule_out(&valuation, &with, &without);
            self.emit("E0506", span, message, Some(text), anchor);
        }
        let head = self.head(st, &changed, &cands, &alive, &axioms);
        let mut facts = Head {
            span: tree.stmt(s).span,
            kept: Vec::new(),
            dropped: Vec::new(),
        };
        for (c, ok) in cands.iter().zip(&alive) {
            let text = self.cand_text(*c);
            if *ok {
                facts.kept.push(text);
            } else {
                facts.dropped.push(text);
            }
        }
        self.heads.push(facts);
        let (_, exit) = self.pass(&head, (s, cond, body), st);
        self.heads.pop();
        *st = exit;
        if let Some(entry) = entry {
            self.loop_memo.push((s, entry, st.clone()));
        }
    }

    /// The states a proven written invariant was decided on, for its query: on entry, and
    /// after a round of the body from the head of Houdini's last pass, each with the invariant
    /// evaluated there as it was then. Called by [`Fx::record`], which records nothing meanwhile.
    fn invariant_paths(&mut self, entry: &Octagon, back: &Octagon, c: Cand) -> Vec<Path> {
        [entry, back]
            .into_iter()
            .map(|from| {
                let mut s = from.clone();
                let k = self.cand(&mut s, c);
                (s, Goal::Holds(k))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use mente_syntax::diagnostics::apply_fix;
    use mente_syntax::ItemDecl;

    use super::*;
    use crate::check::Signature;
    use crate::check_source;

    fn module(src: &str) -> String {
        format!("module t\n{}\n", src)
    }

    /// Tier 1's report for each function of the module that has a body.
    fn reports(src: &str) -> Vec<Report> {
        let text = module(src);
        let m = match mente_syntax::parse_module(&text) {
            Ok(m) => m,
            Err(d) => panic!("{:?}", d),
        };
        let parsed = crate::parse_qualified(&text, &m);
        let parsed_structs = mente_syntax::parse_structs(&text, &m);
        let parsed_enums = mente_syntax::parse_enums(&text, &m);
        let (structs, _) = crate::structs::declare(&m, &parsed_structs, &parsed_enums);
        let routings: Vec<Routing> = m
            .items
            .iter()
            .filter_map(|i| match i {
                ItemDecl::Fn(f) => Some(Routing::of(&f.attrs)),
                ItemDecl::Unit(_)
                | ItemDecl::Struct(_)
                | ItemDecl::BitStruct(_)
                | ItemDecl::Enum(_)
                | ItemDecl::Trait(_)
                | ItemDecl::Example(_) => None,
            })
            .collect();
        let names: Vec<String> = parsed.iter().map(|p| p.name.clone()).collect();
        let signatures: Vec<Option<Signature>> = parsed
            .iter()
            .map(|p| p.syntax.as_ref().map(Signature::of))
            .collect();
        let callees: Vec<Option<Callee>> = parsed
            .iter()
            .map(|p| p.syntax.as_ref().map(|syntax| Callee { syntax }))
            .collect();
        let mut out = Vec::new();
        for (p, routing) in parsed.iter().zip(&routings) {
            let Some(syntax) = &p.syntax else {
                continue;
            };
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let (resolution, errors) = crate::names::resolve(syntax, &names, &structs);
            assert!(errors.is_empty(), "{:?}", errors);
            let (typing, errors) =
                crate::check::check(syntax, &resolution, &signatures, &structs, &text);
            assert!(errors.is_empty(), "{:?}", errors);
            let f = Function {
                syntax,
                resolution: &resolution,
                typing: &typing,
                routing: *routing,
                structs: &structs,
            };
            out.push(check(&f, &callees, &text));
        }
        out
    }

    /// The passes over loop bodies Tier 1 makes in the module's last function.
    fn passes(src: &str) -> usize {
        let text = module(src);
        let m = mente_syntax::parse_module(&text).unwrap();
        let parsed = mente_syntax::parse_bodies(&text, &m);
        let names: Vec<String> = parsed.iter().map(|p| p.name.clone()).collect();
        let signatures: Vec<Option<Signature>> = parsed
            .iter()
            .map(|p| p.syntax.as_ref().map(Signature::of))
            .collect();
        let callees: Vec<Option<Callee>> = parsed
            .iter()
            .map(|p| p.syntax.as_ref().map(|syntax| Callee { syntax }))
            .collect();
        let syntax = parsed.last().and_then(|p| p.syntax.as_ref()).unwrap();
        let structs = crate::structs::Structs::default();
        let (resolution, _) = crate::names::resolve(syntax, &names, &structs);
        let (typing, _) = crate::check::check(syntax, &resolution, &signatures, &structs, &text);
        let f = Function {
            syntax,
            resolution: &resolution,
            typing: &typing,
            routing: Routing::default(),
            structs: &structs,
        };
        let mut fx = Fx::new(&f, &callees, &text);
        fx.route();
        let mut st = Octagon::new(fx.count);
        fx.entry(&mut st);
        fx.body(&mut st);
        fx.passes
    }

    #[test]
    fn test_nested_loops_are_analysed_in_time_linear_in_their_nesting() {
        let nested = |depth: usize| {
            format!(
                "pub fn deep(x: i32) -> i32 {{\n{}{}  x\n}}",
                "  while x > 0 {\n".repeat(depth),
                "  }\n".repeat(depth)
            )
        };
        let (ten, twenty, forty) = (
            passes(&nested(10)),
            passes(&nested(20)),
            passes(&nested(40)),
        );
        // Each loop met again in the same state is not analysed again, so doubling the nesting
        // doubles the passes; before, every level doubled them.
        assert!(twenty <= 2 * ten + 4, "{} then {}", ten, twenty);
        assert!(forty <= 2 * twenty + 4, "{} then {}", twenty, forty);
        assert!(forty <= 8 * 40, "{}", forty);
        assert!(ten >= 10, "{}", ten);
        assert!(codes(&nested(40)).is_empty());
    }

    /// A function past [`RELATIONAL_LIMIT`](crate::octagon::RELATIONAL_LIMIT) variables is
    /// proven over each variable's bounds alone (Norman's decision, `tier1-variable-cap`): a
    /// relation between two variables proves nothing there, each one's bounds still do, and its
    /// cost no longer grows with the cube of its size.
    #[test]
    fn test_a_function_past_the_variable_limit_is_proven_over_bounds() {
        // Each nested call of `g` takes variables of its own: its argument and its result.
        let with = |calls: usize| {
            format!(
                "pub fn g(v: u8) -> u8 = v\npub fn r(n: u8) -> u8\n  decreases n\n{{\n  if n == 0 {{\n    return 0\n  }}\n  _ = {}1{}\n  r(n - 1)\n}}",
                "g(".repeat(calls),
                ")".repeat(calls)
            )
        };
        // That the argument `n - 1` is below `n` is a relation: an octagon proves the measure
        // smaller, and bounds alone do not.
        assert_eq!(codes(&with(10)), NONE);
        assert_eq!(codes(&with(70)), ["E0507"]);
        // Bounds alone still prove what bounds prove.
        assert_eq!(
            codes("pub fn g(v: u8) -> u8 = v\npub fn f(x: u8[0..10]) -> u8 {\n  _ = g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(g(1))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))))\n  x + 1\n}"),
            NONE
        );
        // Two hundred nested calls once took seconds; over bounds they take a moment.
        let start = std::time::Instant::now();
        let deep = with(200);
        assert_eq!(codes(&deep), ["E0507"]);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn test_a_loop_met_in_another_state_is_analysed_again() {
        // The outer loop's first round assumes `s <= 5`, its second does not: the inner loop is
        // met in two states, and the second must not reuse the first's exit, which would keep
        // `s < 100` and prove the result in range.
        let src = "pub fn f() -> u8[0..100] {\n  var i: u8 = 0\n  var s: u8 = 0\n  while s < 100 and i < 10\n    invariant s <= 5\n  {\n    var j: u8 = 0\n    while j < s {\n      j += 1\n    }\n    s = j + 1\n    i += 1\n  }\n  s\n}";
        assert_eq!(codes(src), ["E0506", "E0502"]);
        // Two loops met in the same state are two loops: the second, which raises `s` past the
        // invariant, must not reuse the first's exit, which leaves the state as it was.
        let siblings = "pub fn f() -> u8 {\n  var i: u8 = 0\n  var s: u8 = 0\n  while i < 10\n    invariant s <= 10\n  {\n    while i < 0 {\n    }\n    while s < 50 {\n      s += 1\n    }\n    i += 1\n  }\n  s\n}";
        assert_eq!(codes(siblings), ["E0506"]);
    }

    /// The report of the module's last function.
    fn report(src: &str) -> Report {
        reports(src).pop().unwrap_or_default()
    }

    fn diagnostics(src: &str) -> Vec<Diagnostic> {
        check_source(&module(src)).err().unwrap_or_default()
    }

    fn codes(src: &str) -> Vec<&'static str> {
        diagnostics(src).iter().map(|d| d.code).collect()
    }

    fn messages(src: &str) -> Vec<String> {
        diagnostics(src).into_iter().map(|d| d.message).collect()
    }

    /// The kind, `must` and `proven` of each obligation of the last function.
    fn obligations(src: &str) -> Vec<(ObligationKind, bool, bool)> {
        report(src)
            .obligations
            .iter()
            .map(|o| (o.kind, o.must, o.proven))
            .collect()
    }

    /// The module after the first fix of its first diagnostic.
    fn fixed(src: &str) -> String {
        let text = module(src);
        let d = diagnostics(src);
        assert!(!d.is_empty(), "no diagnostic in {}", text);
        assert_eq!(d[0].fixes[0].applicability, Applicability::Suggestion);
        apply_fix(&text, &d[0].fixes[0]).unwrap()
    }

    /// Applying the first fix of the first diagnostic leaves a module that checks.
    fn repairs(src: &str) -> String {
        let text = fixed(src);
        assert_eq!(check_source(&text).err(), None, "{}", text);
        text
    }

    /// The enum of the tests of `match`: a payload of a refined field.
    const READING: &str =
        "pub enum Reading {\n  Missing\n  Level(n: u8[0..10], m: u16)\n}\nfn next(mut i: u8) -> u8 {\n  i = i / 2 + 1\n  i\n}\n";

    /// A `match`'s integer scrutinee is narrowed into its type ([`match-scrutinee-narrowing`]):
    /// Tier 1 proves it fits, or reports `E0502` with a true valuation and a `guard` that makes
    /// the module check once applied, or `E0508` where it never fits; a place or a call's
    /// result fits by its type, and the arms read the value within the type.
    ///
    /// [`match-scrutinee-narrowing`]: ../../docs/design-review.md#match-scrutinee-narrowing
    #[test]
    fn test_a_scrutinee_is_narrowed_into_its_type() {
        let sum = "pub fn f(x: u8) -> u8 = match x + 1 {\n  0 => 1\n  _ => 2\n}";
        assert_eq!(
            messages(sum),
            ["Tier 1 cannot rule out `x = 255`, where `x + 1` is 256, outside `u8`, the type of what the `match` reads"]
        );
        repairs(sum);
        let neg = "pub fn f(x: u8) -> u8 {\n  val r: u8 = match -x {\n    0 => 1\n    _ => 2\n  }\n  r\n}";
        assert_eq!(
            messages(neg),
            ["Tier 1 cannot rule out `x = 1`, where `-x` is -1, outside `u8`, the type of what the `match` reads"]
        );
        repairs(neg);
        assert_eq!(
            codes("pub fn f(x: u8[1..=255]) -> u8 = match -x {\n  0 => 1\n  _ => 2\n}"),
            ["E0508"]
        );
        for src in [
            "pub fn f(x: u8) -> u8 = match x / 2 {\n  0 => 1\n  _ => 2\n}",
            "pub fn f(x: u8[0..10]) -> u8 = match x + 1 {\n  1 => 1\n  _ => 2\n}",
            "pub fn f(x: u8) -> u8 {\n  guard x < 255 else {\n    return 0\n  }\n  match x + 1 {\n    0 => 1\n    _ => 2\n  }\n}",
            "pub fn f(x: u128, y: i128) -> u8 = match x {\n  0 => match y {\n    0 => 1\n    _ => 2\n  }\n  _ => 3\n}",
            "fn g() -> u8 = 7\npub fn f() -> u8 = match g() {\n  7 => 1\n  _ => 2\n}",
        ] {
            assert!(codes(src).is_empty(), "{}: {:?}", src, messages(src));
        }
        // A signed type's lower bound is negative, and an unsigned type's guard reads 0.
        let low = "pub fn f(a: i8) -> u8 = match a - 1 {\n  0 => 1\n  _ => 2\n}";
        assert_eq!(
            messages(low),
            ["Tier 1 cannot rule out `a = -128`, where `a - 1` is -129, outside `i8`, the type of what the `match` reads"]
        );
        repairs(low);
        assert!(
            diagnostics(neg)[0].fixes[0]
                .message
                .contains("guard -x >= 0 else"),
            "{:?}",
            diagnostics(neg)[0].fixes
        );
        // A value of one 128-bit type is checked at its operator (`core-calculus.md`, item 4),
        // so it lies within its type already: nothing to prove, nothing recorded. A mixed one
        // of narrower operands is not checked there, and may leave the type that joins them.
        for src in [
            "pub fn f(x: i128, c: bool) -> u8 = match (if c { x } else { 0 }) {\n  0 => 1\n  _ => 2\n}",
            "pub fn f(x: i128) -> u8 = match x / 3 {\n  0 => 1\n  _ => 2\n}",
            "pub fn f(x: i128) -> u8 = match x - 1 {\n  0 => 1\n  _ => 2\n}",
            "pub fn f(x: u128) -> u8 = match x + 1 {\n  0 => 1\n  _ => 2\n}",
            "pub fn f(x: u128) -> u8 = match -x {\n  0 => 1\n  _ => 2\n}",
        ] {
            assert!(codes(src).is_empty(), "{}: {:?}", src, messages(src));
            assert!(
                obligations(src).iter().all(|o| o.0 != ObligationKind::Refinement),
                "{}",
                src
            );
        }
        assert_eq!(
            codes("pub fn f(b: u64, a: i8[0..5]) -> u8 = match b * b + a {\n  0 => 1\n  _ => 2\n}"),
            ["E0502"]
        );
        // A literal alone is an `i64`: one beyond it never fits.
        assert_eq!(
            codes("pub fn f() -> u8 = match 9223372036854775808 {\n  _ => 1\n}"),
            ["E0508"]
        );
        // A scrutinee Tier 1 proves fits, or a place, records no refinement to prove.
        for src in [
            "pub fn f(x: u8) -> u8 = match x / 2 {\n  0 => 1\n  _ => 2\n}",
            "pub fn f(x: u128) -> u8 = match x {\n  0 => 1\n  _ => 2\n}",
        ] {
            assert!(
                !obligations(src)
                    .iter()
                    .any(|o| o.0 == ObligationKind::Refinement),
                "{}: {:?}",
                src,
                obligations(src)
            );
        }
        // A name, an element or a call's result already lies within its type, where it was
        // stored or returned (`core-calculus.md`, item 4): nothing is proven at the `match`, so
        // no check is recorded there, and `mn view --checks` lists none. The element's index is
        // checked, as anywhere.
        for (src, kinds) in [
            (
                "pub fn f(x: u8) -> bool = match x {\n  0 => true\n  _ => false\n}",
                vec![],
            ),
            (
                "fn g() -> u8 = 7\npub fn f() -> bool = match g() {\n  7 => true\n  _ => false\n}",
                vec![],
            ),
            (
                "pub fn f(xs: [2]u8, i: u8[0..2]) -> bool = match xs[i] {\n  7 => true\n  _ => false\n}",
                vec![ObligationKind::Index],
            ),
        ] {
            assert_eq!(
                obligations(src).iter().map(|o| o.0).collect::<Vec<_>>(),
                kinds,
                "{}",
                src
            );
        }
        // Past the narrowing, an arm reads the value within the type, at either end, so only the
        // scrutinee is reported.
        for src in [
            "pub fn f(x: u8) -> u8[1..=255] = match x + 1 {\n  0 => 1\n  _ => x + 1\n}",
            "pub fn f(x: u8) -> u8[0..=254] = match x - 1 {\n  _ => x - 1\n}",
        ] {
            assert_eq!(codes(src), ["E0502"], "{}: {:?}", src, messages(src));
        }
        // Past the narrowing the value lies within the type: here `x + 1` is 1 to 255, so the
        // arm `_` gives a value of `u8[1..=255]`.
        let within = "pub fn f(x: u8) -> u8[1..=255] {\n  guard x < 255 else {\n    return 1\n  }\n  match x + 1 {\n    0 => 1\n    _ => x + 1\n  }\n}";
        assert!(codes(within).is_empty(), "{:?}", messages(within));
    }

    /// A `match` is control flow, its arms joined: a literal arm gives a scrutinee Tier 1
    /// follows its value; `_` the range the literals before it leave at either end; `true` and
    /// `false` decide a condition; and a payload's binding takes its field's refinement.
    #[test]
    fn test_match_arms_narrow_the_state() {
        let with = |f: &str| format!("{}{}", READING, f);
        // `_` leaves out the literals before it at each end of the range on its own, an end
        // beyond the octagon's reach included; and an arm no value reaches proves anything.
        for src in [
            "pub fn f(x: u128) -> u128[1..=340282366920938463463374607431768211455] = match x {\n  0 => 1\n  _ => x\n}",
            "fn take(x: i128) -> i128\n  requires x < 5\n= x\npub fn f(a: i128) -> i128 {\n  guard a <= 5 else {\n    return 0\n  }\n  match a {\n    5 => 0\n    _ => take(a)\n  }\n}",
            "pub fn f(x: u8) -> u8[0..5] {\n  guard x == 5 else {\n    return 0\n  }\n  match x {\n    5 => 1\n    _ => x\n  }\n}",
        ] {
            assert!(codes(src).is_empty(), "{}: {:?}", src, messages(src));
        }
        // An integer `match`'s value keeps its relation to each arm's; any other value is the
        // join of the arms' values.
        for src in [
            "pub fn f(x: u8, c: bool) -> u8[0..1] {\n  val k: u8 = match c {\n    true => x\n    false => x\n  }\n  x - k\n}",
            "pub fn f(c: bool) -> [2]u8[0..10] = match c {\n  true => [1, 2]\n  false => [3, 4]\n}",
        ] {
            assert!(codes(src).is_empty(), "{}: {:?}", src, messages(src));
        }
        for src in [
            "pub fn f(x: u8) -> u8[0..1] = match x {\n  0 => x\n  _ => 0\n}",
            "pub fn f(n: u8) -> u8[1..=255] = match n {\n  0 => 1\n  _ => n\n}",
            "pub fn f(n: i8) -> i8[-126..=125] = match n {\n  -128 => 0\n  -127 => 0\n  127 => 0\n  126 => 0\n  _ => n\n}",
            "pub fn f(x: u8) -> u8[0..10] = match x < 10 {\n  true => x\n  false => 0\n}",
            "pub fn f(r: Reading) -> u8[0..10] = match r {\n  Reading.Level(n: n, m: _) => n\n  Reading.Missing => 0\n}",
            "pub fn f(p: Reading, x: u8) -> u8[0..20] {\n  var y: u8 = 0\n  match p {\n    Reading.Level(n: n, m: _) => {\n      y = n + n\n    }\n    Reading.Missing => {}\n  }\n  y\n}",
            "pub fn f(x: u128) -> u128[0..=1] = match x {\n  340282366920938463463374607431768211455 => 0\n  _ => if x < 2 { x } else { 1 }\n}",
        ] {
            assert_eq!(codes(&with(src)), NONE, "{}", src);
        }
        // Without the fact the arm needs, each is an error.
        for src in [
            "pub fn f(x: u8) -> u8[0..1] = match x {\n  1 => x\n  _ => 0\n}",
            "pub fn f(n: u8) -> u8[1..=255] = match n {\n  1 => 1\n  _ => n\n}",
            "pub fn f(x: u8) -> u8[0..10] = match x < 11 {\n  true => x\n  false => 0\n}",
            "pub fn f(r: Reading) -> u8[0..5] = match r {\n  Reading.Level(n: n, m: _) => n\n  Reading.Missing => 0\n}",
            "pub fn f(x: u8) -> Reading = Reading.Level(n: x, m: 0)",
        ] {
            assert_eq!(codes(&with(src)), ["E0502"], "{}", src);
        }
        // A narrowing into a payload's field that always traps is an error too.
        assert_eq!(
            codes(&with("pub fn f() -> Reading = Reading.Level(n: 300, m: 0)")),
            ["E0508"]
        );
        // A `match` is a value as an `if` is: one that flows into a refined type is proven as a
        // whole, and offers no guard, since Tier 1 does not follow it.
        let src = with(
            "pub fn f(r: Reading) -> u8[0..5] = match r {\n  Reading.Level(n: n, m: _) => n\n  Reading.Missing => 0\n}",
        );
        assert!(diagnostics(&src)[0].fixes.is_empty());
        // An obligation inside an arm names the arm's binding in its valuation; its guard goes
        // inside the arm, made a block where it was an expression, and the module then checks.
        let src = with(
            "fn take(k: u8) -> u8\n  requires k < 5\n= k\npub fn f(r: Reading) -> u8 = match r {\n  Reading.Level(n: n, m: _) => take(n)\n  Reading.Missing => 0\n}",
        );
        assert_eq!(codes(&src), ["E0504"]);
        assert!(messages(&src)[0].contains("n = 5"), "{:?}", messages(&src));
        let repaired = repairs(&src);
        assert!(
            repaired.contains("Reading.Level(n: n, m: _) => {\n    guard n < 5 else { todo(\"n < 5 is false here\") }\n    take(n)\n  }"),
            "{}",
            repaired
        );
        let repaired = repairs(&with(
            "pub fn f(x: u8) -> Reading = match x {\n  0 => Reading.Missing\n  _ => Reading.Level(n: x, m: 1)\n}",
        ));
        assert!(repaired.contains("guard x < 10"), "{}", repaired);
        let repaired = repairs(&with(
            "pub fn f(r: Reading) -> u8[0..5] {\n  match r {\n    Reading.Level(n: n, m: _) => {\n      val k: u8[0..5] = n\n      k\n    }\n    Reading.Missing => 0\n  }\n}",
        ));
        assert!(repaired.contains("      guard n < 5"), "{}", repaired);
    }

    /// A `mut` call in a scrutinee, in an arm, and in a `match` inside a loop's condition
    /// changes the binding it is passed: what Tier 1 knew of it before holds no longer, and a
    /// loop forgets at its head what any of them changes.
    #[test]
    fn test_a_match_changes_what_its_mut_calls_pass() {
        let with = |f: &str| format!("{}{}", READING, f);
        for src in [
            // The scrutinee changes `j` before any arm runs.
            "pub fn f() -> u8[0..1] {\n  var j: u8 = 0\n  val k = match next(mut j) {\n    0 => 0\n    _ => j\n  }\n  k\n}",
            // An arm changes `j`, and the join keeps both states.
            "pub fn f(c: bool) -> u8[0..1] {\n  var j: u8 = 0\n  val k = match c {\n    true => next(mut j)\n    false => 0\n  }\n  j + k * 0\n}",
            // A `match` in a loop's condition changes `i` at every round's head.
            "pub fn f() -> u8[0..1] {\n  var i: u8 = 0\n  var r: u8 = 0\n  while (match next(mut i) {\n    1 => r < 3\n    _ => false\n  }) {\n    r += 1\n  }\n  i\n}",
            // A `match` statement in a loop's body binds afresh at every round.
            "pub fn f(rs: [3]Reading) -> u8[0..1] {\n  var i: usize = 0\n  var last: u8 = 0\n  while i < 3 {\n    match rs[i] {\n      Reading.Level(n: n, m: _) => {\n        last = n\n      }\n      Reading.Missing => {}\n    }\n    i += 1\n  }\n  last\n}",
        ] {
            assert_eq!(codes(&with(src)), ["E0502"], "{}", src);
        }
        // What the arms give is joined, whatever the call changed.
        assert_eq!(
            codes(&with(
                "pub fn f() -> u8[1..=2] {\n  var j: u8 = 0\n  val k = match next(mut j) {\n    0 => 1\n    _ => 2\n  }\n  k\n}"
            )),
            NONE
        );
    }

    use ObligationKind::*;

    const NONE: [&str; 0] = [];

    #[test]
    fn test_a_refined_value_must_be_proven_where_it_enters() {
        // A binding's initializer, an assignment, an element store, a returned value, an
        // expression body, an argument, an element of an array literal, a compound assignment.
        let cases = [
            ("pub fn f(x: u8) -> u8 {\n  val y: u8[0..10] = x\n  y\n}", "x < 10"),
            ("pub fn f(x: u8) -> u8 {\n  var y: u8[0..10] = 0\n  y = x\n  y\n}", "x < 10"),
            ("pub fn f(x: u8) -> [2]u8 {\n  var xs: [2]u8[0..5] = [0, 0]\n  xs[1] = x\n  xs\n}", "x < 5"),
            ("pub fn f(x: u8) -> u8[0..10] {\n  return x\n}", "x < 10"),
            ("pub fn f(x: u8) -> u8[0..10] = x", "x < 10"),
            ("pub fn g(i: u8[2..=9]) -> u8 = i\npub fn f(x: u8) -> u8 = g(x)", "x >= 2 and x <= 9"),
            ("pub fn f(x: u8) -> [2]u8[0..5] = [x, 0]", "x < 5"),
            ("pub fn f(x: u8) -> [2]u8[0..5] = [x; 2]", "x < 5"),
            ("pub fn f(x: u8) -> u8 {\n  var y: u8[0..10] = 0\n  y += x\n  y\n}", "y + x < 10"),
            ("pub fn f(x: u8, z: u8[1..3]) -> u8 {\n  var y: u8[0..10] = z\n  y *= x + 1\n  y\n}", "y * (x + 1) < 10"),
        ];
        for (src, guard) in cases {
            let d = diagnostics(src);
            assert_eq!(d.len(), 1, "{}: {:?}", src, d);
            assert_eq!(d[0].code, "E0502", "{}", src);
            assert!(
                d[0].fixes[0]
                    .message
                    .contains(&format!("guard {} else", guard)),
                "{}: {}",
                src,
                d[0].fixes[0].message
            );
            assert!(report(src)
                .obligations
                .iter()
                .any(|o| (o.kind, o.must, o.proven) == (Refinement, true, false)));
        }
        // Their near misses: a fact from a guard, a refined parameter, a literal, a condition.
        for src in [
            "pub fn f(x: u8) -> u8 {\n  guard x < 10 else {\n    return 0\n  }\n  val y: u8[0..10] = x\n  y\n}",
            "pub fn f(x: u8[0..10]) -> u8[0..=9] = x",
            "pub fn f() -> [2]u8[0..5] = [4, 0]",
            "pub fn f(x: u8) -> u8[0..10] = if x < 10 { x } else { 9 }",
            "pub fn g(i: u8[2..=9]) -> u8 = i\npub fn f(x: u8[3..5]) -> u8 = g(x)",
            "pub fn f(n: usize, i: usize[0..n]) -> usize[0..n] = i",
            "pub fn f(xs: [3]u8, i: usize[0..xs.len]) -> usize[0..=2] = i",
            "pub fn f(x: i8[-3..=-1]) -> i8[-3..0] = x",
            "pub fn g(n: usize, i: usize[0..n]) -> usize = i\npub fn f(k: usize[1..5]) -> usize = g(5, k)",
            "pub fn g(xs: [4]u8, i: usize[0..xs.len]) -> u8 = xs[i]\npub fn f(ys: [4]u8) -> u8 = g(ys, 3)",
        ] {
            assert_eq!(codes(src), NONE, "{}", src);
            assert!(
                report(src)
                    .obligations
                    .iter()
                    .filter(|o| o.must)
                    .all(|o| o.proven),
                "{}",
                src
            );
        }
        // A bound that names an argument takes the argument's text in the guard.
        let named = "pub fn g(n: usize, i: usize[0..n]) -> usize = i\npub fn f(k: usize, m: usize) -> usize = g(m + 1, k)";
        assert!(diagnostics(named)[0].fixes[0]
            .message
            .contains("guard k < m + 1 else"));
        let len = "pub fn g(xs: [4]u8, i: usize[0..xs.len]) -> u8 = xs[i]\npub fn f(ys: [4]u8, k: usize) -> u8 = g(ys, k)";
        assert!(diagnostics(len)[0].fixes[0]
            .message
            .contains("guard k < 4 else"));
    }

    /// A refinement whose bound lies beyond the octagon's reach, at a 128-bit type's limit, is
    /// checked exactly: every value Tier 1 bounds within it is proven, `0` and a literal at the
    /// bound included, and so is a value carrying the same refinement or narrowed by a `guard`;
    /// one it cannot bound draws `E0502` with the exact value it cannot rule out, and the guard
    /// that proves it.
    #[test]
    fn test_refinements_at_a_128_bit_limit_are_checked_exactly() {
        let u = "340282366920938463463374607431768211455";
        let take = format!("pub fn take(x: u128[0..{}]) -> u128 = x\n", u);
        let with = |f: &str| format!("{}{}", take, f);
        for f in [
            "pub fn f() -> u128 = take(0)".to_string(),
            "pub fn f(y: u64) -> u128 = take(y)".to_string(),
            "pub fn f() -> u128 = take(340282366920938463463374607431768211454)".to_string(),
            format!("pub fn f(y: u128[0..{}]) -> u128 = take(y)", u),
            format!("pub fn f(y: u128[0..{}]) -> u128[0..{}] = y", u, u),
            format!(
                "pub fn f(y: u128) -> u128 {{\n  guard y < {} else {{\n    return 0\n  }}\n  take(y)\n}}",
                u
            ),
            format!(
                "pub fn f(y: u128) -> u128 {{\n  guard {} > y else {{\n    return 0\n  }}\n  take(y + 0)\n}}",
                u
            ),
        ] {
            assert_eq!(codes(&with(&f)), NONE, "{}", f);
        }
        let d = diagnostics(&with("pub fn f(y: u128) -> u128 = take(y)"));
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            format!(
                "Tier 1 cannot rule out that `y` is {}, outside `u128[0..{}]`, the type of parameter `x` of `take`",
                u, u
            )
        );
        // Its guard is one Tier 1 can use: applied, the function checks.
        let src = format!("module m\n{}", with("pub fn f(y: u128) -> u128 = take(y)"));
        let fixed = mente_syntax::diagnostics::apply_fix(&src, &d[0].fixes[0]).unwrap();
        assert!(
            fixed.contains(&format!("guard y < {} else", u)),
            "{}",
            fixed
        );
        assert!(crate::check_source(&fixed).is_ok(), "{}", fixed);
        assert_eq!(
            messages(&with(&format!("pub fn f() -> u128 = take({})", u))),
            [format!(
                "Tier 1 cannot rule out that `{}` is {}, outside `u128[0..{}]`, the type of parameter `x` of `take`",
                u, u, u
            )]
        );
        // A binding of its own refinement names its value, and a sum names the nearest
        // value past the bound.
        let r = format!("pub fn f(y: u128) -> u128[0..{}] = y", u);
        assert_eq!(
            messages(&r),
            [format!(
                "Tier 1 cannot rule out `y = {}`, outside `u128[0..{}]`, the type of the result of `f`",
                u, u
            )]
        );
        // A value of two terms entering a refinement directly, a result here, is bounded by
        // its interval: a sum of two `i8`s lies within both far bounds, a sum of two `u128`s
        // may pass the upper one, and a negated sum the lower one.
        let both = "i128[-170141183460469231731687303715884105727..=170141183460469231731687303715884105726]";
        assert_eq!(
            codes(&format!("pub fn f(y: i8, z: i8) -> {} = y + z", both)),
            NONE
        );
        assert_eq!(
            codes(&format!(
                "pub fn f(y: u128, z: u128) -> u128[0..{}] = y + z",
                u
            )),
            ["E0502"]
        );
        assert_eq!(
            codes("pub fn f(y: u128, z: u128) -> i128[-170141183460469231731687303715884105727..=0] = -y - z"),
            ["E0502"]
        );
        assert_eq!(
            messages("pub fn f(y: u128, z: u128) -> i128[-170141183460469231731687303715884105727..=0] = -y - z"),
            ["Tier 1 cannot rule out that `-y - z` is -170141183460469231731687303715884105728, outside `i128[-170141183460469231731687303715884105727..=0]`, the type of the result of `f`"]
        );
        // A negative literal in a guard is exact too.
        assert_eq!(
            codes("pub fn take(x: i128[-170141183460469231731687303715884105727..=0]) -> i128 = x\npub fn f(y: i128) -> i128 {\n  guard y >= -170141183460469231731687303715884105727 and y <= 0 else {\n    return 0\n  }\n  take(y)\n}"),
            NONE
        );
        // So is a negated term compared with one.
        assert_eq!(
            codes("pub fn take(x: i128[-170141183460469231731687303715884105727..=0]) -> i128 = x\npub fn f(y: i128) -> i128 {\n  guard -y <= 170141183460469231731687303715884105727 and y <= 0 else {\n    return 0\n  }\n  take(y)\n}"),
            NONE
        );
        // A guard's equality with a far constant bounds its binding both ways, and a guard on
        // two terms that no value passes leaves what follows unreachable.
        let upper = format!("pub fn take(x: u128[0..{}]) -> u128 = x\n", u);
        assert_eq!(
            codes(&format!(
                "{}pub fn f(y: u128) -> u128 {{\n  guard y == 340282366920938463463374607431768211454 else {{\n    return 0\n  }}\n  take(y)\n}}",
                upper
            )),
            NONE
        );
        assert_eq!(
            codes("pub fn f(y: u8, z: u8) -> u8[0..1] {\n  guard y + z - 600 <= -170141183460469231731687303715884105727 else {\n    return 0\n  }\n  y\n}"),
            NONE
        );
        assert_eq!(
            codes("pub fn f(y: u8, z: u8) -> u8[0..1] {\n  guard y + z - 600 < -170141183460469231731687303715884105727 else {\n    return 0\n  }\n  y\n}"),
            NONE
        );
        // The i128 limits: its smallest value and one below its largest.
        let lo = "-170141183460469231731687303715884105728";
        let hi = "170141183460469231731687303715884105726";
        let take = format!("pub fn take(x: i128[{}..={}]) -> i128 = x\n", lo, hi);
        let with = |f: &str| format!("{}{}", take, f);
        for f in [
            "pub fn f() -> i128 = take(0)".to_string(),
            format!("pub fn f() -> i128 = take({})", lo),
            format!("pub fn f() -> i128 = take({})", hi),
            "pub fn f(y: i64) -> i128 = take(y)".to_string(),
            format!(
                "pub fn f(y: i128) -> i128 {{\n  guard y <= {} else {{\n    return 0\n  }}\n  take(y)\n}}",
                hi
            ),
        ] {
            assert_eq!(codes(&with(&f)), NONE, "{}", f);
        }
        assert_eq!(
            messages(&with("pub fn f(y: i128) -> i128 = take(y)")),
            [format!(
                "Tier 1 cannot rule out that `y` is 170141183460469231731687303715884105727, outside `i128[{}..={}]`, the type of parameter `x` of `take`",
                lo, hi
            )]
        );
        // A lower bound beyond reach: values below it are named exactly too.
        let take =
            "pub fn take(x: i128[-170141183460469231731687303715884105727..=0]) -> i128 = x\n";
        assert_eq!(
            messages(&format!("{}pub fn f(y: i128) -> i128 = take(y)", take)),
            ["Tier 1 cannot rule out that `y` is -170141183460469231731687303715884105728, outside `i128[-170141183460469231731687303715884105727..=0]`, the type of parameter `x` of `take`"]
        );
        // The value named is the nearest one outside the bound that the state allows, which a
        // guard may put past the bound's own nearest, on either side.
        assert_eq!(
            messages("pub fn take(x: u128[0..340282366920938463463374607431768211400]) -> u128 = x\npub fn f(y: u128) -> u128 {\n  guard y > 340282366920938463463374607431768211450 else {\n    return 0\n  }\n  take(y)\n}"),
            ["Tier 1 cannot rule out that `y` is 340282366920938463463374607431768211451, outside `u128[0..340282366920938463463374607431768211400]`, the type of parameter `x` of `take`"]
        );
        assert_eq!(
            messages("pub fn take(x: i128[-170141183460469231731687303715884105000..=0]) -> i128 = x\npub fn f(y: i128) -> i128 {\n  guard y < -170141183460469231731687303715884105700 else {\n    return 0\n  }\n  take(y)\n}"),
            ["Tier 1 cannot rule out that `y` is -170141183460469231731687303715884105701, outside `i128[-170141183460469231731687303715884105000..=0]`, the type of parameter `x` of `take`"]
        );
        // So it is for a sum, of the bounds its terms give it.
        assert_eq!(
            messages("pub fn f(y: u64, z: u64) -> i128[100000000000000000000000000000000000000..=170141183460469231731687303715884105727] = y + z"),
            ["Tier 1 cannot rule out that `y + z` is 36893488147419103230, outside `i128[100000000000000000000000000000000000000..=170141183460469231731687303715884105727]`, the type of the result of `f`"]
        );
        // An inclusive bound at a 128-bit type's end holds every value of the type: a value
        // nothing narrows is proven, at either end of a refinement, a sum of two too.
        let full = format!("pub fn take(x: u128[0..={}]) -> u128 = x\n", u);
        for f in [
            "pub fn f(y: u128) -> u128 = take(y)".to_string(),
            format!("pub fn f(y: u128[1..={}]) -> u128[1..={}] = y", u, u),
            format!("pub fn f(y: u128[{}..={}]) -> u128[{}..={}] = y", u, u, u, u),
            format!("pub fn f(y: u128, z: u128) -> u128[0..={}] = y + z", u),
            format!(
                "pub fn f(y: u128) -> u128 {{\n  guard y <= {} else {{\n    return 0\n  }}\n  take(y)\n}}",
                u
            ),
            format!("pub fn f(y: i128) -> i128[{}..=170141183460469231731687303715884105727] = y", lo),
        ] {
            assert_eq!(codes(&format!("{}{}", full, f)), NONE, "{}", f);
        }
        // A valuation names a value beyond the octagon's reach exactly, within every bound Tier 1
        // holds: above `i128::MAX`, below `-BIG`, and from a guard.
        assert_eq!(
            messages(&format!("pub fn f(x: u128[300000000000000000000000000000000000000..{}], y: u8) -> u8[0..1] = y", u)),
            ["Tier 1 cannot rule out `x = 300000000000000000000000000000000000000, y = 1`, outside `u8[0..1]`, the type of the result of `f`"]
        );
        assert_eq!(
            messages(&format!("pub fn f(x: i128[{}..=-100000000000000000000000000000000000000], y: u8) -> u8[0..1] = y", lo)),
            ["Tier 1 cannot rule out `x = -100000000000000000000000000000000000000, y = 1`, outside `u8[0..1]`, the type of the result of `f`"]
        );
        assert_eq!(
            messages("pub fn f(x: u128, y: u8) -> u8[0..1] {\n  guard x > 300000000000000000000000000000000000000 else {\n    return 0\n  }\n  y\n}"),
            ["Tier 1 cannot rule out `x = 300000000000000000000000000000000000001, y = 1`, outside `u8[0..1]`, the type of the result of `f`"]
        );
        assert_eq!(
            messages("pub fn f(x: u8) -> u8[1..2] = x / 42535295865117307932921825928971026432"),
            ["Tier 1 cannot rule out that `x / 42535295865117307932921825928971026432` is 0, outside `u8[1..2]`, the type of the result of `f`"]
        );
    }

    #[test]
    fn test_a_failed_refinement_names_a_valuation_and_a_guard() {
        let src = "pub fn next(x: u8) -> u8[1..=255] {\n  return x + 1\n}";
        let d = &diagnostics(src)[0];
        assert_eq!(
            d.message,
            "Tier 1 cannot rule out `x = 255`, where `x + 1` is 256, outside `u8[1..=255]`, the type of the result of `next`"
        );
        assert_eq!((d.primary.line, d.primary.col), (3, 10));
        assert_eq!(
            d.fixes[0].message,
            "insert `guard x + 1 <= 255 else { todo(\"x + 1 <= 255 is false here\") }`"
        );
        assert_eq!(d.fixes[0].applicability, Applicability::Suggestion);
        let edit = &d.fixes[0].edits[0];
        assert_eq!((edit.span.line, edit.span.col), (3, 1));
        assert_eq!(edit.span.start, edit.span.end);
        let text = repairs(src);
        assert!(
            text.contains("{\n  guard x + 1 <= 255 else { todo(\"x + 1 <= 255 is false here\") }\n  return x + 1\n}"),
            "{}",
            text
        );
        // A value named alone needs no `where`; one Tier 1 does not follow exactly gets no
        // valuation of the bindings, only a value it cannot rule out.
        assert_eq!(
            messages("pub fn f(x: u8) -> u8[0..10] = x"),
            ["Tier 1 cannot rule out `x = 10`, outside `u8[0..10]`, the type of the result of `f`"]
        );
        assert_eq!(
            messages("pub fn f(a: u8, b: u8) -> u16[0..100] = a * b"),
            ["Tier 1 cannot rule out that `a * b` is 100, outside `u16[0..100]`, the type of the result of `f`"]
        );
        assert_eq!(
            messages("pub fn f(x: i8) -> i8[-5..5] = x"),
            ["Tier 1 cannot rule out `x = -6`, outside `i8[-5..5]`, the type of the result of `f`"]
        );
        assert_eq!(
            messages("pub fn f() -> u8[1..5] = 0"),
            ["Tier 1 cannot rule out that `0` is 0, outside `u8[1..5]`, the type of the result of `f`"]
        );
        // Both sides failing put both in the guard.
        assert_eq!(
            diagnostics("pub fn f(x: i8) -> i8[-5..5] = x")[0].fixes[0].message,
            "insert `guard x >= -5 and x < 5 else { todo(\"x >= -5 and x < 5 is false here\") }`"
        );
        // A `guard` that repeats an `if` would give its value a variable of its own, and tell
        // Tier 1 nothing of the one it checks, so none is offered; the message says how to write
        // one that works, and that one checks.
        let d = &diagnostics("pub fn f(x: u8, c: bool) -> u8[0..5] = if c { x } else { 4 }")[0];
        assert!(d.fixes.is_empty(), "{:?}", d.fixes);
        assert!(d
            .message
            .ends_with("so none is offered: bind its value to a `val`, and `guard` that"));
        assert_eq!(
            codes("pub fn f(x: u8, c: bool) -> u8[0..5] {\n  val v = if c { x } else { 4 }\n  guard v < 5 else { todo(\"v\") }\n  v\n}"),
            NONE
        );
        assert_eq!(
            codes("pub fn f(x: u8, c: bool) -> u8[0..5] {\n  guard (if c { x } else { 4 }) < 5 else { todo(\"v\") }\n  if c { x } else { 4 }\n}"),
            ["E0502"]
        );
        assert!(
            diagnostics("pub fn f(x: u8, y: u8) -> u8[0..5] = x & y")[0].fixes[0]
                .message
                .contains("guard x & y < 5 else")
        );
    }

    #[test]
    fn test_requires_are_proven_at_calls_and_ensures_assumed() {
        let pick = "pub fn pick(lo: usize[0..64], hi: usize[0..=64]) -> usize[0..=64]\n  requires lo < hi\n  ensures lo <= result and result < hi\n{\n  return lo\n}\n";
        assert!(reports(pick).iter().all(|r| r.diagnostics.is_empty()));
        // The callee's `ensures` proves the index in bounds; its body is never read.
        let use_it = format!("{}pub fn use_it(a: [64]i32, n: usize[0..=64]) -> i32 {{\n  guard n > 0 else {{\n    return 0\n  }}\n  val k: usize = pick(0, n)\n  return a[k]\n}}", pick);
        assert_eq!(
            obligations(&use_it),
            [
                (Range, false, true),
                (Range, false, true),
                (Range, false, true),
                (Refinement, true, true),
                (Refinement, true, true),
                (Requires, true, true),
                (Range, false, true),
                (Index, false, true),
                (Range, false, true),
            ]
        );
        let bad = format!("{}pub fn bad(n: usize[0..=64]) -> usize = pick(0, n)", pick);
        let d = &diagnostics(&bad)[0];
        assert_eq!(d.code, "E0504");
        assert_eq!(
            d.message,
            "Tier 1 cannot rule out `n = 0`, which violates `requires lo < hi` of `pick` at this call"
        );
        assert_eq!(
            d.fixes[0].message,
            "insert `guard 0 < n else { todo(\"0 < n is false here\") }`"
        );
        repairs(&bad);
        // With no binding in scope, the valuation names the callee's parameters.
        let nothing = format!("{}pub fn nothing() -> usize = pick(5, 5)", pick);
        assert_eq!(
            messages(&nothing),
            ["Tier 1 cannot rule out `lo = 5, hi = 5`, which violates `requires lo < hi` of `pick` at this call"]
        );
        let empty = "pub fn never()\n  requires 1 > 2\n{\n}\npub fn f() {\n  never()\n}";
        assert_eq!(
            messages(empty),
            ["Tier 1 cannot rule out that this call violates `requires 1 > 2` of `never`"]
        );
        // An argument is substituted with parentheses where it needs them.
        let sub = "pub fn g(a: i32, b: i32)\n  requires a - b > 0\n{\n}\npub fn f(x: i32, y: i32) {\n  g(x, y - 1)\n}";
        assert_eq!(
            diagnostics(sub)[0].fixes[0].message,
            "insert `guard x - (y - 1) > 0 else { return }`"
        );
        // A result refinement that names a parameter is a fact about the call.
        let named = "pub fn below(n: usize) -> usize[0..n] = todo(\"later\")\npub fn f(xs: [8]u8) -> u8 = xs[below(8)]";
        assert!(report(named).obligations.iter().all(|o| o.proven));
    }

    #[test]
    fn test_ensures_are_proven_at_every_return() {
        let inc = "pub fn inc(x: u8) -> u8\n  ensures result > x\n{\n  if x == 9 {\n    return x\n  }\n  x + 1\n}";
        let d = diagnostics(inc);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0505");
        assert_eq!(
            d[0].message,
            "Tier 1 cannot rule out `x = 9, result = 9`, which violates `ensures result > x` of `inc` at this return"
        );
        assert_eq!(d[0].primary.line, 6);
        assert_eq!(
            d[0].fixes[0].message,
            "insert `guard x > x else { todo(\"x > x is false here\") }`"
        );
        assert_eq!(
            obligations(inc),
            [
                (Range, false, true),
                (Ensures, true, false),
                (Range, false, false),
                (Ensures, true, true)
            ]
        );
        // The end of the body and an expression body are returns too.
        let tail = "pub fn f(x: u8) -> u8\n  ensures result > 3\n{\n  x\n}";
        assert_eq!(
            messages(tail),
            ["Tier 1 cannot rule out `x = 0, result = 0`, which violates `ensures result > 3` of `f` at its end"]
        );
        assert!(
            fixed(tail).contains("{\n  guard x > 3 else { todo(\"x > 3 is false here\") }\n  x\n}")
        );
        let expr = "pub fn f(x: u8) -> u8\n  ensures result > 3\n= x";
        assert_eq!(codes(expr), ["E0505"]);
        let unit = "pub fn f(mut x: u8)\n  ensures x > 3\n{\n  x = 1\n}";
        assert_eq!(
            messages(unit),
            ["Tier 1 cannot rule out `x = 1`, which violates `ensures x > 3` of `f` at its end"]
        );
        assert_eq!(
            fixed(unit),
            module("pub fn f(mut x: u8)\n  ensures x > 3\n{\n  x = 1\n  guard x > 3 else { return }\n}")
        );
        let no_param = "pub fn f() -> u8\n  ensures result > 3\n= 2";
        assert_eq!(
            messages(no_param),
            ["Tier 1 cannot rule out `result = 2`, which violates `ensures result > 3` of `f` at its end"]
        );
        let unit_empty = "pub fn f()\n  ensures 1 > 2\n{\n}";
        assert_eq!(
            messages(unit_empty),
            ["Tier 1 cannot rule out that `ensures 1 > 2` of `f` at its end fails"]
        );
        assert_eq!(
            fixed(unit_empty),
            module("pub fn f()\n  ensures 1 > 2\n{\n  guard 1 > 2 else { return }\n}")
        );
        for ok in [
            "pub fn f(x: u8) -> u8\n  ensures result >= x\n{\n  if x == 9 {\n    return x\n  }\n  x\n}",
            "pub fn f(x: u8[0..9]) -> u8\n  ensures result > x\n= x + 1",
            "pub fn f(mut x: u8)\n  ensures x == 4\n{\n  x = 4\n}",
            "pub fn f(x: u8) -> u8[0..5]\n  ensures result < 5\n= if x < 5 { x } else { 0 }",
        ] {
            assert_eq!(codes(ok), NONE, "{}", ok);
        }
    }

    #[test]
    fn test_written_invariants_hold_on_entry_and_are_preserved() {
        let preserved = "pub fn count(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n\n    invariant i <= 5\n  {\n    i += 1\n  }\n  i\n}";
        let d = diagnostics(preserved);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0506");
        assert_eq!(
            d[0].message,
            "Tier 1 cannot rule out `n = 6, i = 6`, after a round of this loop's body, which violates `invariant i <= 5`: the body does not preserve it"
        );
        assert_eq!((d[0].primary.line, d[0].primary.col), (5, 15));
        assert_eq!(
            repairs(preserved),
            module("pub fn count(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n\n    invariant i <= 5\n  {\n    i += 1\n    guard i <= 5 else { todo(\"i <= 5 is false here\") }\n  }\n  i\n}")
        );
        let entry = "pub fn f(n: u8) -> u8 {\n  var i: u8 = 7\n  while i < n invariant i <= 5 {\n    i += 1\n  }\n  i\n}";
        assert_eq!(
            messages(entry),
            ["Tier 1 cannot rule out `n = 0, i = 7`, on entry to this loop, which violates `invariant i <= 5`"]
        );
        assert!(fixed(entry).contains(
            "  var i: u8 = 7\n  guard i <= 5 else { todo(\"i <= 5 is false here\") }\n  while"
        ));
        assert_eq!(
            obligations(entry)
                .into_iter()
                .filter(|o| o.0 == Invariant)
                .collect::<Vec<_>>(),
            [(Invariant, true, false)]
        );
        let ok = "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n invariant i <= n {\n    i += 1\n  }\n  i\n}";
        assert_eq!(codes(ok), NONE);
        assert!(obligations(ok).contains(&(Invariant, true, true)));
        // Each conjunct is a candidate and an obligation of its own.
        let both = "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n invariant i <= n and i <= 3 {\n    i += 1\n  }\n  i\n}";
        assert_eq!(codes(both), ["E0506"]);
        assert!(messages(both)[0].contains("`invariant i <= 3`"));
        let none =
            "pub fn f() {\n  var i: u8 = 0\n  while i < 9 invariant 1 > 2 {\n    i += 1\n  }\n}";
        assert_eq!(
            messages(none),
            ["Tier 1 cannot rule out `i = 0`, on entry to this loop, which violates `invariant 1 > 2`"]
        );
        let nothing = "pub fn f(c: bool) {\n  while c invariant 1 > 2 {\n  }\n}";
        assert_eq!(
            messages(nothing),
            ["Tier 1 cannot rule out that this loop is entered with `invariant 1 > 2` false"]
        );
        let kept_false =
            "pub fn f(c: bool) {\n  var i: u8 = 0\n  while c invariant i == 0 {\n    i = 1\n  }\n}";
        assert_eq!(
            messages(kept_false),
            ["Tier 1 cannot rule out `i = 1`, after a round of this loop's body, which violates `invariant i == 0`: the body does not preserve it"]
        );
        let not_kept =
            "pub fn f() {\n  var i: u8 = 0\n  while i < 9 invariant i < 9 {\n    i += 1\n  }\n}";
        assert_eq!(
            messages(not_kept),
            ["Tier 1 cannot rule out `i = 9`, after a round of this loop's body, which violates `invariant i < 9`: the body does not preserve it"]
        );
        let every = "pub fn f() {\n  var i: u8 = 0\n  var j: u8 = 0\n  while i < 9 invariant j == 0 {\n    i += 1\n    j = 1\n  }\n}";
        assert!(messages(every)[0].contains("after a round"));
    }

    #[test]
    fn test_houdini_keeps_only_inductive_candidates() {
        // `i < hi`, the condition, is not inductive and is dropped; its non-strict form
        // `i <= hi` is, so after the loop `i <= hi` holds without being written.
        let src = "pub fn f(lo: usize[0..64], hi: usize[0..=64]) -> usize[0..=64]\n  requires lo < hi\n{\n  var i: usize = lo\n  while i < hi invariant lo <= i {\n    i += 1\n  }\n  return i\n}";
        assert_eq!(codes(src), NONE);
        // A non-strict condition proposes nothing weaker, and `i + 1 <= hi` is not inductive.
        let unbounded = src.replace("while i < hi", "while i + 1 <= hi");
        assert_eq!(codes(&unbounded), ["E0502"]);
        let greater = "pub fn f(n: u8) -> u8\n  ensures result == n\n{\n  var i: u8 = 255\n  while i > n {\n    i -= 1\n  }\n  i\n}";
        assert_eq!(codes(greater), NONE);
        let lower = "pub fn f(n: u8[0..100]) -> u8[0..100] {\n  var i: u8 = 0\n  while n > i {\n    i += 1\n  }\n  i\n}";
        assert_eq!(codes(lower), NONE);
        // A guard in the body is a candidate, from inside a guard's `else` or an `if` too.
        for body in [
            "    guard i <= 50 else {\n      return 0\n    }",
            "    guard c else {\n      guard i <= 50 else {\n        return 0\n      }\n      continue\n    }\n    return 0",
            "    if c {\n      guard i <= 50 else {\n        return 0\n      }\n    } else {\n      return 0\n    }",
            "    if c {\n      return 0\n    } else {\n      guard i <= 50 else {\n        return 0\n      }\n    }",
            "    if c {\n      return 0\n    } else if n > 3 {\n      guard i <= 50 else {\n        return 0\n      }\n    } else {\n      return 0\n    }",
        ] {
            let src = format!(
                "pub fn f(n: u8, c: bool) -> u8[0..=50] {{\n  var i: u8 = 0\n  while i < n {{\n    i += 1\n{}\n  }}\n  i\n}}",
                body
            );
            assert_eq!(codes(&src), NONE, "{}", body);
            let unguarded = src.replace("i <= 50", "i <= 60");
            assert_eq!(codes(&unguarded), ["E0502"], "{}", body);
        }
        // A guard of an inner loop is that loop's candidate, not the outer one's.
        let inner = "pub fn f(n: u8, c: bool) -> u8[0..=50] {\n  var i: u8 = 0\n  while i < n {\n    i += 1\n    while c {\n      guard i <= 50 else {\n        return 0\n      }\n    }\n  }\n  i\n}";
        assert_eq!(codes(inner), ["E0502"]);
        // A binding the loop never changes keeps its facts; one it changes keeps its type.
        let kept = "pub fn f(n: u8) -> u8[0..10] {\n  var k: u8 = 3\n  k = 4\n  var i: u8 = 0\n  while i < n {\n    i += 1\n  }\n  k\n}";
        assert_eq!(codes(kept), NONE);
        for changes in [
            "    k = i",
            "    k += 1",
            "    _ = set(mut k)",
            "    while k < n {\n      k += 1\n    }",
            "    while set(mut k) {\n      return 0\n    }",
            "    guard not set(mut k) else {\n      return 0\n    }",
            "    guard set(mut k) and true else {\n      return 0\n    }",
            "    guard n > 2 else {\n      k = 99\n      break\n    }",
            "    if set(mut k) {\n      return 0\n    }",
            "    if n > 2 {\n      k = 0\n    } else if n > 1 {\n      k = 99\n    }",
            "    if n > 2 {\n      k = 0\n    } else {\n      k = 99\n    }",
            "    _ = [1, 2][id(set(mut k))]",
            "    _ = [set(mut k), true]",
            "    _ = [set(mut k)].len",
            "    _ = [set(mut k); 2]",
            "    _ = if set(mut k) { 1 } else { 2 }",
            "    val m = id(set(mut k))\n    _ = m",
            "    i = i + 0 * id(set(mut k))",
            "    var xs: [2]u8 = [0, 0]\n    xs[id(set(mut k))] = 1\n    _ = xs",
        ] {
            let src = format!(
                "pub fn set(mut x: u8) -> bool {{\n  x = 200\n  true\n}}\npub fn id(b: bool) -> u8 = if b {{ 1 }} else {{ 0 }}\npub fn f(n: u8) -> u8[0..10] {{\n  var k: u8 = 3\n  var i: u8 = 0\n  while i < n {{\n    i += 1\n{}\n  }}\n  k\n}}",
                changes
            );
            assert_eq!(codes(&src), ["E0502"], "{}", changes);
        }
    }

    #[test]
    fn test_break_is_one_more_exit_of_its_loop() {
        let src = "pub fn f(n: usize[0..=10]) -> usize[0..=10] {\n  var i: usize = 0\n  while i < n invariant i <= n {\n    guard i != 5 else {\n      break\n    }\n    i += 1\n  }\n  return i\n}";
        assert_eq!(codes(src), NONE);
        // The state a `break` leaves with reaches the code after the loop.
        let broken = "pub fn f(n: u8) -> u8[0..5] {\n  var i: u8 = 0\n  while i < 3 {\n    if n > 7 {\n      i = n\n      break\n    }\n    i += 1\n  }\n  i\n}";
        assert_eq!(codes(broken), ["E0502"]);
        // `continue` goes back to the head, where its state must keep the invariant.
        let skipped = "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < 9 invariant i <= 9 {\n    i += 1\n    if n > 2 {\n      i = 20\n      continue\n    }\n  }\n  i\n}";
        assert_eq!(codes(skipped), ["E0506"]);
        let infinite =
            "pub fn f() -> u8[0..3] {\n  var i: u8 = 0\n  while true {\n    i = 1 - i\n  }\n  i\n}";
        assert_eq!(codes(infinite), NONE);
    }

    #[test]
    fn test_remainder_and_division_by_a_constant_have_their_facts() {
        for src in [
            "pub fn f(x: u32, y: u32[1..100]) -> u32[0..100] = x % y",
            // A zero divisor traps, so where x % y runs it lies strictly between -y and y.
            "pub fn f(x: u32, y: u32[0..100]) -> u32[0..100] = x % y",
            "pub fn f(x: u8, y: u8) -> u8 {\n  val r: u8[0..y] = x % y\n  r\n}",
            "pub fn f(x: i8, y: i8) -> i16 {\n  guard y > 0 else {\n    return 0\n  }\n  val r = x % y\n  val s: i16[0..=300] = r + y - 1\n  s\n}",
            "pub fn f(x: i8, y: i8[-100..=0]) -> i8 {\n  val r: i8[y..=100] = x % y\n  r\n}",
            "pub fn f(x: i8, y: i8) -> i16 {\n  guard y < 0 else {\n    return 0\n  }\n  val r = x % y\n  val s: i16[-300..0] = r + y\n  s\n}",
            "pub fn f(x: u32, y: u32) -> u32 {\n  guard y > 0 else {\n    return 0\n  }\n  val r: u32[0..y] = x % y\n  r\n}",
            "pub fn f(x: u8[0..20], y: u8[1..=200]) -> u8[0..=20] = x % y",
            "pub fn f(x: u8, y: u8) -> u8 {\n  guard x < 20 else {\n    return 0\n  }\n  val r: u8[0..=x] = x % 7\n  r\n}",
            "pub fn f(x: i8[-20..=0], y: i8[1..=100]) -> i8[-20..=0] = x % y",
            "pub fn f(x: i8, y: i8) -> i8 {\n  guard x <= 0 and x > -20 else {\n    return 0\n  }\n  val r: i8[x..=0] = x % 7\n  r\n}",
            "pub fn f(x: i8[-20..=0], y: i8[-100..=-1]) -> i8[-99..=0] = x % y",
            "pub fn f(x: i8, y: i8[-100..=-1]) -> i8[-99..=99] = x % y",
            "pub fn f(x: i8, y: i8) -> i8 {\n  guard y < 0 else {\n    return 0\n  }\n  val r: i8[y..=127] = x % y\n  r\n}",
            "pub fn f(lo: usize, hi: usize) -> usize\n  requires lo < hi\n  ensures lo <= result and result < hi\n= lo + (hi - lo) / 2",
            "pub fn f(lo: usize, hi: usize) -> usize\n  requires lo < hi\n  ensures lo < result and result <= hi\n= hi - (hi - lo) / 2",
            "pub fn f(x: u8) -> u8[0..=85] = x / 3",
            "pub fn f(x: i8) -> i8[-42..=42] = x / -3",
            "pub fn f(x: u8) -> u8[0..=128] = (x + 1) / 2",
            "pub fn f(x: u8, y: u8[2..=4]) -> u8[0..=127] = x / y",
            "pub fn f(x: u8, y: i8[-4..=-2]) -> i16[-127..=0] = x / y",
            "pub fn f(x: u8, y: i8[-4..=4]) -> i16[-255..=255] = x / y",
            "pub fn f(x: u8) -> u8[0..=255] = x / 1",
            "pub fn f(x: u8, y: u8[1..4]) -> u8[0..=255] = (x * 2) / 4 + 0 * y",
            "pub fn f(lo: u8, hi: u8) -> u8\n  requires lo < hi\n  ensures result <= hi + 1\n= lo + 1 + (hi + 2 - lo) / 2",
        ] {
            assert_eq!(codes(src), NONE, "{}", src);
        }
        for src in [
            "pub fn f(x: i8, y: i8[1..=100]) -> i8[0..100] = x % y",
            "pub fn f(x: i8, y: i8[1..=100]) -> i8[-98..100] = x % y",
            "pub fn f(lo: usize, hi: usize) -> usize\n  requires lo < hi\n  ensures result < hi\n= (lo + hi) / 2",
            "pub fn f(lo: usize, hi: usize) -> usize\n  requires lo < hi\n  ensures lo < result\n= lo + (hi - lo) / 2",
            "pub fn f(x: u8) -> u8[0..=84] = x / 3",
            "pub fn f(lo: u8, hi: u8) -> u8\n  requires lo < hi\n  ensures result <= hi\n= lo + 1 + (hi + 2 - lo) / 2",
            "pub fn f(x: i8) -> i8[-41..=42] = x / -3",
            "pub fn f(x: u8, y: u8[2..=4]) -> u8[0..=126] = x / y",
            "pub fn f(x: u8, y: u8) -> u8[0..=254] = x / y",
            "pub fn f(x: u8, y: i8[-4..=4]) -> i16[-254..=255] = x / y",
            "pub fn f(x: u8) -> u8[0..=254] = x / 0",
        ] {
            assert_ne!(codes(src), NONE, "{}", src);
        }
    }

    #[test]
    fn test_plain_narrowings_indexes_divisors_and_shifts_stay_run_time_checks() {
        // Recorded, proven or not, and never an error.
        let add = "pub fn f(a: i32, b: i32) -> i32 = a + b";
        assert_eq!(obligations(add), [(Range, false, false)]);
        assert_eq!(codes(add), NONE);
        assert_eq!(
            obligations("pub fn f(a: i8, b: i8) -> i32 = a + b"),
            [(Range, false, true)]
        );
        // A value that may be negative is never proven to fit `u128`, whose range starts at 0
        // though its top lies past the domain's bound; one that is never negative is.
        for src in [
            "pub fn f(x: i8) -> u128 = x",
            "pub fn f(p: i32) -> u128 = 4 * p",
            "pub fn f(p: i32) -> u128 = p + 1",
        ] {
            assert_eq!(obligations(src), [(Range, false, false)], "{}", src);
        }
        assert_eq!(
            obligations("pub fn f(x: u64) -> u128 = x"),
            [(Range, false, true)]
        );
        assert_eq!(
            obligations("pub fn f(x: i8[0..=5]) -> u128 = x"),
            [(Range, false, true)]
        );
        // An array argument narrows each element into the parameter's element type.
        assert_eq!(
            obligations("pub fn g(xs: [2]u8) -> u8 = xs[0]\npub fn f(ys: [2]u8) -> u8 = g(ys)"),
            [(Range, false, true), (Range, false, true)]
        );
        assert_eq!(
            obligations("pub fn g(xs: [2]u8) -> u8 = xs[0]\npub fn f(ys: [2]u16) -> u8 = g(ys)"),
            [(Range, false, false), (Range, false, true)]
        );
        let index = "pub fn f(xs: [4]u8, i: usize) -> u8 = xs[i]";
        assert_eq!(
            obligations(index),
            [(Index, false, false), (Range, false, true)]
        );
        assert_eq!(
            obligations("pub fn f(xs: [4]u8, i: usize[0..4]) -> u8 = xs[i]"),
            [(Index, false, true), (Range, false, true)]
        );
        assert_eq!(
            obligations("pub fn f(xs: [4]u8, i: usize[0..=4]) -> u8 = xs[i]")[0],
            (Index, false, false)
        );
        assert_eq!(
            obligations("pub fn f(xs: [4]u8) -> u8 = xs[-1]")[0],
            (Index, false, false)
        );
        assert_eq!(
            obligations("pub fn f(x: u8, y: u8) -> u8 = x / y")[0],
            (Divisor, false, false)
        );
        assert_eq!(
            obligations("pub fn f(x: i8, y: i8[-5..=-1]) -> i8 = x % y")[0],
            (Divisor, false, true)
        );
        assert_eq!(
            obligations("pub fn f(x: i8, y: i8[-5..=0]) -> i8 = x % y")[0],
            (Divisor, false, false)
        );
        assert_eq!(
            obligations("pub fn f(x: u8, y: u8[1..5]) -> u8 = x / y")[0],
            (Divisor, false, true)
        );
        // A shift amount within the width, and the shifted value narrowed into its type.
        assert_eq!(
            obligations("pub fn f(x: u8, k: u8) -> u8 = x << k"),
            [
                (Shift, false, false),
                (Range, false, false),
                (Range, false, true)
            ]
        );
        assert_eq!(
            obligations("pub fn f(x: u8, k: u8[0..8]) -> u8 = x >> k"),
            [
                (Shift, false, true),
                (Range, false, true),
                (Range, false, true)
            ]
        );
        assert_eq!(
            obligations("pub fn f(x: u8, k: u8[0..=8]) -> u8 = x >> k")[0],
            (Shift, false, false)
        );
        assert_eq!(
            obligations("pub fn f(x: u8, k: i8[-1..=3]) -> u8 = x >> k")[0],
            (Shift, false, false)
        );
        assert_eq!(
            obligations("pub fn f(k: u8[0..8]) -> i64 = 1 << k"),
            [
                (Shift, false, false),
                (Range, false, false),
                (Range, false, false)
            ]
        );
        assert_eq!(
            obligations("pub fn f(x: u8[0..16]) -> u8 = x << 4"),
            [
                (Shift, false, true),
                (Range, false, true),
                (Range, false, true)
            ]
        );
        // The operands of a bit operation narrow into their one type.
        assert_eq!(
            obligations("pub fn f(x: u8) -> u8 = x & 15"),
            [
                (Range, false, true),
                (Range, false, true),
                (Range, false, true)
            ]
        );
        // 128-bit operands narrow at each operator, and their range lies beyond the domain.
        assert_eq!(
            obligations("pub fn f(a: u128, b: u128) -> u128 = a + b"),
            [(Range, false, false), (Range, false, false)]
        );
        assert_eq!(
            obligations("pub fn f(a: u128[0..9], b: u128[0..9]) -> u128 = a + b"),
            [(Range, false, true), (Range, false, true)]
        );
        assert_eq!(
            obligations("pub fn f(a: i128[-9..9], b: i128[-9..0]) -> i128 = a * b"),
            [(Range, false, true), (Range, false, true)]
        );
        // An end Tier 1 does not bound is not proven, the other end bounded or not.
        assert_eq!(
            obligations(
                "pub fn f(a: i128) -> i128 {
  guard a < 5 else {
    return 0
  }
  a + 1
}"
            )[1],
            (Range, false, false)
        );
        assert_eq!(
            obligations(
                "pub fn f(a: i128) -> i128 {
  guard a > -5 else {
    return 0
  }
  a + 1
}"
            )[1],
            (Range, false, false)
        );
        // A quotient by a constant is bounded through the sum it is taken from too.
        for src in [
            "pub fn f(lo: u8, hi: u8) -> u8 {
  guard lo < hi else {
    return 0
  }
  lo + (hi - lo) / 2
}",
            "pub fn f(lo: u8, hi: u8) -> u8 {
  guard lo < hi else {
    return 0
  }
  hi - (hi - lo) / 2 - 1
}",
        ] {
            assert!(report(src).obligations.iter().all(|o| o.proven), "{}", src);
        }
        // Where no path reaches, every obligation is proven: a statement after a `return` is
        // not even read, and the right operand of an `and` whose left one is false is proven.
        assert_eq!(
            obligations("pub fn f(x: u8, y: u8) -> u8 {\n  return 0\n  x / y\n}"),
            [(Range, false, true)]
        );
        for (src, kinds) in [
            ("pub fn f(xs: [2]u8, x: u8, y: u8) -> bool = x < 0 and xs[y] > 0", vec![Index]),
            ("pub fn f(x: u8, y: u8) -> bool = x < 0 and x / y > 0", vec![Divisor]),
            ("pub fn f(x: u8, k: u8) -> bool = x < 0 and x << k > 0", vec![Shift, Range]),
            ("pub fn g(a: u8[0..3]) -> bool = true\npub fn f(x: u32) -> bool = x < 0 and g(x)", vec![Range, Refinement]),
            ("pub fn g(a: u8) -> bool\n  requires a > 3\n= true\npub fn f(x: u8) -> bool = x < 0 and g(x)", vec![Range, Requires]),
        ] {
            let found: Vec<ObligationKind> = report(src).obligations.iter().map(|o| o.kind).collect();
            assert_eq!(found, kinds, "{}", src);
            assert!(report(src).obligations.iter().all(|o| o.proven), "{}", src);
        }
    }

    #[test]
    fn test_values_of_operators_bound_what_they_compute() {
        for src in [
            "pub fn f(x: u8) -> u8[0..16] = x & 15",
            "pub fn f(x: u8) -> u8[0..16] = 15 & x",
            "pub fn f(x: u8, y: u8[0..9]) -> u8[0..9] = x & y",
            "pub fn f(x: i8, y: i8[0..9]) -> i8[0..9] = x & y",
            "pub fn f(x: i8, y: i8[0..9]) -> i8[0..9] = y & x",
            "pub fn f(x: u8[0..5], y: u8[0..9]) -> u8[0..16] = x | y",
            "pub fn f(x: u8[0..5], y: u8[0..9]) -> u8[0..16] = x ^ y",
            "pub fn f(x: u8) -> u8[0..16] = x >> 4",
            "pub fn f(x: u8, k: u8[0..8]) -> u8[0..=255] = x >> k",
            "pub fn f(x: i8[-8..=-1], k: u8[0..8]) -> i8[-8..=-1] = x >> k",
            "pub fn f(x: i8[-8..=-1]) -> i8[-4..=-1] = x >> 1",
            "pub fn f(x: u8[0..16]) -> u8[0..=240] = x << 4",
            "pub fn f(x: u8[0..16]) -> u8[240..=255] = ~x",
            "pub fn f(x: i8[0..16]) -> i8[-16..=-1] = ~x",
            "pub fn f(x: u8[2..=3], y: u8[4..=5]) -> u8[8..=15] = x * y",
            "pub fn f(x: i8[-3..=2], y: i8[4..=5]) -> i8[-15..=10] = x * y",
            "pub fn f(x: u8) -> u8[0..=255] = x * 1",
            "pub fn f(x: u8) -> i16[-255..=0] = -1 * x",
            "pub fn f(x: u8) -> i16[-255..=0] = x * -1",
            "pub fn f(x: u8) -> u8[0..1] = 0 * x",
            "pub fn f(x: u8) -> u8[0..1] = x * 0",
            "pub fn f(x: u8, y: u8) -> u8 {\n  guard x < y else {\n    return 0\n  }\n  val d: u8[1..=255] = y - x * 1\n  d\n}",
            "pub fn f(x: u8, y: u8) -> u8 {\n  guard x < y else {\n    return 0\n  }\n  val d: u8[1..=255] = y - 1 * x\n  d\n}",
            "pub fn f(x: u8, y: u8) -> i16 {\n  guard x < y else {\n    return 0\n  }\n  val d: i16[1..=255] = y + x * -1\n  d\n}",
            "pub fn f(x: u8, y: u8) -> i16 {\n  guard x < y else {\n    return 0\n  }\n  val d: i16[1..=255] = y + -1 * x\n  d\n}",
            "pub fn f(x: u8[1..16]) -> u8[16..=240] = x << 4",
            "pub fn f(x: i8[0..=9], y: i8[-8..=-1]) -> i8[0..=9] = x & y",
            "pub fn f(x: i8[-3..=2]) -> i8[-2..=3] = -x",
            "pub fn f(x: u8[0..10], y: u8[0..10]) -> u8[0..=18] = x + y",
            "pub fn f(x: u8[0..10], y: u8[0..10]) -> i8[-9..=9] = x - y",
            "pub fn f(x: u8[0..10], y: u8[0..10], z: u8[0..10]) -> u8[0..=27] = x + y + z",
            "pub fn f(x: u8[0..10], y: u8[0..10], z: u8[0..10]) -> u8[0..=27] = z + (x + y)",
        ] {
            assert_eq!(codes(src), NONE, "{}", src);
        }
        for src in [
            "pub fn f(x: u8) -> u8[0..15] = x & 15",
            "pub fn f(x: u8) -> u8[0..15] = 15 & x",
            "pub fn f(x: i8, y: i8) -> i8[0..9] = x & y",
            "pub fn f(x: u8[0..5], y: u8[0..9]) -> u8[0..15] = x | y",
            "pub fn f(x: u8[0..5], y: u8[0..9]) -> u8[0..15] = x ^ y",
            "pub fn f(x: i8, y: i8[0..9]) -> i8[0..9] = x | y",
            "pub fn f(x: u8) -> u8[0..15] = x >> 4",
            "pub fn f(x: i8[-8..=-1], k: u8[0..8]) -> i8[-8..=-2] = x >> k",
            "pub fn f(x: i8[-8..=-1]) -> i8[-3..=-1] = x >> 1",
            "pub fn f(x: u8[0..16]) -> u8[0..=239] = x << 4",
            "pub fn f(x: u8[1..16]) -> u8[17..=240] = x << 4",
            "pub fn f(x: i8[-8..=-1], y: i8[-8..=-1]) -> i8[0..=127] = x & y",
            "pub fn f(x: i8[0..=9], y: i8[-8..=-1]) -> i8[0..1] = x & y",
            "pub fn f(x: u8[0..16]) -> u8[241..=255] = ~x",
            "pub fn f(x: i8[0..16]) -> i8[-15..=-1] = ~x",
            "pub fn f(x: u8[2..=3], y: u8[4..=5]) -> u8[8..15] = x * y",
            "pub fn f(x: i8[-3..=2], y: i8[4..=5]) -> i8[-14..=10] = x * y",
            "pub fn f(x: i8[-3..=2]) -> i8[-1..=3] = -x",
            "pub fn f(x: u8[0..10], y: u8[0..10]) -> u8[0..18] = x + y",
            "pub fn f(x: u8[0..10], y: u8[0..10]) -> i8[-8..=9] = x - y",
        ] {
            assert_ne!(codes(src), NONE, "{}", src);
        }
    }

    #[test]
    fn test_the_len_of_a_fixed_array_is_a_constant_in_clauses() {
        // `x + y <= xs.len` has two variables and the constant 4, so it is inside the fragment.
        let callee =
            "pub fn take(x: u8, y: u8, xs: [4]u8) -> u8\n  requires x + y <= xs.len\n= xs[0]\n";
        assert_eq!(codes(callee), Vec::<&str>::new());
        let proven = format!("{}pub fn f(xs: [4]u8) -> u8 = take(1, 3, xs)", callee);
        assert_eq!(codes(&proven), Vec::<&str>::new());
        let refuted = format!("{}pub fn f(xs: [4]u8) -> u8 = take(2, 3, xs)", callee);
        assert_eq!(codes(&refuted), ["E0504"]);
        // With a third variable in place of the length, the same clause is outside.
        assert_eq!(
            codes("pub fn g(x: u8, y: u8, z: u8)\n  requires x + y <= z\n{\n}"),
            ["E0503"]
        );
    }

    #[test]
    fn test_clauses_outside_the_fragment_are_routed() {
        let cases = [
            ("x % 2 == 0", "`%`"),
            ("x > 1 or x < 0", "`or`"),
            ("x != 3", "`!=`"),
            ("x * y > 3", "a product of variables"),
            ("2 * x > 3", "a coefficient other than 1 or -1"),
            ("x + x <= 3", "a coefficient other than 1 or -1"),
            ("x <= 2 * y", "a coefficient other than 1 or -1"),
            ("x + 2 * x <= 3", "a coefficient other than 1 or -1"),
            ("x * xs.len <= 9", "a coefficient other than 1 or -1"),
            ("x + y + z > 3", "more than two variables"),
            ("id(x) > 3", "a call"),
            ("not x > 3", "`not`"),
            ("c", "a value that is not a comparison"),
            ("c == true", "a value that is not an integer"),
            ("xs[0] > 3", "an array element"),
            (
                "[xs][0].len > 3",
                "a field of a value that is not a binding",
            ),
            ("x > 0 and x % 2 == 0", "`%`"),
            ("x % 2 == 0 and x > 0", "`%`"),
            ("-x > x / 2", "`/`"),
            (
                "x < todo(\"n\")",
                "an expression that is not linear arithmetic",
            ),
        ];
        for (clause, why) in cases {
            let src = format!(
                "pub fn id(v: u8) -> u8 = v\npub fn f(x: u8, y: u8, z: u8, c: bool, one: [1]u8, xs: [2]u8)\n  requires {}\n{{\n}}",
                clause
            );
            let d = diagnostics(&src);
            assert_eq!(d.len(), 1, "{}: {:?}", clause, d);
            assert_eq!(
                d[0].message,
                format!(
                    "`requires {}` lies outside the Tier-1 fragment, since it uses {}; mark `f` `@checked` to check it at run time, or `@prove` to make it a trusted axiom until `mn prove` runs",
                    clause, why
                ),
                "{}",
                clause
            );
            assert_eq!(d[0].fixes.len(), 2);
            assert_eq!(
                d[0].fixes[0].message,
                "mark `f` `@checked`: the clause becomes a run-time check"
            );
            assert_eq!(
                d[0].fixes[1].message,
                "mark `f` `@prove`: the clause becomes a trusted axiom until `mn prove` runs"
            );
            for (fix, attr) in d[0].fixes.iter().zip(["@checked", "@prove"]) {
                assert_eq!(fix.applicability, Applicability::Suggestion);
                let text = apply_fix(&module(&src), fix).unwrap();
                assert!(text.contains(&format!("{}\npub fn f(", attr)), "{}", text);
                assert_eq!(check_source(&text).err(), None, "{}", text);
            }
        }
        // Inside the fragment, in any arrangement.
        for clause in [
            "x < y",
            "xs.len > 0",
            "x + 1 <= y",
            "-x - y >= -400",
            "x - y == 2 - 2",
            "x + x - x <= 3 * 4",
            "x <= 3 * 4 - 1",
            "x * 1 < y",
            "-x + x + y < z",
            "x - x + y + z > 0",
            "x + y <= x + 3",
            "x * one.len < y",
            "(2 - 1) * x < y",
            "x < y and y < z + 0",
        ] {
            let src = format!(
                "pub fn f(x: u8, y: u8, z: u8, one: [1]u8, xs: [2]u8)\n  requires {}\n{{\n}}",
                clause
            );
            assert_eq!(codes(&src), NONE, "{}", clause);
        }
        // `ensures` and `invariant` clauses are routed too, and an indented function keeps
        // its indentation in the fix.
        assert_eq!(
            codes("pub fn f(x: u8) -> u8\n  ensures result != 3\n= 4"),
            ["E0503"]
        );
        let inv = "pub fn f(n: u8) {\n  var i: u8 = 0\n  while i < n invariant i != 300 {\n    i += 1\n  }\n}";
        assert_eq!(codes(inv), ["E0503"]);
        assert!(messages(inv)[0].starts_with("`invariant i != 300` lies outside"));
        let indented = diagnostics("  pub fn f(x: u8)\n    requires x != 3\n  {\n  }");
        assert_eq!(indented[0].fixes[0].edits[0].replacement, "  @checked\n");
        assert_eq!(
            (
                indented[0].fixes[0].edits[0].span.line,
                indented[0].fixes[0].edits[0].span.col
            ),
            (2, 1)
        );
        // `@checked` keeps the clause as a run-time check, `@prove` as a trusted axiom: Tier 1
        // assumes it and proves it nowhere, at the definition or at a call.
        for attr in ["@checked", "@prove"] {
            let src = format!(
                "{}\npub fn half(x: u8) -> u8\n  requires x % 2 == 0\n  ensures result * 2 == x\n= x / 2\npub fn f(y: u8) -> u8 = half(y)\n{}\npub fn g(n: u8) -> u8 {{\n  var i: u8 = 0\n  while i < n invariant i != 300 {{\n    i += 1\n  }}\n  i\n}}",
                attr, attr
            );
            assert_eq!(codes(&src), NONE, "{}", attr);
        }
        // A routed invariant is assumed at the head; the assumption is the trusted part.
        let assumed = "@prove\npub fn f(n: u8) -> u8[0..10] {\n  var i: u8 = 0\n  while i < n invariant i < 10 or i > 300 {\n    i += 1\n  }\n  i\n}";
        assert_eq!(codes(assumed), NONE);
        // A routed `requires` of the function itself is assumed on entry.
        let entry = "@checked\npub fn f(x: u8) -> u8[0..10]\n  requires x < 10 or x > 300\n= x";
        assert_eq!(codes(entry), NONE);
        // A callee's routed `ensures` is assumed after the call.
        let ensured = "@checked\npub fn small(x: u8) -> u8\n  ensures result < 5 or result > 300\n= x\npub fn f(x: u8) -> u8[0..5] = small(x)";
        assert_eq!(codes(ensured), NONE);
    }

    #[test]
    fn test_guard_fixes_go_on_their_own_line() {
        // Before the statement, indented like it.
        let deep = "pub fn f(x: u8, c: bool) -> u8 {\n  if c {\n      val y: u8[0..10] = x\n      return y\n  }\n  0\n}";
        assert_eq!(
            repairs(deep),
            module("pub fn f(x: u8, c: bool) -> u8 {\n  if c {\n      guard x < 10 else { todo(\"x < 10 is false here\") }\n      val y: u8[0..10] = x\n      return y\n  }\n  0\n}")
        );
        // A statement that shares its line with a brace: the guard takes the line, the
        // statement moves to the next one.
        let shared = "pub fn f(x: u8, c: bool) -> u8[0..10] {\n  if c { return x }\n  0\n}";
        assert_eq!(
            repairs(shared),
            module("pub fn f(x: u8, c: bool) -> u8[0..10] {\n  if c { guard x < 10 else { todo(\"x < 10 is false here\") }\n  return x }\n  0\n}")
        );
        let edit = &diagnostics(shared)[0].fixes[0].edits[0];
        assert_eq!((edit.span.line, edit.span.col), (3, 10));
        // An expression body becomes a block.
        assert_eq!(
            repairs("pub fn f(x: u8) -> u8[0..10] = x"),
            module("pub fn f(x: u8) -> u8[0..10] {\n  guard x < 10 else { todo(\"x < 10 is false here\") }\n  x\n}")
        );
        assert_eq!(
            repairs("pub fn f(x: u8) -> u8[0..10]\n=\n  x"),
            module("pub fn f(x: u8) -> u8[0..10]\n{\n  guard x < 10 else { todo(\"x < 10 is false here\") }\n  x\n}")
        );
        assert_eq!(
            repairs("  pub fn f(x: u8) -> u8[0..10] = x"),
            module("  pub fn f(x: u8) -> u8[0..10] {\n    guard x < 10 else { todo(\"x < 10 is false here\") }\n    x\n  }")
        );
        // Without a result, the guard returns.
        let unit = "pub fn g(x: u8[0..10]) {\n}\npub fn f(x: u8) {\n  g(x)\n}";
        assert_eq!(
            repairs(unit),
            module("pub fn g(x: u8[0..10]) {\n}\npub fn f(x: u8) {\n  guard x < 10 else { return }\n  g(x)\n}")
        );
        // In a tab-indented block, a tab.
        assert!(
            fixed("pub fn f(x: u8) -> u8[0..10] {\n\treturn x\n}").contains("{\n\tguard x < 10")
        );
        // An obligation in a function's own `requires` has no statement to guard.
        let header = "pub fn g(x: u8) -> bool\n  requires x > 0\n= true\n@checked\npub fn f(y: u8)\n  requires g(y)\n{\n}";
        let d = diagnostics(header);
        assert_eq!(d[0].code, "E0504");
        assert!(d[0].fixes.is_empty());
    }

    #[test]
    fn test_a_guard_leaves_by_todo_or_return() {
        assert_eq!(
            guard_line(true, "x < 3"),
            "guard x < 3 else { todo(\"x < 3 is false here\") }"
        );
        assert_eq!(guard_line(false, "x < 3"), "guard x < 3 else { return }");
        // A quote or a backslash, which no M1 clause has, is escaped in the reason.
        assert_eq!(
            guard_line(true, r#"a"b\c"#),
            r#"guard a"b\c else { todo("a\"b\\c is false here") }"#
        );
    }

    #[test]
    fn test_obligations_inside_a_loop_name_its_head_facts() {
        let src = "pub fn f(n: usize[0..=10], a: [10]u8) -> u8 {\n  var i: usize = 0\n  var t: u8[0..100] = 0\n  while i < n {\n    t = a[i]\n    i += 1\n  }\n  t\n}";
        let d = &diagnostics(src)[0];
        assert_eq!(d.code, "E0502");
        assert_eq!(
            d.message,
            "Tier 1 cannot rule out that `a[i]` is 100, outside `u8[0..100]`, the type of `t`"
        );
        assert_eq!(d.related.len(), 1);
        assert_eq!(d.related[0].span.line, 5);
        assert_eq!(
            d.related[0].message,
            "at the head of this loop Tier 1 holds `i <= n`, and dropped `i < n`, which it cannot show inductive"
        );
        let always = "pub fn f(n: u8, a: [10]u8) -> u8 {\n  var t: u8[0..100] = 0\n  while true {\n    t = a[n]\n  }\n  t\n}";
        assert_eq!(
            diagnostics(always)[0].related[0].message,
            "at the head of this loop Tier 1 holds `true`"
        );
        let none_kept = "pub fn f(n: u8, a: [10]u8) -> u8 {\n  var t: u8[0..100] = 0\n  var i: u8 = 9\n  while i > n {\n    t = a[n]\n    i = n + 3\n  }\n  t\n}";
        assert_eq!(
            diagnostics(none_kept)[0].related[0].message,
            "at the head of this loop Tier 1 holds no candidate fact, and dropped `i > n`, `i >= n`, which it cannot show inductive"
        );
        // After the loop, no head is named.
        let after = "pub fn f(n: u8) -> u8[0..10] {\n  var i: u8 = 0\n  while i < n {\n    i += 1\n  }\n  i\n}";
        assert!(diagnostics(after)[0].related.is_empty());
    }

    #[test]
    fn test_mut_arguments_are_proven_at_the_call_and_on_the_way_back() {
        let bump = "pub fn bump(mut x: u8[0..10])\n  ensures x > 3\n{\n  x = 5\n}\n";
        // In: the place's value enters the parameter's refinement.
        let into = format!(
            "{}pub fn f(y: u8) {{\n  var z: u8 = y\n  bump(mut z)\n}}",
            bump
        );
        assert_eq!(codes(&into), ["E0502"]);
        // Back: the parameter's final value, which `ensures` bounds, enters the place's.
        let back = format!(
            "{}pub fn f() -> u8[4..10] {{\n  var z: u8 = 1\n  bump(mut z)\n  z\n}}",
            bump
        );
        assert_eq!(codes(&back), NONE);
        let narrow = format!(
            "{}pub fn f() -> u8 {{\n  var z: u8[0..5] = 1\n  bump(mut z)\n  z\n}}",
            bump
        );
        assert_eq!(
            messages(&narrow),
            ["Tier 1 cannot rule out that the value is 5, outside `u8[0..5]`, the type of `z`"]
        );
        assert!(diagnostics(&narrow)[0].fixes.is_empty());
        // An element of an array passed as `mut`.
        let element = format!(
            "{}pub fn f() -> [2]u8 {{\n  var zs: [2]u8[0..5] = [1, 1]\n  bump(mut zs[0])\n  zs\n}}",
            bump
        );
        assert_eq!(codes(&element), ["E0502"]);
        // A function whose contract says nothing of its `mut` argument may change it to anything
        // of its type, whatever its body does.
        let opaque = "pub fn touch(mut x: u8) {\n  x = 1\n}\npub fn f() -> u8[0..5] {\n  var z: u8 = 1\n  touch(mut z)\n  z\n}";
        assert_eq!(codes(opaque), ["E0502"]);
        let opaque_element = "pub fn touch(mut x: u8) {\n  x = 1\n}\npub fn f() -> u8[0..5] {\n  var zs: [1]u8 = [1]\n  touch(mut zs[0])\n  3\n}";
        assert_eq!(codes(opaque_element), NONE);
        let array = "pub fn fill(mut xs: [2]u8[0..3]) {\n  xs[0] = 1\n}\npub fn f() -> [2]u8[0..3] {\n  var zs: [2]u8[0..3] = [0, 0]\n  fill(mut zs)\n  zs\n}";
        assert_eq!(codes(array), NONE);
        let read_only = "pub fn keep(x: u8) -> u8\n  ensures result == x\n= x\npub fn f(y: u8[0..5]) -> u8[0..5] = keep(y)";
        assert_eq!(codes(read_only), NONE);
    }

    /// A value is read when the reference interpreter reads it: an operator's left operand, a
    /// compound assignment's place and a struct literal's earlier field before what follows
    /// runs, and what the right operand of `and` or `or` changes stays changed after it.
    #[test]
    fn test_values_are_read_in_the_order_the_interpreter_reads_them() {
        let bump = "fn bump(mut x: u8) -> u8\n  ensures x == 9\n  ensures result == 0\n{\n  x = 9\n  0\n}\n";
        let with = |f: &str| format!("{}{}", bump, f);
        for f in [
            "pub fn f() -> u8[9..=9] {\n  var x: u8 = 0\n  val y = x * 1 + bump(mut x)\n  y\n}",
            "pub fn f() -> u8[0..=0] {\n  var x: u8 = 0\n  if x == bump(mut x) {\n    return 1\n  }\n  0\n}",
            "pub fn f() -> u8[9..=9] {\n  var x: u8 = 0\n  x += bump(mut x)\n  x\n}",
            "pub struct P {\n  a: u8\n}\npub fn f(q: P) -> u8[9..=9] {\n  var p = q\n  p.a = 0\n  p.a += bump(mut p.a)\n  p.a\n}",
            "pub struct Q {\n  a: u8\n}\npub struct O {\n  p: u8\n  r: Q\n}\npub fn f() -> u8[9..=9] {\n  var x: u8 = 0\n  val o = O(p: x, r: Q(a: bump(mut x)))\n  o.p\n}",
            // What the right operand changes stays changed ...
            "pub fn f() -> u8[0..=0] {\n  var x: u8 = 0\n  if x == 0 and bump(mut x) == 0 {\n    return 0\n  }\n  x\n}",
            "pub fn f() -> u8[0..=0] {\n  var x: u8 = 0\n  if x != 0 or bump(mut x) != 0 {\n    return 0\n  }\n  x\n}",
            // ... and a left operand that reads it held before it ran, wherever it reads it.
            "pub fn f() -> u8[0..=0] {\n  var x: u8 = 0\n  if x == 0 and bump(mut x) == 0 {\n    return x\n  }\n  0\n}",
            "pub fn f() -> u8[0..=0] {\n  var x: u8 = 0\n  if 0 == x and bump(mut x) == 0 {\n    return x\n  }\n  0\n}",
            "pub fn f() -> u8[0..=0] {\n  var x: u8 = 0\n  if not (x != 0) and bump(mut x) == 0 {\n    return x\n  }\n  0\n}",
            "pub fn f(y: u8) -> u8[0..=0] {\n  var x: u8 = 0\n  if (x == 0 and y == 0) and bump(mut x) == 0 {\n    return x\n  }\n  0\n}",
            "pub fn f(y: u8) -> u8[0..=0] {\n  var x: u8 = 0\n  if (y == 0 and x == 0) and bump(mut x) == 0 {\n    return x\n  }\n  0\n}",
        ] {
            assert_eq!(codes(&with(f)), ["E0502"], "{}", f);
        }
        // A value that passes a binding `mut` is offered no `guard`, which would repeat the call
        // and change the binding again; one that does not is.
        for f in [
            "pub fn f() -> u8[9..=9] {\n  var x: u8 = 0\n  x * 1 + bump(mut x)\n}",
            "fn set9(mut xs: [2]u8) -> usize {\n  xs[0] = 9\n  0\n}\npub fn f() -> u8[0..=0] {\n  var xs: [2]u8 = [0, 0]\n  xs[set9(mut xs)]\n}",
        ] {
            let d = diagnostics(&with(f));
            assert_eq!(d.len(), 1, "{}: {:?}", f, d);
            assert!(d[0].fixes.is_empty(), "{}: {:?}", f, d[0].fixes);
            assert!(
                d[0].message.ends_with("; it passes a binding `mut`, which a `guard` repeating it would change again, so none is offered"),
                "{}",
                d[0].message
            );
        }
        let pure = diagnostics(&with(
            "fn same(x: u8) -> u8 = x\npub fn f(y: u8) -> u8[9..=9] {\n  y * 1 + same(y)\n}",
        ));
        assert!(
            pure[0].fixes[0]
                .message
                .contains("guard y * 1 + same(y) >= 9"),
            "{:?}",
            pure[0].fixes
        );
        // What the later operand leaves alone keeps its relations.
        for f in [
            "pub fn f() -> u8[9..=9] {\n  var x: u8 = 0\n  val y = bump(mut x) + x\n  y\n}",
            "pub fn f(a: u8[0..100]) -> u8[0..=0] {\n  var z: u8 = 0\n  val y = a + bump(mut z)\n  y - a\n}",
            "pub fn f(a: u8[0..100]) -> u8[0..=0] {\n  var x: u8 = a\n  var z: u8 = 0\n  x += bump(mut z)\n  x - a\n}",
            "pub fn f(i: u8) -> u8[0..4] {\n  var z: u8 = 0\n  if i < 4 and bump(mut z) == 0 {\n    return i\n  }\n  0\n}",
            // Where the left operand decides alone, it holds; here it never does.
            "pub fn f() -> u8[9..=9] {\n  var x: u8 = 0\n  if x == 0 and bump(mut x) == 0 {\n    return x\n  }\n  x\n}",
        ] {
            assert_eq!(codes(&with(f)), NONE, "{}", f);
        }
    }

    /// A dividend smaller in magnitude than every divisor has a quotient of 0 and is its own
    /// remainder, a divisor beyond the octagon's reach and a negative one included; one that
    /// reaches the divisor's least magnitude, on either side, does not.
    #[test]
    fn test_a_dividend_below_every_divisor_has_a_quotient_of_0() {
        let far = "42535295865117307932921825928971026432";
        for f in [
            format!("pub fn f(x: u8) -> i128[-1..=1] = x / {}", far),
            format!("pub fn f(x: u8) -> i128[0..=0] = x / -{}", far),
            format!("pub fn f(x: u8) -> i128[0..=0] = x % {} - x", far),
            "pub fn f(x: i16[-254..=0], y: i16[255..=400]) -> i16[0..=0] = x / y".to_string(),
            "pub fn f(x: i16[0..=255], y: i16[-400..=-256]) -> i16[0..=0] = x / y".to_string(),
            "pub fn f(x: u16[0..=255], y: u16[256..=400]) -> u16[0..=0] = x % y - x".to_string(),
        ] {
            assert_eq!(codes(&f), NONE, "{}", f);
        }
        assert_eq!(
            messages("pub fn f(x: u16[0..=255], y: u16[255..=400]) -> u16[0..=0] = x / y"),
            ["Tier 1 cannot rule out that `x / y` is 1, outside `u16[0..=0]`, the type of the result of `f`"]
        );
        assert_eq!(
            messages("pub fn f(x: i16[-255..=0], y: i16[255..=400]) -> i16[0..=0] = x / y"),
            ["Tier 1 cannot rule out that `x / y` is -1, outside `i16[0..=0]`, the type of the result of `f`"]
        );
        // A dividend of no bound may reach any divisor.
        assert_eq!(
            codes(&format!("pub fn f(x: i128) -> i128[0..=0] = x / {}", far)),
            ["E0502"]
        );
    }

    /// A loop's condition runs at the head of every round, so a binding or a struct it passes
    /// `mut` keeps only its type and refinement there; and a call in the condition, which the
    /// head reads as a candidate fact too, reports a failure once.
    #[test]
    fn test_a_loops_condition_changes_what_it_passes_mut() {
        let decls = "pub struct P {\n  a: u8\n}\nfn bump(mut x: u8) -> u8\n  ensures x == 9\n{\n  x = 9\n  0\n}\nfn touch(mut p: P) -> u8 {\n  p.a = 9\n  0\n}\nfn take0(x: u8) -> u8[0..=0]\n  requires x == 0\n= x\n";
        let with = |f: &str| format!("{}{}", decls, f);
        for f in [
            "pub fn f() -> u8 {\n  var x: u8 = 0\n  var i: u8 = 0\n  while i < 2 and take0(x) == 0 and bump(mut x) == 0\n    decreases 2 - i\n  {\n    i += 1\n  }\n  i\n}",
            "pub fn f(q: P) -> u8 {\n  var p = q\n  p.a = 0\n  var i: u8 = 0\n  while i < 2 and take0(p.a) == 0 and touch(mut p) == 0\n    decreases 2 - i\n  {\n    i += 1\n  }\n  i\n}",
            "pub fn f() -> u8 {\n  var x: u8 = 0\n  var i: u8 = 0\n  while i < 2 and take0(x) == 0\n    decreases 2 - i\n  {\n    x = 1\n    i += 1\n  }\n  i\n}",
        ] {
            assert_eq!(codes(&with(f)), ["E0504"], "{}", f);
        }
        // There a `guard` before the loop would not hold at a later round: none is offered.
        for f in [
            "pub fn f() -> u8 {\n  var x: u8 = 0\n  var i: u8 = 0\n  while i < 2 and take0(x) == 0 and bump(mut x) == 0\n    decreases 2 - i\n  {\n    i += 1\n  }\n  i\n}",
            "pub fn f(q: P) -> u8 {\n  var p = q\n  p.a = 0\n  var i: u8 = 0\n  while i < 2 and take0(p.a) == 0 and touch(mut p) == 0\n    decreases 2 - i\n  {\n    i += 1\n  }\n  i\n}",
        ] {
            let d = diagnostics(&with(f));
            assert!(d[0].fixes.is_empty(), "{}: {:?}", f, d[0].fixes);
            assert!(
                d[0].message.ends_with("; this loop changes what it reads, so a `guard` before the loop would not hold at a later round, and none is offered"),
                "{}",
                d[0].message
            );
        }
        // Over what the loop leaves alone, the guard before it holds at every round, and
        // proves the clause once applied.
        let alone = with("pub fn f(x: u8) -> u8 {\n  var i: u8 = 0\n  while i < 2 and take0(x) == 0\n    decreases 2 - i\n  {\n    i += 1\n  }\n  i\n}");
        assert!(diagnostics(&alone)[0].fixes[0]
            .message
            .contains("guard x == 0 else"));
        repairs(&alone);
        // A condition that changes nothing keeps what holds before the loop.
        assert_eq!(
            codes(&with("pub fn f() -> u8 {\n  val x: u8 = 0\n  var i: u8 = 0\n  while i < 2 and take0(x) == 0\n    decreases 2 - i\n  {\n    i += 1\n  }\n  i\n}")),
            NONE
        );
    }

    #[test]
    fn test_arrays_carry_the_bounds_of_their_elements() {
        for src in [
            "pub fn f(xs: [4]u8[0..10], i: usize[0..4]) -> u8[0..10] = xs[i]",
            "pub fn f(i: usize[0..2]) -> u8[0..10] = [3, 9][i]",
            "pub fn f(i: usize[0..2]) -> u8[0..10] = [7; 2][i]",
            "pub fn f(c: bool) -> [2]u8[0..10] = if c { [1, 2] } else { [3, 9] }",
            "pub fn f(xs: [2][3]u8[0..10]) -> u8[0..10] = xs[1][2]",
            "pub fn g() -> [2]u8[0..10] = [1, 2]\npub fn f() -> u8[0..10] = g()[0]",
            "pub fn f(xs: [2]u8[0..10]) -> [2]u8[0..10] = xs",
            "pub fn f() -> [2]u8[0..10] = [5; 2]",
            "pub fn f(c: bool) -> [2]u8[0..10] = if c { [1, 2] } else { todo(\"later\") }",
            "pub fn f(c: bool) -> [2]u8[0..10] = if c { todo(\"later\") } else { [1, 2] }",
            "pub fn f(n: usize, xs: [2]usize[0..n]) -> usize[0..n] = xs[0]",
            "pub fn f(xs: [2]u8) -> u8[0..=255] = xs[0]",
            "pub fn g(n: usize) -> [2]usize[0..=n] = todo(\"later\")\npub fn f() -> usize[0..=3] = g(3)[1]",
        ] {
            assert_eq!(codes(src), NONE, "{}", src);
        }
        for src in [
            "pub fn f(xs: [4]u8, i: usize[0..4]) -> u8[0..10] = xs[i]",
            "pub fn f(i: usize[0..2]) -> u8[0..9] = [3, 9][i]",
            "pub fn f(xs: [2]u8) -> [2]u8[0..10] = xs",
            "pub fn f() -> [2]u8[0..10] = [12; 2]",
            "pub fn f(xs: [2][3]u8[0..10]) -> u8[0..9] = xs[1][2]",
            "pub fn f(c: bool) -> [2]u8[0..9] = if c { [1, 2] } else { [3, 9] }",
            "pub fn f(c: bool, i: usize[0..2]) -> u8[0..10] {\n  val xs = if c { [1, 2] } else { [3, 9] }\n  xs[i]\n}",
        ] {
            assert_eq!(codes(src), ["E0502"], "{}", src);
        }
        assert_eq!(
            messages("pub fn f(xs: [2]u8) -> [2]u8[0..10] = xs"),
            ["Tier 1 cannot rule out that the value is 10, outside `u8[0..10]`, the type of each element of the result of `f`"]
        );
    }

    #[test]
    fn test_contracts_are_used_modularly() {
        // The callee's body could prove more; Tier 1 reads only its header.
        let src = "pub fn three() -> u8 = 3\npub fn f() -> u8[0..5] = three()";
        assert_eq!(codes(src), ["E0502"]);
        let stated =
            "pub fn three() -> u8\n  ensures result == 3\n= 3\npub fn f() -> u8[0..5] = three()";
        assert_eq!(codes(stated), NONE);
        let refined = "pub fn three() -> u8[3..=3] = 3\npub fn f() -> u8[0..5] = three()";
        assert_eq!(codes(refined), NONE);
        // Recursion uses the function's own contract.
        let rec = "pub fn down(n: u8) -> u8\n  ensures result == 0\n{\n  if n == 0 {\n    return 0\n  }\n  down(n - 1)\n}";
        assert_eq!(codes(rec), NONE);
        // A callee's `requires` about an array length, and a callee whose result its contract
        // does not bound.
        let len = "pub fn first(xs: [3]u8)\n  requires xs.len > 2\n{\n}\npub fn f(ys: [3]u8) {\n  first(ys)\n}";
        assert_eq!(codes(len), NONE);
        let short = "pub fn first(xs: [3]u8)\n  requires xs.len > 3\n{\n}\npub fn f(ys: [3]u8) {\n  first(ys)\n}";
        assert_eq!(codes(short), ["E0504"]);
        let opaque = "pub fn ext(x: u8) -> u8 = x\npub fn f(x: u8) -> u8[0..5] = ext(x)";
        assert_eq!(codes(opaque), ["E0502"]);
        // The facts of the callee's `ensures` relate its result to the caller's bindings.
        let rel = "pub fn more(x: u8[0..200]) -> u8\n  ensures result > x and result <= x + 5\n= x + 1\npub fn f(y: u8[0..10]) -> u8[1..16] = more(y)";
        assert_eq!(codes(rel), NONE);
        let negated = "pub fn g(a: i8) -> i8\n  ensures -result == a\n= -a\npub fn f(x: i8[0..5]) -> i8[-4..=0] = g(x)";
        assert_eq!(codes(negated), NONE);
        let math = "@checked\npub fn g(a: u8[0..10]) -> u8\n  ensures result == a * 1 + a / 1 - a % 1\n= a\npub fn f(x: u8[0..3]) -> u8[0..5] = g(x)";
        assert_eq!(codes(math), NONE);
        let never = "@checked\npub fn g(a: u8) -> u8\n  ensures false\n= todo(\"never\")\npub fn f(x: u8) -> u8[0..5] = g(x)";
        assert_eq!(codes(never), NONE);
        // A callee's clause over `bool` parameters lies outside the fragment and is skipped at
        // its calls, though the callee has an integer parameter too.
        let bools = "@checked\npub fn g(c: bool, d: bool, n: u8)\n  requires c == d\n{\n}\npub fn f(x: bool) {\n  g(x, x, 1)\n}";
        assert_eq!(codes(bools), NONE);
        let logic ="@checked\npub fn g(a: u8) -> u8\n  ensures (result < 5 and true) or not (result >= 5)\n= 1\npub fn f(x: u8) -> u8[0..5] = g(x)";
        assert_eq!(codes(logic), NONE);
        let unknown = "@checked\npub fn g(a: u8, xs: [2]u8) -> u8\n  ensures result == xs[0] and result < 5 and result == q\n= 1\npub fn f(x: u8, ys: [2]u8) -> u8[0..5] = g(x, ys)";
        assert_eq!(codes(unknown), ["E0201"]);
        let unknown_ok = "@checked\npub fn g(a: u8, xs: [2]u8) -> u8\n  ensures result == xs[0] and result < 5 and a.len == 2\n= 1\npub fn f(x: u8, ys: [2]u8) -> u8[0..5] = g(x, ys)";
        assert!(codes(unknown_ok).iter().all(|c| *c == "E0215"));
    }

    #[test]
    fn test_conditions_narrow_the_state() {
        for src in [
            "pub fn f(x: u8) -> u8[0..10] = if x < 10 { x } else { 0 }",
            "pub fn f(x: u8) -> u8[0..10] = if not (x >= 10) { x } else { 0 }",
            "pub fn f(x: u8, c: bool) -> u8[0..10] = if x < 10 and c { x } else { 0 }",
            "pub fn f(x: u8) -> u8[0..10] = if x >= 10 or x == 3 { 3 } else { x }",
            "pub fn f(x: u8) -> u8[0..10] = if x == 4 { x } else { 0 }",
            "pub fn f(x: u8) -> u8[0..10] = if x != 4 { 0 } else { x }",
            "pub fn f(x: u8) -> u8[0..10] = if 10 > x { x } else { 0 }",
            "pub fn f(x: u8) -> u8[0..10] = if 9 >= x { x } else { 0 }",
            "pub fn f(x: u8) -> u8[0..10] = if x <= 9 { x } else { 0 }",
            "pub fn f(x: u8) -> u8[1..10] = if x > 9 { 1 } else if x == 0 { 1 } else { x }",
            "pub fn f(x: u8, y: u8) -> u8[0..10] = if x - y < 10 and y <= x { x - y } else { 0 }",
            "pub fn f(x: u8, y: u8) -> u8[0..10] = if x + y < 10 { x } else { 0 }",
            "pub fn f(a: u8, b: u8, c: u8) -> u8[0..10] = if a + b < c + 10 and c == 0 { a } else { 0 }",
            "pub fn f(x: u8) -> u8[0..10] {\n  if x >= 10 {\n    return 0\n  }\n  x\n}",
            "pub fn f(x: u8) -> u8[0..10] = if false { x } else { 1 }",
            "pub fn f(x: u8) -> u8[0..10] = if true { 1 } else { x }",
            "pub fn f(x: u8, c: bool) -> u8[0..10] {\n  val ok = x < 10\n  if ok and x < 10 { x } else { 0 }\n}",
        ] {
            assert_eq!(codes(src), NONE, "{}", src);
        }
        for src in [
            "pub fn f(x: u8) -> u8[0..10] = if x <= 10 { x } else { 0 }",
            "pub fn f(x: u8, c: bool) -> u8[0..10] = if x < 10 or c { x } else { 0 }",
            "pub fn f(x: u8) -> u8[0..10] = if x != 4 { x } else { 0 }",
            "pub fn f(x: u8, c: bool) -> u8[0..10] = if c { x } else { 0 }",
            "pub fn f(x: u8) -> u8[0..10] = if 10 >= x { x } else { 0 }",
            "pub fn f(x: u8) -> u8[0..10] = if x > 9 { x } else { 0 }",
        ] {
            assert_eq!(codes(src), ["E0502"], "{}", src);
        }
    }

    #[test]
    fn test_pure_helpers_keep_their_bounds() {
        assert_eq!(sat_add(INF, -5), INF);
        assert_eq!(sat_add(-5, INF), INF);
        assert_eq!(sat_add(-INF, 5), -INF);
        assert_eq!(sat_add(5, -INF), -INF);
        assert_eq!(sat_add(INF - 1, INF - 1), INF);
        assert_eq!(sat_add(1 - INF, 1 - INF), -INF);
        assert_eq!(sat_add(2, 3), 5);
        assert_eq!(trunc_div(-7, 2), -3);
        assert_eq!(trunc_div(INF, 2), INF);
        assert_eq!(trunc_div(-INF, 2), -INF);
        assert_eq!(rest(-7, 2), -4);
        assert_eq!(rest(INF, 2), INF);
        assert_eq!(rest(-INF, 2), -INF);
        assert_eq!(mul_bound(0, INF), 0);
        assert_eq!(mul_bound(INF, 0), 0);
        assert_eq!(mul_bound(-INF, 2), -INF);
        assert_eq!(mul_bound(3, -INF), -INF);
        assert_eq!(mul_bound(-3, -INF), INF);
        assert_eq!(mul_bound(i128::MAX / 8, 3), INF);
        assert_eq!(mul_bound(-3, 4), -12);
        assert_eq!(mul_bound(INF, INF), INF);
        assert_eq!(mul_bound(-INF, INF), -INF);
        assert_eq!(hull([3, -1, 7, 2]), Val::Itv(-1, 7));
        assert_eq!(hull([-1, 3, 2, 7]), Val::Itv(-1, 7));
        assert_eq!(hull([7, 2, -1, 3]), Val::Itv(-1, 7));
        assert_eq!(hull([2, 7, 3, -1]), Val::Itv(-1, 7));
        assert_eq!(ones(0), 0);
        assert_eq!(ones(15), 15);
        assert_eq!(ones(1), 1);
        assert_eq!(ones(9), 15);
        assert_eq!(ones(16), 31);
        assert!(ones(INF) >= INF);
        // Beyond the domain's bound, the mask stops at it.
        assert_eq!(ones(INF + 5), INF);
        assert_eq!(int_literal(u128::MAX), TOP);
        assert_eq!(lin(vec![], BIG), TOP);
        assert_eq!(lin(vec![], -BIG), TOP);
        assert_eq!(lin(vec![], BIG - 1), Val::Lin(vec![], BIG - 1));
        let u8_ = IntTy::named("u8").unwrap_or(crate::types::I64);
        let i128_ = IntTy::named("i128").unwrap_or(crate::types::I64);
        let u64_ = IntTy::named("u64").unwrap_or(crate::types::I64);
        assert_eq!(int_range(u8_), (0, 255));
        assert_eq!(int_range(i128_), (-INF, INF));
        let u128_ = IntTy::named("u128").unwrap_or(crate::types::I64);
        assert_eq!(int_range(u128_), (0, INF));
        assert_eq!(int_range(u64_), (0, u64::MAX as i128));
        assert_eq!(bit_not(&TOP, Some(u8_)), TOP);
        assert_eq!(bit_not(&Val::Lin(vec![], 1), None), TOP);
        assert_eq!(
            bit_not(&Val::Lin(vec![], 1), Some(u8_)),
            Val::Lin(vec![], 254)
        );
        let x = Term::pos(0);
        let y = Term::pos(1);
        assert_eq!(lin_add(&[x], &[x.negated()]), Some(vec![]));
        assert_eq!(lin_add(&[x], &[x]), None);
        assert_eq!(lin_add(&[x, y], &[Term::pos(2)]), None);
        assert_eq!(lin_add(&[x], &[y]), Some(vec![x, y]));
        assert_eq!(negate(&Val::Other), Val::Other);
        assert_eq!(negate(&Val::Itv(1, 3)), Val::Itv(-3, -1));
        assert_eq!(as_const(&Val::Lin(vec![x], 3)), None);
        assert_eq!(as_const(&Val::Itv(3, 3)), None);
        let st = Octagon::new(3);
        assert_eq!(summary_of(&st, &Val::Other), Val::Other);
        assert_eq!(summary_of(&st, &Val::Lin(vec![], 4)), Val::Itv(4, 4));
        assert_eq!(join_summary(&Val::Other, &Val::Itv(0, 1)), Val::Other);
        assert_eq!(
            join_summary(&Val::Itv(2, 7), &Val::Itv(0, 1)),
            Val::Itv(0, 7)
        );
        assert_eq!(
            join_summary(
                &Val::Arr(Box::new(Val::Itv(0, 1))),
                &Val::Arr(Box::new(Val::Itv(5, 6)))
            ),
            Val::Arr(Box::new(Val::Itv(0, 6)))
        );
        assert_eq!(interval(&st, &Val::Other), (-INF, INF));
        // `x0 + 1 + (x1 - x0 + 2) / 2` with `0 <= x0 < x1 <= 10`: the sum `x1 + 3` bounds it
        // above by 11, tighter than `x0 + 1` and the quotient's bounds apart.
        let mut rel = Octagon::new(2);
        rel.add_lin(&[x.negated()], 0);
        rel.add_lin(&[y], 10);
        rel.add_lin(&[x, y.negated()], -1);
        let q = Val::Quot {
            base: (vec![x], 1),
            num: (vec![y, x.negated()], 2),
            div: 2,
        };
        assert_eq!(interval(&rel, &q), (2, 11));
        assert_eq!(interval(&st, &Val::Lin(vec![x], 3)), (-INF, INF));
        assert_eq!(flip(Rel::Ne), Rel::Eq);
        assert_eq!(flip(Rel::Eq), Rel::Ne);
        // Each comparison is the operator that writes it, and flipping or mirroring it twice gives
        // it back.
        for r in [Rel::Eq, Rel::Ne, Rel::Lt, Rel::Le, Rel::Gt, Rel::Ge] {
            assert_eq!(Rel::of(r.op()), Some(r));
            assert_eq!(flip(flip(r)), r);
            assert_eq!(mirror(mirror(r)), r);
        }
        assert_eq!(Rel::of(BinaryOp::Add), None);
        // Division: by a constant, by a positive or a negative divisor, by one that may be
        // zero, and with an unbounded dividend.
        assert_eq!(
            div_vals(&st, &Val::Itv(1, 9), &lin(vec![], 0)),
            Val::Itv(-9, 9)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(1, 9), &Val::Itv(0, 0)),
            Val::Itv(-9, 9)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(1, 9), &lin(vec![], 2)),
            Val::Itv(0, 4)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(1, 9), &lin(vec![], -2)),
            Val::Itv(-4, 0)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(-9, 9), &Val::Itv(-1, 1)),
            Val::Itv(-9, 9)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(-9, 9), &Val::Itv(-3, 0)),
            Val::Itv(-9, 9)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(-9, 7), &Val::Itv(2, 3)),
            Val::Itv(-4, 3)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(-9, 7), &Val::Itv(-3, -2)),
            Val::Itv(-3, 4)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(5, 7), &Val::Itv(2, 3)),
            Val::Itv(1, 3)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(-7, -5), &Val::Itv(2, 3)),
            Val::Itv(-3, -1)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(-INF, 10), &Val::Itv(2, 4)),
            Val::Itv(-INF, 5)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(-10, INF), &Val::Itv(2, 4)),
            Val::Itv(-5, INF)
        );
        assert_eq!(
            div_vals(&st, &Val::Itv(-10, 10), &Val::Itv(2, INF)),
            Val::Itv(-5, 5)
        );
        assert_eq!(
            add_vals(
                &st,
                &Val::Lin(vec![x, y], 0),
                &Val::Lin(vec![Term::pos(2)], 0)
            ),
            TOP
        );
        let q = Val::Quot {
            base: (vec![x], 0),
            num: (vec![y], 0),
            div: 2,
        };
        assert_eq!(add_vals(&st, &q, &Val::Lin(vec![], BIG)), TOP);
        let q5 = Val::Quot {
            base: (vec![x], 5),
            num: (vec![y], 7),
            div: 2,
        };
        assert_eq!(add_vals(&st, &q5, &Val::Lin(vec![], BIG - 3)), TOP);
        assert_eq!(
            negate(&q5),
            Val::Quot {
                base: (vec![x.negated()], -5),
                num: (vec![y.negated()], -7),
                div: 2
            }
        );
        assert_eq!(add_vals(&st, &q, &Val::Lin(vec![x], 0)), TOP);
        assert_eq!(
            add_vals(&st, &Val::Lin(vec![], 3), &q),
            Val::Quot {
                base: (vec![x], 3),
                num: (vec![y], 0),
                div: 2
            }
        );
        assert_eq!(
            negate(&q),
            Val::Quot {
                base: (vec![x.negated()], 0),
                num: (vec![y.negated()], 0),
                div: 2
            }
        );
        assert_eq!(
            rem_interval(&st, &Val::Itv(-9, 9), &Val::Itv(-1, 3)),
            (-2, 2)
        );
        assert_eq!(
            rem_interval(&st, &Val::Itv(-9, 9), &Val::Itv(-INF, 3)),
            (-9, 9)
        );
        assert_eq!(
            rem_interval(&st, &Val::Itv(-9, 9), &Val::Itv(1, 3)),
            (-2, 2)
        );
        assert_eq!(
            shift_vals(&st, BinaryOp::Shl, &Val::Itv(1, 2), &Val::Itv(0, 1), None),
            TOP
        );
        assert_eq!(
            shift_vals(&st, BinaryOp::Shr, &TOP, &lin(vec![], 2), None),
            TOP
        );
        assert_eq!(
            shift_vals(&st, BinaryOp::Shl, &Val::Itv(1, 2), &lin(vec![], 121), None),
            TOP
        );
        assert_eq!(
            shift_vals(&st, BinaryOp::Shl, &Val::Itv(1, 2), &lin(vec![], -1), None),
            TOP
        );
        assert_eq!(
            shift_vals(&st, BinaryOp::Shr, &Val::Itv(-9, -2), &Val::Itv(0, 3), None),
            Val::Itv(-9, -1)
        );
        assert_eq!(
            shift_vals(&st, BinaryOp::Shr, &Val::Itv(2, 9), &Val::Itv(0, 3), None),
            Val::Itv(0, 9)
        );
        assert_eq!(
            bits_vals(
                &st,
                BinaryOp::BitXor,
                &Val::Itv(-1, 2),
                &Val::Itv(0, 3),
                None
            ),
            TOP
        );
        assert_eq!(
            clamp_to(Val::Itv(-5, 300), Some(u8_), &st),
            Val::Itv(0, 255)
        );
        assert_eq!(clamp_to(Val::Itv(-5, 300), None, &st), Val::Itv(-5, 300));
        assert_eq!(
            bound_text(&Bound::Int {
                negative: true,
                value: 3
            }),
            "-3"
        );
        assert_eq!(bound_text(&Bound::Len("xs".to_string())), "xs.len");
        assert_eq!(bound_int(true, 3), Val::Lin(vec![], -3));
        assert!(matches!(Shape::Other.refinements(), Shape::Other));
        assert!(matches!(Shape::Other.element(), Shape::Other));
        assert!(matches!(Shape::plain(&Ty::Bool), Shape::Other));
        assert!(matches!(
            Shape::plain(&Ty::Int(Int::Literal)),
            Shape::Int {
                int: None,
                refine: None
            }
        ));
        assert_eq!(paren(("a + b".to_string(), 9), 10), "(a + b)");
        assert_eq!(paren(("a + b".to_string(), 9), 9), "a + b");
    }

    #[test]
    fn test_assignments_relate_their_targets() {
        let mut st = Octagon::new(3);
        st.add_lin(&[Term::pos(0)], 5);
        st.add_lin(&[Term::pos(0).negated()], -1);
        st.add_lin(&[Term::pos(1)], 3);
        st.add_lin(&[Term::pos(1).negated()], 0);
        constrain_offset(&mut st, 2, &[Term::pos(0), Term::pos(1)], 1, 0, 0);
        assert_eq!(st.bounds(2), (2, 9));
        assert!(st.implies(&[Term::pos(2), Term::pos(0).negated()], 4));
        assert!(st.implies(&[Term::pos(0), Term::pos(2).negated()], -1));
        assert!(st.implies(&[Term::pos(2), Term::pos(1).negated()], 6));
        assert!(st.implies(&[Term::pos(1), Term::pos(2).negated()], -2));
        assert!(!st.implies(&[Term::pos(2), Term::pos(1).negated()], 5));
        assert!(!st.implies(&[Term::pos(1), Term::pos(2).negated()], -3));
        let mut one = Octagon::new(2);
        constrain_offset(&mut one, 1, &[Term::pos(0)], 2, -1, 3);
        assert!(one.implies(&[Term::pos(1), Term::pos(0).negated()], 5));
        assert!(!one.implies(&[Term::pos(1), Term::pos(0).negated()], 4));
        assert!(one.implies(&[Term::pos(0), Term::pos(1).negated()], -1));
        assert!(!one.implies(&[Term::pos(0), Term::pos(1).negated()], -2));
        let mut none = Octagon::new(1);
        constrain_offset(&mut none, 0, &[], 2, -1, 3);
        assert_eq!(none.bounds(0), (1, 5));
        // A self-referential assignment, `x = x + 1`, and one through a quotient.
        for (src, ok) in [
            ("pub fn f(x: u8[0..9]) -> u8[1..=9] {\n  var y: u8 = x\n  y = y + 1\n  y\n}", true),
            ("pub fn f(x: u8[0..9]) -> u8[2..=10] {\n  var y: u8 = x\n  y = y + 1\n  y\n}", false),
            ("pub fn f(x: u8[0..9]) -> u8[0..=4] {\n  var y: u8 = x\n  y = y / 2\n  y\n}", true),
            ("pub fn f(x: u8[0..9]) -> u8[0..=3] {\n  var y: u8 = x\n  y = y / 2\n  y\n}", false),
            ("pub fn f(lo: u8, hi: u8) -> u8\n  requires lo < hi\n  ensures result < hi\n{\n  var m: u8 = lo\n  m = m + (hi - m) / 2\n  m\n}", true),
        ] {
            assert_eq!(codes(src).is_empty(), ok, "{}", src);
        }
    }

    #[test]
    fn test_printing_follows_the_grammar() {
        for clause in [
            "a - (b - c) > 0",
            "a - b - c > 0",
            "(a < b) == c",
            "not a < b",
            "-a[0] + ~b * -(-c) >= 0",
            "f(a, mut b) < [1, 2][0] and [a; 3].len == 3",
            "a | b ^ c & d << 1 >> 2 == (a | b) & c",
            "a < b or c and not d",
            "(a or b) and c",
            "(not a) == c",
            "todo(\"x\") == unreachable(\"y\")",
            "1_u8 == true",
            "a % b / c * d == 0",
        ] {
            let text = module(&format!("pub fn f()\n  requires {}\n{{\n}}", clause));
            let m = mente_syntax::parse_module(&text).unwrap();
            let f = mente_syntax::parse_bodies(&text, &m)
                .remove(0)
                .syntax
                .unwrap();
            assert_eq!(print(&f.tree, f.contracts[0].expr, &no_subst).0, clause);
        }
        let text = module("pub fn f()\n  requires (if a { 1 } else { 2 }) == 1\n{\n}");
        let m = mente_syntax::parse_module(&text).unwrap();
        let f = mente_syntax::parse_bodies(&text, &m)
            .remove(0)
            .syntax
            .unwrap();
        assert_eq!(
            print(&f.tree, f.contracts[0].expr, &no_subst).0,
            "(if a { ... }) == 1"
        );
    }

    #[test]
    fn test_loop_measures_are_never_negative_and_get_smaller() {
        for src in [
            "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n decreases n - i {\n    i += 1\n  }\n  i\n}",
            "pub fn gcd(a: u32, b: u32) -> u32 {\n  var x: u32 = a\n  var y: u32 = b\n  while y != 0 decreases y {\n    val t = x % y\n    x = y\n    y = t\n  }\n  x\n}",
            // Each branch makes the measure smaller, which the join of the two does not show.
            "pub fn f(n: u64) -> u64 {\n  var lo: u64 = 0\n  var hi: u64 = 4294967296\n  while hi - lo > 1 decreases hi - lo {\n    val mid = lo + (hi - lo) / 2\n    if mid * mid <= n {\n      lo = mid\n    } else {\n      hi = mid\n    }\n  }\n  lo\n}",
            "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  var j: u8 = 0\n  while i < n invariant j <= 9 decreases n - i decreases 9 - j {\n    if j < 9 {\n      j += 1\n    } else if n > 200 {\n      i += 2\n      j = 0\n    } else {\n      i += 1\n      j = 0\n    }\n  }\n  i\n}",
            "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n decreases n - i {\n    i += 1\n    if i == 3 {\n      continue\n    }\n  }\n  i\n}",
            "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  var j: u8 = 0\n  while i < 8 decreases 8 - i {\n    j = 0\n    while j < 7 - i decreases 7 - i - j {\n      j += 1\n    }\n    i += 1\n  }\n  j + n\n}",
            "pub fn dist(i: u8) -> u8\n  requires i <= 100\n  ensures result == 100 - i\n= 100 - i\npub fn f() -> u8 {\n  var i: u8 = 0\n  while i < 100 decreases dist(i) {\n    i += 1\n  }\n  i\n}",
            "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n decreases n - i {\n    i += 1\n    if i > 3 {\n      break\n    }\n  }\n  i\n}",
            "pub fn f(c: bool) -> u8 {\n  var i: u8 = 0\n  while c and i < 3 decreases 3 - i {\n    i += 1\n  }\n  i\n}",
        ] {
            assert_eq!(codes(src), NONE, "{}", src);
            assert!(obligations(src).contains(&(Decreases, true, true)), "{}", src);
        }
        let stuck = "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n decreases n - i {\n    i += 0\n  }\n  i\n}";
        let d = diagnostics(stuck);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0507");
        assert_eq!(
            d[0].message,
            "Tier 1 cannot rule out `n = 1, i = 0`, after a round of this loop, where its measure `n - i` changes by 0: a `decreases` measure gets smaller at every round"
        );
        assert_eq!((d[0].primary.line, d[0].primary.col), (4, 25));
        assert!(d[0].fixes.is_empty());
        assert!(obligations(stuck).contains(&(Decreases, true, false)));
        // Several measures go in lexicographic order, each path of the round on its own.
        let lex = "pub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  var j: u8 = 0\n  while i < n invariant j <= 9 decreases n - i decreases 9 - j {\n    if j < 9 {\n      j += 0\n    } else {\n      i += 1\n      j = 0\n    }\n  }\n  i\n}";
        assert_eq!(
            messages(lex),
            ["Tier 1 cannot rule out `n = 1, i = 0, j = 0`, after a round of this loop, where its measure `n - i, 9 - j` changes by (0, 0): a `decreases` measure gets smaller at every round"]
        );
        let up = lex
            .replace("j += 0", "j += 1")
            .replace("i += 1\n", "i += 0\n");
        assert_eq!(codes(&up), ["E0507"]);
        // A measure Tier 1 does not follow binding by binding is compared with its start.
        let product = "pub fn f() -> u8 {\n  var i: u8 = 0\n  while i < 9 decreases 20 - 2 * i {\n    i += 1\n  }\n  i\n}";
        assert_eq!(codes(product), ["E0507"]);
        let call = "pub fn dist(i: u8) -> u8\n  requires i <= 100\n  ensures result == 100 - i\n= 100 - i\npub fn f() -> u8 {\n  var i: u8 = 0\n  while i < 100 decreases dist(i) {\n    i += 0\n  }\n  i\n}";
        assert_eq!(
            messages(call),
            ["Tier 1 cannot rule out `i = 0`, after a round of this loop, where its measure `dist(i)` changes by 0: a `decreases` measure gets smaller at every round"]
        );
        let nothing = "pub fn three() -> u8\n  ensures result == 3\n= 3\npub fn f(c: bool) {\n  while c decreases three() {\n  }\n}";
        assert_eq!(
            messages(nothing),
            ["Tier 1 cannot rule out that a round of this loop changes its measure `three()` by 0: a `decreases` measure gets smaller at every round"]
        );
    }

    #[test]
    fn test_a_loop_measure_follows_every_change_of_its_bindings() {
        let base = "pub fn bump(mut x: u8)\n  ensures x > 0\n{\n  x = 1\n}\npub fn ext(mut x: u8) {\n  x = 1\n}\npub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n decreases n - i {\n    i += 1\nCHANGE\n  }\n  i\n}";
        assert_eq!(codes(&base.replace("CHANGE\n", "")), NONE);
        for change in [
            "    bump(mut i)\n",
            "    ext(mut i)\n",
            "    i = i - 1\n",
            "    while i > 0 {\n      i -= 1\n    }\n",
        ] {
            let src = base.replace("CHANGE\n", change);
            assert_eq!(codes(&src), ["E0507"], "{}", change);
        }
        // A binding of the measure that only an inner loop leaves alone keeps its change.
        let kept = base.replace(
            "CHANGE\n",
            "    var k: u8 = 0\n    while k < 3 {\n      k += 1\n    }\n",
        );
        assert_eq!(codes(&kept), NONE);
    }

    #[test]
    fn test_a_negative_measure_gets_a_guard_where_it_can() {
        let neg = "pub fn f(n: i8) -> i8 {\n  var i: i8 = n\n  while i != 0 decreases i {\n    i -= 1\n  }\n  i\n}";
        let d = &diagnostics(neg)[0];
        assert_eq!(d.code, "E0507");
        // The first round can break it, and a guard before the loop says nothing of a later
        // round, where the loop has changed `i`: none is offered, and the message says why.
        assert_eq!(
            d.message,
            "Tier 1 cannot rule out `n = 0, i = -1`, at the head of a round of this loop, where its measure `i` is negative: a `decreases` measure is never negative; this loop changes what it reads, so a `guard` before the loop would not hold at a later round, and none is offered"
        );
        assert!(d.fixes.is_empty(), "{:?}", d.fixes);
        // Only a later round can: the guard ends the body, where the loop reads it as a fact
        // at its head, and proves the measure once applied.
        let later = "pub fn f() -> i8 {\n  var i: i8 = 5\n  while i != 0 decreases i {\n    i -= 2\n  }\n  i\n}";
        assert_eq!(
            diagnostics(later)[0].fixes[0].message,
            "insert `guard i >= 0 else { todo(\"i >= 0 is false here\") }`"
        );
        assert!(repairs(later)
            .contains("    i -= 2\n    guard i >= 0 else { todo(\"i >= 0 is false here\") }\n  }"));
        let none = "pub fn f(c: bool) {\n  while c decreases -1 {\n  }\n}";
        assert_eq!(
            messages(none),
            [
                "Tier 1 cannot rule out that this loop's measure `-1` is negative at the head of a round: a `decreases` measure is never negative",
                "Tier 1 cannot rule out that a round of this loop changes its measure `-1` by 0: a `decreases` measure gets smaller at every round"
            ]
        );
        let shifted = "pub fn f() -> i8 {\n  var i: i8 = 5\n  while i != 0 decreases i + 1 - 1 {\n    i -= 2\n  }\n  i\n}";
        assert!(diagnostics(shifted)[0].fixes[0]
            .message
            .contains("guard i + 1 - 1 >= 0 else"));
        // A measure over what the loop leaves alone keeps the guard before it, which narrows
        // the head too.
        let kept = "pub fn f(n: i8) -> i8 {\n  var i: i8 = 0\n  while i < 3 decreases n {\n    i += 1\n  }\n  i\n}";
        assert!(diagnostics(kept)[0].fixes[0]
            .message
            .contains("guard n >= 0 else"));
    }

    #[test]
    fn test_a_function_measure_is_proven_at_its_calls() {
        let ok = "pub fn r(n: u8) -> u8\n  decreases n\n{\n  if n == 0 {\n    return 0\n  }\n  r(n - 1)\n}";
        assert_eq!(codes(ok), NONE);
        assert_eq!(
            obligations(ok)
                .into_iter()
                .filter(|o| o.0 == Decreases)
                .collect::<Vec<_>>(),
            [(Decreases, true, true), (Decreases, true, true)]
        );
        let same =
            "pub fn r(n: u8) -> u8\n  decreases n\n{\n  if n == 0 {\n    return 0\n  }\n  r(n)\n}";
        let d = &diagnostics(same)[0];
        assert_eq!(d.code, "E0507");
        assert_eq!(
            d.message,
            "Tier 1 cannot rule out `n = 1`, where the measure `n` of `r` is 1 at this recursive call and 1 on entry: a `decreases` measure gets smaller at every recursive call"
        );
        assert!(d.fixes.is_empty());
        let negative = "pub fn down(n: i8) -> i8\n  decreases n\n{\n  if n <= 0 {\n    return 0\n  }\n  down(n - 1)\n}\npub fn start(k: i8) -> i8 = down(k)";
        let d = diagnostics(negative);
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "Tier 1 cannot rule out `k = -1`, where the measure `n` of `down` is negative at this call: a `decreases` measure is never negative"
        );
        assert_eq!(
            repairs(negative),
            module("pub fn down(n: i8) -> i8\n  decreases n\n{\n  if n <= 0 {\n    return 0\n  }\n  down(n - 1)\n}\npub fn start(k: i8) -> i8 {\n  guard k >= 0 else { todo(\"k >= 0 is false here\") }\n  down(k)\n}")
        );
        let constant =
            "pub fn down(n: i8) -> i8\n  decreases n\n= 0\npub fn start() -> i8 = down(-3)";
        assert_eq!(
            messages(constant),
            ["Tier 1 cannot rule out `n = -3`, where the measure `n` of `down` is negative at this call: a `decreases` measure is never negative"]
        );
        let empty = "pub fn never()\n  decreases -1\n{\n}\npub fn f() {\n  never()\n}";
        assert_eq!(
            messages(empty),
            ["Tier 1 cannot rule out that the measure `-1` of `never` is negative at this call: a `decreases` measure is never negative"]
        );
        // Another function's measure only stays non-negative; it need not be smaller.
        let other = "pub fn b(m: u8) -> u8\n  decreases m\n= 0\npub fn a(n: u8) -> u8\n  decreases n\n= b(n + 5)";
        assert_eq!(codes(other), NONE);
        // The caller's measure is its value on entry, before the body changes a `mut` parameter.
        let entry = "pub fn r(mut n: u8) -> u8\n  decreases n\n{\n  if n == 0 {\n    return 0\n  }\n  n = n - 1\n  r(mut n)\n}";
        assert_eq!(codes(entry), NONE);
        let raised = "pub fn r(mut n: u8) -> u8\n  decreases n\n{\n  if n == 0 or n == 255 {\n    return 0\n  }\n  n = n + 1\n  r(mut n)\n}";
        assert_eq!(codes(raised), ["E0507"]);
        let empty_rec = "pub fn r(c: bool)\n  decreases 3\n{\n  if c {\n    r(c)\n  }\n}";
        assert_eq!(
            messages(empty_rec),
            ["Tier 1 cannot rule out that the measure `3` of `r` is 3 at this recursive call and 3 on entry: a `decreases` measure gets smaller at every recursive call"]
        );
    }

    #[test]
    fn test_function_measures_are_lexicographic() {
        // The first part stays and the second gets smaller.
        let second = "pub fn r(a: u8, b: u8) -> u8\n  decreases a\n  decreases b\n{\n  if b == 0 {\n    return a\n  }\n  r(a, b - 1)\n}";
        assert_eq!(codes(second), NONE);
        // Neither gets smaller.
        let neither = "pub fn r(a: u8, b: u8) -> u8\n  decreases a\n  decreases b\n{\n  if b == 0 {\n    return a\n  }\n  r(a, b)\n}";
        assert_eq!(
            messages(neither),
            ["Tier 1 cannot rule out `a = 0, b = 1`, where the measure `a, b` of `r` is (0, 1) at this recursive call and (0, 1) on entry: a `decreases` measure gets smaller at every recursive call"]
        );
        // The first gets smaller while the second grows.
        let first = "pub fn r(a: u8, b: u8) -> u8\n  decreases a\n  decreases b\n{\n  if a == 0 or b > 200 {\n    return b\n  }\n  r(a - 1, b + 10)\n}";
        assert_eq!(codes(&format!("@checked\n{}", first)), NONE);
        // The second gets smaller while the first grows.
        let grows = "pub fn r(a: u8, b: u8) -> u8\n  decreases a\n  decreases b\n{\n  if b == 0 or a > 200 {\n    return a\n  }\n  r(a + 1, b - 1)\n}";
        assert_eq!(codes(&format!("@checked\n{}", grows)), ["E0507"]);
        let constant =
            "pub fn r(c: bool)\n  decreases 3\n  decreases 1\n{\n  if c {\n    r(c)\n  }\n}";
        assert_eq!(
            messages(constant),
            ["Tier 1 cannot rule out that the measure `3, 1` of `r` is (3, 1) at this recursive call and (3, 1) on entry: a `decreases` measure gets smaller at every recursive call"]
        );
    }

    #[test]
    fn test_a_narrowing_that_always_traps_is_an_error() {
        let d = diagnostics("pub fn f(x: u8) -> u8 {\n  val y: u8 = x + 300\n  y\n}");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0508");
        assert_eq!(
            d[0].message,
            "Tier 1 proves `x + 300` is at least 300 here, outside `u8` (0 to 255), so this narrowing always traps"
        );
        assert!(d[0].fixes.is_empty());
        assert_eq!(
            messages("pub fn f(x: u8) -> i8 = x - 400"),
            ["Tier 1 proves `x - 400` is at most -145 here, outside `i8` (-128 to 127), so this narrowing always traps"]
        );
        // A constant near the domain's bound, whose bounds' sums once overflowed the octagon.
        assert_eq!(
            messages("pub fn f() -> usize = 5_i128 - 14761039043817650164173161250659845712_i128"),
            ["Tier 1 proves `5_i128 - 14761039043817650164173161250659845712_i128` is at most -14761039043817650164173161250659845707 here, outside `usize` (0 to 18446744073709551615), so this narrowing always traps"]
        );
        for src in [
            "pub fn f(x: u8) {\n  var y: u8 = 0\n  y = x + 256\n  _ = y\n}",
            "pub fn f(x: u8) -> [2]u8 {\n  var xs: [2]u8 = [0, 0]\n  xs[0] = x + 256\n  xs\n}",
            "pub fn g(a: u8) -> u8 = a\npub fn f(x: u8) -> u8 = g(x + 256)",
            "pub fn f(x: u8) -> u8 {\n  return x + 256\n}",
            "pub fn f(x: u8) -> [2]u8 = [x + 256, 0]",
            "pub fn f() -> u8[0..10] = 300",
            "pub fn f(xs: [2]u16[300..400]) -> [2]u8 = [xs[0], 0]",
        ] {
            assert_eq!(codes(src), ["E0508"], "{}", src);
        }
        // A value that may fit, at either end, and a place no path reaches, are not errors.
        for src in [
            "pub fn f(x: u8) -> u8 = x + 255 - 255",
            "pub fn f(x: u8) -> u8 {\n  val y: u8 = x - 255\n  y\n}",
            "pub fn f(x: u8) -> i16 {\n  val y: u8 = x + 255 - 255\n  y\n}",
            "pub fn f(x: u8) -> u8 {\n  if x > 255 {\n    val y: u8 = x + 300\n    return y\n  }\n  0\n}",
            "pub fn f(x: u128) -> u128 = x + 1",
            "pub fn f(x: u8[1..3]) -> u8 = x << 8 >> 8",
            "pub fn f(x: u8[1..3]) -> u8 = x << 8",
            "pub fn g(a: u8) -> bool = true\npub fn f(x: u8) -> bool = x > 255 and g(x + 300)",
        ] {
            assert!(!codes(src).contains(&"E0508"), "{}", src);
        }
        assert_eq!(codes("pub fn f(x: u8) -> u8 = x + 255"), NONE);
        assert_eq!(
            messages("pub fn f(xs: [2]u16[300..400]) -> [2]u8 = xs"),
            ["Tier 1 proves the value is at least 300 here, outside `u8` (0 to 255), so this narrowing always traps"]
        );
    }

    #[test]
    fn test_run_time_checks_say_what_they_check() {
        let details = |src: &str| -> Vec<(ObligationKind, String)> {
            report(src)
                .obligations
                .into_iter()
                .map(|o| (o.kind, o.detail))
                .collect()
        };
        assert_eq!(
            details("pub fn f(xs: [4]u8, i: usize, d: u8, k: u8) -> u8 = xs[i] / d << k"),
            [
                (Index, "into an array of 4".to_string()),
                (Divisor, String::new()),
                (Shift, "below 8".to_string()),
                (Range, "into `u8`".to_string()),
                (Range, "into `u8`".to_string()),
            ]
        );
        assert_eq!(
            details("pub fn f(k: u8[0..8]) -> i64 = 1 << k"),
            [
                (Shift, String::new()),
                (Range, "into its type".to_string()),
                (Range, "into `i64`".to_string()),
            ]
        );
        assert_eq!(
            details("pub fn f(x: u8) -> u8[0..10]\n  ensures result < 9\n= x")
                .into_iter()
                .filter(|d| d.0 != Range)
                .collect::<Vec<_>>(),
            [(Refinement, String::new()), (Ensures, String::new())]
        );
    }

    #[test]
    fn test_a_prove_function_trusts_the_clauses_outside_the_fragment() {
        let src = "@prove\npub fn half(x: u8) -> u8\n  requires x % 2 == 0\n  requires x < 200\n  ensures result * 2 == x\n= x / 2\n@prove\npub fn g(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n invariant i != 300 invariant i <= n {\n    i += 1\n  }\n  i\n}\n@checked\npub fn h(x: u8)\n  requires x != 3\n{\n}";
        let all: Vec<(u32, u32, String)> = reports(src)
            .into_iter()
            .flat_map(|r| r.axioms)
            .map(|(s, t)| (s.line, s.col, t))
            .collect();
        assert_eq!(
            all,
            [
                (4, 12, "requires x % 2 == 0 of half".to_string()),
                (6, 11, "ensures result * 2 == x of half".to_string()),
                (11, 25, "invariant i != 300 in g".to_string()),
            ]
        );
        assert_eq!(codes(src), NONE);
    }

    #[test]
    fn test_the_header_holds_what_callers_read() {
        let text = module("pub fn f(mut x: u8[0..3], ys: [2]bool) -> u8\n  requires x > 0\n  ensures result < 9\n{\n  x\n}");
        let m = mente_syntax::parse_module(&text).unwrap();
        let f = mente_syntax::parse_bodies(&text, &m)
            .remove(0)
            .syntax
            .unwrap();
        assert_eq!(header(&f), "(u8[0..3] [2]bool) u8 ((> x 0) (< result 9))");
        let text = module("pub fn g() {\n}");
        let m = mente_syntax::parse_module(&text).unwrap();
        let g = mente_syntax::parse_bodies(&text, &m)
            .remove(0)
            .syntax
            .unwrap();
        assert_eq!(header(&g), "()  ()");
        assert_eq!(Routing::of(&[]), Routing::default());
    }

    /// The messages of the fixes of the only diagnostic of `src`, which must be an `E0502`.
    fn fix_messages(src: &str) -> Vec<String> {
        let d = diagnostics(src);
        assert_eq!(codes(src), ["E0502"], "{}", src);
        d[0].fixes.iter().map(|f| f.message.clone()).collect()
    }

    #[test]
    fn test_a_refined_var_is_offered_before_the_guard() {
        let src = "pub fn index_of_max(a: [8]i32) -> usize[0..8] {\n  var best: usize = 0\n  var i: usize = 1\n  while i < 8 {\n    if a[i] > a[best] {\n      best = i\n    }\n    i += 1\n  }\n  best\n}";
        let d = diagnostics(src);
        assert_eq!(codes(src), ["E0502"]);
        assert_eq!(d[0].fixes.len(), 2);
        let fix = &d[0].fixes[0];
        assert_eq!(fix.message, "declare `best` as `usize[0..8]`");
        assert_eq!(fix.applicability, Applicability::MachineApplicable);
        assert!(d[0].fixes[1].message.starts_with("insert `guard best < 8"));
        let text = module(src);
        let applied = apply_fix(&text, fix).unwrap();
        assert!(applied.contains("var best: usize[0..8] = 0"), "{}", applied);
        assert!(check_source(&applied).is_ok(), "{}", applied);
    }

    #[test]
    fn test_the_refined_var_may_be_one_the_value_does_not_read() {
        // `lo`, which the result reads, is tried first and fails at `lo = mid + 1`; `hi`, named
        // with it in the loop's condition, directly or through arithmetic, a negation included,
        // bounds it.
        let search = |cond: &str| {
            format!("pub fn lower_bound(a: [16]i32, x: i32) -> usize[0..=16] {{\n  var lo: usize = 0\n  var hi: usize = 16\n  while {} {{\n    val mid = lo + (hi - lo) / 2\n    if a[mid] < x {{\n      lo = mid + 1\n    }} else {{\n      hi = mid\n    }}\n  }}\n  lo\n}}", cond)
        };
        // A parameter is bounded by a `var` a `guard` compares it with.
        let param = "pub fn f(a: [16]i32, n: usize) -> usize[0..=16] {\n  var hi: usize = 16\n  while hi > 0 and a[hi - 1] > 0 {\n    hi -= 1\n  }\n  guard n <= hi else {\n    return 0\n  }\n  n\n}";
        for src in [
            search("lo < hi"),
            search("hi - lo > 0"),
            search("-hi < -lo"),
            param.to_string(),
        ] {
            assert_eq!(
                fix_messages(&src)[0],
                "declare `hi` as `usize[0..=16]`",
                "{}",
                src
            );
            let applied = apply_fix(&module(&src), &diagnostics(&src)[0].fixes[0]).unwrap();
            assert!(check_source(&applied).is_ok(), "{}", applied);
        }
    }

    #[test]
    fn test_no_refined_var_is_offered_unless_tier1_then_proves_the_function() {
        // Declared `usize[0..8]`, `x` would fail at its own declaration instead.
        let moved = "pub fn f(n: usize) -> usize[0..8] {\n  var x: usize = n\n  x = x / 2\n  x\n}";
        // `n`, a parameter, is never offered: refined, it would only move the obligation to every
        // caller. Declared `usize[0..8]`, `k`, which the loop compares with `n`, fits, but the
        // result still fails.
        let unrelated = "pub fn f(n: usize) -> usize[0..8] {\n  var k: usize = 0\n  while k < n and k < 3 {\n    k += 1\n  }\n  n\n}";
        // With only a `val`, nothing is offered, and a `var` of another integer type is not.
        let val = "pub fn f(n: usize) -> usize[0..8] {\n  val x: usize = n % 9\n  x\n}";
        let other_type = "pub fn f(a: [8]i32) -> usize[0..8] {\n  var best: u32 = 0\n  var i: u32 = 1\n  while i < 8 {\n    if a[i] > a[best] {\n      best = i\n    }\n    i += 1\n  }\n  best\n}";
        // A bound that names a parameter is not offered.
        let named = "pub fn f(a: [8]i32, n: usize[1..=8]) -> usize[0..n] {\n  var best: usize = 0\n  var i: usize = 1\n  while i < n {\n    if a[i] > a[best] {\n      best = i\n    }\n    i += 1\n  }\n  best\n}";
        for src in [moved, unrelated, val, other_type, named] {
            let messages = fix_messages(src);
            assert!(
                messages.iter().all(|m| !m.starts_with("declare")),
                "{}: {:?}",
                src,
                messages
            );
        }
    }
    /// A divisor whose value is empty, after a shift whose narrowing always traps, is no
    /// division by zero in Tier 1 itself: the shift stays a run-time check, and the rest is
    /// checked.
    #[test]
    fn test_a_divisor_after_a_narrowing_that_always_traps_is_checked_without_a_crash() {
        for src in [
            "pub fn f() -> i8 = 7_i8 / (0_u8 - 1 >> 5)",
            "pub fn f() -> i8 = 7_i8 % (0_u8 - 1 >> 5)",
            "pub fn f(x: i8) -> i8 = x / (0_u8 - 1 >> 5)",
        ] {
            assert_eq!(codes(src), NONE, "{}", src);
            assert!(!report(src).obligations.is_empty(), "{}", src);
        }
    }

    /// Every argument of a call is evaluated before any enters its parameter's type, as the
    /// reference interpreter has it: a narrowing inside a later argument comes first, and knows
    /// nothing that an earlier argument's narrowing would establish.
    #[test]
    fn test_arguments_are_narrowed_once_all_are_evaluated() {
        let src = "fn g(a: u16) -> u8 = 0
fn f(a: u8, b: u8) -> u8 = a
pub fn h(x: i8) -> u8 = f(x, g(x))";
        let r = report(src);
        let at = |col: u32| {
            r.obligations
                .iter()
                .filter(|o| o.kind == Range && o.span.line == 4 && o.span.col == col)
                .map(|o| o.proven)
                .collect::<Vec<_>>()
        };
        // `x` into `g`'s `u16` first, which may trap; then `x` into `f`'s `u8`, which
        // the check before makes safe.
        assert_eq!(at(32), [false]);
        assert_eq!(at(27), [true]);
        let structs = "pub struct P {
  a: u128
}
fn take(a: u8, p: P) -> u8 = a
pub fn k(x: i8) -> u8 = take(x, P(a: x))";
        let r = report(structs);
        let field: Vec<bool> = r
            .obligations
            .iter()
            .filter(|o| o.kind == Range && o.span.line == 6 && o.span.col == 38)
            .map(|o| o.proven)
            .collect();
        assert_eq!(field, [false]);
    }

    /// Tier 1 follows a scalar field path of a local binding or a parameter, nested ones
    /// included, as it follows a local: a store and a struct literal set it, a copy of the
    /// struct copies it, a whole-struct assignment resets it, all of a literal's values read as
    /// they were before any lands, and a call that takes the struct `mut` leaves it its type
    /// and refinement alone. An element of an array of structs stays unfollowed.
    #[test]
    fn test_field_paths_are_followed_as_locals() {
        let decls = "pub struct P {\n  a: u8\n  b: u8\n}\npub struct O {\n  p: P\n  w: u8\n}\nfn touch(mut p: P) {\n  p.a = 7\n}\n";
        let with = |f: &str| format!("{}{}", decls, f);
        for f in [
            // A store, then a read.
            "pub fn f(mut p: P) -> u8[0..1] {\n  p.a = 0\n  p.a\n}",
            // A struct literal sets each path, from a refined value.
            "pub fn f(x: u8[0..5]) -> u8[0..5] {\n  val p = P(a: x, b: 2)\n  p.a\n}",
            // A nested path.
            "pub fn f(mut o: O) -> u8[3..4] {\n  o.p.b = 3\n  o.p.b\n}",
            "pub fn f(x: u8[1..2]) -> u8[1..2] {\n  val o = O(p: P(a: x, b: 0), w: 0)\n  o.p.a\n}",
            // A copy of the struct copies its paths, as a relation.
            "pub fn f(mut p: P) -> u8[3..4] {\n  p.a = 3\n  val q = p\n  q.a\n}",
            // A struct field stored whole.
            "pub fn f(mut o: O) -> u8[4..5] {\n  o.p = P(a: 4, b: 4)\n  o.p.a\n}",
            // A swap reads each value as it was.
            "pub fn f(mut p: P) -> u8[2..3] {\n  p.a = 1\n  p.b = 2\n  p = P(a: p.b, b: p.a)\n  p.a\n}",
            "pub fn f(mut p: P) -> u8[1..2] {\n  p.a = 1\n  p.b = 2\n  p = P(a: p.b, b: p.a)\n  p.b\n}",
            // A compound store.
            "pub fn f(mut p: P) -> u8[6..7] {\n  p.a = 5\n  p.a += 1\n  p.a\n}",
            // A quotient that reads a path another lands in first reads it as it was.
            "pub fn f(mut p: P) -> u8[2..3] {\n  p.a = 4\n  p.b = 8\n  p = P(a: p.b, b: p.a / 2)\n  p.b\n}",
            // A value that reads its own path keeps its relation to it.
            "pub fn f(mut p: P, x: u8[0..100]) -> u8[1..2] {\n  p.a = x\n  p = P(a: p.a + 1, b: 0)\n  p.a - x\n}",
            // A `requires` over a `mut` parameter's field reads the argument's at the call, and
            // an `ensures` over a read-only one's reads it after.
            "fn need(mut q: P)\n  requires q.a > 2\n{\n  q.a = 0\n}\npub fn f(x: u8[3..9]) {\n  var q = P(a: x, b: 0)\n  need(mut q)\n}",
            "fn get(q: P) -> u8\n  ensures result == q.a\n= q.a\npub fn f(x: u8[3..4]) -> u8[3..4] = get(P(a: x, b: 0))",
            // A clause over a nested path reads the argument's.
            "fn deep(o: O) -> u8\n  requires o.p.a > 2\n= o.p.a\npub fn f(x: u8[3..9]) -> u8 = deep(O(p: P(a: x, b: 0), w: 0))",
            // An array field's `.len` is a constant in a clause.
            "pub struct B {\n  xs: [3]u8\n}\npub fn f(b: B) -> u8\n  requires b.xs.len == 3\n= b.xs[2]",
            // A literal's earlier value that a later one changes reads as it was.
            "pub fn f(mut p: P) -> u8[0..1] {\n  p.a = 0\n  val q = P(a: p.a, b: bump(mut p))\n  q.a\n}\nfn bump(mut p: P) -> u8 {\n  p.a = 9\n  0\n}",
        ] {
            assert_eq!(codes(&with(f)), NONE, "{}", f);
        }
        for f in [
            // A whole-struct assignment resets it.
            "pub fn f(mut p: P, q: P) -> u8[0..1] {\n  p.a = 0\n  p = q\n  p.a\n}",
            // A call that takes the struct `mut` leaves its type alone.
            "pub fn f(mut p: P) -> u8[0..1] {\n  p.a = 0\n  touch(mut p)\n  p.a\n}",
            "pub fn f(mut o: O) -> u8[0..1] {\n  o.p.a = 0\n  touch(mut o.p)\n  o.p.a\n}",
            // A loop that changes it leaves it its type at the loop's head, a call inside a struct
            // literal included.
            "pub fn f(mut p: P) -> u8[0..1] {\n  p.a = 0\n  var i: u8 = 0\n  while i < 3 {\n    p.a = 5\n    i += 1\n  }\n  p.a\n}",
            "pub fn f(mut p: P) -> u8[0..1] {\n  p.a = 0\n  var i: u8 = 0\n  while i < 3 {\n    _ = O(p: P(a: 0, b: bump(mut p)), w: 0)\n    i += 1\n  }\n  p.a\n}\nfn bump(mut p: P) -> u8 {\n  p.a = 9\n  0\n}",
            // An element of an array of structs is not followed.
            "pub fn f(mut ps: [2]P) -> u8[0..1] {\n  ps[0].a = 0\n  ps[0].a\n}",
        ] {
            assert_eq!(codes(&with(f)), ["E0502"], "{}", f);
        }
        assert_eq!(
            messages(&with("pub fn f(mut p: P) -> u8[0..1] {\n  p.a = 0\n  touch(mut p)\n  p.a\n}")),
            ["Tier 1 cannot rule out `p.a = 1, p.b = 0`, outside `u8[0..1]`, the type of the result of `f`"]
        );
        // A value of paths in scope that is not one names them; one Tier 1 reads afresh, an
        // array element's field, is named alone though a struct is in scope.
        assert_eq!(
            messages(&with("pub fn f(mut p: P) -> u8[0..1] {\n  p.a = 0\n  touch(mut p)\n  p.a + 0\n}")),
            ["Tier 1 cannot rule out `p.a = 1, p.b = 0`, where `p.a + 0` is 1, outside `u8[0..1]`, the type of the result of `f`"]
        );
        assert_eq!(
            messages(&with("pub fn f(mut ps: [2]P, q: P) -> u8[0..1] {\n  ps[0].a = 0\n  ps[0].a\n}")),
            ["Tier 1 cannot rule out that `ps[0].a` is 1, outside `u8[0..1]`, the type of the result of `f`"]
        );
        // A binding's paths are followed only where, with them, the function stays within the
        // cap: here the result, `x`, `y`, the two field reads, the temporaries, the scratch and
        // the probe take 9 variables, so 119 fields fit, the function staying relational, and
        // 120 do not, and a binding past it is not followed, as an element is.
        let wide = |n: usize| {
            let fields: String = (0..n)
                .map(|k| {
                    format!(
                        "  f{}: u8
",
                        k
                    )
                })
                .collect();
            format!(
                "pub struct W {{
{}}}
",
                fields
            )
        };
        let edge = "pub fn f(mut w: W, x: u8, y: u8) -> u8[1..=255] {
  w.f0 = 0
  guard x < y else {
    return 1
  }
  y - x + w.f0
}";
        assert_eq!(codes(&format!("{}{}", wide(119), edge)), NONE);
        assert_eq!(codes(&format!("{}{}", wide(120), edge)), ["E0502"]);
        // Bindings share what is left, 120 here: after a struct of 2 fields, one of 119 does not
        // fit, though it would alone.
        let two = "pub struct V {
  a: u8
  b: u8
}
pub fn f(mut p: V, mut q: W) -> u8[0..1] {
  p.a = 0
  q.f0 = 0
  q.f0
}";
        assert_eq!(codes(&format!("{}{}", wide(119), two)), ["E0502"]);
    }

    /// The field paths of a struct are listed no further than the room they may take: structs
    /// nested level by level, whose paths double with each level, are given up on once the room
    /// is used, so a struct of 2^41 paths costs as much as one of 128.
    #[test]
    fn test_field_paths_are_listed_no_further_than_the_cap() {
        let mut src = String::from("module t\npub struct S0 {\n  a: u8\n  b: u8\n}\n");
        for k in 1..=40 {
            src.push_str(&format!(
                "pub struct S{} {{\n  l: S{}\n  r: S{}\n}}\n",
                k,
                k - 1,
                k - 1
            ));
        }
        let m = mente_syntax::parse_module(&src).unwrap();
        let parsed = mente_syntax::parse_structs(&src, &m);
        let (structs, _) = crate::structs::declare(&m, &parsed, &[]);
        let mut room = RELATIONAL_LIMIT;
        let paths = scalar_paths(&structs, "S5", &mut room).unwrap();
        assert_eq!((paths.len(), room), (64, RELATIONAL_LIMIT - 64));
        assert_eq!(paths[1].0, ["l", "l", "l", "l", "l", "b"]);
        let mut room = RELATIONAL_LIMIT;
        assert!(scalar_paths(&structs, "S40", &mut room).is_none());
        assert_eq!(room, 0);
        // A function of such bindings, of 2^21 paths each within the size a value may have,
        // follows none of them, and checks.
        let upto = src.find("pub struct S21").unwrap();
        let path = format!("u{}.a", ".l".repeat(20));
        let deep = format!(
            "{}pub fn f(s: S20, t: S20) -> u8[0..1] {{\n  var u = s\n  u = t\n  {}\n}}\n",
            &src["module t\n".len()..upto],
            path
        );
        assert_eq!(
            messages(&deep),
            [format!(
                "Tier 1 cannot rule out that `{}` is 1, outside `u8[0..1]`, the type of the result of `f`",
                path
            )]
        );
    }

    /// A measure over a field path proves where it gets smaller: a loop's `decreases k.n` with
    /// `k.n -= 1` each round, and a recursion's over a `u32` field. One that fails names a
    /// valuation, and offers a `guard` only where Tier 1 follows what it reads: at a call
    /// whose argument is a struct literal, the field's value, not the literal's field.
    #[test]
    fn test_measures_over_field_paths() {
        let decls = "pub struct C {\n  n: u32\n  s: i32\n}\nfn next(c: C) -> C = c\n";
        let with = |f: &str| format!("{}{}", decls, f);
        for f in [
            "pub fn f(c: C) -> u32 {\n  var k = c\n  while k.n > 0\n    decreases k.n\n  {\n    k.n -= 1\n  }\n  k.n\n}",
            "pub fn f(c: C) -> u32\n  decreases c.n\n{\n  if c.n == 0 {\n    return 0\n  }\n  f(C(n: c.n - 1, s: c.s))\n}",
        ] {
            assert_eq!(codes(&with(f)), NONE, "{}", f);
        }
        let negative = with("pub fn f(c: C) -> u32\n  decreases c.s\n{\n  if c.s == 0 {\n    return 0\n  }\n  f(C(n: c.n, s: c.s - 1))\n}");
        let d = diagnostics(&negative);
        assert_eq!(d.len(), 1, "{:?}", d);
        assert_eq!(
            d[0].message,
            "Tier 1 cannot rule out `c.n = 0, c.s = 0`, where the measure `c.s` of `f` is negative at this call: a `decreases` measure is never negative"
        );
        assert!(
            d[0].fixes[0].edits[0]
                .replacement
                .contains("guard c.s - 1 >= 0 else"),
            "{}",
            d[0].fixes[0].edits[0].replacement
        );
        // Applied, the guard proves it.
        let src = module(&negative);
        let fixed = mente_syntax::diagnostics::apply_fix(&src, &d[0].fixes[0]).unwrap();
        assert!(crate::check_source(&fixed).is_ok(), "{}", fixed);
        // Of a call's result, which Tier 1 does not follow: no guard, and the message says so.
        let opaque = with("pub fn f(c: C) -> u32\n  decreases c.s\n{\n  if c.s == 0 {\n    return 0\n  }\n  f(next(c))\n}");
        let d = diagnostics(&opaque);
        assert_eq!(
            d.iter().map(|d| d.code).collect::<Vec<_>>(),
            ["E0507", "E0507"]
        );
        assert!(d[0].fixes.is_empty(), "{:?}", d[0].fixes);
        assert!(
            d[0].message.ends_with("at this call it is `next(c).s`, which reads a value Tier 1 does not follow, so no `guard` it can use states it"),
            "{}",
            d[0].message
        );
        // A loop's measure over an element's field: no guard either.
        let element = with("pub fn f(mut cs: [1]C) -> u32 {\n  while cs[0].s > 0\n    decreases cs[0].s\n  {\n    cs[0].s -= 1\n  }\n  0\n}");
        let d = diagnostics(&element);
        assert!(d.iter().all(|d| d.code == "E0507"), "{:?}", d);
        assert!(d.iter().all(|d| d.fixes.is_empty()), "{:?}", d);
    }

    /// A failed `requires` or `ensures` offers a `guard` in the terms of the call or the return
    /// only where Tier 1 follows every value it reads there, a struct literal's field value, a
    /// followed path or an array's `.len`, and that guard, applied, proves it; otherwise the
    /// message says what the clause reads there.
    #[test]
    fn test_clauses_offer_only_guards_tier1_can_use() {
        let decls = "pub struct O {\n  w: u8\n}\nfn mk(x: u8) -> O = O(w: x)\nfn needs(i: O) -> u8\n  requires i.w > 2\n= i.w\nfn below(i: O) -> u8\n  requires -i.w < -2\n= i.w\nfn lens(xs: [3]u8) -> u8\n  requires xs.len > 3\n= 0\n";
        let with = |f: &str| format!("{}{}", decls, f);
        // What Tier 1 follows proves the clause where it holds.
        assert_eq!(
            codes(&with("pub fn f(x: u8[3..9]) -> u8 = below(O(w: x))")),
            NONE
        );
        for (f, guard) in [
            (
                "pub fn f(x: u8, y: u8) -> O\n  ensures result.w == x\n= O(w: y)",
                "y == x",
            ),
            (
                "pub fn f(i: O) -> O\n  ensures result.w > 2\n= i",
                "i.w > 2",
            ),
            ("pub fn f(x: u8) -> u8 = needs(O(w: x))", "x > 2"),
            (
                "pub fn f(x: u8[0..9]) -> u8 = needs(O(w: x + 1))",
                "x + 1 > 2",
            ),
            ("pub fn f(i: O) -> u8 = needs(i)", "i.w > 2"),
            ("pub fn f(x: u8) -> u8 = below(O(w: x))", "-x < -2"),
            ("pub fn f(ys: [3]u8) -> u8 = lens(ys)", "ys.len > 3"),
            (
                "pub fn f(xs: [3]u8) -> u8\n  ensures result == xs.len\n= 0",
                "0 == xs.len",
            ),
        ] {
            let src = with(f);
            let d = diagnostics(&src);
            assert_eq!(d.len(), 1, "{}: {:?}", f, d);
            assert!(
                d[0].fixes[0]
                    .message
                    .contains(&format!("guard {} else", guard)),
                "{}: {}",
                f,
                d[0].fixes[0].message
            );
            let fixed =
                mente_syntax::diagnostics::apply_fix(&module(&src), &d[0].fixes[0]).unwrap();
            assert!(crate::check_source(&fixed).is_ok(), "{}", fixed);
        }
        for (f, there) in [
            (
                "pub fn f(x: u8) -> u8 = needs(mk(x))",
                "at this call the clause is `mk(x).w > 2`",
            ),
            (
                "pub fn f(xs: [2]O) -> u8 = needs(xs[0])",
                "at this call the clause is `xs[0].w > 2`",
            ),
            (
                "pub fn f(x: u8) -> u8 = needs(O(w: x * 2))",
                "at this call the clause is `x * 2 > 2`",
            ),
            (
                "pub fn f(x: u8) -> O\n  ensures result.w == x\n= mk(x)",
                "at its end the clause is `mk(x).w == x`",
            ),
            (
                "pub fn f(x: u8) -> u8\n  ensures result == x\n= mk(x).w",
                "at its end the clause is `mk(x).w == x`",
            ),
            (
                "pub fn f(x: u8[0..9]) -> u8 = needs(O(w: x + mk(x).w))",
                "at this call the clause is `x + mk(x).w > 2`",
            ),
            (
                "pub struct L {\n  len: u8\n}\npub fn f(ls: [1]L) -> u8 = needs(O(w: ls[0].len))",
                "at this call the clause is `ls[0].len > 2`",
            ),
        ] {
            let d = diagnostics(&with(f));
            assert_eq!(d.len(), 1, "{}: {:?}", f, d);
            assert!(d[0].fixes.is_empty(), "{}: {:?}", f, d[0].fixes);
            assert!(
                d[0].message.ends_with(&format!(
                    "; {}, which reads a value Tier 1 does not follow, so no `guard` it can use states it",
                    there
                )),
                "{}: {}",
                f,
                d[0].message
            );
        }
    }

    /// A value entering a refined field is proven where the struct is built and where the field
    /// is stored, a `mut` field argument's on its way back too; a refined field's range is a
    /// fact wherever it is read, of a field of a field and of an element's field too.
    #[test]
    fn test_refined_fields_are_proven_where_they_enter_and_known_where_read() {
        let d = "pub struct D {
  d: u8[0..10]
}
";
        let with = |f: &str| format!("{}{}", d, f);
        assert_eq!(codes(&with("pub fn f(x: u8) -> D = D(d: x)")), ["E0502"]);
        assert!(messages(&with("pub fn f(x: u8) -> D = D(d: x)"))[0]
            .ends_with("outside `u8[0..10]`, the type of field `d` of `D`"));
        assert_eq!(codes(&with("pub fn f(x: u8[0..10]) -> D = D(d: x)")), NONE);
        assert_eq!(
            codes(&with(
                "pub fn f(x: u8) -> D {
  guard x < 10 else {
    return D(d: 0)
  }
  D(d: x)
}"
            )),
            NONE
        );
        assert_eq!(
            codes(&with(
                "pub fn f(mut g: D, x: u8) {
  g.d = x
}"
            )),
            ["E0502"]
        );
        assert_eq!(
            codes(&with(
                "pub fn f(mut g: D) {
  g.d = 9
}"
            )),
            NONE
        );
        // A refined field of a struct inside another, and of a struct in an array inside
        // another, is proven where it is stored through the outer one.
        let nested = "pub struct O {
  inner: D
  ds: [2]D
}
";
        let with_o = |f: &str| format!("{}{}{}", d, nested, f);
        for store in ["o.inner.d = x", "o.ds[1].d = x"] {
            let f = format!("pub fn f(mut o: O, x: u8) {{\n  {}\n}}", store);
            assert_eq!(codes(&with_o(&f)), ["E0502"], "{}", store);
            let f = format!(
                "pub fn f(mut o: O, x: u8) {{\n  guard x < 10 else {{\n    return\n  }}\n  {}\n}}",
                store
            );
            assert_eq!(codes(&with_o(&f)), NONE, "{}", store);
        }
        assert_eq!(
            codes(&with("pub fn f(g: D) -> u8[0..19] = g.d + g.d")),
            NONE
        );
        assert_eq!(codes(&with("pub fn f(g: D) -> u8[0..9] = g.d")), ["E0502"]);
        let e = format!(
            "{}pub struct E {{
  inner: D
  xs: [2]D
}}
",
            d
        );
        assert_eq!(
            codes(&format!("{}pub fn f(e: E) -> u8[0..10] = e.inner.d", e)),
            NONE
        );
        assert_eq!(
            codes(&format!(
                "{}pub fn f(e: E, i: usize) -> u8[0..10] = e.xs[i].d",
                e
            )),
            NONE
        );
        // A `mut` field argument is proven on its way back into the field's refinement.
        assert_eq!(
            codes(&with(
                "fn bump(mut n: u8) {
  n += 1
}
pub fn f(mut g: D) {
  bump(mut g.d)
}"
            )),
            ["E0502"]
        );
        assert_eq!(
            codes(&with(
                "fn zero(mut n: u8[0..10]) {
  n = 0
}
pub fn f(mut g: D) {
  zero(mut g.d)
}"
            )),
            NONE
        );
        // A narrowing into a field that always traps, and a field path Tier 1 follows from a
        // store to a read.
        assert_eq!(
            codes(
                "pub struct B {
  b: u8
}
pub fn f() -> B = B(b: 300)"
            ),
            ["E0508"]
        );
        assert_eq!(
            codes(&with(
                "pub fn f(mut g: D) -> u8[0..1] {
  g.d = 0
  g.d
}"
            )),
            NONE
        );
        // A clause over an integer field path lies in the Tier-1 fragment, a field named `len`
        // too, which is no array's length; one through an array's element does not.
        for (field, decl) in [("d", ""), ("len", "pub struct L {\n  len: u8\n}\n")] {
            let src = format!(
                "{}pub fn f(g: {}) -> u8\n  requires g.{} > 2\n= g.{}",
                decl,
                if decl.is_empty() { "D" } else { "L" },
                field,
                field
            );
            assert_eq!(codes(&with(&src)), NONE, "{}", src);
        }
        let src = with("pub fn f(gs: [2]D) -> u8\n  requires gs[0].d > 2\n= gs[0].d");
        assert_eq!(codes(&src), ["E0503"]);
        assert!(
            messages(&src)[0].contains("a field of a value that is not a binding"),
            "{}",
            messages(&src)[0]
        );
    }
}

#[cfg(test)]
mod timing {
    use std::time::Instant;

    use super::*;
    use crate::check::Signature;

    /// Prints how long Tier 1 takes on one function of the Tier-1 spike's shape: two loops
    /// over an array, with indexing and loop bounds, then a call whose `requires` it proves.
    #[test]
    #[ignore]
    fn tier1_cost_sketch() {
        let text = "module m\npub fn prev(a: [64]i32, lo: usize[0..64], hi: usize[0..=64]) -> i64\n  requires lo < hi\n{\n  0\n}\npub fn f(a: [64]i32, lo: usize[0..64], hi: usize[0..=64]) -> i64\n  requires lo < hi\n{\n  var i: usize = lo\n  var s: i64 = 0\n  while i < hi invariant lo <= i {\n    s += a[i]\n    i += 1\n  }\n  var j: usize = lo\n  while j < hi {\n    guard a[j] >= 0 else {\n      break\n    }\n    s += a[j] / 2\n    j += 1\n  }\n  s + prev(a, lo, hi)\n}\n";
        let m = mente_syntax::parse_module(text).unwrap();
        let parsed = mente_syntax::parse_bodies(text, &m);
        let names: Vec<String> = parsed.iter().map(|p| p.name.clone()).collect();
        let signatures: Vec<Option<Signature>> = parsed
            .iter()
            .map(|p| p.syntax.as_ref().map(Signature::of))
            .collect();
        let callees: Vec<Option<Callee>> = parsed
            .iter()
            .map(|p| p.syntax.as_ref().map(|syntax| Callee { syntax }))
            .collect();
        let syntax = parsed[1].syntax.as_ref().unwrap();
        let structs = crate::structs::Structs::default();
        let (resolution, _) = crate::names::resolve(syntax, &names, &structs);
        let (typing, _) = crate::check::check(syntax, &resolution, &signatures, &structs, text);
        let f = Function {
            syntax,
            resolution: &resolution,
            typing: &typing,
            routing: Routing::default(),
            structs: &structs,
        };
        let first = check(&f, &callees, text);
        assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
        let mut times = Vec::new();
        for _ in 0..500 {
            let t = Instant::now();
            let r = check(&f, &callees, text);
            times.push(t.elapsed().as_micros());
            assert_eq!(r.obligations.len(), first.obligations.len());
        }
        times.sort_unstable();
        println!(
            "Tier 1 on one function of {} obligations: median {} us, p99 {} us",
            first.obligations.len(),
            times[times.len() / 2],
            times[times.len() * 99 / 100]
        );
    }
}
