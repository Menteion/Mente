//! The type checker of M1's functions (`spec/core-calculus.md`, items 2 to 7).
//!
//! [`check`] gives each expression and binding of one function its [`Ty`], after
//! [`crate::names::resolve`] has bound its names. It reports a value of the wrong kind where
//! the spec fixes one (`E0203`), a body that can end without its result (`E0210`), a `guard`
//! whose `else` can end (`E0211`), an unannotated binding computed from operands of different
//! integer types (`E0212`), a call with the wrong number of arguments (`E0213`) or the wrong
//! `mut` marks (`E0214`), `.len` of a value that is not an array (`E0215`), and a bit
//! operation on operands of different integer types (`E0216`). It checks the parameter modes
//! too: only a `var` or a `mut` parameter is assigned (`E0205`) or passed as `mut` (`E0206`),
//! and a `mut` argument's binding appears in no other argument of its call (`E0207`).

use std::cmp::Ordering;

use mente_syntax::diagnostics::{insertion_point, Applicability, Diagnostic};
use mente_syntax::tree::BinaryOp::{BitAnd, BitOr, BitXor, Shl, Shr};
use mente_syntax::tree::{
    Arg, AssignOp, BinaryOp, BlockId, Body, ContractKind, Else, ExprId, ExprKind, FnSyntax, StmtId,
    StmtKind, Tree, TypeId, UnaryOp,
};
use mente_syntax::Span;

use crate::names::{BindingId, BindingKind, Resolution};
use crate::types::{Int, IntTy, Ty, USIZE};

/// Any integer type, as a place that takes one.
const ANY_INT: Ty = Ty::Int(Int::Literal);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigParam {
    pub name: String,
    pub is_mut: bool,
    pub ty: Ty,
}

/// What a call needs to know of the function it calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub params: Vec<SigParam>,
    pub result: Option<Ty>,
}

impl Signature {
    pub fn of(f: &FnSyntax) -> Signature {
        Signature {
            params: f
                .params
                .iter()
                .map(|p| SigParam {
                    name: p.name.clone(),
                    is_mut: p.is_mut,
                    ty: Ty::of(&f.tree, p.ty),
                })
                .collect(),
            result: f.result.map(|t| Ty::of(&f.tree, t)),
        }
    }
}

/// The type of each expression and each binding of one function, indexed like its tree and
/// its [`Resolution`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Typing {
    pub exprs: Vec<Ty>,
    pub bindings: Vec<Ty>,
}

/// Checks the types of `f`, whose names `resolution` binds. `signatures` holds, for each
/// function of the module in order, its signature when its body was parsed.
pub fn check(
    f: &FnSyntax,
    resolution: &Resolution,
    signatures: &[Option<Signature>],
) -> (Typing, Vec<Diagnostic>) {
    let tree = &f.tree;
    let result = f.result.map(|t| Ty::of(tree, t));
    let bindings = resolution
        .bindings
        .iter()
        .map(|b| match (b.kind, b.ty) {
            (BindingKind::Result, _) => result.clone().unwrap_or(Ty::Unit),
            (_, Some(t)) => Ty::of(tree, t),
            (_, None) => Ty::Any,
        })
        .collect();
    let mut c = Checker {
        tree,
        name: &f.name,
        resolution,
        signatures,
        result,
        typing: Typing {
            exprs: vec![Ty::Any; tree.exprs.len()],
            bindings,
        },
        errors: Vec::new(),
    };
    for contract in &f.contracts {
        match contract.kind {
            ContractKind::Requires | ContractKind::Ensures => {
                c.expect(contract.expr, &Ty::Bool, "as a contract clause");
            }
            ContractKind::Decreases => {
                c.expect(contract.expr, &ANY_INT, "as a `decreases` measure");
            }
        }
    }
    match f.body {
        Some(Body::Expr(e)) => match c.result.clone() {
            Some(want) => {
                let place = c.as_result();
                c.expect(e, &want, &place);
            }
            None => c.effect(e),
        },
        Some(Body::Block(b)) => c.body(b),
        None => {}
    }
    (c.typing, c.errors)
}

struct Checker<'a> {
    tree: &'a Tree,
    name: &'a str,
    resolution: &'a Resolution,
    signatures: &'a [Option<Signature>],
    result: Option<Ty>,
    typing: Typing,
    errors: Vec<Diagnostic>,
}

