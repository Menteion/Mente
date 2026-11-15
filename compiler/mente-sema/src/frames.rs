//! The frame of each function: the bytes a call of it counts against the stack budget
//! (`core-calculus.md`, item 25; [`stack-budget`]).
//!
//! A frame is 64 bytes for the call, what each parameter counts, 16 bytes for each `decreases`
//! clause, and the most the function's clauses and body hold at one moment. How long anything
//! is held is item 12's: a value no binding holds lives to the end of the innermost statement
//! it stands in, a binding to the end of its block; the last expression of a branch's or an
//! arm's block ends what it makes and consumes with the block, and only its value lives on;
//! the condition of an `if`, a `guard` or a `while`, the left operand of `and` and `or`, and a
//! clause, are statements of their own. Each function below is one equation of item 25, over
//! the checked tree: [`Ev`] carries `t(e)`, what an expression leaves held to the end of its
//! statement, `m(e)`, the most it holds at once, and `v(e)`, what its value holds.
//!
//! Every type it sizes is one the checker settled: an expression's, a binding's, the literal
//! table's for literals and for arrays of them, and the type of `p op e` in `p op= e`. It
//! derives none of its own.
//!
//! [`stack-budget`]: ../../docs/design-review.md#stack-budget

use mente_syntax::tree::{
    Arg, ArmBody, AssignOp, BinaryOp, BlockId, Body, ContractKind, Else, ExprId, ExprKind, Payload,
    StmtId, StmtKind, Tree,
};

use crate::check::Typing;
use crate::literals::{self, Taken};
use crate::names::{BindingId, Resolution};
use crate::structs::Structs;
use crate::types::{Int, IntTy, Ty};
use crate::CheckedFn;

/// The bytes every call counts for itself (item 25).
pub const CALL: u128 = 64;

/// A run's stack budget, in bytes: the most the frames of the calls that have begun and not
/// returned may sum to, the same on every execution path and for every program (item 25).
pub const BUDGET: u128 = 1_048_576;

/// A function's frame, in bytes, and its parts as item 25 sums them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The whole frame: what a call of the function counts.
    pub bytes: u128,
    /// What its parameters count.
    pub params: u128,
    /// 16 bytes for each `decreases` clause of the function.
    pub decreases: u128,
    /// The most its clauses and its body hold at one moment.
    pub held: u128,
}

/// Each frame of a module as `mn view --frames` prints it: a line for each function with a
/// body, in the module's order, with the bytes a call of it counts and the parts item 25 sums.
pub fn report(module: &crate::CheckedModule) -> String {
    let mut out = String::new();
    for (name, f) in module.names.iter().zip(&module.frames) {
        if let Some(f) = f {
            out.push_str(&format!(
                "fn {}: {} bytes = {} for the call + {} for the parameters + {} for decreases + {} held at most\n",
                name, f.bytes, CALL, f.params, f.decreases, f.held
            ));
        }
    }
    out
}

/// The frame of `f`, whose types `structs` lays out.
pub fn frame(f: &CheckedFn, structs: &Structs) -> Frame {
    let c = Count {
        tree: &f.syntax.tree,
        res: &f.resolution,
        typing: &f.typing,
        structs,
    };
    let params = f
        .syntax
        .params
        .iter()
        .map(|p| {
            let ty = Ty::of(c.tree, p.ty);
            if !p.is_mut && is_scalar(&ty) {
                c.size(&ty)
            } else {
                8
            }
        })
        .sum::<u128>();
    let clauses = |kinds: &[ContractKind]| {
        f.syntax
            .contracts
            .iter()
            .filter(|k| kinds.contains(&k.kind))
            .map(|k| c.eval(k.expr, false).m)
            .max()
            .unwrap_or(0)
    };
    let decreases = 16
        * f.syntax
            .contracts
            .iter()
            .filter(|k| k.kind == ContractKind::Decreases)
            .count() as u128;
    let pre = clauses(&[ContractKind::Requires, ContractKind::Decreases]);
    let post = clauses(&[ContractKind::Ensures]);
    let body = match f.syntax.body {
        Some(Body::Block(b)) => c.block(b),
        Some(Body::Expr(e)) => c.landed(e),
        None => 0,
    };
    let held = pre.max(body + post);
    Frame {
        bytes: CALL + params + decreases + held,
        params,
        decreases,
        held,
    }
}

