//! A prototype of Tier 1 for one function of M1 (`spec/core-calculus.md`, item 8;
//! `spec/ghost-contracts-and-totality.md`, items 2 and 4): octagons over the function's integer
//! bindings, loop-head facts by Houdini over candidates from the loop's text, contracts used
//! modularly, and a valuation Tier 1 cannot rule out for every obligation it must prove and
//! cannot.

use std::collections::HashMap;

use crate::ast::*;
use crate::octagon::{Octagon, Term, INF};
use crate::Diag;

pub type Sigs = HashMap<String, Signature>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObKind {
    /// A value entering a type: proven if the type is refined, else perhaps a run-time check.
    Narrow,
    Index,
    DivisorNonZero,
    ShiftInRange,
    Requires,
    Ensures,
    Invariant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    pub kind: ObKind,
    pub span: Span,
    /// Must be proven (refined types, contracts, written invariants) or may stay a run-time check.
    pub must: bool,
    pub proven: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub obligations: Vec<Obligation>,
    pub diags: Vec<Diag>,
}

/// An abstract value.
#[derive(Clone, Debug)]
enum Val {
    /// `terms + c`, exactly, with at most two terms.
    Lin(Vec<Term>, i128),
    /// Somewhere in `[lo, hi]`.
    Itv(i128, i128),
    Bool,
    Array(i128, Ty),
    Unit,
    Bad,
}

/// A condition whose operands are already evaluated.
#[derive(Clone, Debug)]
enum Cond {
    Const(bool),
    Cmp(BinOp, Box<Val>, Box<Val>),
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    Not(Box<Cond>),
    Unknown,
}

#[derive(Clone, Debug)]
struct Binding {
    ty: Ty,
    var: Option<usize>,
    mutable: bool,
}

#[derive(Default)]
struct LoopFrame {
    breaks: Vec<Octagon>,
    continues: Vec<Octagon>,
}

struct Fx {
    sig: Signature,
    bindings: HashMap<String, Binding>,
    /// User-visible bindings in declaration order, for valuations.
    order: Vec<String>,
    next_var: usize,
    call_vars: HashMap<usize, Vec<usize>>,
    scratch: usize,
    result_var: usize,
    report: Report,
    recording: bool,
    loops: Vec<LoopFrame>,
}

fn neg_terms(terms: &[Term]) -> Vec<Term> {
    terms.iter().map(|t| t.negated()).collect()
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

/// Adds two linear expressions; None when the sum has more than two terms or a coefficient
/// other than 1 or -1, which octagons cannot hold.
fn lin_add(a: &[Term], b: &[Term]) -> Option<Vec<Term>> {
    let mut out: Vec<Term> = Vec::new();
    for t in a.iter().chain(b) {
        if let Some(k) = out.iter().position(|u| u.var == t.var) {
            if out[k].neg != t.neg {
                out.remove(k);
                continue;
            }
            return None;
        }
        out.push(*t);
    }
    (out.len() <= 2).then_some(out)
}

/// Counts the octagon variables a function needs: its integer bindings, two per call
/// argument or result, a scratch variable and the result.
fn count_vars(f: &Function, sigs: &Sigs) -> usize {
    fn expr(e: &Expr, sigs: &Sigs, n: &mut usize) {
        match &e.kind {
            ExprKind::Unary(_, a) | ExprKind::Len(a) | ExprKind::ArrayRepeat(a, _) => {
                expr(a, sigs, n)
            }
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                expr(a, sigs, n);
                expr(b, sigs, n);
            }
            ExprKind::Call(name, args) => {
                *n += sigs.get(name).map_or(0, |s| s.params.len()) + 1;
                for a in args {
                    expr(a, sigs, n);
                }
            }
            _ => {}
        }
    }
    fn stmts(ss: &[Stmt], sigs: &Sigs, n: &mut usize) {
        for s in ss {
            match &s.kind {
                StmtKind::Let { init, .. } => {
                    *n += 1;
                    expr(init, sigs, n);
                }
                StmtKind::Assign { target, value, .. } => {
                    for i in &target.indices {
                        expr(i, sigs, n);
                    }
                    expr(value, sigs, n);
                }
                StmtKind::While {
                    cond,
                    invariants,
                    body,
                    ..
                } => {
                    expr(cond, sigs, n);
                    for i in invariants {
                        expr(i, sigs, n);
                    }
                    stmts(body, sigs, n);
                }
                StmtKind::If { cond, then, els } => {
                    expr(cond, sigs, n);
                    stmts(then, sigs, n);
                    stmts(els, sigs, n);
                }
                StmtKind::Guard { cond, body } => {
                    expr(cond, sigs, n);
                    stmts(body, sigs, n);
                }
                StmtKind::Return(Some(e)) | StmtKind::Expr(e) => expr(e, sigs, n),
                _ => {}
            }
        }
    }
    let mut n = f.sig.params.len() + 2;
    for e in f.sig.requires.iter().chain(&f.sig.ensures) {
        expr(e, sigs, &mut n);
    }
    stmts(&f.body, sigs, &mut n);
    n
}

/// Names a loop body assigns or declares.
fn assigned(ss: &[Stmt], out: &mut Vec<String>) {
    for s in ss {
        match &s.kind {
            StmtKind::Let { name, .. } => out.push(name.clone()),
            StmtKind::Assign { target, .. } if target.indices.is_empty() => {
                out.push(target.name.clone())
            }
            StmtKind::While { body, .. } | StmtKind::Guard { body, .. } => assigned(body, out),
            StmtKind::If { then, els, .. } => {
                assigned(then, out);
                assigned(els, out);
            }
            _ => {}
        }
    }
}