impl Checker<'_> {
    fn span(&self, e: ExprId) -> Span {
        self.tree.expr(e).span
    }

    fn as_result(&self) -> String {
        format!("as the result of `{}`", self.name)
    }

    fn mismatch(&mut self, e: ExprId, message: String) {
        let span = self.span(e);
        self.errors.push(Diagnostic::error("E0203", span, message));
    }

    /// Why `e` has no value, when its kind says.
    fn no_value_hint(&self, e: ExprId) -> String {
        match &self.tree.expr(e).kind {
            ExprKind::If {
                otherwise: None, ..
            } => "; an `if` without `else` has no value".to_string(),
            ExprKind::Call { callee, .. } => format!("; `{}` has no result type", callee),
            _ => String::new(),
        }
    }

    /// Reports `e`, of type `found`, unless it flows into `want`.
    fn check_ty(&mut self, e: ExprId, found: &Ty, want: &Ty, place: &str) {
        if !want.accepts(found) {
            let hint = if *found == Ty::Unit {
                self.no_value_hint(e)
            } else {
                String::new()
            };
            self.mismatch(
                e,
                format!(
                    "expected {} {}, found {}{}",
                    want.describe(),
                    place,
                    found.describe(),
                    hint
                ),
            );
        }
    }

    /// The type of `e`, which must flow into `want`.
    fn expect(&mut self, e: ExprId, want: &Ty, place: &str) -> Ty {
        let found = self.value(e);
        self.check_ty(e, &found, want, place);
        found
    }

    /// The type of `e`, whose value is used.
    fn value(&mut self, e: ExprId) -> Ty {
        let ty = self.compute(e, true);
        self.typing.exprs[e.0 as usize] = ty.clone();
        ty
    }

    /// Checks `e`, whose value is discarded.
    fn effect(&mut self, e: ExprId) {
        let ty = self.compute(e, false);
        self.typing.exprs[e.0 as usize] = ty;
    }

    fn compute(&mut self, e: ExprId, used: bool) -> Ty {
        let tree = self.tree;
        match &tree.expr(e).kind {
            ExprKind::Int { suffix, .. } => suffix
                .as_deref()
                .and_then(IntTy::named)
                .map_or(ANY_INT, Ty::int),
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Name(_) => self.resolution.names[e.0 as usize]
                .map_or(Ty::Any, |id| self.typing.bindings[id.0 as usize].clone()),
            ExprKind::Todo(_) | ExprKind::Unreachable(_) | ExprKind::Error => Ty::Any,
            ExprKind::Unary { op, operand } => self.unary(*op, *operand),
            ExprKind::Binary { op, lhs, rhs } => {
                let l = self.value(*lhs);
                let r = self.value(*rhs);
                self.binary(e, *op, (*lhs, &l), (*rhs, &r))
            }
            ExprKind::Index { base, index } => self.index(*base, *index),
            ExprKind::Field { base, .. } => self.field(e, *base),
            ExprKind::Call { callee, args } => self.call(e, callee, args),
            ExprKind::Array(items) => self.array(items),
            ExprKind::Repeat { value, count } => {
                let elem = self.value(*value);
                Ty::Array(*count, Box::new(elem))
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => self.if_expr(e, *cond, *then, *otherwise, used),
        }
    }

    fn unary(&mut self, op: UnaryOp, operand: ExprId) -> Ty {
        match op {
            UnaryOp::Not => {
                self.expect(operand, &Ty::Bool, "as the operand of `not`");
                Ty::Bool
            }
            UnaryOp::Neg | UnaryOp::BitNot => {
                let symbol = if op == UnaryOp::Neg { "-" } else { "~" };
                let found = self.expect(
                    operand,
                    &ANY_INT,
                    &format!("as the operand of `{}`", symbol),
                );
                match found {
                    Ty::Int(_) => found,
                    _ => Ty::Any,
                }
            }
        }
    }

    /// The integer type of an operand, or `None` after reporting one that is not an integer.
    /// An operand of any type counts as a literal, so the operation still has an integer type.
    fn int_operand(&mut self, e: ExprId, found: &Ty, place: &str) -> Option<Int> {
        self.check_ty(e, found, &ANY_INT, place);
        match found {
            Ty::Int(i) => Some(*i),
            Ty::Any => Some(Int::Literal),
            _ => None,
        }
    }

    fn binary(
        &mut self,
        at: ExprId,
        op: BinaryOp,
        (l, lt): (ExprId, &Ty),
        (r, rt): (ExprId, &Ty),
    ) -> Ty {
        let symbol = op.symbol();
        let place = format!("as an operand of `{}`", symbol);
        match op {
            BinaryOp::And | BinaryOp::Or => {
                self.check_ty(l, lt, &Ty::Bool, &place);
                self.check_ty(r, rt, &Ty::Bool, &place);
                Ty::Bool
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let comparable = matches!(
                    (lt, rt),
                    (Ty::Int(_), Ty::Int(_)) | (Ty::Bool, Ty::Bool) | (Ty::Any, _) | (_, Ty::Any)
                );
                if !comparable {
                    self.mismatch(
                        at,
                        format!(
                            "`{}` compares two integers or two bools, not {} and {}",
                            symbol,
                            lt.describe(),
                            rt.describe()
                        ),
                    );
                }
                Ty::Bool
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                self.int_operand(l, lt, &place);
                self.int_operand(r, rt, &place);
                Ty::Bool
            }
            _ => {
                let a = self.int_operand(l, lt, &place);
                let b = self.int_operand(r, rt, &place);
                let (Some(a), Some(b)) = (a, b) else {
                    return Ty::Any;
                };
                match op {
                    Shl | Shr => Ty::Int(a),
                    BitAnd | BitOr | BitXor => match a.join(b) {
                        Int::Mixed(_) => {
                            self.errors.push(Diagnostic::error(
                                "E0216",
                                self.tree.expr(at).span,
                                format!(
                                    "`{}` works on the bits of one integer type, but its operands are {} and {}; bind one of them to a `val` of the other's type first",
                                    symbol,
                                    lt.describe(),
                                    rt.describe()
                                ),
                            ));
                            Ty::Any
                        }
                        one => Ty::Int(one),
                    },
                    _ => Ty::Int(a.join(b)),
                }
            }
        }
    }

    fn index(&mut self, base: ExprId, index: ExprId) -> Ty {
        let found = self.value(base);
        self.expect(index, &ANY_INT, "as an index");
        match found {
            Ty::Array(_, elem) => *elem,
            Ty::Any => Ty::Any,
            other => {
                self.mismatch(
                    base,
                    format!(
                        "only an array can be indexed, but this is {}",
                        other.describe()
                    ),
                );
                Ty::Any
            }
        }
    }

    /// `.len`, the only field the body parser accepts.
    fn field(&mut self, at: ExprId, base: ExprId) -> Ty {
        match self.value(base) {
            Ty::Array(..) => Ty::int(USIZE),
            Ty::Any => Ty::Any,
            other => {
                let span = self.tree.expr(at).span;
                self.errors.push(Diagnostic::error(
                    "E0215",
                    span,
                    format!(
                        "`.len` is an array's length, but this is {}",
                        other.describe()
                    ),
                ));
                Ty::Any
            }
        }
    }

    fn call(&mut self, at: ExprId, callee: &str, args: &[Arg]) -> Ty {
        let signature = self.resolution.calls[at.0 as usize]
            .and_then(|i| self.signatures.get(i).cloned().flatten());
        let Some(sig) = signature else {
            for (i, a) in args.iter().enumerate() {
                self.value(a.value);
                if a.is_mut {
                    self.mut_argument(args, i);
                }
            }
            return Ty::Any;
        };
        let result = sig.result.clone().unwrap_or(Ty::Unit);
        if args.len() != sig.params.len() {
            for a in args {
                self.value(a.value);
            }
            let span = self.tree.expr(at).span;
            self.errors.push(Diagnostic::error(
                "E0213",
                span,
                format!(
                    "`{}` takes {} argument{}, but this call passes {}",
                    callee,
                    sig.params.len(),
                    if sig.params.len() == 1 { "" } else { "s" },
                    args.len()
                ),
            ));
            return result;
        }
        for (i, (a, p)) in args.iter().zip(&sig.params).enumerate() {
            let found = self.value(a.value);
            let span = self.span(a.value);
            if p.is_mut && !a.is_mut {
                let mut d = Diagnostic::error(
                    "E0214",
                    span,
                    format!(
                        "parameter `{}` of `{}` is `mut`, so the call marks its argument `mut`, as in `{}(mut x)`",
                        p.name, callee, callee
                    ),
                );
                if is_place(self.tree, a.value) {
                    d = d.with_fix(
                        Applicability::MachineApplicable,
                        "mark the argument `mut`",
                        insertion_point(span),
                        "mut ",
                    );
                }
                self.errors.push(d);
            } else if a.is_mut && !p.is_mut {
                self.errors.push(Diagnostic::error(
                    "E0214",
                    span,
                    format!(
                        "parameter `{}` of `{}` is read-only, so its argument is not marked `mut`",
                        p.name, callee
                    ),
                ));
            } else {
                if a.is_mut {
                    self.mut_argument(args, i);
                    self.mut_type(a.value, &found, p, callee);
                } else {
                    let place = format!("for parameter `{}` of `{}`", p.name, callee);
                    self.check_ty(a.value, &found, &p.ty, &place);
                }
            }
        }
        result
    }

    /// A `mut` argument has its parameter's type exactly (`E0203`). When it is a `var` declared
    /// without a type, the fix states the parameter's.
    fn mut_type(&mut self, place: ExprId, found: &Ty, p: &SigParam, callee: &str) {
        if p.ty.same_as(found) {
            return;
        }
        let mut d = Diagnostic::error(
            "E0203",
            self.span(place),
            format!(
                "a `mut` argument has its parameter's type exactly, since the call writes it back: `{}` of `{}` is {}, but this is {}",
                p.name,
                callee,
                p.ty.describe(),
                found.describe()
            ),
        );
        let binding = match &self.tree.expr(place).kind {
            ExprKind::Name(_) => self.resolution.names[place.0 as usize]
                .map(|id| &self.resolution.bindings[id.0 as usize]),
            _ => None,
        };
        if let Some(b) = binding.filter(|b| b.kind == BindingKind::Var && b.ty.is_none()) {
            let width = (b.span.end - b.span.start) as u32;
            let at = Span {
                start: b.span.end,
                end: b.span.end,
                line: b.span.line,
                col: b.span.col + width,
            };
            d = d.with_fix(
                Applicability::Suggestion,
                format!("declare `{}` as `{}`", b.name, p.ty.spelling()),
                at,
                format!(": {}", p.ty.spelling()),
            );
        }
        self.errors.push(d);
    }

    /// A `mut` argument names a `var` or a `mut` parameter (`E0206`), which no other argument
    /// of the call names (`E0207`).
    fn mut_argument(&mut self, args: &[Arg], i: usize) {
        let place = args[i].value;
        self.changeable(place, "E0206", "pass as `mut`");
        let Some(root) = self.root(place) else {
            return;
        };
        for (j, other) in args.iter().enumerate() {
            match j.cmp(&i) {
                Ordering::Equal => continue,
                // Two `mut` arguments that name one binding are reported once, at the second.
                Ordering::Less if other.is_mut => continue,
                _ => {}
            }
            let mut used = Vec::new();
            self.names_in(other.value, &mut used);
            for at in used
                .into_iter()
                .filter(|(id, _)| *id == root)
                .map(|(_, at)| at)
            {
                let name = self.resolution.bindings[root.0 as usize].name.clone();
                self.errors.push(
                    Diagnostic::error(
                        "E0207",
                        at,
                        format!(
                            "`{}` is passed as `mut` in this call, so no other argument may name it; copy what this argument needs into a `val` first",
                            name
                        ),
                    )
                    .with_related(self.span(place), "passed as `mut` here"),
                );
            }
        }
    }

    /// The binding at the root of a place: `a` for `a`, `a[i]` and `a[i][j]`.
    fn root(&self, place: ExprId) -> Option<BindingId> {
        match &self.tree.expr(place).kind {
            ExprKind::Name(_) => self.resolution.names[place.0 as usize],
            ExprKind::Index { base, .. } => self.root(*base),
            _ => None,
        }
    }

    /// Every resolved name in `e`, with its span.
    fn names_in(&self, e: ExprId, out: &mut Vec<(BindingId, Span)>) {
        if let Some(id) = self.resolution.names[e.0 as usize] {
            out.push((id, self.span(e)));
        }
        let tree = self.tree;
        match &tree.expr(e).kind {
            ExprKind::Unary { operand, .. } => self.names_in(*operand, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.names_in(*lhs, out);
                self.names_in(*rhs, out);
            }
            ExprKind::Index { base, index } => {
                self.names_in(*base, out);
                self.names_in(*index, out);
            }
            ExprKind::Field { base, .. } => self.names_in(*base, out),
            ExprKind::Call { args, .. } => {
                for a in args {
                    self.names_in(a.value, out);
                }
            }
            ExprKind::Array(items) => {
                for &i in items {
                    self.names_in(i, out);
                }
            }
            ExprKind::Repeat { value, .. } => self.names_in(*value, out),
            _ => {}
        }
    }

    /// A place that is assigned or passed as `mut` is a `var` or a `mut` parameter.
    fn changeable(&mut self, place: ExprId, code: &'static str, action: &str) {
        let Some(id) = self.root(place) else {
            return;
        };
        let b = &self.resolution.bindings[id.0 as usize];
        let at = self.span(place);
        let d = match b.kind {
            BindingKind::Val => Diagnostic::error(
                code,
                at,
                format!(
                    "cannot {} `{}`: it is a `val`, which never changes; declare it with `var`",
                    action, b.name
                ),
            )
            .with_related(b.span, "declared here"),
            BindingKind::Param { is_mut: false } => Diagnostic::error(
                code,
                at,
                format!(
                    "cannot {} `{}`: it is a read-only parameter; make it `mut {}`, and pass `mut` at its calls",
                    action, b.name, b.name
                ),
            )
            .with_fix(
                Applicability::Suggestion,
                format!("make `{}` a `mut` parameter", b.name),
                insertion_point(b.span),
                "mut ",
            ),
            BindingKind::Result => Diagnostic::error(
                code,
                at,
                format!(
                    "cannot {} `result`, the value an `ensures` clause reads",
                    action
                ),
            ),
            BindingKind::Var | BindingKind::Param { is_mut: true } => return,
        };
        self.errors.push(d);
    }

    fn array(&mut self, items: &[ExprId]) -> Ty {
        let mut elem = Ty::Any;
        for &i in items {
            let found = self.value(i);
            match elem.join(&found) {
                Some(joined) => elem = joined,
                None => self.mismatch(
                    i,
                    format!(
                        "the elements of an array literal have one type, but this is {} after {}",
                        found.describe(),
                        elem.describe()
                    ),
                ),
            }
        }
        Ty::Array(items.len() as u128, Box::new(elem))
    }

    fn if_expr(
        &mut self,
        at: ExprId,
        cond: ExprId,
        then: BlockId,
        otherwise: Option<Else>,
        used: bool,
    ) -> Ty {
        self.expect(cond, &Ty::Bool, "as the condition of `if`");
        let then_ty = self.block(then, used);
        let Some(otherwise) = otherwise else {
            return Ty::Unit;
        };
        let else_ty = match otherwise {
            Else::Block(b) => self.block(b, used),
            Else::If(i) if used => self.value(i),
            Else::If(i) => {
                self.effect(i);
                Ty::Unit
            }
        };
        if !used {
            return Ty::Unit;
        }
        // A branch that leaves has any type.
        let then_ty = if block_ends(self.tree, then) {
            then_ty
        } else {
            Ty::Any
        };
        let else_ty = if else_ends(self.tree, otherwise) {
            else_ty
        } else {
            Ty::Any
        };
        then_ty.join(&else_ty).unwrap_or_else(|| {
            self.mismatch(
                at,
                format!(
                    "the branches of this `if` produce its value, so they have one type, but they are {} and {}",
                    then_ty.describe(),
                    else_ty.describe()
                ),
            );
            Ty::Any
        })
    }

    /// The type of a block: its last statement's when its value is used and that is an
    /// expression, otherwise no value.
    fn block(&mut self, b: BlockId, used: bool) -> Ty {
        let stmts = &self.tree.block(b).stmts;
        let mut ty = Ty::Unit;
        for (i, &s) in stmts.iter().enumerate() {
            ty = self.stmt(s, used && i + 1 == stmts.len());
        }
        ty
    }

    fn stmt(&mut self, s: StmtId, used: bool) -> Ty {
        match &self.tree.stmt(s).kind {
            StmtKind::Expr(e) if used => return self.value(*e),
            StmtKind::Expr(e) => self.effect(*e),
            StmtKind::Binding {
                name,
                name_span,
                ty,
                init,
                ..
            } => self.binding(s, name, *name_span, *ty, *init),
            StmtKind::Assign { place, op, value } => self.assign(*place, *op, *value),
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                self.expect(*cond, &Ty::Bool, "as the condition of `while`");
                for &i in invariants {
                    self.expect(i, &Ty::Bool, "as a loop `invariant`");
                }
                for &d in decreases {
                    self.expect(d, &ANY_INT, "as a `decreases` measure");
                }
                self.block(*body, false);
            }
            StmtKind::Guard { cond, otherwise } => {
                self.expect(*cond, &Ty::Bool, "as the condition of `guard`");
                self.block(*otherwise, false);
                if block_ends(self.tree, *otherwise) {
                    let span = self.tree.block(*otherwise).span;
                    self.errors.push(Diagnostic::error(
                        "E0211",
                        span,
                        "the `else` of a `guard` must not reach its end: leave by `return`, `break`, `continue`, `todo` or `unreachable`",
                    ));
                }
            }
            StmtKind::Return(value) => self.ret(s, *value),
            StmtKind::Break | StmtKind::Continue => {}
        }
        Ty::Unit
    }

    fn binding(
        &mut self,
        s: StmtId,
        name: &str,
        name_span: Span,
        ty: Option<TypeId>,
        init: ExprId,
    ) {
        let bound = match ty {
            Some(t) => {
                let want = Ty::of(self.tree, t);
                let place = format!("for `{}`", name);
                self.expect(init, &want, &place);
                want
            }
            None => {
                let found = self.value(init);
                if found == Ty::Unit {
                    let hint = self.no_value_hint(init);
                    self.mismatch(
                        init,
                        format!(
                            "`{}` is bound to this, which produces no value{}",
                            name, hint
                        ),
                    );
                    Ty::Any
                } else if let Some(holder) = found.mixed() {
                    self.mixed_binding(name, name_span, &found, holder);
                    holder.map_or(Ty::Any, |h| found.with_int(h))
                } else {
                    found.settled()
                }
            }
        };
        if let Some(id) = self.resolution.stmts[s.0 as usize] {
            self.typing.bindings[id.0 as usize] = bound;
        }
    }

    /// `E0212`, with the type that holds every operand as the suggestion.
    fn mixed_binding(&mut self, name: &str, name_span: Span, found: &Ty, holder: Option<IntTy>) {
        let message = match holder {
            Some(h) => format!(
                "`{}` is computed from integers of different types, so it states its type; `{}` holds them all",
                name, h.name
            ),
            None => format!(
                "`{}` is computed from integers of different types, so it states its type; no standard type holds them all, so split the computation",
                name
            ),
        };
        let mut d = Diagnostic::error("E0212", name_span, message);
        if let Some(h) = holder {
            let width = (name_span.end - name_span.start) as u32;
            let at = Span {
                start: name_span.end,
                end: name_span.end,
                line: name_span.line,
                col: name_span.col + width,
            };
            d = d.with_fix(
                Applicability::Suggestion,
                format!("declare `{}` as `{}`", name, found.with_int(h).spelling()),
                at,
                format!(": {}", found.with_int(h).spelling()),
            );
        }
        self.errors.push(d);
    }

    fn assign(&mut self, place: ExprId, op: AssignOp, value: ExprId) {
        let target = self.value(place);
        self.changeable(place, "E0205", "assign to");
        match op {
            AssignOp::Set => {
                self.expect(value, &target, "in this assignment");
            }
            AssignOp::Compound(op) => {
                let found = self.value(value);
                let computed = self.binary(value, op, (place, &target), (value, &found));
                self.check_ty(value, &computed, &target, "in this assignment");
            }
        }
    }

    fn ret(&mut self, s: StmtId, value: Option<ExprId>) {
        match (self.result.clone(), value) {
            (Some(want), Some(e)) => {
                let place = self.as_result();
                self.expect(e, &want, &place);
            }
            (Some(want), None) => {
                let span = self.tree.stmt(s).span;
                self.errors.push(Diagnostic::error(
                    "E0203",
                    span,
                    format!(
                        "`{}` returns {}, so `return` takes a value: `return value`",
                        self.name,
                        want.describe()
                    ),
                ));
            }
            (None, Some(e)) => {
                self.value(e);
                self.mismatch(
                    e,
                    format!(
                        "`{}` has no result type, so `return` takes no value",
                        self.name
                    ),
                );
            }
            (None, None) => {}
        }
    }

    /// The body of a function: one with a result type produces it on every path that reaches
    /// the body's end.
    fn body(&mut self, b: BlockId) {
        let Some(want) = self.result.clone() else {
            self.block(b, false);
            return;
        };
        let found = self.block(b, true);
        if !block_ends(self.tree, b) {
            return;
        }
        let last = self.tree.block(b).stmts.last().copied();
        if let Some(StmtKind::Expr(e)) = last.map(|s| &self.tree.stmt(s).kind) {
            if found != Ty::Unit {
                let place = self.as_result();
                self.check_ty(*e, &found, &want, &place);
                return;
            }
        }
        let (span, hint) = match last {
            Some(s) => {
                let hint = match &self.tree.stmt(s).kind {
                    StmtKind::Expr(e) => self.no_value_hint(*e),
                    _ => String::new(),
                };
                (self.tree.stmt(s).span, hint)
            }
            None => (self.tree.block(b).span, String::new()),
        };
        self.errors.push(Diagnostic::error(
            "E0210",
            span,
            format!(
                "`{}` returns {}, but its body can reach its end without one{}; end it with a value of that type, `return`, or `unreachable(\"reason\")`",
                self.name,
                want.describe(),
                hint
            ),
        ));
    }
}

