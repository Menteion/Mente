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
//! [`tier1-counterexample`]: ../../docs/design-review.md#tier1-counterexample

use mente_syntax::diagnostics::{line_col, Applicability, Diagnostic, Edit, Fix};
use mente_syntax::tree::{
    Arg, AssignOp, BinaryOp, BlockId, Body, Bound, ContractKind, Else, ExprId, ExprKind, FnSyntax,
    Refinement, StmtId, StmtKind, Tree, TypeId, TypeKind, UnaryOp,
};
use mente_syntax::{Attr, Span};

use crate::check::Typing;
use crate::names::{BindingKind, Resolution};
use crate::octagon::{Octagon, Term, BIG, INF};
use crate::types::{Int, IntTy, Ty};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    pub kind: ObligationKind,
    pub span: Span,
    pub must: bool,
    pub proven: bool,
    /// What a run-time check checks against, as `mn view --checks` prints it: `into `u8``,
    /// `into an array of 4`, `below 8`; empty when there is nothing to say.
    pub detail: String,
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
    pub len_of: &'a dyn Fn(&str) -> Option<u128>,
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
        ExprKind::Field { base, name } => match &tree.expr(*base).kind {
            ExprKind::Name(n) => Ok(match (names.len_of)(n) {
                Some(len) => Form {
                    vars: Vec::new(),
                    constant: i128::try_from(len).unwrap_or(i128::MAX),
                },
                None => var(format!("{}.{}", n, name)),
            }),
            _ => Err("a field of a value that is not a binding".to_string()),
        },
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
        _ => Err("an expression that is not linear arithmetic".to_string()),
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
        _ => Some("a value that is not a comparison".to_string()),
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
        other => other.clone(),
    }
}

fn as_const(v: &Val) -> Option<i128> {
    match v {
        Val::Lin(t, c) if t.is_empty() => Some(*c),
        _ => None,
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
        _ => (-INF, INF),
    }
}