/// Splits a condition into its conjuncts.
fn conjuncts(e: &Expr, out: &mut Vec<Expr>) {
    if let ExprKind::Binary(BinOp::And, a, b) = &e.kind {
        conjuncts(a, out);
        conjuncts(b, out);
    } else {
        out.push(e.clone());
    }
}

/// The conditions of the `guard`s in a loop body, which are candidate loop-head facts.
fn guard_conditions(ss: &[Stmt], out: &mut Vec<Expr>) {
    for s in ss {
        match &s.kind {
            StmtKind::Guard { cond, body } => {
                conjuncts(cond, out);
                guard_conditions(body, out);
            }
            StmtKind::If { then, els, .. } => {
                guard_conditions(then, out);
                guard_conditions(els, out);
            }
            _ => {}
        }
    }
}

/// Checks one function against the signatures of the module.
pub fn check_function(f: &Function, sigs: &Sigs) -> Report {
    let n = count_vars(f, sigs);
    let mut fx = Fx {
        sig: f.sig.clone(),
        bindings: HashMap::new(),
        order: Vec::new(),
        next_var: 2,
        call_vars: HashMap::new(),
        scratch: 0,
        result_var: 1,
        report: Report::default(),
        recording: true,
        loops: Vec::new(),
    };
    let mut st = Octagon::new(n);
    for p in &f.sig.params {
        fx.declare(&mut st, &p.name, &p.ty, false);
    }
    for r in &f.sig.requires {
        let c = fx.cond(&mut st, r, sigs);
        assume(&mut st, &c, true);
    }
    fx.block(&mut st, &f.body, sigs);
    if f.sig.ret != Ty::Unit && !st.is_empty() {
        fx.report.diags.push(Diag::new(
            f.span.end - 1,
            f.span.end,
            "a path reaches the end of the function without returning a value",
        ));
    }
    fx.report
}

impl Fx {
    fn alloc(&mut self, st: &Octagon) -> usize {
        let v = self.next_var;
        self.next_var = (self.next_var + 1).min(st.vars() - 1);
        v
    }

    /// Declares a binding (once per name: M1 has no shadowing) and returns its variable.
    fn declare(&mut self, st: &mut Octagon, name: &str, ty: &Ty, mutable: bool) -> Option<usize> {
        if let Some(b) = self.bindings.get(name) {
            return b.var;
        }
        let var = ty.int_range().map(|_| self.alloc(st));
        if let (Some(v), Some((r, _))) = (var, ty.int_range()) {
            st.forget(v);
            add_range(st, v, r);
        }
        self.bindings.insert(
            name.to_string(),
            Binding {
                ty: ty.clone(),
                var,
                mutable,
            },
        );
        self.order.push(name.to_string());
        var
    }

    fn diag(&mut self, span: Span, msg: String) {
        if self.recording {
            self.report
                .diags
                .push(Diag::new(span.start, span.end, &msg));
        }
    }