/// `t(e)`, `m(e)` and `v(e)` of item 25 for one expression.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Ev {
    t: u128,
    m: u128,
    v: u128,
}

impl Ev {
    fn leaf(v: u128) -> Ev {
        Ev { t: v, m: v, v }
    }
}

struct Count<'a> {
    tree: &'a Tree,
    res: &'a Resolution,
    typing: &'a Typing,
    structs: &'a Structs,
}

/// Whether a value of `t` is held at its size where a place is read: an integer or a `bool`.
fn is_scalar(t: &Ty) -> bool {
    match t {
        Ty::Int(_) | Ty::Bool => true,
        Ty::Array(..) | Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => false,
    }
}

fn bytes(i: IntTy) -> u128 {
    u128::from(i.bits / 8)
}

impl Count<'_> {
    fn ty(&self, e: ExprId) -> &Ty {
        &self.typing.exprs[e.0 as usize]
    }

    /// The size of a value of `t` (item 21). An integer of no one type counts 16 (item 25): a
    /// mixed one before it lands, and a literal no standard type holds, which keeps the type of
    /// literals alone ([`Count::value_ty`]).
    fn size(&self, t: &Ty) -> u128 {
        match t {
            Ty::Int(Int::One(i)) => bytes(*i),
            Ty::Int(Int::Mixed(_) | Int::Literal) => 16,
            Ty::Bool => 1,
            Ty::Array(n, elem) => n.saturating_mul(self.size(elem)),
            Ty::Struct(_) | Ty::Enum(_) => self.structs.layout_of(t).map_or(0, |(s, _)| s),
            Ty::Unit | Ty::Any => 0,
        }
    }

    /// The type `e`'s value has, as the checker settled it, which the module never derives on
    /// its own (item 5): a literal's, or an array of literals', with the type the checker's
    /// literal table gives those literals. A literal no standard type holds keeps the type of
    /// literals alone, of no one type.
    fn value_ty(&self, e: ExprId) -> Ty {
        let t = self.ty(e);
        let table = match t {
            Ty::Int(Int::Literal) => &self.typing.literals,
            Ty::Array(..) => &self.typing.arrays,
            Ty::Int(Int::One(_) | Int::Mixed(_))
            | Ty::Bool
            | Ty::Struct(_)
            | Ty::Enum(_)
            | Ty::Unit
            | Ty::Any => return t.clone(),
        };
        match table[e.0 as usize] {
            Some(Taken {
                ty,
                from: literals::From::Operands | literals::From::Alone,
            }) => t.with_int(ty),
            Some(Taken {
                from: literals::From::Unheld,
                ..
            })
            | None => t.clone(),
        }
    }

    /// What `e`'s value holds where it is no place read: the size of its type.
    fn vsize(&self, e: ExprId) -> u128 {
        self.size(&self.value_ty(e))
    }

    /// `c(e)`: the size of the type `e` lands in, where that is another type than its own: an
    /// integer into another integer type, an array whose elements are narrowed (item 25, the
    /// row of a value narrowed). A refinement is no other type.
    fn c(&self, e: ExprId) -> u128 {
        match &self.typing.places[e.0 as usize] {
            Some(place) if narrows(&self.value_ty(e), place) => self.size(place),
            Some(_) | None => 0,
        }
    }

    /// `m'(e)`: `e`, then the value it is narrowed into. Every caller reads only this; what
    /// stays held after it, `t'(e)`, is summed where it is an operand ([`Count::operands`],
    /// [`Count::call`]).
    fn landed(&self, e: ExprId) -> u128 {
        let x = self.eval(e, false);
        x.m.max(x.t + self.c(e))
    }

    /// The binding at the root of a place.
    fn root(&self, e: ExprId) -> Option<BindingId> {
        match &self.tree.expr(e).kind {
            ExprKind::Name(_) => self.res.names[e.0 as usize],
            ExprKind::Index { base, .. } | ExprKind::Field { base, .. } => self.root(*base),
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

    /// Whether `e` is a place read: a name, or a field or an element read whose base is a
    /// place. A `.len` is one too: item 25's row for it is a field read's, its operands its
    /// place's indexes and the bases kept, or its base where that is no place, and its value a
    /// `usize`, 8 bytes, the size of its type.
    fn is_place_read(&self, e: ExprId) -> bool {
        crate::check::is_place(self.tree, e)
    }

    /// The indexes of a place, in the order they are evaluated, and the sizes of the bases a
    /// later index keeps: a base is kept where an index of it holds a write of its root.
    fn place_parts(&self, e: ExprId, keep_bases: bool) -> (Vec<ExprId>, u128) {
        match &self.tree.expr(e).kind {
            ExprKind::Index { base, index } => {
                let (mut idx, mut k) = self.place_parts(*base, keep_bases);
                if keep_bases {
                    if let Some(r) = self.root(*base) {
                        if self.writes(*index, r) {
                            k += self.size(self.ty(*base));
                        }
                    }
                }
                idx.push(*index);
                (idx, k)
            }
            ExprKind::Field { base, .. } => self.place_parts(*base, keep_bases),
            ExprKind::Name(_)
            | ExprKind::Int { .. }
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
            | ExprKind::Error => (Vec::new(), 0),
        }
    }

    /// What a place read's value holds: its size for an integer or a `bool`, or where it is a
    /// value kept; else 8 bytes, an address.
    fn place_value(&self, e: ExprId, kept: bool) -> u128 {
        if kept || is_scalar(self.ty(e)) {
            self.size(self.ty(e))
        } else {
            8
        }
    }

    /// The equation over operands e1 .. en and a value, with `k` bytes of bases kept held from
    /// the start; `land` where the operands land as they are evaluated, each narrowed into its
    /// place (a value built).
    fn operands(&self, ops: &[(ExprId, bool)], v: u128, k: u128, land: bool) -> Ev {
        let mut held = k;
        let mut m = 0;
        for &(e, kept) in ops {
            let x = self.eval(e, kept);
            // The narrowed value stays held, so the list's own `t`, its last term, covers
            // the moment it is made.
            let (t, mx) = if land {
                (x.t + self.c(e), x.m)
            } else {
                (x.t, x.m)
            };
            m = m.max(held + mx);
            held += t;
        }
        let t = held + v;
        Ev { t, m: m.max(t), v }
    }

    /// The operands of a list, each marked kept where it has a root binding and a later operand
    /// of the same list holds a write of it (item 25, the rule of a value kept). Being kept
    /// makes a place read of a plain type other than an integer type or `bool` hold its size
    /// ([`Count::place_value`]); no other operand holds more or less for it.
    fn kept_list(&self, ops: &[ExprId]) -> Vec<(ExprId, bool)> {
        ops.iter()
            .enumerate()
            .map(|(i, &e)| {
                let kept = self
                    .root(e)
                    .is_some_and(|r| ops[i + 1..].iter().any(|&later| self.writes(later, r)));
                (e, kept)
            })
            .collect()
    }

    /// A place read, its indexes its operands and the bases a later index keeps held with them.
    fn place_read(&self, e: ExprId, kept: bool) -> Ev {
        let (idx, k) = self.place_parts(e, true);
        let ops: Vec<_> = idx.into_iter().map(|i| (i, false)).collect();
        self.operands(&ops, self.place_value(e, kept), k, false)
    }

    /// A call: each argument evaluated, then each narrowed, then the call returns (item 7). A
    /// `mut` argument's operands are its indexes, and it holds an address.
    fn call(&self, args: &[Arg], v: u128) -> Ev {
        let values: Vec<ExprId> = args.iter().map(|a| a.value).collect();
        let kept = self.kept_list(&values);
        let mut held = 0;
        let mut m = 0;
        let mut narrowed = 0;
        for (a, &(e, kept)) in args.iter().zip(&kept) {
            let x = if a.is_mut {
                let (idx, _) = self.place_parts(e, false);
                let ops: Vec<_> = idx.into_iter().map(|i| (i, false)).collect();
                self.operands(&ops, 8, 0, false)
            } else {
                self.eval(e, kept)
            };
            m = m.max(held + x.m);
            held += x.t;
            narrowed += if a.is_mut { 0 } else { self.c(e) };
        }
        let t = held + narrowed + v;
        Ev { t, m: m.max(t), v }
    }

    /// `if`, `match` and `catch` give their value's size where they give a value, and nothing
    /// where they stand as statements.
    fn construct_value(&self, e: ExprId) -> u128 {
        match self.ty(e) {
            Ty::Unit | Ty::Any => 0,
            Ty::Int(_) | Ty::Bool | Ty::Array(..) | Ty::Struct(_) | Ty::Enum(_) => self.vsize(e),
        }
    }

    /// A branch or an arm that is a block whose construct makes a value of `v` bytes from its
    /// value: `(t(A), m(A))`. By the tail rule only the block's value outlives it. A block that
    /// gives no value ends in no expression that has one, which would be `E0217`, so its last
    /// expression, if any, holds nothing after it: `t(A)` is 0, as the equations have it.
    fn branch_block(&self, b: BlockId, v: u128) -> (u128, u128) {
        let stmts = &self.tree.block(b).stmts;
        let mut held = 0;
        let mut m = 0;
        for (i, &s) in stmts.iter().enumerate() {
            match self.tree.stmt(s).kind {
                StmtKind::Expr(a) if i + 1 == stmts.len() => {
                    let x = self.eval(a, false);
                    m = m.max(held + x.m).max(held + x.t + v);
                    return (x.v, m);
                }
                StmtKind::Binding { .. }
                | StmtKind::Assign { .. }
                | StmtKind::While { .. }
                | StmtKind::Guard { .. }
                | StmtKind::Return(_)
                | StmtKind::Break
                | StmtKind::Continue
                | StmtKind::Discard(_)
                | StmtKind::Expr(_) => {}
            }
            m = m.max(held + self.stmt(s));
            held += self.binding_size(s);
        }
        (0, m)
    }

    /// An arm that is an expression: `t(A) = t(a)`, and `m(A)` is `m(a)`, its `t(a) + v` being
    /// within the `match`'s own `t`, since `a`'s values stay held while the value is made.
    fn branch_expr(&self, a: ExprId) -> (u128, u128) {
        let x = self.eval(a, false);
        (x.t, x.m)
    }

    /// One expression, as an operand, kept or not.
    fn eval(&self, e: ExprId, kept: bool) -> Ev {
        match &self.tree.expr(e).kind {
            ExprKind::Int { .. } | ExprKind::Bool(_) => Ev::leaf(self.vsize(e)),
            ExprKind::Todo(_) | ExprKind::Unreachable(_) | ExprKind::Error => Ev::default(),
            ExprKind::Name(_) => Ev::leaf(self.place_value(e, kept)),
            ExprKind::Index { base, index } => {
                if self.is_place_read(e) {
                    self.place_read(e, kept)
                } else {
                    self.operands(&[(*base, false), (*index, false)], self.vsize(e), 0, false)
                }
            }
            ExprKind::Field { base, .. } => {
                if self.is_place_read(e) {
                    self.place_read(e, kept)
                } else {
                    self.operands(&[(*base, false)], self.vsize(e), 0, false)
                }
            }
            ExprKind::Unary { operand, .. } => {
                self.operands(&[(*operand, false)], self.vsize(e), 0, false)
            }
            ExprKind::Binary {
                op: BinaryOp::And | BinaryOp::Or,
                lhs,
                rhs,
            } => {
                // `a and b` is `if a { b } else { false }`: the condition a statement of its
                // own, `b` the last expression of a branch's block (item 24).
                let a = self.eval(*lhs, false);
                let b = self.eval(*rhs, false);
                let v = self.vsize(e);
                Ev {
                    t: b.v.max(1) + v,
                    m: a.m.max(b.m).max(b.t + v).max(1 + v),
                    v,
                }
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                let ops = self.kept_list(&[*lhs, *rhs]);
                self.operands(&ops, self.vsize(e), 0, false)
            }
            ExprKind::Call { args, .. } => self.call(args, self.vsize(e)),
            ExprKind::Construct { fields, .. } => {
                let values: Vec<ExprId> = fields.iter().map(|f| f.value).collect();
                self.operands(&self.kept_list(&values), self.vsize(e), 0, true)
            }
            ExprKind::Variant { payload, .. } => {
                let values: Vec<ExprId> = match payload {
                    Payload::None => Vec::new(),
                    Payload::Named(fields) => fields.iter().map(|f| f.value).collect(),
                    Payload::Positional(args) => args.iter().map(|a| a.value).collect(),
                };
                self.operands(&self.kept_list(&values), self.vsize(e), 0, true)
            }
            ExprKind::Array(elems) => self.operands(&self.kept_list(elems), self.vsize(e), 0, true),
            ExprKind::Repeat { value, .. } => {
                self.operands(&[(*value, false)], self.vsize(e), 0, true)
            }
            ExprKind::Match { scrutinee, arms } => {
                let v = self.construct_value(e);
                let s = self.eval(*scrutinee, false);
                let (mut t_arms, mut m) = (0, s.m);
                for arm in arms {
                    let (ta, ma) = match arm.body {
                        ArmBody::Expr(a) => self.branch_expr(a),
                        ArmBody::Block(b) => self.branch_block(b, v),
                    };
                    t_arms = t_arms.max(ta);
                    m = m.max(s.t + ma);
                }
                let t = s.t + t_arms + v;
                Ev { t, m: m.max(t), v }
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                let v = self.construct_value(e);
                let c = self.eval(*cond, false);
                let (t1, m1) = self.branch_block(*then, v);
                let (t2, m2) = match otherwise {
                    None => (0, 0),
                    Some(Else::Block(b)) => self.branch_block(*b, v),
                    // `else if` is `else { if ... }`, a block whose last expression is that
                    // `if` (item 12). As a statement, the inner `if` gives no value either.
                    Some(Else::If(inner)) => {
                        let x = self.eval(*inner, false);
                        (x.v, x.m.max(x.t + v))
                    }
                };
                let t = t1.max(t2) + v;
                Ev {
                    t,
                    m: c.m.max(m1).max(m2).max(t),
                    v,
                }
            }
        }
    }

    /// What a statement that declares a binding adds to its block's bindings.
    fn binding_size(&self, s: StmtId) -> u128 {
        match self.tree.stmt(s).kind {
            StmtKind::Binding { .. } => self.res.stmts[s.0 as usize]
                .map_or(0, |b| self.size(&self.typing.bindings[b.0 as usize])),
            StmtKind::Assign { .. }
            | StmtKind::While { .. }
            | StmtKind::Guard { .. }
            | StmtKind::Return(_)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Discard(_)
            | StmtKind::Expr(_) => 0,
        }
    }

    /// `M(s)`: the most a statement holds while it runs.
    fn stmt(&self, s: StmtId) -> u128 {
        match &self.tree.stmt(s).kind {
            StmtKind::Binding { init, .. } => self.binding_size(s) + self.landed(*init),
            StmtKind::Assign { place, op, value } => {
                let (idx, _) = self.place_parts(*place, false);
                let mut held = 0;
                let mut m = 0;
                for i in idx {
                    let x = self.eval(i, false);
                    m = m.max(held + x.m);
                    held += x.t;
                }
                let x = match op {
                    AssignOp::Set => self.landed(*value),
                    AssignOp::Compound(_) => self.compound(*place, *value),
                };
                m.max(held + x)
            }
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                let n = decreases.len() as u128;
                let d = if n > 0 { 16 * n + 1 } else { 0 };
                let clauses = invariants
                    .iter()
                    .chain(decreases)
                    .map(|&g| self.eval(g, false).m)
                    .max()
                    .unwrap_or(0);
                d + self
                    .eval(*cond, false)
                    .m
                    .max(clauses)
                    .max(self.block(*body))
            }
            StmtKind::Guard { cond, otherwise } => {
                self.eval(*cond, false).m.max(self.block(*otherwise))
            }
            StmtKind::Return(Some(e)) => self.landed(*e),
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => 0,
            StmtKind::Discard(e) => self.vsize(*e) + self.eval(*e, false).m,
            // The value a body gives, its last expression, lands as a returned value (item
            // 25); an expression that gives no value lands nowhere.
            StmtKind::Expr(e) => self.landed(*e),
        }
    }

    /// `m'(x)` of `p op= e`, `p = p op e` with the indexes of `p` evaluated once (item 6): the
    /// value read from `p`, then `e`; the operator's value, of the type the checker gave `p op e`,
    /// which then lands in `p`.
    fn compound(&self, place: ExprId, value: ExprId) -> u128 {
        let p = self.ty(place);
        let read = self.size(p);
        let e = self.eval(value, false);
        let (v, c) = self.typing.compounds[value.0 as usize]
            .as_ref()
            .map_or((0, 0), |op| {
                (self.size(op), if narrows(op, p) { read } else { 0 })
            });
        (read + e.m).max(read + e.t + v + c)
    }

    /// `M(B)` of a block that is a body, a loop's body, a `guard`'s `else` or a branch of no
    /// value.
    fn block(&self, b: BlockId) -> u128 {
        let mut held = 0;
        let mut m = 0;
        for &s in &self.tree.block(b).stmts {
            m = m.max(held + self.stmt(s));
            held += self.binding_size(s);
        }
        m
    }

    /// Whether `e` holds, at any depth, a write of the binding `r`, as item 20 defines one: an
    /// assignment to a place whose root it is, or a `mut` argument whose place has it as its
    /// root. The checker's own walk finds them, as it does for a frozen scrutinee (item 14).
    fn writes(&self, e: ExprId, r: BindingId) -> bool {
        let mut changed = Vec::new();
        crate::check::changed_places(self.tree, e, &mut changed);
        changed.into_iter().any(|p| self.root(p) == Some(r))
    }
}

/// Whether a value of `from` lands in a place of `to` as another type: an integer of another
/// type or of no one type, or an array whose elements do. A refinement is no other type.
fn narrows(from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (Ty::Int(Int::One(a)), Ty::Int(Int::One(b))) => a != b,
        (Ty::Int(Int::Mixed(_) | Int::Literal), Ty::Int(_)) => true,
        (Ty::Array(_, a), Ty::Array(_, b)) => narrows(a, b),
        (Ty::Int(_), _)
        | (Ty::Bool, _)
        | (Ty::Array(..), _)
        | (Ty::Struct(_), _)
        | (Ty::Enum(_), _)
        | (Ty::Unit, _)
        | (Ty::Any, _) => false,
    }
}