/// A value with no reference to a variable: an interval, an array of them, or `Other`.
fn summary_of(st: &Octagon, v: &Val) -> Val {
    match v {
        Val::Arr(_) | Val::Other => v.clone(),
        _ => {
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
fn int_range(t: IntTy) -> (i128, i128) {
    if t.bits == 128 {
        return (-INF, INF);
    }
    (-(t.min_magnitude() as i128), t.max() as i128)
}

fn int_of(t: &Ty) -> Option<IntTy> {
    match t {
        Ty::Int(Int::One(t)) => Some(*t),
        _ => None,
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
        _ => TOP,
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
    Other,
}

impl Shape {
    fn plain(t: &Ty) -> Shape {
        match t {
            Ty::Int(_) => Shape::Int {
                int: int_of(t),
                refine: None,
            },
            Ty::Array(n, e) => Shape::Array(*n, Box::new(Shape::plain(e))),
            _ => Shape::Other,
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
            Shape::Other => Shape::Other,
        }
    }

    fn element(&self) -> Shape {
        match self {
            Shape::Array(_, e) => (**e).clone(),
            _ => Shape::Other,
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

/// A condition whose operands are evaluated.
#[derive(Clone, Debug)]
enum Cond {
    Const(bool),
    Cmp(BinaryOp, Box<Val>, Box<Val>),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    Not(Box<Cond>),
    Unknown,
}

fn cmp(op: BinaryOp, l: Val, r: Val) -> Cond {
    Cond::Cmp(op, Box::new(l), Box::new(r))
}

/// `a < b` in lexicographic order, for two measures of one length: the last pair compares
/// with `false` after it.
fn lex_less(a: &[Val], b: &[Val]) -> Cond {
    match (a, b) {
        ([x, xs @ ..], [y, ys @ ..]) => Cond::Or(
            Box::new(cmp(BinaryOp::Lt, x.clone(), y.clone())),
            Box::new(Cond::And(
                Box::new(cmp(BinaryOp::Eq, x.clone(), y.clone())),
                Box::new(lex_less(xs, ys)),
            )),
        ),
        _ => Cond::Const(false),
    }
}

/// The values of a measure as a message writes them: `3`, or `(3, 0)` for several.
fn tuple(values: &[i128]) -> String {
    let parts: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    match parts.as_slice() {
        [one] => one.clone(),
        _ => format!("({})", parts.join(", ")),
    }
}

fn flip(op: BinaryOp) -> BinaryOp {
    match op {
        BinaryOp::Lt => BinaryOp::Ge,
        BinaryOp::Le => BinaryOp::Gt,
        BinaryOp::Gt => BinaryOp::Le,
        BinaryOp::Ge => BinaryOp::Lt,
        BinaryOp::Eq => BinaryOp::Ne,
        _ => BinaryOp::Eq,
    }
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
        ExprKind::If { .. } => 0,
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
        _ => 12,
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
        ExprKind::Field { base, name } => format!("{}.{}", at(*base, 12), name),
        ExprKind::Call { callee, args } => {
            let args: Vec<String> = args
                .iter()
                .map(|a| format!("{}{}", if a.is_mut { "mut " } else { "" }, at(a.value, 0)))
                .collect();
            format!("{}({})", callee, args.join(", "))
        }
        ExprKind::Array(items) => {
            let items: Vec<String> = items.iter().map(|i| at(*i, 0)).collect();
            format!("[{}]", items.join(", "))
        }
        ExprKind::Repeat { value, count } => format!("[{}; {}]", at(*value, 0), count),
        ExprKind::Todo(r) => format!("todo({:?})", r),
        ExprKind::Unreachable(r) => format!("unreachable({:?})", r),
        ExprKind::If { cond, .. } => format!("if {} {{ ... }}", at(*cond, 0)),
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
    callees: &'a [Option<Callee<'a>>],
    src: &'a str,
    /// The octagon variable of each integer binding.
    vars: Vec<Option<usize>>,
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
    let (mut report, retries) = run(f, callees, src);
    for r in &retries {
        if let Some(fix) = refined_var_fix(f, callees, src, &report.diagnostics, r) {
            report.diagnostics[r.diagnostic].fixes.insert(0, fix);
        }
    }
    report.diagnostics.sort_by_key(|d| d.primary.start);
    report
}

/// One pass of Tier 1 over a function, its diagnostics in the order it found them.
fn run(f: &Function, callees: &[Option<Callee>], src: &str) -> (Report, Vec<Retry>) {
    let mut fx = Fx::new(f, callees, src);
    fx.route();
    let mut st = Octagon::new(fx.count);
    fx.entry(&mut st);
    fx.body(&mut st);
    (fx.report, fx.retries)
}

/// The fix that declares a `var` refined, offered before the `guard` for the `E0502` of `r`:
/// Tier 1 checks the function again with each candidate declared so, the `var` the value reads
/// first and then every other `var` of the same integer type declared without a refinement, in
/// order, and offers the first after which that obligation holds and no other fails. It runs only
/// once an obligation has failed, so code that proves pays nothing for it.
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
    let others = (0..bindings.len()).filter(|b| Some(*b) != r.read);
    for b in r.read.into_iter().chain(others) {
        let Some(t) = bindings[b]
            .ty
            .filter(|_| bindings[b].kind == BindingKind::Var)
        else {
            continue;
        };
        let tree = &f.syntax.tree;
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
        let after = run(&g, callees, src).0.diagnostics;
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

fn is_int_type(tree: &Tree, t: TypeId) -> bool {
    matches!(tree.ty(t).kind, TypeKind::Int { .. })
}

impl<'a> Fx<'a> {
    fn new(f: &Function<'a>, callees: &'a [Option<Callee<'a>>], src: &'a str) -> Fx<'a> {
        let tree = &f.syntax.tree;
        let mut count = 0;
        let mut alloc = |wanted: bool| {
            count += usize::from(wanted);
            wanted.then(|| count - 1)
        };
        let result = alloc(f.syntax.result.is_some_and(|t| is_int_type(tree, t)));
        let vars = f
            .resolution
            .bindings
            .iter()
            .zip(&f.typing.bindings)
            .map(|(b, t)| match b.kind {
                BindingKind::Result => result,
                _ => alloc(matches!(t, Ty::Int(_))),
            })
            .collect();
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
                                        .map(|p| alloc(is_int_type(ct, p.ty)))
                                        .collect(),
                                    outs: params
                                        .iter()
                                        .map(|p| alloc(p.is_mut && is_int_type(ct, p.ty)))
                                        .collect(),
                                    result: alloc(
                                        c.syntax.result.is_some_and(|t| is_int_type(ct, t)),
                                    ),
                                })
                            }
                            None => Site::None,
                        }
                    }
                    ExprKind::If { .. }
                    | ExprKind::Index { .. }
                    | ExprKind::Binary {
                        op: BinaryOp::Rem, ..
                    } if int => Site::Value(alloc(true).unwrap_or(0)),
                    _ => Site::None,
                }
            })
            .collect();
        let temps = [alloc(true).unwrap_or(0), alloc(true).unwrap_or(0)];
        let scratch = alloc(true).unwrap_or(0);
        let probe = alloc(true).unwrap_or(0);
        let measures = tree
            .stmts
            .iter()
            .map(|s| match &s.kind {
                StmtKind::While { decreases, .. } => decreases
                    .iter()
                    .map(|_| (alloc(true).unwrap_or(0), alloc(true).unwrap_or(0)))
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        let entry_measures = f
            .syntax
            .contracts
            .iter()
            .filter(|c| c.kind == ContractKind::Decreases)
            .map(|_| (alloc(true).unwrap_or(0), alloc(true).unwrap_or(0)))
            .collect();
        let mut fx = Fx {
            syntax: f.syntax,
            tree,
            resolution: f.resolution,
            typing: f.typing,
            routing: f.routing,
            callees,
            src,
            vars,
            sites,
            result,
            result_shape: None,
            temps,
            scratch,
            probe,
            count,
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
        };
        let shape = f.syntax.result.map(|t| fx.own_shape(t));
        fx.result_shape = shape;
        fx
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
                    })
                }),
            },
            TypeKind::Array { len, elem } => Shape::Array(*len, Box::new(self.own_shape(*elem))),
            _ => Shape::Other,
        }
    }

    fn binding_shape(&self, b: usize) -> Shape {
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
            _ => None,
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
                    })
                }),
            },
            TypeKind::Array { len, elem } => {
                Shape::Array(*len, Box::new(self.callee_shape(c, site, args, *elem)))
            }
            _ => Shape::Other,
        }
    }

    /// An expression of this function as written, with how tightly it binds.
    fn own_text(&self, e: ExprId) -> (String, u8) {
        let expr = self.tree.expr(e);
        match self.src.get(expr.span.start..expr.span.end) {
            Some(text) => (text.to_string(), prec(&expr.kind)),
            None => print(self.tree, e, &no_subst),
        }
    }

    fn own_len(&self, name: &str) -> Option<u128> {
        self.resolution
            .bindings
            .iter()
            .zip(&self.typing.bindings)
            .find_map(|(b, t)| match t {
                Ty::Array(len, _) if b.name == name => Some(*len),
                _ => None,
            })
    }

    fn own_int(&self, name: &str) -> bool {
        self.resolution
            .bindings
            .iter()
            .zip(&self.typing.bindings)
            .any(|(b, t)| b.name == name && matches!(t, Ty::Int(_)))
    }

    fn outside(&self, e: ExprId) -> Option<String> {
        let names = Names {
            is_int: &|n| self.own_int(n),
            len_of: &|n| self.own_len(n),
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
        let len_of = |n: &str| {
            let p = s.params.iter().find(|p| p.name == n)?;
            match s.tree.ty(p.ty).kind {
                TypeKind::Array { len, .. } => Some(len),
                _ => None,
            }
        };
        let names = Names {
            is_int: &is_int,
            len_of: &len_of,
        };
        outside_fragment(&s.tree, e, &names)
    }

    fn span(&self, e: ExprId) -> Span {
        self.tree.expr(e).span
    }

    // -- diagnostics -------------------------------------------------------------------------

    fn record(&mut self, kind: ObligationKind, span: Span, must: bool, proven: bool) {
        self.record_check(kind, span, must, proven, String::new());
    }

    fn record_check(
        &mut self,
        kind: ObligationKind,
        span: Span,
        must: bool,
        proven: bool,
        detail: String,
    ) {
        if self.recording {
            self.report.obligations.push(Obligation {
                kind,
                span,
                must,
                proven,
                detail,
            });
        }
    }

    fn anchor(&self) -> Anchor {
        match (self.stmt, self.syntax.body) {
            (Some(s), _) => Anchor::Before(s),
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
    ) -> (String, Vec<i128>) {
        let mut names = Vec::new();
        let mut vars = Vec::new();
        for &b in &self.scope {
            if let Some(v) = self.vars[b] {
                names.push(self.resolution.bindings[b].name.clone());
                vars.push(v);
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
            _ => false,
        };
        if self_ref {
            let s = self.scratch;
            self.assign(st, s, v);
            st.forget(var);
            constrain_offset(st, var, &[Term::pos(s)], 0, 0, 0);
            st.forget(s);
            return;
        }
        st.forget(var);
        match v {
            Val::Lin(t, c) => constrain_offset(st, var, t, *c, 0, 0),
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
            _ => {}
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
            _ => None,
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

    fn constrain(&self, st: &mut Octagon, op: BinaryOp, l: &Val, r: &Val) {
        match op {
            BinaryOp::Lt => self.le(st, l, r, -1),
            BinaryOp::Le => self.le(st, l, r, 0),
            BinaryOp::Gt => self.le(st, r, l, -1),
            BinaryOp::Ge => self.le(st, r, l, 0),
            BinaryOp::Eq => {
                self.le(st, l, r, 0);
                self.le(st, r, l, 0);
            }
            BinaryOp::Ne => {
                let mut below = st.clone();
                self.le(&mut below, l, r, -1);
                self.le(st, r, l, -1);
                *st = below.join(st);
            }
            _ => {}
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
            Cond::Unknown => {}
        }
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
            }
            if let Some(r) = refine {
                let v = var_val(var);
                self.constrain(st, BinaryOp::Ge, &v, &r.lo);
                let op = if r.inclusive {
                    BinaryOp::Le
                } else {
                    BinaryOp::Lt
                };
                self.constrain(st, op, &v, &r.hi);
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
            Shape::Other => Val::Other,
        }
    }

    // -- obligations -------------------------------------------------------------------------

    /// A value entering a plain integer type: a run-time check unless its bounds fit.
    /// Where the value always lies outside the type, when this point is reached: `at most -1`,
    /// `at least 256`.
    fn fits(&mut self, st: &Octagon, v: &Val, t: Option<IntTy>, span: Span) -> Option<String> {
        let (lo, hi) = interval(st, v);
        let range = t.map(int_range);
        let proven =
            range.is_some_and(|(tlo, thi)| tlo <= lo && hi <= thi && -INF < lo && hi < INF);
        let detail = t.map_or("into its type".to_string(), |t| {
            format!("into `{}`", t.name)
        });
        self.record_check(
            ObligationKind::Range,
            span,
            false,
            st.is_empty() || proven,
            detail,
        );
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
        span: Span,
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
                    self.refinement(st, v, r, span, place, what);
                }
            }
            Shape::Array(_, elem) => {
                let each = match v {
                    Val::Arr(s) => (**s).clone(),
                    _ => TOP,
                };
                let place = format!("each element of {}", place);
                self.narrow(st, &each, elem, span, &place, None);
            }
            Shape::Other => {}
        }
    }

    fn refinement(
        &mut self,
        st: &Octagon,
        v: &Val,
        r: &Refine,
        span: Span,
        place: &str,
        what: Option<(String, u8)>,
    ) {
        let high_op = if r.inclusive {
            BinaryOp::Le
        } else {
            BinaryOp::Lt
        };
        let low = cmp(BinaryOp::Ge, v.clone(), r.lo.clone());
        let high = cmp(high_op, v.clone(), r.hi.clone());
        let fail_low = self.refute(st, &low);
        let fail_high = self.refute(st, &high);
        let failing = fail_low.clone().or(fail_high.clone());
        self.record(ObligationKind::Refinement, span, true, failing.is_none());
        let (Some(mut s), true) = (failing, self.recording) else {
            return;
        };
        // The probe holds the value, outside the refinement on the side that fails.
        let probe = var_val(self.probe);
        self.assign(&mut s, self.probe, v);
        let (op, bound) = match fail_low {
            Some(_) => (BinaryOp::Ge, &r.lo),
            None => (high_op, &r.hi),
        };
        self.assume(&mut s, &cmp(op, probe, bound.clone()), false);
        let (valuation, values) = self.valuation(&s, &[], &[self.probe]);
        let value = values.first().copied();
        let outside = format!("outside `{}`, the type of {}", r.text, place);
        let shown = what
            .as_ref()
            .map_or("the value".to_string(), |w| format!("`{}`", w.0));
        let core = format!("{} is {}, {}", shown, value.unwrap_or(0), outside);
        // The valuation determines the value only when the value is linear in the bindings in
        // scope; otherwise Tier 1 knows only an interval for it, and names a point of that alone.
        let in_scope = |t: &Term| self.scope.iter().any(|b| self.vars[*b] == Some(t.var));
        let message = match v {
            Val::Lin(t, _) if !t.is_empty() && t.iter().all(in_scope) => {
                let named = what.as_ref().is_some_and(|w| {
                    self.scope
                        .iter()
                        .any(|b| self.resolution.bindings[*b].name == w.0)
                });
                if named {
                    format!("Tier 1 cannot rule out `{}`, {}", valuation, outside)
                } else {
                    format!("Tier 1 cannot rule out `{}`, where {}", valuation, core)
                }
            }
            _ => format!("Tier 1 cannot rule out that {}", core),
        };
        let read = what.as_ref().and_then(|w| {
            self.scope
                .iter()
                .copied()
                .find(|b| self.resolution.bindings[*b].name == w.0)
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
        self.record_check(
            ObligationKind::Index,
            span,
            false,
            st.is_empty() || (lo >= 0 && hi < n),
            format!("into an array of {}", len),
        );
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
        let name = self.syntax.name.clone();
        let syntax = self.syntax;
        for c in &syntax.contracts {
            if c.kind != ContractKind::Ensures || self.outside(c.expr).is_some() {
                continue;
            }
            let mut s = out.clone();
            let k = self.cond(&mut s, c.expr);
            let failing = self.refute(&s, &k);
            self.record(ObligationKind::Ensures, span, true, failing.is_none());
            let Some(failing) = failing else {
                continue;
            };
            let clause = print(self.tree, c.expr, &no_subst).0;
            let value_text = value.map(|e| self.own_text(e));
            let resolution = self.resolution;
            let subst = |id: ExprId, _: &str| {
                let b = resolution.names[id.0 as usize]?;
                (resolution.bindings[b.0 as usize].kind == BindingKind::Result)
                    .then(|| value_text.clone())
                    .flatten()
            };
            let guard = print(self.tree, c.expr, &subst).0;
            let (valuation, _) = self.valuation(&failing, &extra, &[]);
            let what = format!("`ensures {}` of `{}` {}", clause, name, at);
            let message = Fx::cannot_rule_out(
                &valuation,
                &format!("which violates {}", what),
                &format!("{} fails", what),
            );
            self.emit("E0505", span, message, Some(guard), anchor);
        }
    }

    // -- the walk ------------------------------------------------------------------------------

    fn entry(&mut self, st: &mut Octagon) {
        for i in 0..self.syntax.params.len() {
            if let Some(v) = self.vars[i] {
                let shape = self.binding_shape(i);
                self.facts(st, v, &shape);
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
                    _ => None,
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

    /// The root binding of a place and its indexes, outermost first.
    fn place_parts(&self, e: ExprId, indexes: &mut Vec<ExprId>) -> Option<usize> {
        match &self.tree.expr(e).kind {
            ExprKind::Index { base, index } => {
                let root = self.place_parts(*base, indexes);
                indexes.push(*index);
                root
            }
            _ => self.resolution.names[e.0 as usize].map(|b| b.0 as usize),
        }
    }

    fn assign_stmt(&mut self, st: &mut Octagon, place: ExprId, op: AssignOp, value: ExprId) {
        let mut indexes = Vec::new();
        let root = self.place_parts(place, &mut indexes);
        let mut shape = root.map_or(Shape::Other, |b| self.binding_shape(b));
        for &i in &indexes {
            let iv = self.eval(st, i);
            if let Shape::Array(n, _) = shape {
                self.index_check(st, &iv, n, self.span(i));
            }
            shape = shape.element();
        }
        let scalar = root
            .filter(|_| indexes.is_empty())
            .and_then(|b| self.vars[b]);
        let text = self.own_text(place);
        let place_name = format!("`{}`", text.0);
        let v = match op {
            AssignOp::Set => self.eval_into(st, value, &shape, &place_name),
            AssignOp::Compound(bop) => {
                let current = scalar.map_or_else(|| self.summary(st, &shape), var_val);
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
                    self.span(value),
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
                self.narrow(st, &v, shape, self.span(e), place, Some(what));
                v
            }
        }
    }

    fn eval(&mut self, st: &mut Octagon, e: ExprId) -> Val {
        let (tree, typing) = (self.tree, self.typing);
        match &tree.expr(e).kind {
            ExprKind::Int { value, .. } => int_literal(*value),
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
                _ => {
                    let l = self.eval(st, *lhs);
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
                    _ => self.summary(st, &Shape::plain(&typing.exprs[e.0 as usize])),
                };
                // An element of a binding has its declared element type's refinement, whose
                // bounds may name other bindings.
                let Site::Value(var) = self.sites[e.0 as usize] else {
                    return each;
                };
                self.assign(st, var, &each);
                let mut indexes = Vec::new();
                if let Some(root) = self.place_parts(e, &mut indexes) {
                    let shape = indexes
                        .iter()
                        .fold(self.binding_shape(root), |s, _| s.element());
                    self.facts(st, var, &shape);
                }
                var_val(var)
            }
            ExprKind::Field { base, .. } => {
                self.eval(st, *base);
                match &typing.exprs[base.0 as usize] {
                    Ty::Array(n, _) => int_literal(*n),
                    _ => TOP,
                }
            }
            ExprKind::Call { args, .. } => self.call(st, e, args),
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
                self.record_check(
                    ObligationKind::Shift,
                    rspan,
                    false,
                    st.is_empty() || (lo >= 0 && hi < width),
                    lt.map_or(String::new(), |t| format!("below {}", t.bits)),
                );
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
                );
                if op == BinaryOp::Div {
                    div_vals(st, l, r)
                } else {
                    self.rem(st, e, l, r)
                }
            }
            BinaryOp::Sub => add_vals(st, l, &negate(r)),
            BinaryOp::Mul => mul_vals(st, l, r),
            _ => add_vals(st, l, r),
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
            _ => {
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
                if and {
                    Cond::And(Box::new(l), Box::new(r))
                } else {
                    Cond::Or(Box::new(l), Box::new(r))
                }
            }
            ExprKind::Binary {
                op:
                    op @ (BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge),
                lhs,
                rhs,
            } => {
                let l = self.eval(st, *lhs);
                let r = self.eval(st, *rhs);
                cmp(*op, l, r)
            }
            _ => {
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
                    _ => TOP,
                }
            }
            ExprKind::Field { base, .. } => match &tree.expr(*base).kind {
                ExprKind::Name(n) => c
                    .syntax
                    .params
                    .iter()
                    .find(|p| p.name == *n)
                    .and_then(|p| match &tree.ty(p.ty).kind {
                        TypeKind::Array { len, .. } => Some(int_literal(*len)),
                        _ => None,
                    })
                    .unwrap_or(TOP),
                _ => TOP,
            },
            _ => TOP,
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
            ExprKind::Binary {
                op:
                    op @ (BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge),
                lhs,
                rhs,
            } => cmp(
                *op,
                self.callee_val(st, c, site, *lhs, after),
                self.callee_val(st, c, site, *rhs, after),
            ),
            _ => Cond::Unknown,
        }
    }

    /// A place a call may change through a `mut` parameter: a scalar binding loses its facts
    /// but its type's.
    fn clobber(&self, st: &mut Octagon, place: ExprId) {
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

    fn call(&mut self, st: &mut Octagon, e: ExprId, args: &[Arg]) -> Val {
        let known = match &self.sites[e.0 as usize] {
            Site::Call(site) => self
                .callees
                .get(site.callee)
                .copied()
                .flatten()
                .map(|c| (site.clone(), c)),
            _ => None,
        };
        let Some((site, c)) = known else {
            // A function without a body: its `mut` arguments may change.
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
        for &v in site.ins.iter().chain(&site.outs).flatten() {
            st.forget(v);
        }
        // Each argument enters its parameter's plain type, left to right ...
        let mut vals = Vec::new();
        for (k, (a, p)) in args.iter().zip(&callee.params).enumerate() {
            let v = self.eval(st, a.value);
            let plain = Shape::plain(&Ty::of(&callee.tree, p.ty));
            self.narrow(st, &v, &plain, self.span(a.value), "", None);
            if let Some(var) = site.ins[k] {
                self.assign(st, var, &v);
                self.facts(st, var, &plain);
            }
            vals.push(v);
        }
        // ... then its refinement, once every argument is in.
        for (k, (a, p)) in args.iter().zip(&callee.params).enumerate() {
            let shape = self.callee_shape(&c, &site, args, p.ty).refinements();
            let place = format!("parameter `{}` of `{}`", p.name, callee.name);
            let what = self.own_text(a.value);
            let v = site.ins[k].map_or_else(|| vals[k].clone(), var_val);
            self.narrow(st, &v, &shape, self.span(a.value), &place, Some(what));
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
            self.record(ObligationKind::Requires, span, true, failing.is_none());
            let Some(failing) = failing else {
                continue;
            };
            let extra = self.call_named(callee, &site);
            let (valuation, _) = self.valuation(&failing, &extra, &[]);
            let text = print(&callee.tree, clause.expr, &no_subst).0;
            let subst = |_: ExprId, n: &str| {
                let k = callee.params.iter().position(|p| p.name == n)?;
                args.get(k).map(|a| self.own_text(a.value))
            };
            let guard = print(&callee.tree, clause.expr, &subst).0;
            let what = format!("`requires {}` of `{}`", text, callee.name);
            let message = Fx::cannot_rule_out(
                &valuation,
                &format!("which violates {} at this call", what),
                &format!("this call violates {}", what),
            );
            let anchor = self.anchor();
            self.emit("E0504", span, message, Some(guard), anchor);
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
            let mut indexes = Vec::new();
            let root = self.place_parts(a.value, &mut indexes);
            let mut shape = root.map_or(Shape::Other, |b| self.binding_shape(b));
            for _ in &indexes {
                shape = shape.element();
            }
            let place = format!("`{}`", self.own_text(a.value).0);
            self.narrow(
                st,
                &back,
                &shape.refinements(),
                self.span(a.value),
                &place,
                None,
            );
            if let Some(var) = root
                .filter(|_| indexes.is_empty())
                .and_then(|b| self.vars[b])
            {
                self.track(st, var, &back);
                self.assign(st, var, &back);
                self.facts(st, var, &shape);
            }
        }
        let value = match (site.result, callee.result) {
            (Some(r), _) => var_val(r),
            (None, Some(t)) => {
                let shape = self.callee_shape(&c, &site, args, t);
                self.summary(st, &shape)
            }
            (None, None) => Val::Other,
        };
        for &v in site.ins.iter().chain(&site.outs).flatten() {
            st.forget(v);
        }
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
            _ => out.push(Cand::Expr(e)),
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
                _ => {}
            }
        }
    }

    fn guards_in_if(&self, e: ExprId, out: &mut Vec<Cand>) {
        if let ExprKind::If {
            then, otherwise, ..
        } = &self.tree.expr(e).kind
        {
            self.guards(*then, out);
            match otherwise {
                Some(Else::Block(b)) => self.guards(*b, out),
                Some(Else::If(i)) => self.guards_in_if(*i, out),
                None => {}
            }
        }
    }

    /// The bindings a block declares, assigns or passes as `mut`.
    fn changed_block(&self, b: BlockId, out: &mut Vec<usize>) {
        let tree = self.tree;
        for &s in &tree.block(b).stmts {
            match &tree.stmt(s).kind {
                StmtKind::Binding { init, .. } => {
                    out.extend(self.resolution.stmts[s.0 as usize].map(|b| b.0 as usize));
                    self.changed_expr(*init, out);
                }
                StmtKind::Assign { place, value, .. } => {
                    out.extend(self.place_parts(*place, &mut Vec::new()));
                    self.changed_expr(*place, out);
                    self.changed_expr(*value, out);
                }
                StmtKind::While { cond, body, .. } => {
                    self.changed_expr(*cond, out);
                    self.changed_block(*body, out);
                }
                StmtKind::Guard { cond, otherwise } => {
                    self.changed_expr(*cond, out);
                    self.changed_block(*otherwise, out);
                }
                StmtKind::Return(Some(e)) | StmtKind::Discard(e) | StmtKind::Expr(e) => {
                    self.changed_expr(*e, out)
                }
                _ => {}
            }
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
            ExprKind::Field { base, .. } | ExprKind::Repeat { value: base, .. } => {
                self.changed_expr(*base, out)
            }
            _ => {}
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
                    BinaryOp::Le
                } else {
                    BinaryOp::Ge
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
        }
        for (c, ok) in cands.iter().zip(alive) {
            if *ok {
                let k = self.cand(&mut h, *c);
                self.assume(&mut h, &k, true);
            }
        }
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
                _ => None,
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
            _ => Vec::new(),
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
        let failing = self.refute(b, &cmp(BinaryOp::Ge, v.clone(), lin(Vec::new(), 0)));
        self.record(
            ObligationKind::Decreases,
            self.span(e),
            true,
            failing.is_none(),
        );
        let Some(failing) = failing else {
            return;
        };
        self.recording = false;
        let mut first = entry.clone();
        let c = self.cond(&mut first, cond);
        self.assume(&mut first, &c, true);
        let v0 = self.eval(&mut first, e);
        let on_entry = self.refute(&first, &cmp(BinaryOp::Ge, v0, lin(Vec::new(), 0)));
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
        let guard = format!("{} >= 0", paren(text, OPERAND));
        self.emit("E0507", self.span(e), message, Some(guard), anchor);
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
                _ => {
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
        let failing = self.refute(&next, &lex_less(&after, &before))?;
        Some((failing, after, before))
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
        self.record(ObligationKind::Decreases, span, true, found.is_none());
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
            let failing = self.refute(st, &cmp(BinaryOp::Ge, v.clone(), lin(Vec::new(), 0)));
            self.record(ObligationKind::Decreases, span, true, failing.is_none());
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
                let subst = |_: ExprId, n: &str| {
                    let k = callee.params.iter().position(|p| p.name == n)?;
                    args.get(k).map(|a| self.own_text(a.value))
                };
                let guard = format!(
                    "{} >= 0",
                    paren(print(&callee.tree, clause.expr, &subst), OPERAND)
                );
                let anchor = self.anchor();
                self.emit("E0507", span, message, Some(guard), anchor);
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
        let failing = self.refute(st, &lex_less(&values, &entry));
        self.record(ObligationKind::Decreases, span, true, failing.is_none());
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
        let mut changed = Vec::new();
        self.changed_block(body, &mut changed);
        // Candidates come only from the loop's text: the written invariants first, then the
        // condition's conjuncts with the non-strict form of each strict one, then the guards.
        let mut cands = Vec::new();
        let mut axioms = Vec::new();
        for &i in invariants {
            if self.outside(i).is_some() {
                axioms.push(i);
            } else {
                self.conjuncts(i, &mut cands);
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
                    _ => None,
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
            self.record(ObligationKind::Invariant, span, true, failing.is_none());
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
        let parsed = mente_syntax::parse_bodies(&text, &m);
        let routings: Vec<Routing> = m
            .items
            .iter()
            .filter_map(|i| match i {
                ItemDecl::Fn(f) => Some(Routing::of(&f.attrs)),
                _ => None,
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
            let (resolution, errors) = crate::names::resolve(syntax, &names);
            assert!(errors.is_empty(), "{:?}", errors);
            let (typing, errors) = crate::check::check(syntax, &resolution, &signatures);
            assert!(errors.is_empty(), "{:?}", errors);
            let f = Function {
                syntax,
                resolution: &resolution,
                typing: &typing,
                routing: *routing,
            };
            out.push(check(&f, &callees, &text));
        }
        out
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
        // A value of looser binding than a comparison gets parentheses.
        assert!(
            diagnostics("pub fn f(x: u8, c: bool) -> u8[0..5] = if c { x } else { 4 }")[0].fixes[0]
                .message
                .contains("guard (if c { x } else { 4 }) < 5 else")
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
        // A function without a body may change its `mut` argument to anything of its type.
        let opaque = "pub fn touch(mut x: u8)\npub fn f() -> u8[0..5] {\n  var z: u8 = 1\n  touch(mut z)\n  z\n}";
        assert_eq!(codes(opaque), ["E0502"]);
        let opaque_element = "pub fn touch(mut x: u8)\npub fn f() -> u8[0..5] {\n  var zs: [1]u8 = [1]\n  touch(mut zs[0])\n  3\n}";
        assert_eq!(codes(opaque_element), NONE);
        let array = "pub fn fill(mut xs: [2]u8[0..3]) {\n  xs[0] = 1\n}\npub fn f() -> [2]u8[0..3] {\n  var zs: [2]u8[0..3] = [0, 0]\n  fill(mut zs)\n  zs\n}";
        assert_eq!(codes(array), NONE);
        let read_only = "pub fn keep(x: u8) -> u8\n  ensures result == x\n= x\npub fn f(y: u8[0..5]) -> u8[0..5] = keep(y)";
        assert_eq!(codes(read_only), NONE);
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
        // A callee's `requires` about an array length, and a call of a function without a body.
        let len = "pub fn first(xs: [3]u8)\n  requires xs.len > 2\n{\n}\npub fn f(ys: [3]u8) {\n  first(ys)\n}";
        assert_eq!(codes(len), NONE);
        let short = "pub fn first(xs: [3]u8)\n  requires xs.len > 3\n{\n}\npub fn f(ys: [3]u8) {\n  first(ys)\n}";
        assert_eq!(codes(short), ["E0504"]);
        let opaque = "pub fn ext(x: u8) -> u8\npub fn f(x: u8) -> u8[0..5] = ext(x)";
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
        assert_eq!(flip(BinaryOp::Ne), BinaryOp::Eq);
        assert_eq!(flip(BinaryOp::Eq), BinaryOp::Ne);
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
        let base = "pub fn bump(mut x: u8)\n  ensures x > 0\n{\n  x = 1\n}\npub fn ext(mut x: u8)\npub fn f(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n decreases n - i {\n    i += 1\nCHANGE\n  }\n  i\n}";
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
        assert_eq!(
            d.message,
            "Tier 1 cannot rule out `n = 0, i = -1`, at the head of a round of this loop, where its measure `i` is negative: a `decreases` measure is never negative"
        );
        assert_eq!(
            d.fixes[0].message,
            "insert `guard i >= 0 else { todo(\"i >= 0 is false here\") }`"
        );
        // The first round can break it: the guard goes before the loop.
        assert!(fixed(neg).contains(
            "  var i: i8 = n\n  guard i >= 0 else { todo(\"i >= 0 is false here\") }\n  while"
        ));
        // Only a later round can: the guard ends the body.
        let later = "pub fn f() -> i8 {\n  var i: i8 = 5\n  while i != 0 decreases i {\n    i -= 2\n  }\n  i\n}";
        assert!(fixed(later)
            .contains("    i -= 2\n    guard i >= 0 else { todo(\"i >= 0 is false here\") }\n  }"));
        let none = "pub fn f(c: bool) {\n  while c decreases -1 {\n  }\n}";
        assert_eq!(
            messages(none),
            [
                "Tier 1 cannot rule out that this loop's measure `-1` is negative at the head of a round: a `decreases` measure is never negative",
                "Tier 1 cannot rule out that a round of this loop changes its measure `-1` by 0: a `decreases` measure gets smaller at every round"
            ]
        );
        let shifted = "pub fn f(n: i8) -> i8 {\n  var i: i8 = n\n  while i != 0 decreases i + 1 - 1 {\n    i -= 1\n  }\n  i\n}";
        assert!(diagnostics(shifted)[0].fixes[0]
            .message
            .contains("guard i + 1 - 1 >= 0 else"));
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
        // `lo`, which the result reads, is tried first and fails at `lo = mid + 1`; `hi` bounds it.
        let src = "pub fn lower_bound(a: [16]i32, x: i32) -> usize[0..=16] {\n  var lo: usize = 0\n  var hi: usize = 16\n  while lo < hi {\n    val mid = lo + (hi - lo) / 2\n    if a[mid] < x {\n      lo = mid + 1\n    } else {\n      hi = mid\n    }\n  }\n  lo\n}";
        assert_eq!(fix_messages(src)[0], "declare `hi` as `usize[0..=16]`");
        let applied = apply_fix(&module(src), &diagnostics(src)[0].fixes[0]).unwrap();
        assert!(check_source(&applied).is_ok(), "{}", applied);
    }

    #[test]
    fn test_no_refined_var_is_offered_unless_tier1_then_proves_the_function() {
        // Declared `usize[0..8]`, `x` would fail at its own declaration instead.
        let moved = "pub fn f(n: usize) -> usize[0..8] {\n  var x: usize = n\n  x = x / 2\n  x\n}";
        // `n`, a parameter, is never offered: refined, it would only move the obligation to every
        // caller. Declared `usize[0..8]`, `k` fits, but the result still fails.
        let unrelated = "pub fn f(n: usize) -> usize[0..8] {\n  var k: usize = 0\n  while k < 3 {\n    k += 1\n  }\n  n\n}";
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
        let (resolution, _) = crate::names::resolve(syntax, &names);
        let (typing, _) = crate::check::check(syntax, &resolution, &signatures);
        let f = Function {
            syntax,
            resolution: &resolution,
            typing: &typing,
            routing: Routing::default(),
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