    /// Records an obligation. When it must be proven and is not, `failing` is a state in which
    /// it fails, and the diagnostic gives a valuation of the user's bindings read from it.
    fn record(
        &mut self,
        kind: ObKind,
        span: Span,
        must: bool,
        failing: Option<Octagon>,
        what: &str,
        guard: &str,
    ) {
        if !self.recording {
            return;
        }
        let proven = failing.as_ref().is_none_or(|s| s.is_empty());
        self.report.obligations.push(Obligation {
            kind,
            span,
            must,
            proven,
        });
        if must && !proven {
            let mut names = Vec::new();
            let mut vars = Vec::new();
            for n in &self.order {
                if let Some(Binding { var: Some(v), .. }) = self.bindings.get(n) {
                    names.push(n.clone());
                    vars.push(*v);
                }
            }
            let valuation = failing
                .and_then(|s| s.pick(&vars))
                .map(|vals| {
                    names
                        .iter()
                        .zip(vals)
                        .map(|(n, v)| format!("{n} = {v}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let msg = format!("Tier 1 cannot rule out {valuation}, which violates {what}; suggestion: guard {guard} else {{ return ... }}");
            self.report
                .diags
                .push(Diag::new(span.start, span.end, &msg));
        }
    }

    fn infer(&self, e: &Expr, sigs: &Sigs) -> Ty {
        let i64_ty = Ty::Int {
            name: "i64".into(),
            base: int_base("i64").unwrap_or(Range { lo: 0, hi: 0 }),
            refined: None,
        };
        match &e.kind {
            ExprKind::Int(_) => i64_ty,
            ExprKind::Bool(_) | ExprKind::Unary(UnOp::Not, _) => Ty::Bool,
            ExprKind::Name(n) => self.bindings.get(n).map_or(Ty::Unit, |b| b.ty.plain()),
            ExprKind::Unary(_, a) => self.infer(a, sigs),
            ExprKind::Binary(op, a, b) => match op {
                BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or => Ty::Bool,
                _ => match self.infer(a, sigs) {
                    t @ Ty::Int { .. } if !matches!(a.kind, ExprKind::Int(_)) => t,
                    _ => self.infer(b, sigs),
                },
            },
            ExprKind::Index(a, _) => match self.infer(a, sigs) {
                Ty::Array { elem, .. } => elem.plain(),
                _ => Ty::Unit,
            },
            ExprKind::Len(_) => Ty::Int {
                name: "usize".into(),
                base: int_base("usize").unwrap_or(Range { lo: 0, hi: 0 }),
                refined: None,
            },
            ExprKind::Call(name, _) => sigs.get(name).map_or(Ty::Unit, |s| s.ret.plain()),
            ExprKind::ArrayRepeat(v, n) => Ty::Array {
                len: *n,
                elem: Box::new(self.infer(v, sigs)),
            },
        }
    }

    fn eval(&mut self, st: &mut Octagon, e: &Expr, sigs: &Sigs) -> Val {
        match &e.kind {
            ExprKind::Int(v) => Val::Lin(Vec::new(), *v),
            ExprKind::Bool(_) => Val::Bool,
            ExprKind::Name(n) => match self.bindings.get(n) {
                Some(Binding { var: Some(v), .. }) => Val::Lin(vec![Term::pos(*v)], 0),
                Some(Binding {
                    ty: Ty::Array { len, elem },
                    ..
                }) => Val::Array(*len, (**elem).clone()),
                Some(Binding { ty: Ty::Bool, .. }) => Val::Bool,
                _ => {
                    self.diag(e.span, format!("unknown name {n}"));
                    Val::Bad
                }
            },
            ExprKind::Unary(op, a) => {
                let v = self.eval(st, a, sigs);
                match op {
                    UnOp::Not => Val::Bool,
                    UnOp::Neg => negate(&v),
                    UnOp::BitNot => match negate(&v) {
                        Val::Lin(t, c) => Val::Lin(t, c - 1),
                        Val::Itv(lo, hi) => Val::Itv(sat_add(lo, -1), sat_add(hi, -1)),
                        other => other,
                    },
                }
            }
            ExprKind::Binary(op, a, b) => match op {
                BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or => {
                    self.cond(st, e, sigs);
                    Val::Bool
                }
                _ => {
                    let l = self.eval(st, a, sigs);
                    let r = self.eval(st, b, sigs);
                    self.arith(st, *op, &l, &r, b.span)
                }
            },
            ExprKind::Index(a, i) => {
                let av = self.eval(st, a, sigs);
                let iv = self.eval(st, i, sigs);
                match av {
                    Val::Array(len, elem) => {
                        let failing = range_violation(st, &iv, 0, len - 1);
                        let text = expr_text(i);
                        self.record(
                            ObKind::Index,
                            i.span,
                            false,
                            failing,
                            &format!("0 <= {text} < {len}"),
                            &format!("{text} < {len}"),
                        );
                        match elem {
                            Ty::Int { .. } => elem
                                .int_range()
                                .map_or(Val::Bad, |(r, _)| Val::Itv(r.lo, r.hi)),
                            Ty::Array { len, elem } => Val::Array(len, *elem),
                            Ty::Bool => Val::Bool,
                            Ty::Unit => Val::Bad,
                        }
                    }
                    _ => {
                        self.diag(e.span, "only an array can be indexed".into());
                        Val::Bad
                    }
                }
            }
            ExprKind::Len(a) => match self.eval(st, a, sigs) {
                Val::Array(len, _) => Val::Lin(Vec::new(), len),
                _ => {
                    self.diag(e.span, "only an array has .len".into());
                    Val::Bad
                }
            },
            ExprKind::Call(name, args) => self.call(st, name, args, e.span, sigs),
            ExprKind::ArrayRepeat(v, n) => {
                let ty = self.infer(v, sigs);
                self.eval(st, v, sigs);
                Val::Array(*n, ty)
            }
        }
    }

    fn arith(&mut self, st: &Octagon, op: BinOp, l: &Val, r: &Val, rspan: Span) -> Val {
        let (llo, lhi) = interval(st, l);
        let (rlo, rhi) = interval(st, r);
        match op {
            BinOp::Add | BinOp::Sub => {
                let r = if op == BinOp::Sub {
                    negate(r)
                } else {
                    r.clone()
                };
                if let (Val::Lin(a, c), Val::Lin(b, d)) = (l, &r) {
                    if let Some(t) = lin_add(a, b) {
                        return Val::Lin(t, c + d);
                    }
                }
                let (rlo, rhi) = interval(st, &r);
                Val::Itv(sat_add(llo, rlo), sat_add(lhi, rhi))
            }
            BinOp::Mul => {
                if let (Val::Lin(t, c), Val::Lin(u, k)) = (l, r) {
                    if u.is_empty() && (*k == 1 || *k == -1 || *k == 0) {
                        return if *k == 0 {
                            Val::Lin(Vec::new(), 0)
                        } else if *k == 1 {
                            Val::Lin(t.clone(), *c)
                        } else {
                            negate(l)
                        };
                    }
                    if t.is_empty() && (*c == 1 || *c == -1 || *c == 0) {
                        return if *c == 0 {
                            Val::Lin(Vec::new(), 0)
                        } else if *c == 1 {
                            r.clone()
                        } else {
                            negate(r)
                        };
                    }
                }
                if [llo, lhi, rlo, rhi]
                    .iter()
                    .all(|v| v.abs() < INF / 4 && v.abs() < (1i128 << 62))
                {
                    let ps = [llo * rlo, llo * rhi, lhi * rlo, lhi * rhi];
                    Val::Itv(
                        ps.iter().copied().min().unwrap_or(-INF),
                        ps.iter().copied().max().unwrap_or(INF),
                    )
                } else {
                    Val::Itv(-INF, INF)
                }
            }
            BinOp::Div | BinOp::Rem => {
                let failing = nonzero_violation(st, r);
                self.record(
                    ObKind::DivisorNonZero,
                    rspan,
                    false,
                    failing,
                    "a non-zero divisor",
                    "divisor != 0",
                );
                let m = llo.abs().max(lhi.abs());
                if m >= INF {
                    return Val::Itv(-INF, INF);
                }
                if op == BinOp::Div {
                    Val::Itv(-m, m)
                } else {
                    let b = rlo.abs().max(rhi.abs());
                    let b = if b >= INF { m } else { (b - 1).min(m) };
                    if llo >= 0 {
                        Val::Itv(0, b)
                    } else if lhi <= 0 {
                        Val::Itv(-b, 0)
                    } else {
                        Val::Itv(-b, b)
                    }
                }
            }
            BinOp::Shl | BinOp::Shr => {
                let failing = range_violation(st, r, 0, 63);
                self.record(
                    ObKind::ShiftInRange,
                    rspan,
                    false,
                    failing,
                    "a shift amount in range",
                    "amount < 64",
                );
                if op == BinOp::Shr && llo >= 0 {
                    Val::Itv(0, lhi)
                } else {
                    Val::Itv(-INF, INF)
                }
            }
            BinOp::BitAnd if llo >= 0 || rlo >= 0 => Val::Itv(
                0,
                if llo >= 0 && rlo >= 0 {
                    lhi.min(rhi)
                } else if llo >= 0 {
                    lhi
                } else {
                    rhi
                },
            ),
            _ => Val::Itv(-INF, INF),
        }
    }

    /// Evaluates a condition's operands once, recording their obligations; the right operand of
    /// `and` and `or` is evaluated where the left one has decided that it runs.
    fn cond(&mut self, st: &mut Octagon, e: &Expr, sigs: &Sigs) -> Cond {
        match &e.kind {
            ExprKind::Bool(b) => Cond::Const(*b),
            ExprKind::Unary(UnOp::Not, a) => Cond::Not(Box::new(self.cond(st, a, sigs))),
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), a, b) => {
                let l = self.cond(st, a, sigs);
                let mut s2 = st.clone();
                assume(&mut s2, &l, *op == BinOp::And);
                let r = self.cond(&mut s2, b, sigs);
                if *op == BinOp::And {
                    Cond::And(Box::new(l), Box::new(r))
                } else {
                    Cond::Or(Box::new(l), Box::new(r))
                }
            }
            ExprKind::Binary(
                op @ (BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge),
                a,
                b,
            ) => {
                let l = self.eval(st, a, sigs);
                let r = self.eval(st, b, sigs);
                Cond::Cmp(*op, Box::new(l), Box::new(r))
            }
            _ => {
                self.eval(st, e, sigs);
                Cond::Unknown
            }
        }
    }

    /// A state in which `e` fails, or None when it holds wherever `st` does.
    fn refute(&mut self, st: &Octagon, e: &Expr, sigs: &Sigs) -> Option<Octagon> {
        let mut s = st.clone();
        let c = self.cond(&mut s, e, sigs);
        assume(&mut s, &c, false);
        (!s.is_empty()).then_some(s)
    }

    /// Narrows `val` into `ty` at a narrowing point, then gives it to `var`.
    fn narrow_into(
        &mut self,
        st: &mut Octagon,
        var: Option<usize>,
        val: &Val,
        ty: &Ty,
        span: Span,
        what: &str,
    ) {
        if let Some((r, must)) = ty.int_range() {
            let failing = range_violation(st, val, r.lo, r.hi);
            self.record(
                ObKind::Narrow,
                span,
                must,
                failing,
                &format!("{what}: {}", type_text(ty)),
                &format!("{} <= value <= {}", r.lo, r.hi),
            );
            if let Some(v) = var {
                self.assign(st, v, val);
                add_range(st, v, r);
            }
        }
    }

    fn assign(&mut self, st: &mut Octagon, var: usize, val: &Val) {
        match val {
            Val::Lin(terms, c) if terms.iter().any(|t| t.var == var) => {
                let s = self.scratch;
                st.forget(s);
                constrain_eq(st, s, terms, *c);
                st.forget(var);
                constrain_eq(st, var, &[Term::pos(s)], 0);
                st.forget(s);
            }
            Val::Lin(terms, c) => {
                st.forget(var);
                constrain_eq(st, var, terms, *c);
            }
            Val::Itv(lo, hi) => {
                st.forget(var);
                if *hi < INF {
                    st.add_lin(&[Term::pos(var)], *hi);
                }
                if *lo > -INF {
                    st.add_lin(&[Term::pos(var).negated()], -lo);
                }
            }
            _ => st.forget(var),
        }
    }

    fn call(
        &mut self,
        st: &mut Octagon,
        name: &str,
        args: &[Expr],
        span: Span,
        sigs: &Sigs,
    ) -> Val {
        let Some(sig) = sigs.get(name) else {
            self.diag(span, format!("unknown function {name}"));
            return Val::Bad;
        };
        if args.len() != sig.params.len() {
            self.diag(span, format!("{name} takes {} arguments", sig.params.len()));
            return Val::Bad;
        }
        let vars = match self.call_vars.get(&span.start) {
            Some(v) => v.clone(),
            None => {
                let v: Vec<usize> = (0..=sig.params.len()).map(|_| self.alloc(st)).collect();
                self.call_vars.insert(span.start, v.clone());
                v
            }
        };
        let vals: Vec<Val> = args.iter().map(|a| self.eval(st, a, sigs)).collect();
        for (k, (p, v)) in sig.params.iter().zip(&vals).enumerate() {
            self.narrow_into(
                st,
                Some(vars[k]),
                v,
                &p.ty,
                args[k].span,
                &format!("argument {} of {name}", p.name),
            );
        }
        // The callee's parameters, then `result`, name the call's variables while its contracts are read.
        let mut saved = Vec::new();
        for (k, p) in sig.params.iter().enumerate() {
            let var = p.ty.int_range().map(|_| vars[k]);
            saved.push((
                p.name.clone(),
                self.bindings.insert(
                    p.name.clone(),
                    Binding {
                        ty: p.ty.clone(),
                        var,
                        mutable: false,
                    },
                ),
            ));
        }
        for r in &sig.requires {
            let failing = self.refute(st, r, sigs);
            let text = expr_text(r);
            self.record(
                ObKind::Requires,
                span,
                true,
                failing,
                &format!("requires {text} of {name}"),
                &text,
            );
        }
        let rv = vars[sig.params.len()];
        st.forget(rv);
        if let Some((r, _)) = sig.ret.int_range() {
            add_range(st, rv, r);
        }
        let var = sig.ret.int_range().map(|_| rv);
        saved.push((
            "result".to_string(),
            self.bindings.insert(
                "result".to_string(),
                Binding {
                    ty: sig.ret.clone(),
                    var,
                    mutable: false,
                },
            ),
        ));
        let was = self.recording;
        self.recording = false;
        for e in &sig.ensures {
            let c = self.cond(st, e, sigs);
            assume(st, &c, true);
        }
        self.recording = was;
        for (n, b) in saved.into_iter().rev() {
            match b {
                Some(b) => self.bindings.insert(n, b),
                None => self.bindings.remove(&n),
            };
        }
        for &v in &vars[..sig.params.len()] {
            st.forget(v);
        }
        match &sig.ret {
            Ty::Int { .. } => Val::Lin(vec![Term::pos(rv)], 0),
            Ty::Bool => Val::Bool,
            Ty::Array { len, elem } => Val::Array(*len, (**elem).clone()),
            Ty::Unit => Val::Unit,
        }
    }

    fn block(&mut self, st: &mut Octagon, ss: &[Stmt], sigs: &Sigs) {
        for s in ss {
            if st.is_empty() {
                return;
            }
            self.stmt(st, s, sigs);
        }
    }

    fn stmt(&mut self, st: &mut Octagon, s: &Stmt, sigs: &Sigs) {
        match &s.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                init,
            } => {
                let ty = ty.clone().unwrap_or_else(|| self.infer(init, sigs));
                let val = self.eval(st, init, sigs);
                let var = self.declare(st, name, &ty, *mutable);
                self.narrow_into(
                    st,
                    var,
                    &val,
                    &ty,
                    init.span,
                    &format!("{name}'s initializer"),
                );
            }
            StmtKind::Assign { target, op, value } => {
                let Some(b) = self.bindings.get(&target.name).cloned() else {
                    self.diag(target.span, format!("unknown name {}", target.name));
                    return;
                };
                if !b.mutable {
                    self.diag(
                        target.span,
                        format!(
                            "{} is a val or a read-only parameter, which cannot be assigned",
                            target.name
                        ),
                    );
                }
                let mut ty = b.ty.clone();
                for i in &target.indices {
                    let iv = self.eval(st, i, sigs);
                    if let Ty::Array { len, elem } = ty {
                        let failing = range_violation(st, &iv, 0, len - 1);
                        let text = expr_text(i);
                        self.record(
                            ObKind::Index,
                            i.span,
                            false,
                            failing,
                            &format!("0 <= {text} < {len}"),
                            &format!("{text} < {len}"),
                        );
                        ty = *elem;
                    } else {
                        self.diag(i.span, "only an array can be indexed".into());
                        return;
                    }
                }
                let mut val = self.eval(st, value, sigs);
                if let Some(op) = op {
                    let cur = match (target.indices.is_empty(), b.var) {
                        (true, Some(v)) => Val::Lin(vec![Term::pos(v)], 0),
                        _ => ty
                            .int_range()
                            .map_or(Val::Bad, |(r, _)| Val::Itv(r.lo, r.hi)),
                    };
                    val = self.arith(st, *op, &cur, &val, value.span);
                }
                let var = if target.indices.is_empty() {
                    b.var
                } else {
                    None
                };
                self.narrow_into(
                    st,
                    var,
                    &val,
                    &ty,
                    value.span,
                    &format!("the assignment to {}", target.name),
                );
            }
            StmtKind::While {
                cond,
                invariants,
                body,
                ..
            } => self.while_loop(st, cond, invariants, body, sigs),
            StmtKind::If { cond, then, els } => {
                let c = self.cond(st, cond, sigs);
                let mut t = st.clone();
                assume(&mut t, &c, true);
                self.block(&mut t, then, sigs);
                let mut e = st.clone();
                assume(&mut e, &c, false);
                self.block(&mut e, els, sigs);
                *st = t.join(&e);
            }
            StmtKind::Guard { cond, body } => {
                let c = self.cond(st, cond, sigs);
                let mut b = st.clone();
                assume(&mut b, &c, false);
                self.block(&mut b, body, sigs);
                if !b.is_empty() {
                    self.diag(s.span, "a guard's body must not reach its end: leave by return, break, continue or todo".into());
                }
                assume(st, &c, true);
            }
            StmtKind::Return(e) => {
                let val = match e {
                    Some(e) => self.eval(st, e, sigs),
                    None => Val::Unit,
                };
                let span = e.as_ref().map_or(s.span, |e| e.span);
                let ret = self.sig.ret.clone();
                let mut out = st.clone();
                let rv = self.result_var;
                self.narrow_into(&mut out, Some(rv), &val, &ret, span, "the returned value");
                let saved = self.bindings.insert(
                    "result".to_string(),
                    Binding {
                        ty: ret.clone(),
                        var: ret.int_range().map(|_| rv),
                        mutable: false,
                    },
                );
                for en in self.sig.ensures.clone() {
                    let failing = self.refute(&out, &en, sigs);
                    let text = expr_text(&en);
                    self.record(
                        ObKind::Ensures,
                        span,
                        true,
                        failing,
                        &format!("ensures {text}"),
                        &text,
                    );
                }
                match saved {
                    Some(b) => self.bindings.insert("result".to_string(), b),
                    None => self.bindings.remove("result"),
                };
                st.set_empty();
            }
            StmtKind::Break | StmtKind::Continue => {
                let snapshot = st.clone();
                match self.loops.last_mut() {
                    Some(f) if matches!(s.kind, StmtKind::Break) => f.breaks.push(snapshot),
                    Some(f) => f.continues.push(snapshot),
                    None => self.diag(s.span, "break and continue belong inside a loop".into()),
                }
                st.set_empty();
            }
            StmtKind::Expr(e) => {
                self.eval(st, e, sigs);
            }
        }
    }