/// Whether `e` names a place: a binding, or an element of one.
pub fn is_place(tree: &Tree, e: ExprId) -> bool {
    match &tree.expr(e).kind {
        ExprKind::Name(_) => true,
        ExprKind::Index { base, .. } => is_place(tree, *base),
        _ => false,
    }
}

/// Whether control can reach the end of `b`, rather than leave it by `return`, `break`,
/// `continue`, `todo` or `unreachable`.
pub fn block_ends(tree: &Tree, b: BlockId) -> bool {
    tree.block(b)
        .stmts
        .iter()
        .all(|&s| match &tree.stmt(s).kind {
            StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue => false,
            StmtKind::Expr(e)
            | StmtKind::Binding { init: e, .. }
            | StmtKind::Assign { value: e, .. } => expr_ends(tree, *e),
            StmtKind::While { .. } | StmtKind::Guard { .. } => true,
        })
}

fn expr_ends(tree: &Tree, e: ExprId) -> bool {
    match &tree.expr(e).kind {
        ExprKind::Todo(_) | ExprKind::Unreachable(_) => false,
        ExprKind::If {
            then,
            otherwise: Some(o),
            ..
        } => block_ends(tree, *then) || else_ends(tree, *o),
        _ => true,
    }
}

fn else_ends(tree: &Tree, o: Else) -> bool {
    match o {
        Else::Block(b) => block_ends(tree, b),
        Else::If(i) => expr_ends(tree, i),
    }
}