    /// The state at a loop head: the entry state with every variable the loop changes
    /// forgotten, their types' ranges, and the surviving candidate facts.
    fn loop_head(
        &mut self,
        entry: &Octagon,
        modified: &[usize],
        ranges: &[(usize, Range)],
        cands: &[Expr],
        alive: &[bool],
        sigs: &Sigs,
    ) -> Octagon {
        let mut head = entry.clone();
        for &v in modified {
            head.forget(v);
        }
        for &(v, r) in ranges {
            add_range(&mut head, v, r);
        }
        for (c, ok) in cands.iter().zip(alive) {
            if *ok {
                let cv = self.cond(&mut head, c, sigs);
                assume(&mut head, &cv, true);
            }
        }
        head
    }

    /// Runs the body once from the head; returns the state at the back edge and at the exit.
    fn loop_pass(
        &mut self,
        head: &Octagon,
        cond: &Expr,
        body: &[Stmt],
        sigs: &Sigs,
    ) -> (Octagon, Octagon) {
        let mut s = head.clone();
        let c = self.cond(&mut s, cond, sigs);
        let mut b = s.clone();
        assume(&mut b, &c, true);
        self.loops.push(LoopFrame::default());
        self.block(&mut b, body, sigs);
        let frame = self.loops.pop().unwrap_or_default();
        let back = frame.continues.iter().fold(b, |acc, x| acc.join(x));
        let mut exit = s;
        assume(&mut exit, &c, false);
        let exit = frame.breaks.iter().fold(exit, |acc, x| acc.join(x));
        (back, exit)
    }

    fn while_loop(
        &mut self,
        st: &mut Octagon,
        cond: &Expr,
        invariants: &[Expr],
        body: &[Stmt],
        sigs: &Sigs,
    ) {
        let mut names = Vec::new();
        assigned(body, &mut names);
        let mut modified = Vec::new();
        let mut ranges = Vec::new();
        for n in &names {
            if let Some(Binding {
                var: Some(v), ty, ..
            }) = self.bindings.get(n)
            {
                modified.push(*v);
                if let Some((r, _)) = ty.int_range() {
                    ranges.push((*v, r));
                }
            }
        }
        // Candidates come only from the loop's text: written invariants first, then the
        // condition's conjuncts and the guards' conditions.
        let mut cands: Vec<Expr> = Vec::new();
        for i in invariants {
            conjuncts(i, &mut cands);
        }
        let written = cands.len();
        conjuncts(cond, &mut cands);
        guard_conditions(body, &mut cands);

        let was = self.recording;
        self.recording = false;
        let entry_ok: Vec<bool> = cands
            .iter()
            .map(|c| self.refute(st, c, sigs).is_none())
            .collect();
        let mut alive = entry_ok.clone();
        let mut back;
        loop {
            let head = self.loop_head(st, &modified, &ranges, &cands, &alive, sigs);
            back = self.loop_pass(&head, cond, body, sigs).0;
            let mut changed = false;
            for k in 0..cands.len() {
                if alive[k] && self.refute(&back, &cands[k], sigs).is_some() {
                    alive[k] = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.recording = was;

        for k in 0..written {
            let failing = if !entry_ok[k] {
                self.refute(st, &cands[k], sigs)
            } else if !alive[k] {
                self.refute(&back, &cands[k], sigs)
            } else {
                None
            };
            let text = expr_text(&cands[k]);
            let what = if entry_ok[k] {
                format!("invariant {text}, which the body does not preserve")
            } else {
                format!("invariant {text} on entry")
            };
            self.record(
                ObKind::Invariant,
                cands[k].span,
                true,
                failing,
                &what,
                &text,
            );
        }
        let head = self.loop_head(st, &modified, &ranges, &cands, &alive, sigs);
        let (_, exit) = self.loop_pass(&head, cond, body, sigs);
        *st = exit;
    }
}

fn add_range(st: &mut Octagon, v: usize, r: Range) {
    st.add_lin(&[Term::pos(v)], r.hi);
    st.add_lin(&[Term::pos(v).negated()], -r.lo);
}

/// `x = terms + c`, exactly when `terms` has at most one term, else as the octagon facts
/// the sum allows.
fn constrain_eq(st: &mut Octagon, x: usize, terms: &[Term], c: i128) {
    let xt = Term::pos(x);
    match terms {
        [] => {
            st.add_lin(&[xt], c);
            st.add_lin(&[xt.negated()], -c);
        }
        [t] => {
            st.add_lin(&[xt, t.negated()], c);
            st.add_lin(&[xt.negated(), *t], -c);
        }
        [a, b] => {
            for (t, other) in [(*a, *b), (*b, *a)] {
                let hi = sat_add(st.upper(&[other]), c);
                let lo = sat_add(-st.upper(&[other.negated()]), c);
                if hi < INF {
                    st.add_lin(&[xt, t.negated()], hi);
                }
                if lo > -INF {
                    st.add_lin(&[xt.negated(), t], -lo);
                }
            }
            let (lo, hi) = interval(st, &Val::Lin(terms.to_vec(), c));
            if hi < INF {
                st.add_lin(&[xt], hi);
            }
            if lo > -INF {
                st.add_lin(&[xt.negated()], -lo);
            }
        }
        _ => {}
    }
}

fn negate(v: &Val) -> Val {
    match v {
        Val::Lin(t, c) => Val::Lin(neg_terms(t), -c),
        Val::Itv(lo, hi) => Val::Itv(-hi, -lo),
        other => other.clone(),
    }
}

fn interval(st: &Octagon, v: &Val) -> (i128, i128) {
    match v {
        Val::Lin(t, c) => {
            let hi = st.upper(t);
            let lo = st.upper(&neg_terms(t));
            (
                if lo >= INF { -INF } else { sat_add(-lo, *c) },
                sat_add(hi, *c),
            )
        }
        Val::Itv(lo, hi) => (*lo, *hi),
        _ => (-INF, INF),
    }
}

/// A state in which `val` falls outside `[lo, hi]`, or None when it never does.
fn range_violation(st: &Octagon, val: &Val, lo: i128, hi: i128) -> Option<Octagon> {
    match val {
        Val::Lin(t, c) => {
            if !st.implies(t, hi - c) {
                let mut s = st.clone();
                s.add_lin(&neg_terms(t), c - hi - 1);
                return Some(s);
            }
            if !st.implies(&neg_terms(t), c - lo) {
                let mut s = st.clone();
                s.add_lin(t, lo - c - 1);
                return Some(s);
            }
            None
        }
        _ => {
            let (a, b) = interval(st, val);
            (a < lo || b > hi).then(|| st.clone())
        }
    }
}

fn nonzero_violation(st: &Octagon, val: &Val) -> Option<Octagon> {
    let (lo, hi) = interval(st, val);
    if lo > 0 || hi < 0 {
        return None;
    }
    let mut s = st.clone();
    if let Val::Lin(t, c) = val {
        s.add_lin(t, -c);
        s.add_lin(&neg_terms(t), *c);
    }
    (!s.is_empty()).then_some(s)
}

fn flip(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Ge,
        BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le,
        BinOp::Ge => BinOp::Lt,
        BinOp::Eq => BinOp::Ne,
        BinOp::Ne => BinOp::Eq,
        other => other,
    }
}

/// Narrows `st` to the points where `c` has the value `polarity`.
fn assume(st: &mut Octagon, c: &Cond, polarity: bool) {
    if st.is_empty() {
        return;
    }
    match c {
        Cond::Const(b) => {
            if *b != polarity {
                st.set_empty();
            }
        }
        Cond::Not(a) => assume(st, a, !polarity),
        Cond::And(a, b) | Cond::Or(a, b) => {
            let conj = matches!(c, Cond::And(..)) == polarity;
            if conj {
                assume(st, a, polarity);
                assume(st, b, polarity);
            } else {
                let mut x = st.clone();
                assume(&mut x, a, polarity);
                assume(st, b, polarity);
                *st = x.join(st);
            }
        }
        Cond::Cmp(op, l, r) => {
            let op = if polarity { *op } else { flip(*op) };
            let (l, r) = (&**l, &**r);
            let diff = match (l, r) {
                (Val::Lin(a, c), Val::Lin(b, d)) => lin_add(a, &neg_terms(b)).map(|t| (t, c - d)),
                _ => None,
            };
            if let Some((t, k)) = diff {
                // terms + k (op) 0
                match op {
                    BinOp::Lt => st.add_lin(&t, -k - 1),
                    BinOp::Le => st.add_lin(&t, -k),
                    BinOp::Gt => st.add_lin(&neg_terms(&t), k - 1),
                    BinOp::Ge => st.add_lin(&neg_terms(&t), k),
                    BinOp::Eq => {
                        st.add_lin(&t, -k);
                        st.add_lin(&neg_terms(&t), k);
                    }
                    BinOp::Ne => {
                        // Below or above: the join of the two, which is empty when both are.
                        let mut below = st.clone();
                        below.add_lin(&t, -k - 1);
                        st.add_lin(&neg_terms(&t), k - 1);
                        *st = below.join(st);
                    }
                    _ => {}
                }
                return;
            }
            // One side exact, the other an interval: bound the exact side.
            let (lo, hi) = interval(st, r);
            if let Val::Lin(t, c) = l {
                match op {
                    BinOp::Lt if hi < INF => st.add_lin(t, hi - 1 - c),
                    BinOp::Le if hi < INF => st.add_lin(t, hi - c),
                    BinOp::Gt if lo > -INF => st.add_lin(&neg_terms(t), c - lo - 1),
                    BinOp::Ge if lo > -INF => st.add_lin(&neg_terms(t), c - lo),
                    _ => {}
                }
            }
            let (lo, hi) = interval(st, l);
            if let Val::Lin(t, c) = r {
                match op {
                    BinOp::Gt if hi < INF => st.add_lin(t, hi - 1 - c),
                    BinOp::Ge if hi < INF => st.add_lin(t, hi - c),
                    BinOp::Lt if lo > -INF => st.add_lin(&neg_terms(t), c - lo - 1),
                    BinOp::Le if lo > -INF => st.add_lin(&neg_terms(t), c - lo),
                    _ => {}
                }
            }
        }
        Cond::Unknown => {}
    }
}

fn type_text(ty: &Ty) -> String {
    match ty {
        Ty::Int {
            name,
            refined: Some(r),
            ..
        } => format!("{name}[{}..={}]", r.lo, r.hi),
        Ty::Int { name, .. } => name.clone(),
        Ty::Bool => "bool".into(),
        Ty::Array { len, elem } => format!("[{len}]{}", type_text(elem)),
        Ty::Unit => "()".into(),
    }
}

/// Source-like text of an expression, for diagnostics.
pub fn expr_text(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(v) => v.to_string(),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::Name(n) => n.clone(),
        ExprKind::Unary(op, a) => format!(
            "{}{}",
            match op {
                UnOp::Neg => "-",
                UnOp::BitNot => "~",
                UnOp::Not => "not ",
            },
            expr_text(a)
        ),
        ExprKind::Binary(op, a, b) => {
            let o = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Rem => "%",
                BinOp::BitAnd => "&",
                BinOp::BitOr => "|",
                BinOp::BitXor => "^",
                BinOp::Shl => "<<",
                BinOp::Shr => ">>",
                BinOp::Eq => "==",
                BinOp::Ne => "!=",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                BinOp::Ge => ">=",
                BinOp::And => "and",
                BinOp::Or => "or",
            };
            format!("{} {o} {}", expr_text(a), expr_text(b))
        }
        ExprKind::Index(a, i) => format!("{}[{}]", expr_text(a), expr_text(i)),
        ExprKind::Len(a) => format!("{}.len", expr_text(a)),
        ExprKind::Call(n, args) => format!(
            "{n}({})",
            args.iter().map(expr_text).collect::<Vec<_>>().join(", ")
        ),
        ExprKind::ArrayRepeat(v, n) => format!("[{}; {n}]", expr_text(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_module;

    fn check(src: &str) -> Vec<Report> {
        let fs = parse_module(src).unwrap();
        let sigs: Sigs = fs
            .iter()
            .map(|f| (f.sig.name.clone(), f.sig.clone()))
            .collect();
        fs.iter().map(|f| check_function(f, &sigs)).collect()
    }

    fn must_failures(r: &Report) -> usize {
        r.obligations.iter().filter(|o| o.must && !o.proven).count()
    }

    #[test]
    fn test_a_loop_over_an_array_is_proven_in_bounds() {
        let r = &check(
            "fn sum(a: [16]i32, n: usize[0..=16]) -> i64 {\n    var i: usize = 0\n    var s: i64 = 0\n    while i < n {\n        s += a[i]\n        i += 1\n    }\n    return s\n}\n",
        )[0];
        let index = r
            .obligations
            .iter()
            .find(|o| o.kind == ObKind::Index)
            .unwrap();
        assert!(index.proven, "{r:?}");
        assert!(r.diags.is_empty(), "{r:?}");
    }

    #[test]
    fn test_a_refined_result_that_can_overflow_gets_a_valuation_and_a_guard() {
        let r = &check("fn next(x: u8) -> u8[1..=255] {\n    return x + 1\n}\n")[0];
        assert_eq!(must_failures(r), 1, "{r:?}");
        let msg = &r.diags[0].message;
        assert!(msg.contains("cannot rule out x = 255"), "{msg}");
        assert!(msg.contains("guard"), "{msg}");
        let ok = &check("fn next(x: u8) -> u8[1..=255] {\n    guard x < 255 else { return 1 }\n    return x + 1\n}\n")[0];
        assert_eq!(must_failures(ok), 0, "{ok:?}");
    }

    #[test]
    fn test_requires_are_checked_at_calls_and_ensures_assumed() {
        let src = "fn pick(lo: usize[0..64], hi: usize[0..=64]) -> usize[0..=64]\n    requires lo < hi\n    ensures lo <= result and result < hi\n{\n    return lo\n}\n\nfn use_it(a: [64]i32, n: usize[0..=64]) -> i32 {\n    guard n > 0 else { return 0 }\n    val k: usize = pick(0, n)\n    return a[k]\n}\n\nfn bad(n: usize[0..=64]) -> usize {\n    return pick(0, n)\n}\n";
        let rs = check(src);
        assert_eq!(must_failures(&rs[0]), 0, "{:?}", rs[0]);
        assert_eq!(must_failures(&rs[1]), 0, "{:?}", rs[1]);
        assert!(
            rs[1]
                .obligations
                .iter()
                .any(|o| o.kind == ObKind::Index && o.proven),
            "{:?}",
            rs[1]
        );
        assert_eq!(must_failures(&rs[2]), 1, "{:?}", rs[2]);
        assert!(
            rs[2].diags[0].message.contains("requires lo < hi of pick"),
            "{:?}",
            rs[2].diags
        );
        assert!(
            rs[2].diags[0].message.contains("n = 0"),
            "{:?}",
            rs[2].diags
        );
    }

    #[test]
    fn test_houdini_keeps_the_inductive_candidates_only() {
        // `lo <= i` and `i <= hi` are inductive and written; `i < hi` (the condition) is not,
        // and is dropped, so without the written `i <= hi` nothing bounds `i` after the loop.
        let ok = &check(
            "fn f(lo: usize[0..64], hi: usize[0..=64]) -> usize[0..=64]\n    requires lo < hi\n{\n    var i: usize = lo\n    while i < hi invariant lo <= i invariant i <= hi {\n        i += 1\n    }\n    return i\n}\n",
        )[0];
        assert_eq!(must_failures(ok), 0, "{ok:?}");
        let unwritten = &check(
            "fn f(lo: usize[0..64], hi: usize[0..=64]) -> usize[0..=64]\n    requires lo < hi\n{\n    var i: usize = lo\n    while i < hi invariant lo <= i {\n        i += 1\n    }\n    return i\n}\n",
        )[0];
        assert_eq!(must_failures(unwritten), 1, "{unwritten:?}");
        let bad = &check(
            "fn f(lo: usize[0..64], hi: usize[0..=64]) -> usize\n    requires lo < hi\n{\n    var i: usize = lo\n    while i < hi invariant i == lo {\n        i += 1\n    }\n    return i\n}\n",
        )[0];
        assert_eq!(must_failures(bad), 1, "{bad:?}");
        assert!(
            bad.diags[0].message.contains("does not preserve"),
            "{:?}",
            bad.diags
        );
    }

    #[test]
    fn test_a_plain_narrowing_tier_1_cannot_prove_stays_a_run_time_check() {
        let r = &check("fn add(a: i32, b: i32) -> i32 {\n    return a + b\n}\n")[0];
        assert_eq!(r.obligations.len(), 1);
        assert!(!r.obligations[0].proven && !r.obligations[0].must);
        assert!(r.diags.is_empty());
    }

    #[test]
    fn test_break_is_one_more_exit_of_its_loop() {
        let r = &check(
            "fn f(n: usize[0..=10]) -> usize[0..=10] {\n    var i: usize = 0\n    while i < n invariant i <= n {\n        guard i != 5 else { break }\n        i += 1\n    }\n    return i\n}\n",
        )[0];
        assert_eq!(must_failures(r), 0, "{r:?}");
    }
}