#[cfg(test)]
mod tests {
    use mente_syntax::diagnostics::Diagnostic;

    use super::*;
    use crate::check_source;

    /// A module with a few functions to call, then `src`.
    fn module(src: &str) -> String {
        format!(
            "module m\npub fn id(x: u8) -> u8 = x\npub fn touch(mut x: u8) {{\n  x = 1\n}}\npub fn nothing() {{\n}}\npub fn declared(x: u8) -> u8\n{}\n",
            src
        )
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

    /// The type of each binding of the module's last function.
    fn binding_types(src: &str) -> Vec<(String, String)> {
        let text = module(src);
        let Ok(module) = mente_syntax::parse_module(&text) else {
            return Vec::new();
        };
        let parsed = mente_syntax::parse_bodies(&text, &module);
        let names: Vec<String> = parsed.iter().map(|p| p.name.clone()).collect();
        let signatures: Vec<Option<Signature>> = parsed
            .iter()
            .map(|p| p.syntax.as_ref().map(Signature::of))
            .collect();
        let Some(f) = parsed.last().and_then(|p| p.syntax.clone()) else {
            return Vec::new();
        };
        let (resolution, _) = crate::names::resolve(&f, &names);
        let (typing, errors) = check(&f, &resolution, &signatures);
        assert_eq!(errors, Vec::new());
        assert_eq!(typing.exprs.len(), f.tree.exprs.len());
        resolution
            .bindings
            .iter()
            .zip(&typing.bindings)
            .map(|(b, t)| (b.name.clone(), t.spelling()))
            .collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    const NONE: [&str; 0] = [];

    #[test]
    fn test_bindings_take_their_types_from_their_declarations_and_initializers() {
        assert_eq!(
            binding_types(
                "pub fn f(a: u8, b: u8, xs: [3]u16) -> u8\n  ensures result <= 9\n{\n  val lit = 1\n  val sum = a + b\n  val n = xs.len\n  val big = 255_u16\n  val stated: i32 = 1\n  val arr = [1, 2]\n  val rep = [a; 2]\n  val item = xs[0]\n  val flag = a < b\n  val wide: u64 = 1 << 40\n  val shifted = a << 1\n  val anything = todo(\"later\")\n  val picked = if flag { a } else { 2 }\n  val called = id(a)\n  val neg = -a\n  0\n}"
            ),
            pairs(&[
                ("a", "u8"),
                ("b", "u8"),
                ("xs", "[3]u16"),
                ("result", "u8"),
                ("lit", "i64"),
                ("sum", "u8"),
                ("n", "usize"),
                ("big", "u16"),
                ("stated", "i32"),
                ("arr", "[2]i64"),
                ("rep", "[2]u8"),
                ("item", "u16"),
                ("flag", "bool"),
                ("wide", "u64"),
                ("shifted", "u8"),
                ("anything", "_"),
                ("picked", "u8"),
                ("called", "u8"),
                ("neg", "u8"),
            ])
        );
        assert_eq!(
            binding_types("pub fn g(c: bool)\n  ensures true\n{\n  val d = not c\n}"),
            pairs(&[("c", "bool"), ("result", "()"), ("d", "bool")])
        );
    }

    #[test]
    fn test_conditions_and_contracts_are_bool_and_measures_are_integers() {
        assert_eq!(codes("pub fn f(x: u8) {\n  if x {\n  }\n}"), ["E0203"]);
        assert_eq!(codes("pub fn f(x: u8) {\n  while x {\n  }\n}"), ["E0203"]);
        assert_eq!(
            codes("pub fn f(x: u8) {\n  guard x else {\n    return\n  }\n}"),
            ["E0203"]
        );
        assert_eq!(codes("pub fn f(x: u8)\n  requires x\n{\n}"), ["E0203"]);
        assert_eq!(
            codes("pub fn f(x: u8) -> u8\n  ensures result\n= x"),
            ["E0203"]
        );
        assert_eq!(codes("pub fn f(x: bool)\n  decreases x\n{\n}"), ["E0203"]);
        assert_eq!(
            codes("pub fn f(x: u8) {\n  var i = x\n  while i > 0\n    invariant i\n  {\n    i -= 1\n  }\n}"),
            ["E0203"]
        );
        assert_eq!(
            codes("pub fn f(x: bool) {\n  var i = 1\n  while i > 0\n    decreases x\n  {\n    i -= 1\n  }\n}"),
            ["E0203"]
        );
        assert_eq!(
            codes("pub fn f(x: u8, c: bool)\n  requires x > 0 and c\n  decreases x\n{\n  var i = x\n  while i > 0\n    invariant i <= x\n    decreases i\n  {\n    i -= 1\n  }\n  guard c else {\n    return\n  }\n}"),
            NONE
        );
        assert_eq!(
            messages("pub fn f(x: u8) {\n  if x {\n  }\n}"),
            ["expected `bool` as the condition of `if`, found `u8`"]
        );
    }

    #[test]
    fn test_operators_take_the_kinds_of_operands_the_spec_gives_them() {
        assert_eq!(codes("pub fn f(c: bool) = c + 1"), ["E0203"]);
        assert_eq!(codes("pub fn f(c: bool) = 1 - c"), ["E0203"]);
        assert_eq!(codes("pub fn f(c: bool) = c < 1"), ["E0203"]);
        assert_eq!(codes("pub fn f(c: bool) = 1 >= c"), ["E0203"]);
        assert_eq!(codes("pub fn f(x: u8) = x and true"), ["E0203"]);
        assert_eq!(codes("pub fn f(x: u8) = true or x"), ["E0203"]);
        assert_eq!(codes("pub fn f(x: u8) = not x"), ["E0203"]);
        assert_eq!(codes("pub fn f(c: bool) = -c"), ["E0203"]);
        assert_eq!(codes("pub fn f(c: bool) = ~c"), ["E0203"]);
        assert_eq!(codes("pub fn f(c: bool, x: u8) = c == x"), ["E0203"]);
        assert_eq!(codes("pub fn f(c: bool, x: u8) = x != c"), ["E0203"]);
        assert_eq!(
            messages("pub fn f(c: bool, x: u8) = c == x"),
            ["`==` compares two integers or two bools, not `bool` and `u8`"]
        );
        assert_eq!(
            messages("pub fn f(c: bool) = c + 1"),
            ["expected an integer as an operand of `+`, found `bool`"]
        );
        assert_eq!(
            messages("pub fn f(c: bool) = ~c"),
            ["expected an integer as the operand of `~`, found `bool`"]
        );
        assert_eq!(
            messages("pub fn f(c: bool) = -c"),
            ["expected an integer as the operand of `-`, found `bool`"]
        );
        assert_eq!(
            codes("pub fn f(a: u8, b: u32, c: bool, d: bool) {\n  val e = c == d and a != b or not (a < b)\n  val g: u8 = -a * ~a / 2 % 3\n  val h: u64 = a << b >> 1\n  val i: u8 = (a & 1) | (2 ^ a)\n  val j = todo(\"x\") + 1\n  val k = todo(\"x\") == c\n  val l = a == todo(\"y\")\n}"),
            NONE
        );
    }

    #[test]
    fn test_bit_operations_need_one_integer_type() {
        assert_eq!(codes("pub fn f(a: u8, b: u16) -> u16 = a & b"), ["E0216"]);
        assert_eq!(codes("pub fn f(a: u8, b: u16) -> u16 = a | b"), ["E0216"]);
        assert_eq!(codes("pub fn f(a: u8, b: u16) -> u16 = a ^ b"), ["E0216"]);
        assert_eq!(
            messages("pub fn f(a: u8, b: u16) -> u16 = a & b"),
            ["`&` works on the bits of one integer type, but its operands are `u8` and `u16`; bind one of them to a `val` of the other's type first"]
        );
        assert_eq!(codes("pub fn f(a: u8, b: u16) -> u16 = a << b"), NONE);
        assert_eq!(codes("pub fn f(a: u8, b: u16) -> u16 = b >> a"), NONE);
        assert_eq!(
            codes(
                "pub fn f(a: u8, b: u32) {
  val s = a << b
  val t = b >> a
}"
            ),
            NONE
        );
        assert_eq!(codes("pub fn f(a: u8) -> u8 = a & 0xF0"), NONE);
        assert_eq!(
            codes("pub fn f(a: u8, b: u16) {\n  val c = (a + b) << 1\n}"),
            ["E0212"]
        );
    }

    #[test]
    fn test_indexes_and_fields_apply_to_arrays() {
        assert_eq!(codes("pub fn f(x: u8) = x[0]"), ["E0203"]);
        assert_eq!(
            messages("pub fn f(x: u8) = x[0]"),
            ["only an array can be indexed, but this is `u8`"]
        );
        assert_eq!(codes("pub fn f(xs: [2]u8) = xs[true]"), ["E0203"]);
        assert_eq!(codes("pub fn f(xs: [2]u8) -> bool = xs[0]"), ["E0203"]);
        assert_eq!(codes("pub fn f(xs: [2]u8, i: u32) -> u8 = xs[i]"), NONE);
        assert_eq!(codes("pub fn f(x: u8) = x.len"), ["E0215"]);
        assert_eq!(
            messages("pub fn f(x: u8) = x.len"),
            ["`.len` is an array's length, but this is `u8`"]
        );
        assert_eq!(codes("pub fn f(xs: [2]u8) -> bool = xs.len"), ["E0203"]);
        assert_eq!(
            codes("pub fn f() -> bool = todo(\"x\").len + todo(\"y\")[0]"),
            ["E0203"]
        );
        assert_eq!(
            codes("pub fn f() -> u8 = todo(\"x\").len + todo(\"y\")[0]"),
            NONE
        );
    }

    #[test]
    fn test_calls_match_their_signatures() {
        assert_eq!(codes("pub fn f() = id(1, 2)"), ["E0213"]);
        assert_eq!(
            messages("pub fn f() = id(1, 2)"),
            ["`id` takes 1 argument, but this call passes 2"]
        );
        assert_eq!(
            messages("pub fn f() = nothing(1)"),
            ["`nothing` takes 0 arguments, but this call passes 1"]
        );
        assert_eq!(codes("pub fn f() = id(true)"), ["E0203"]);
        assert_eq!(
            messages("pub fn f() = id(true)"),
            ["expected `u8` for parameter `x` of `id`, found `bool`"]
        );
        assert_eq!(codes("pub fn f() -> bool = id(1, 2)"), ["E0213", "E0203"]);
        assert_eq!(
            codes("pub fn f() -> bool = id(not 1, 2)"),
            ["E0203", "E0213", "E0203"]
        );
        assert_eq!(codes("pub fn f() = declared(true, 1)"), NONE);
        assert_eq!(codes("pub fn f() -> u8 = declared(not 1)"), ["E0203"]);
        assert_eq!(codes("pub fn f() -> u8 = nothing()"), ["E0203"]);
        assert_eq!(
            messages("pub fn f() -> u8 = nothing()"),
            ["expected `u8` as the result of `f`, found no value; `nothing` has no result type"]
        );
        assert_eq!(
            codes("pub fn f() {\n  var v: u8 = 0\n  touch(mut v)\n}"),
            NONE
        );
    }

    #[test]
    fn test_mut_parameters_are_marked_at_the_call_and_receive_places() {
        let missing = diagnostics("pub fn f() {\n  var v: u8 = 0\n  touch(v)\n}");
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].code, "E0214");
        assert_eq!(
            missing[0].message,
            "parameter `x` of `touch` is `mut`, so the call marks its argument `mut`, as in `touch(mut x)`"
        );
        let fix = &missing[0].fixes[0];
        assert_eq!(fix.applicability, Applicability::MachineApplicable);
        assert_eq!(fix.edits[0].replacement, "mut ");
        assert_eq!(fix.edits[0].span.start, fix.edits[0].span.end);
        assert_eq!(fix.edits[0].span.start, missing[0].primary.start);
        let not_place = diagnostics("pub fn f() {\n  touch(1)\n}");
        assert_eq!(not_place[0].code, "E0214");
        assert!(not_place[0].fixes.is_empty());
        assert_eq!(codes("pub fn f(v: u8) = id(mut v)"), ["E0214"]);
        assert_eq!(
            messages("pub fn f(v: u8) = id(mut v)"),
            ["parameter `x` of `id` is read-only, so its argument is not marked `mut`"]
        );
        assert_eq!(
            codes("pub fn f() {\n  var c = true\n  touch(mut c)\n}"),
            ["E0203"]
        );
        assert_eq!(
            codes("pub fn f() {\n  var xs: [2]u8 = [0, 0]\n  touch(mut xs[1])\n}"),
            NONE
        );
        let wide = diagnostics("pub fn f() {\n  var count = 0\n  touch(mut count)\n}");
        assert_eq!(wide.len(), 1);
        assert_eq!(wide[0].code, "E0203");
        assert_eq!(
            wide[0].message,
            "a `mut` argument has its parameter's type exactly, since the call writes it back: `x` of `touch` is `u8`, but this is `i64`"
        );
        assert_eq!((wide[0].primary.line, wide[0].primary.col), (11, 13));
        let fix = &wide[0].fixes[0];
        assert_eq!(fix.message, "declare `count` as `u8`");
        assert_eq!(fix.applicability, Applicability::Suggestion);
        assert_eq!(fix.edits[0].replacement, ": u8");
        assert_eq!((fix.edits[0].span.line, fix.edits[0].span.col), (10, 12));
        assert_eq!(fix.edits[0].span.start, fix.edits[0].span.end);
        for no_fix in [
            "pub fn f() {\n  var v: u64 = 0\n  touch(mut v)\n}",
            "pub fn f() {\n  var xs: [2]u16 = [0, 0]\n  touch(mut xs[1])\n}",
            "pub fn f(mut v: u16) {\n  touch(mut v)\n}",
        ] {
            let d = diagnostics(no_fix);
            assert_eq!(d.len(), 1, "{}", no_fix);
            assert_eq!(d[0].code, "E0203", "{}", no_fix);
            assert!(d[0].fixes.is_empty(), "{}", no_fix);
        }
        assert_eq!(
            codes("pub fn f() {\n  var v = todo(\"x\")\n  touch(mut v)\n}"),
            NONE
        );
    }

    #[test]
    fn test_array_literals_have_one_element_type() {
        assert_eq!(codes("pub fn f() {\n  val xs = [1, true]\n}"), ["E0203"]);
        assert_eq!(
            messages("pub fn f() {\n  val xs = [1, true]\n}"),
            ["the elements of an array literal have one type, but this is `bool` after an integer"]
        );
        assert_eq!(codes("pub fn f(a: [2]u8) -> [3]u8 = a"), ["E0203"]);
        assert_eq!(codes("pub fn f() -> [2]u8 = [1, 300]"), NONE);
        assert_eq!(codes("pub fn f() -> [2]bool = [true; 2]"), NONE);
        assert_eq!(codes("pub fn f() -> [2]bool = [1; 2]"), ["E0203"]);
        assert_eq!(codes("pub fn f() -> [2]bool = [todo(\"x\"), true]"), NONE);
    }

    #[test]
    fn test_if_branches_that_give_a_value_share_its_type() {
        assert_eq!(
            codes("pub fn f(c: bool) -> u8 = if c { 1 } else { true }"),
            ["E0203"]
        );
        assert_eq!(
            messages("pub fn f(c: bool) -> u8 = if c { 1 } else { true }"),
            ["the branches of this `if` produce its value, so they have one type, but they are an integer and `bool`"]
        );
        assert_eq!(
            codes("pub fn f(c: bool, d: bool) -> u8 = if c { 1 } else if d { 2 } else { false }"),
            ["E0203"]
        );
        assert_eq!(
            codes(
                "pub fn f(c: bool) -> u8 {\n  if c {\n    return 1\n  } else {\n    true\n  }\n}"
            ),
            ["E0203"]
        );
        assert_eq!(
            messages("pub fn f(c: bool) -> bool {\n  val x = if c {\n    return false\n  } else {\n    1\n  }\n  x\n}"),
            ["expected `bool` as the result of `f`, found `i64`"]
        );
        assert_eq!(
            messages("pub fn f(c: bool) -> bool {\n  val x = if c {\n    1\n  } else {\n    return false\n  }\n  x\n}"),
            ["expected `bool` as the result of `f`, found `i64`"]
        );
        assert_eq!(
            codes("pub fn f(c: bool) -> u8 = if c { 1 } else { if c { 2 } else { true } }"),
            ["E0203"]
        );
        assert_eq!(
            codes("pub fn f(c: bool, d: bool) {\n  if c {\n    id(1)\n  } else if d {\n    nothing()\n  } else {\n    true\n  }\n}"),
            NONE
        );
        assert_eq!(
            codes("pub fn f(c: bool, d: bool) {\n  if c {\n    id(1)\n  } else if d {\n    not 1\n  }\n}"),
            ["E0203"]
        );
        assert_eq!(
            codes("pub fn f(c: bool) {\n  val x = if c { 1 }\n}"),
            ["E0203"]
        );
        assert_eq!(
            messages("pub fn f(c: bool) {\n  val x = if c { 1 }\n}"),
            ["`x` is bound to this, which produces no value; an `if` without `else` has no value"]
        );
        assert_eq!(
            messages("pub fn f() {\n  val x = 1 + 2 + nothing()\n}"),
            ["expected an integer as an operand of `+`, found no value; `nothing` has no result type"]
        );
        assert_eq!(messages("pub fn f(c: bool) {\n  val x = { c }\n}").len(), 1);
    }

    #[test]
    fn test_unannotated_bindings_of_mixed_integers_state_their_type() {
        let d = diagnostics("pub fn f(a: u8, b: u32) {\n  val total = a + b\n}");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0212");
        assert_eq!(
            d[0].message,
            "`total` is computed from integers of different types, so it states its type; `u32` holds them all"
        );
        assert_eq!((d[0].primary.line, d[0].primary.col), (10, 7));
        let edit = &d[0].fixes[0].edits[0];
        assert_eq!(edit.replacement, ": u32");
        assert_eq!(edit.span.start, d[0].primary.end);
        assert_eq!(edit.span.end, d[0].primary.end);
        assert_eq!((edit.span.line, edit.span.col), (10, 12));
        assert_eq!(d[0].fixes[0].applicability, Applicability::Suggestion);
        assert_eq!(d[0].fixes[0].message, "declare `total` as `u32`");
        let none = diagnostics("pub fn f(a: u128, b: i8) {\n  val total = a + b\n}");
        assert_eq!(none[0].code, "E0212");
        assert!(none[0].fixes.is_empty());
        assert!(none[0]
            .message
            .ends_with("no standard type holds them all, so split the computation"));
        let arrays = diagnostics("pub fn f(a: u8, b: i8) {\n  val pair = [a, b]\n}");
        assert_eq!(arrays[0].fixes[0].edits[0].replacement, ": [2]i16");
        assert_eq!(
            codes("pub fn f(a: u8, b: u32) -> bool {\n  val total = a + b\n  total\n}"),
            ["E0212", "E0203"]
        );
        assert_eq!(
            codes("pub fn f(a: u128, b: i8) -> bool {\n  val total = a + b\n  total\n}"),
            ["E0212"]
        );
        assert_eq!(
            binding_types("pub fn f(a: u8, b: u32) {\n  val total: u64 = a + b\n}")
                .last()
                .map(|(_, t)| t.clone()),
            Some("u64".to_string())
        );
        assert_eq!(codes("pub fn f(a: u8, b: u32) -> u64 = a + b"), NONE);
    }

    #[test]
    fn test_assignments_put_values_of_the_place_kind_into_places() {
        assert_eq!(codes("pub fn f() {\n  var c = true\n  c = 1\n}"), ["E0203"]);
        assert_eq!(
            messages("pub fn f() {\n  var c = true\n  c = 1\n}"),
            ["expected `bool` in this assignment, found an integer"]
        );
        assert_eq!(
            codes("pub fn f() {\n  var c = true\n  c += 1\n}"),
            ["E0203"]
        );
        assert_eq!(
            codes("pub fn f() {\n  var c = true\n  c += not 1\n}"),
            ["E0203", "E0203", "E0203"]
        );
        assert_eq!(
            codes("pub fn f(b: u16) {\n  var a: u8 = 0\n  a &= b\n}"),
            ["E0216"]
        );
        assert_eq!(
            codes("pub fn f(b: u16) {\n  var a: u8 = 0\n  a += b\n  a <<= 1\n  var xs = [0, 0]\n  xs[1] = 5\n}"),
            NONE
        );
    }

    #[test]
    fn test_returns_match_the_result_type() {
        assert_eq!(codes("pub fn f() {\n  return 1\n}"), ["E0203"]);
        assert_eq!(
            messages("pub fn f() {\n  return 1\n}"),
            ["`f` has no result type, so `return` takes no value"]
        );
        assert_eq!(codes("pub fn f() {\n  return not 1\n}"), ["E0203", "E0203"]);
        assert_eq!(codes("pub fn f() -> u8 {\n  return\n}"), ["E0203"]);
        assert_eq!(
            messages("pub fn f() -> u8 {\n  return\n}"),
            ["`f` returns `u8`, so `return` takes a value: `return value`"]
        );
        assert_eq!(
            diagnostics("pub fn f() -> u8 {\n  return\n}")[0]
                .primary
                .line,
            10
        );
        assert_eq!(codes("pub fn f() -> u8 {\n  return true\n}"), ["E0203"]);
        assert_eq!(codes("pub fn f() {\n  return\n}"), NONE);
    }

    #[test]
    fn test_a_body_with_a_result_type_produces_it_on_every_path() {
        assert_eq!(
            codes("pub fn f(c: bool) -> u8 {\n  if c {\n    return 1\n  }\n}"),
            ["E0210"]
        );
        assert_eq!(
            messages("pub fn f(c: bool) -> u8 {\n  if c {\n    return 1\n  }\n}"),
            ["`f` returns `u8`, but its body can reach its end without one; an `if` without `else` has no value; end it with a value of that type, `return`, or `unreachable(\"reason\")`"]
        );
        let empty = diagnostics("pub fn f() -> u8 {\n}");
        assert_eq!(empty[0].code, "E0210");
        assert_eq!((empty[0].primary.line, empty[0].primary.col), (9, 18));
        let after = diagnostics("pub fn f() -> u8 {\n  val x = 1\n}");
        assert_eq!(after[0].code, "E0210");
        assert_eq!((after[0].primary.line, after[0].primary.col), (10, 3));
        assert!(after[0].message.contains("without one; end it"));
        assert_eq!(codes("pub fn f() -> u8 {\n  nothing()\n}"), ["E0210"]);
        assert_eq!(codes("pub fn f() -> u8 {\n  true\n}"), ["E0203"]);
        assert_eq!(codes("pub fn f() -> u8 = true"), ["E0203"]);
        assert_eq!(codes("pub fn f() {\n  not 1\n}"), ["E0203"]);
        assert_eq!(codes("pub fn f() = not 1"), ["E0203"]);
        for ok in [
            "pub fn f() -> u8 {\n  1\n}",
            "pub fn f() -> u8 {\n  return 1\n}",
            "pub fn f() -> u8 {\n  todo(\"later\")\n}",
            "pub fn f(c: bool) -> u8 {\n  if c {\n    return 1\n  } else {\n    return 2\n  }\n}",
            "pub fn f(c: bool, d: bool) -> u8 {\n  if c {\n    return 1\n  } else if d {\n    unreachable(\"d\")\n  } else {\n    return 2\n  }\n}",
            "pub fn f() {\n  1\n}",
            "pub fn f() = 1",
        ] {
            assert_eq!(codes(ok), NONE, "{}", ok);
        }
    }

    #[test]
    fn test_the_else_of_a_guard_leaves() {
        let d = diagnostics("pub fn f(c: bool) {\n  guard c else {\n    nothing()\n  }\n}");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0211");
        assert_eq!((d[0].primary.line, d[0].primary.col), (10, 16));
        assert_eq!(
            codes("pub fn f(c: bool) {\n  guard c else {\n  }\n}"),
            ["E0211"]
        );
        assert_eq!(
            codes("pub fn f(c: bool, d: bool) {\n  guard c else {\n    if d {\n      return\n    }\n  }\n}"),
            ["E0211"]
        );
        assert_eq!(
            codes("pub fn f(c: bool, d: bool) {\n  guard c else {\n    if d {\n      return\n    } else if c {\n      nothing()\n    } else {\n      return\n    }\n  }\n}"),
            ["E0211"]
        );
        assert_eq!(
            codes("pub fn f(c: bool) {\n  guard c else {\n    val x = 1\n  }\n}"),
            ["E0211"]
        );
        assert_eq!(
            codes("pub fn f(c: bool) {\n  var x = 0\n  guard c else {\n    x = 1\n  }\n}"),
            ["E0211"]
        );
        assert_eq!(
            codes(
                "pub fn f(c: bool) {\n  guard c else {\n    while c {\n      return\n    }\n  }\n}"
            ),
            ["E0211"]
        );
        assert_eq!(
            codes("pub fn f(c: bool) {\n  guard c else {\n    guard c else {\n      return\n    }\n  }\n}"),
            ["E0211"]
        );
        for ok in [
            "pub fn f(c: bool) {\n  guard c else {\n    return\n  }\n}",
            "pub fn f(c: bool) {\n  guard c else {\n    todo(\"c\")\n  }\n}",
            "pub fn f(c: bool) {\n  guard c else {\n    val x: u8 = unreachable(\"c\")\n  }\n}",
            "pub fn f(c: bool) {\n  var x: u8 = 0\n  guard c else {\n    x = todo(\"c\")\n  }\n}",
            "pub fn f(c: bool) {\n  while true {\n    guard c else {\n      break\n    }\n    guard c else {\n      continue\n    }\n  }\n}",
            "pub fn f(c: bool, d: bool) {\n  guard c else {\n    if d {\n      return\n    } else {\n      return\n    }\n  }\n}",
            "pub fn f(c: bool, d: bool) {\n  guard c else {\n    if d {\n      return\n    } else if c {\n      return\n    } else {\n      todo(\"x\")\n    }\n  }\n}",
        ] {
            assert_eq!(codes(ok), NONE, "{}", ok);
        }
    }

    #[test]
    fn test_places_are_bindings_and_their_elements() {
        let src = "module m\npub fn f(xs: [2][2]u8) = xs[0][1] + xs[1][0] + 1\n";
        let Ok(module) = mente_syntax::parse_module(src) else {
            panic!("the module parses");
        };
        let parsed = mente_syntax::parse_bodies(src, &module);
        let Some(f) = parsed[0].syntax.as_ref() else {
            panic!("f has a body");
        };
        let places: Vec<String> = (0..f.tree.exprs.len() as u32)
            .map(ExprId)
            .filter(|&e| is_place(&f.tree, e))
            .map(|e| f.tree.render_expr(e))
            .collect();
        assert_eq!(
            places,
            [
                "xs",
                "(index xs 0)",
                "(index (index xs 0) 1)",
                "xs",
                "(index xs 1)",
                "(index (index xs 1) 0)"
            ]
        );
    }

    #[test]
    fn test_a_function_with_a_body_error_is_not_checked_further() {
        assert_eq!(codes("pub fn f() {\n  val x = (1\n  y\n}"), ["E0111"]);
        assert_eq!(
            codes("pub fn f() {\n  val x = (1\n  y\n}\npub fn g() = not 1"),
            ["E0111", "E0203"]
        );
    }

    #[test]
    fn test_only_a_var_or_a_mut_parameter_is_assigned() {
        let val = diagnostics("pub fn f() {\n  val x = 1\n  x = 2\n}");
        assert_eq!(val.len(), 1);
        assert_eq!(val[0].code, "E0205");
        assert_eq!(
            val[0].message,
            "cannot assign to `x`: it is a `val`, which never changes; declare it with `var`"
        );
        assert_eq!(val[0].related[0].span.line, 10);
        assert_eq!(val[0].related[0].message, "declared here");
        let param = diagnostics("pub fn f(n: u8) {\n  n += 1\n}");
        assert_eq!(param[0].code, "E0205");
        assert_eq!(
            param[0].message,
            "cannot assign to `n`: it is a read-only parameter; make it `mut n`, and pass `mut` at its calls"
        );
        let fix = &param[0].fixes[0];
        assert_eq!(fix.message, "make `n` a `mut` parameter");
        assert_eq!(fix.edits[0].replacement, "mut ");
        assert_eq!((fix.edits[0].span.line, fix.edits[0].span.col), (9, 10));
        assert_eq!(fix.edits[0].span.start, fix.edits[0].span.end);
        assert_eq!(
            codes("pub fn f() {\n  val xs = [1, 2]\n  xs[0] = 3\n}"),
            ["E0205"]
        );
        assert_eq!(
            codes("pub fn f(mut n: u8, mut xs: [2]u8) {\n  var m = 1\n  m = 2\n  n = 3\n  xs[1] = 4\n}"),
            NONE
        );
    }

    #[test]
    fn test_only_a_var_or_a_mut_parameter_is_passed_as_mut() {
        let val = diagnostics("pub fn f() {\n  val v: u8 = 0\n  touch(mut v)\n}");
        assert_eq!(val.len(), 1);
        assert_eq!(val[0].code, "E0206");
        assert_eq!(
            val[0].message,
            "cannot pass as `mut` `v`: it is a `val`, which never changes; declare it with `var`"
        );
        assert_eq!(codes("pub fn f(v: u8) {\n  touch(mut v)\n}"), ["E0206"]);
        assert_eq!(
            messages("pub fn bump(mut x: u8) -> bool = true\npub fn f() -> u8\n  ensures bump(mut result)\n= 1"),
            ["cannot pass as `mut` `result`, the value an `ensures` clause reads"]
        );
        assert_eq!(codes("pub fn f(v: u8) {\n  declared(mut v)\n}"), ["E0206"]);
        assert_eq!(codes("pub fn f(v: u8) {\n  touch(mut v, 1)\n}"), ["E0213"]);
    }

    #[test]
    fn test_a_mut_argument_is_named_by_no_other_argument() {
        let two = "pub fn two(mut a: u8, b: u8) {\n}\npub fn both(mut a: u8, mut b: u8) {\n}\n";
        let d = diagnostics(&format!(
            "{}pub fn f() {{\n  var xs: [2]u8 = [0, 0]\n  two(mut xs[0], xs[1])\n}}",
            two
        ));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0207");
        assert_eq!(
            d[0].message,
            "`xs` is passed as `mut` in this call, so no other argument may name it; copy what this argument needs into a `val` first"
        );
        assert_eq!((d[0].primary.line, d[0].primary.col), (15, 18));
        assert_eq!(d[0].related[0].message, "passed as `mut` here");
        assert_eq!(
            (d[0].related[0].span.line, d[0].related[0].span.col),
            (15, 11)
        );
        let codes_of = |body: &str| {
            diagnostics(&format!(
                "{}pub fn f(w: u8) {{\n  var v: u8 = 0\n  var xs: [2]u8 = [0, 0]\n{}\n}}",
                two, body
            ))
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>()
        };
        assert_eq!(codes_of("  both(mut v, mut v)"), ["E0207"]);
        let twice = diagnostics(&format!(
            "{}pub fn f(w: u8) {{\n  var v: u8 = 0\n  var xs: [2]u8 = [0, 0]\n  both(mut v, mut v)\n}}",
            two
        ));
        assert_eq!((twice[0].primary.line, twice[0].primary.col), (16, 19));
        assert_eq!(twice[0].related[0].span.col, 12);
        assert_eq!(codes_of("  both(mut xs[0], mut xs[1])"), ["E0207"]);
        assert_eq!(codes_of("  two(mut v, id(v) + 1)"), ["E0207"]);
        assert_eq!(codes_of("  two(mut v, -v)"), ["E0207"]);
        assert_eq!(codes_of("  two(mut v, xs[v])"), ["E0207"]);
        assert_eq!(codes_of("  two(mut xs[v], v)"), NONE);
        assert_eq!(codes_of("  two(mut v, [v; 2][0])"), ["E0207"]);
        assert_eq!(codes_of("  two(mut v, [w, v][0])"), ["E0207"]);
        assert_eq!(codes_of("  two(mut xs[0], xs.len)"), ["E0207"]);
        assert_eq!(codes_of("  two(mut v, v + v)"), ["E0207", "E0207"]);
        assert_eq!(codes_of("  two(mut v, w)"), NONE);
        assert_eq!(codes_of("  both(mut v, mut xs[0])"), NONE);
        assert_eq!(codes_of("  declared(mut v)"), NONE);
        assert_eq!(
            codes(
                "pub fn rev(b: u8, mut a: u8) {
}
pub fn f() {
  var v: u8 = 0
  rev(v + 1, mut v)
}"
            ),
            ["E0207"]
        );
    }
}
