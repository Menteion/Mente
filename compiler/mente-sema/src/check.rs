//! The type checker of M1's functions (`spec/core-calculus.md`, items 2 to 7).
//!
//! [`check`] gives each expression and binding of one function its [`Ty`], after
//! [`crate::names::resolve`] has bound its names. It reports a value of the wrong kind where
//! the spec fixes one (`E0203`), a body that can end without its result (`E0210`), a `guard`
//! whose `else` can end (`E0211`), an unannotated binding computed from operands of different
//! integer types (`E0212`), a call with the wrong number of arguments (`E0213`) or the wrong
//! `mut` marks (`E0214`), `.len` of a value that is not an array (`E0215`), and a bit
//! operation on operands of different integer types (`E0216`), a value nobody uses, which
//! `_ = e` discards on purpose (`E0217`), and a literal outside its suffix's type (`E0221`); `wide.rs` adds an intermediate that might need more than 128 bits (`E0222`). It checks the parameter modes
//! too: only a `var` or a `mut` parameter is assigned (`E0205`) or passed as `mut` (`E0206`),
//! and a `mut` argument's binding appears in no other argument of its call (`E0207`).

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};

use mente_syntax::diagnostics::{insertion_point, Applicability, Diagnostic, Edit, Fix};
use mente_syntax::tree::BinaryOp::{BitAnd, BitOr, BitXor, Shl, Shr};
use mente_syntax::tree::{
    Arg, Arm, ArmBody, AssignOp, BinaryOp, BlockId, Body, Bound, ContractKind, Else, ExprId,
    ExprKind, FieldInit, FnSyntax, PatBinding, PatternKind, Payload, StmtId, StmtKind, Tree,
    TypeId, TypeKind, UnaryOp,
};
use mente_syntax::Span;

use crate::names::{closest, BindingId, BindingKind, Resolution};
use crate::octagon::Exact;
use crate::structs::{too_many_elements, Structs, ARRAY_ELEMENTS, VALUE_BYTES};
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
/// function of the module in order, its signature when its body was parsed, and `structs` the
/// module's structs.
pub fn check(
    f: &FnSyntax,
    resolution: &Resolution,
    signatures: &[Option<Signature>],
    structs: &Structs,
    src: &str,
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
        structs,
        name: &f.name,
        resolution,
        signatures,
        result,
        typing: Typing {
            exprs: vec![Ty::Any; tree.exprs.len()],
            bindings,
        },
        negated: negated_literals(tree),
        src,
        pattern_decls: HashMap::new(),
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
            None => c.discarded(e, false),
        },
        Some(Body::Block(b)) => c.body(b),
        None => {}
    }
    c.arrays_in_one_piece();
    c.bounds_have_their_types();
    (c.typing, c.errors)
}

/// The most bytes an array may hold: it is a value kept in one piece, in one stack slot, whose
/// size native code holds in 32 bits.
pub const ARRAY_BYTES: u128 = VALUE_BYTES;

impl Checker<'_> {
    /// The most bytes a value of `t` takes, as its layout gives them (`Structs::layout_of`);
    /// none for a type without one.
    fn bytes(&self, t: &Ty) -> u128 {
        self.structs.layout_of(t).map_or(0, |(size, _)| size)
    }

    /// `E0223` for each array type, and each array literal, that holds more than
    /// [`ARRAY_BYTES`]: the innermost such type, and a literal whose type no written type
    /// already reports; and for each one that holds more than [`ARRAY_ELEMENTS`] elements of
    /// a size that keeps it within those bytes.
    fn arrays_in_one_piece(&mut self) {
        let mut reported = Vec::new();
        for (i, node) in self.tree.types.iter().enumerate() {
            let TypeKind::Array { elem, len } = &node.kind else {
                continue;
            };
            let t = Ty::of(self.tree, TypeId(i as u32));
            let elem = Ty::of(self.tree, *elem);
            let text = self.tree.render_type(TypeId(i as u32));
            if self.bytes(&t) > ARRAY_BYTES && self.bytes(&elem) <= ARRAY_BYTES {
                reported.push(node.span);
                self.too_large(node.span, &text, self.bytes(&t));
            } else if *len > ARRAY_ELEMENTS {
                reported.push(node.span);
                self.errors.push(too_many_elements(node.span, &text, *len));
            }
        }
        for (i, e) in self.tree.exprs.iter().enumerate() {
            if !matches!(e.kind, ExprKind::Array(_) | ExprKind::Repeat { .. })
                || !reported.is_empty()
            {
                continue;
            }
            let t = self.typing.exprs[i].clone();
            if self.bytes(&t) > ARRAY_BYTES {
                self.too_large(e.span, &t.spelling(), self.bytes(&t));
            } else if let ExprKind::Repeat { count, .. } = &e.kind {
                if *count > ARRAY_ELEMENTS {
                    self.errors
                        .push(too_many_elements(e.span, &t.spelling(), *count));
                }
            }
        }
    }

    /// `E0203` for each refinement bound whose binding has another type than it reads: a bound
    /// `name` is an integer, and `name.len` reads an array's length. A bound that names an array
    /// is given the fix that reads its length.
    fn bounds_have_their_types(&mut self) {
        for b in &self.resolution.bounds {
            let ty = &self.typing.bindings[b.binding.0 as usize];
            let name = &self.resolution.bindings[b.binding.0 as usize].name;
            let span = self.tree.ty(b.ty).span;
            let message = match (b.len, ty) {
                (false, Ty::Int(_) | Ty::Any) | (true, Ty::Array(..) | Ty::Any) => continue,
                (false, t) => format!(
                    "a refinement's bound is an integer, but `{}` is `{}`",
                    name,
                    t.spelling()
                ),
                (true, t) => format!(
                    "`{}.len` reads an array's length, but `{}` is `{}`",
                    name,
                    name,
                    t.spelling()
                ),
            };
            self.errors.push(Diagnostic::error("E0203", span, message));
        }
    }

    fn too_large(&mut self, span: Span, text: &str, bytes: u128) {
        self.errors.push(Diagnostic::error(
            "E0223",
            span,
            format!(
                "an array of type `{}` holds {} bytes, more than the {} an array, a value kept in one piece, may hold",
                text, bytes, ARRAY_BYTES
            ),
        ));
    }
}

struct Checker<'a> {
    tree: &'a Tree,
    structs: &'a Structs,
    name: &'a str,
    resolution: &'a Resolution,
    signatures: &'a [Option<Signature>],
    result: Option<Ty>,
    typing: Typing,
    /// The literals that are the operand of a unary `-`, whose range reaches one further.
    negated: HashSet<ExprId>,
    /// The text the spans point into, for the fixes.
    src: &'a str,
    /// The declared type of each pattern binding: its payload field's, in its enum's tree.
    pattern_decls: HashMap<BindingId, (&'a Tree, TypeId)>,
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
            ExprKind::Int { value, suffix } => match suffix.as_deref().and_then(IntTy::named) {
                Some(t) => {
                    self.literal(e, *value, t);
                    Ty::int(t)
                }
                None => ANY_INT,
            },
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
            ExprKind::Field { base, name } => self.field(e, *base, name),
            ExprKind::Call { callee, args } => self.call(e, callee, args),
            ExprKind::Construct { name, fields } => self.construct(e, name, fields),
            ExprKind::Variant {
                enum_name,
                name,
                name_span,
                payload,
                ..
            } => self.variant(e, enum_name, (name, *name_span), payload),
            ExprKind::Match { scrutinee, arms } => self.match_expr(e, *scrutinee, arms, used),
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

    /// A literal with a suffix lies in the suffix's type (`E0221`).
    fn literal(&mut self, e: ExprId, value: u128, t: IntTy) {
        let limit = if self.negated.contains(&e) {
            t.min_magnitude().max(t.max())
        } else {
            t.max()
        };
        if value > limit {
            let span = self.span(e);
            self.errors.push(Diagnostic::error(
                "E0221",
                span,
                format!(
                    "the literal {}_{} lies outside `{}`, which holds {}",
                    value,
                    t.name,
                    t.name,
                    t.range_text()
                ),
            ));
        }
    }

    /// An expression statement whose value nobody uses: an error unless it has no value
    /// (`E0217`), so that a result is never dropped by mistake. In an arm of a `match` that
    /// stands as a statement, `in_arm`, the discard its fix writes is a statement, so the fix
    /// makes the arm a block that holds it.
    fn discarded(&mut self, e: ExprId, in_arm: bool) {
        self.effect(e);
        let ty = self.typing.exprs[e.0 as usize].clone();
        if matches!(ty, Ty::Unit | Ty::Any) {
            return;
        }
        let span = self.span(e);
        let d = Diagnostic::error(
            "E0217",
            span,
            format!(
                "this value, {}, is never used; use it, or discard it on purpose with `_ = ...`",
                ty.describe()
            ),
        );
        let message = "discard it on purpose";
        let d = match (in_arm, self.src.get(span.start..span.end)) {
            (true, Some(text)) => d.with_fix(
                Applicability::Suggestion,
                message,
                span,
                format!("{{ _ = {} }}", text),
            ),
            (true, None) => d,
            (false, _) => d.with_fix(
                Applicability::Suggestion,
                message,
                insertion_point(span),
                "_ = ",
            ),
        };
        self.errors.push(d);
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
                // Structs and enums compare field by field and by variant (`core-calculus.md`,
                // item 22), which comes later in M2.
                let named = |t: &Ty| matches!(t, Ty::Struct(_) | Ty::Enum(_));
                if named(lt) || named(rt) {
                    let what = format!("comparing structs and enums with `{}`", symbol);
                    self.errors.push(Diagnostic::error(
                        "E0109",
                        self.tree.expr(at).span,
                        mente_syntax::diagnostics::later_message(&what, Some("M2")),
                    ));
                    return Ty::Bool;
                }
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

    /// `a.name`: an array's `len`, or a field of a struct (`E0215` for one the value lacks).
    fn field(&mut self, at: ExprId, base: ExprId, name: &str) -> Ty {
        let span = self.tree.expr(at).span;
        // The name ends the expression's text.
        let name_span = Span {
            start: span.end - name.len(),
            col: span.col + (span.end - span.start - name.len()) as u32,
            ..span
        };
        let found = self.value(base);
        let message = match &found {
            Ty::Array(..) if name == "len" => return Ty::int(USIZE),
            Ty::Any => return Ty::Any,
            Ty::Array(..) => format!(
                "an array has one field, `.len`, its length, but not `.{}`; an element is read with an index, as in `a[i]`",
                name
            ),
            Ty::Struct(s) => {
                let Some(def) = self.structs.get(s) else {
                    return Ty::Any;
                };
                if let Some((_, f)) = def.field(name) {
                    return f.ty.clone();
                }
                // A declaration that did not check may lack the field it meant to have.
                if !def.ok {
                    return Ty::Any;
                }
                let names: Vec<String> = def.fields.iter().map(|f| f.name.clone()).collect();
                let close = closest(name, &names);
                let message = match &close {
                    Some(c) => format!("`{}` has no field `{}`; did you mean `{}`?", s, name, c),
                    None => format!(
                        "`{}` has no field `{}`; its fields are {}",
                        s,
                        name,
                        names
                            .iter()
                            .map(|n| format!("`{}`", n))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                let mut d = Diagnostic::error("E0215", name_span, message);
                if let Some(c) = close {
                    d = d.with_fix(
                        Applicability::Suggestion,
                        format!("write `{}`", c),
                        name_span,
                        c,
                    );
                }
                self.errors.push(d);
                return Ty::Any;
            }
            other if name == "len" => format!(
                "`.len` is an array's length, but this is {}",
                other.describe()
            ),
            other => format!(
                "`.{}` reads a field of a struct, but this is {}",
                name,
                other.describe()
            ),
        };
        self.errors.push(Diagnostic::error("E0215", span, message));
        Ty::Any
    }

    /// `Name(field: value, ...)`: a struct built with every field once, in the order of its
    /// declaration (`E0226`), each value flowing into its field's type; or a function called
    /// with named arguments (`E0227`).
    fn construct(&mut self, at: ExprId, name: &str, fields: &[FieldInit]) -> Ty {
        let span = self.span(at);
        if self.resolution.calls[at.0 as usize].is_some() {
            for f in fields {
                self.value(f.value);
            }
            let edits = fields
                .iter()
                .map(|f| Edit {
                    span: Span {
                        end: self.span(f.value).start,
                        ..f.name_span
                    },
                    replacement: String::new(),
                })
                .collect();
            let mut d = Diagnostic::error(
                "E0227",
                span,
                format!(
                    "`{}` is a function, whose arguments are positional; only the arguments that build a struct are named",
                    name
                ),
            );
            d.fixes.push(Fix {
                message: "pass the arguments by position, in the order of the parameters".into(),
                applicability: Applicability::Suggestion,
                edits,
            });
            self.errors.push(d);
            return Ty::Any;
        }
        let Some(def) = self.structs.get(name) else {
            for f in fields {
                self.value(f.value);
            }
            return Ty::Any;
        };
        let declared: Vec<(String, Ty)> = def
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        let ok = def.ok;
        // Each value flows into its field's type, wherever it stands.
        for f in fields {
            match declared.iter().find(|(n, _)| *n == f.name) {
                Some((_, ty)) => {
                    let place = format!("for field `{}` of `{}`", f.name, name);
                    self.expect(f.value, ty, &place);
                }
                None => {
                    self.value(f.value);
                }
            }
        }
        if ok {
            if let Some(d) = self.field_order(span, (name, "a struct"), fields, &declared) {
                self.errors.push(d);
            }
        }
        Ty::Struct(name.to_string())
    }

    /// `E0226` for a struct built without naming each field once, in the order of its
    /// declaration: the first field it names that the struct lacks, or names a second time, or
    /// out of order, or else the fields it leaves out.
    fn field_order(
        &self,
        span: Span,
        (name, noun): (&str, &str),
        fields: &[FieldInit],
        declared: &[(String, Ty)],
    ) -> Option<Diagnostic> {
        let names: Vec<String> = declared.iter().map(|(n, _)| n.clone()).collect();
        let list = |ns: &[&String]| {
            ns.iter()
                .map(|n| format!("`{}`", n))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let order = list(&names.iter().collect::<Vec<_>>());
        for (k, f) in fields.iter().enumerate() {
            if !names.contains(&f.name) {
                let close = closest(&f.name, &names);
                let message = match &close {
                    Some(c) => format!(
                        "`{}` has no field `{}`; did you mean `{}`?",
                        name, f.name, c
                    ),
                    None => format!(
                        "`{}` has no field `{}`; its fields are {}",
                        name, f.name, order
                    ),
                };
                let mut d = Diagnostic::error("E0226", f.name_span, message);
                if let Some(c) = close {
                    d = d.with_fix(
                        Applicability::Suggestion,
                        format!("write `{}`", c),
                        f.name_span,
                        c,
                    );
                }
                return Some(d);
            }
            if fields[..k].iter().any(|g| g.name == f.name) {
                return Some(Diagnostic::error(
                    "E0226",
                    f.name_span,
                    format!(
                        "field `{}` of `{}` is named twice; {} is built with each field once",
                        f.name, name, noun
                    ),
                ));
            }
        }
        let written: Vec<&String> = fields.iter().map(|f| &f.name).collect();
        let missing: Vec<&String> = names.iter().filter(|n| !written.contains(n)).collect();
        if !missing.is_empty() {
            return Some(Diagnostic::error(
                "E0226",
                span,
                format!(
                    "this `{}` leaves out {}; a field has no default, so {} is built with every field, in the order of its declaration: {}",
                    name,
                    list(&missing),
                    noun,
                    order
                ),
            ));
        }
        if written.iter().zip(&names).all(|(a, b)| *a == b) {
            return None;
        }
        // Every field once, out of order.
        let first = fields.first()?.name_span;
        let last = fields.last()?;
        let whole = Span {
            end: self.span(last.value).end,
            ..first
        };
        Some(Diagnostic::error(
            "E0226",
            whole,
            format!(
                "the fields of `{}` are named in the order of its declaration, which is the order their values are evaluated in: {}",
                name, order
            ),
        ))
    }

    fn call(&mut self, at: ExprId, callee: &str, args: &[Arg]) -> Ty {
        let resolved = self.resolution.calls[at.0 as usize];
        // A struct built with positional arguments.
        if resolved.is_none() {
            if let Some(def) = self.structs.get(callee) {
                let declared: Vec<(String, Ty)> = def
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.ty.clone()))
                    .collect();
                for (k, a) in args.iter().enumerate() {
                    match declared.get(k).filter(|_| args.len() == declared.len()) {
                        Some((n, ty)) => {
                            let place = format!("for field `{}` of `{}`", n, callee);
                            self.expect(a.value, ty, &place);
                        }
                        None => {
                            self.value(a.value);
                        }
                    }
                }
                let span = self.span(at);
                let order = declared
                    .iter()
                    .map(|(n, _)| format!("`{}`", n))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut d = Diagnostic::error(
                    "E0226",
                    span,
                    format!(
                        "a struct is built with its fields named, in the order of its declaration, as in `{}({}: ...)`: {}",
                        callee,
                        declared.first().map_or("", |(n, _)| n.as_str()),
                        order
                    ),
                );
                if args.len() == declared.len() && !args.iter().any(|a| a.is_mut) {
                    d.fixes.push(Fix {
                        message: "name each field".into(),
                        applicability: Applicability::MachineApplicable,
                        edits: args
                            .iter()
                            .zip(&declared)
                            .map(|(a, (n, _))| Edit {
                                span: insertion_point(self.span(a.value)),
                                replacement: format!("{}: ", n),
                            })
                            .collect(),
                    });
                }
                self.errors.push(d);
                return Ty::Struct(callee.to_string());
            }
        }
        let signature = resolved.and_then(|i| self.signatures.get(i).cloned().flatten());
        // A function of the module without a signature has no body: a declaration, which
        // nothing in M1 can run.
        if resolved.is_some() && signature.is_none() {
            self.errors.push(Diagnostic::error(
                "E0224",
                self.tree.expr(at).span,
                format!(
                    "`{}` has no body, so this call has nothing to run; {} calls only a function with a body",
                    callee,
                    mente_syntax::diagnostics::MILESTONE
                ),
            ));
        }
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

    /// The binding at the root of a place: `a` for `a`, `a[i]`, `a[i][j]` and `a.f[i]`.
    fn root(&self, place: ExprId) -> Option<BindingId> {
        match &self.tree.expr(place).kind {
            ExprKind::Name(_) => self.resolution.names[place.0 as usize],
            ExprKind::Index { base, .. } | ExprKind::Field { base, .. } => self.root(*base),
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
            ExprKind::Construct { fields, .. } => {
                for f in fields {
                    self.names_in(f.value, out);
                }
            }
            ExprKind::Variant { payload, .. } => match payload {
                Payload::None => {}
                Payload::Named(fields) => {
                    for f in fields {
                        self.names_in(f.value, out);
                    }
                }
                Payload::Positional(args) => {
                    for a in args {
                        self.names_in(a.value, out);
                    }
                }
            },
            ExprKind::Match { scrutinee, arms } => {
                self.names_in(*scrutinee, out);
                for arm in arms {
                    match arm.body {
                        ArmBody::Expr(x) => self.names_in(x, out),
                        ArmBody::Block(b) => self.names_in_block(b, out),
                    }
                }
            }
            ExprKind::Array(items) => {
                for &i in items {
                    self.names_in(i, out);
                }
            }
            ExprKind::Repeat { value, .. } => self.names_in(*value, out),
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.names_in(*cond, out);
                self.names_in_block(*then, out);
                match otherwise {
                    Some(Else::Block(b)) => self.names_in_block(*b, out),
                    Some(Else::If(i)) => self.names_in(*i, out),
                    None => {}
                }
            }
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Error => {}
        }
    }

    /// Every resolved name in the statements of block `b`.
    fn names_in_block(&self, b: BlockId, out: &mut Vec<(BindingId, Span)>) {
        for &s in &self.tree.block(b).stmts {
            match &self.tree.stmt(s).kind {
                StmtKind::Binding { init: e, .. }
                | StmtKind::Discard(e)
                | StmtKind::Expr(e)
                | StmtKind::Return(Some(e)) => self.names_in(*e, out),
                StmtKind::Assign { place, value, .. } => {
                    self.names_in(*place, out);
                    self.names_in(*value, out);
                }
                StmtKind::While {
                    cond,
                    invariants,
                    decreases,
                    body,
                } => {
                    self.names_in(*cond, out);
                    for &e in invariants.iter().chain(decreases) {
                        self.names_in(e, out);
                    }
                    self.names_in_block(*body, out);
                }
                StmtKind::Guard { cond, otherwise } => {
                    self.names_in(*cond, out);
                    self.names_in_block(*otherwise, out);
                }
                StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
            }
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
            BindingKind::Pattern => Diagnostic::error(
                code,
                at,
                format!(
                    "cannot {} `{}`: a pattern binds it to a part of the value matched, which it only reads; copy it into a `var` first",
                    action, b.name
                ),
            )
            .with_related(b.span, "bound here"),
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
            StmtKind::Expr(e) => self.discarded(*e, false),
            StmtKind::Discard(e) => {
                if self.value(*e) == Ty::Unit {
                    let hint = self.no_value_hint(*e);
                    self.mismatch(
                        *e,
                        format!("`_ =` discards a value, but this produces none{}", hint),
                    );
                }
            }
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

/// What a `match` reads, for its patterns, its coverage and the arms no value reaches.
enum Read<'a> {
    Enum(&'a crate::structs::EnumDef),
    Bool,
    /// An integer of the range its type or its refinement gives it, as a message writes it.
    Int(Exact, Exact, String),
    /// A value whose type a diagnostic already reported, or none a `match` reads.
    Other,
}

/// A value of a pattern, as coverage counts it: a variant by its tag, a `bool`, or an integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Value {
    Variant(usize),
    Bool(bool),
    Int(Exact),
}

impl<'a> Checker<'a> {
    /// `Name.A` or `Name.B(field: value, ...)`: a variant of an enum of the module (`E0215` for
    /// one the enum lacks), its payload built as a struct is, every field once, in order, each
    /// value flowing into its field's type (`E0226`).
    fn variant(
        &mut self,
        at: ExprId,
        enum_name: &str,
        (name, name_span): (&str, Span),
        payload: &Payload,
    ) -> Ty {
        let values: Vec<ExprId> = match payload {
            Payload::None => Vec::new(),
            Payload::Named(fields) => fields.iter().map(|f| f.value).collect(),
            Payload::Positional(args) => args.iter().map(|a| a.value).collect(),
        };
        let Some(def) = self.structs.get_enum(enum_name) else {
            // Name resolution reported it: a call on a receiver, or an unknown name.
            for v in values {
                self.value(v);
            }
            return Ty::Any;
        };
        let ty = Ty::Enum(enum_name.to_string());
        let Some((_, vdef)) = def.variant(name) else {
            for v in values {
                self.value(v);
            }
            if def.ok {
                self.errors.push(self.no_such_variant(def, name, name_span));
            }
            return ty;
        };
        let declared: Vec<(String, Ty)> = vdef
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        let path = format!("{}.{}", enum_name, name);
        let span = self.span(at);
        match payload {
            Payload::None if declared.is_empty() => {}
            Payload::None => {
                let order = declared
                    .iter()
                    .map(|(n, _)| format!("{}: ...", n))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.errors.push(Diagnostic::error(
                    "E0226",
                    span,
                    format!(
                        "`{}` holds a payload, built with each field named, in the order of its declaration: `{}({})`",
                        path, path, order
                    ),
                ));
            }
            Payload::Named(_) | Payload::Positional(_) if declared.is_empty() => {
                for v in values {
                    self.value(v);
                }
                let parens = Span {
                    start: name_span.end,
                    ..span
                };
                let mut d = Diagnostic::error(
                    "E0226",
                    span,
                    format!(
                        "`{}` holds nothing, so it is written without parentheses: `{}`",
                        path, path
                    ),
                );
                if matches!(payload, Payload::Positional(args) if args.is_empty()) {
                    d = d.with_fix(
                        Applicability::MachineApplicable,
                        "remove the parentheses",
                        parens,
                        "",
                    );
                }
                self.errors.push(d);
            }
            Payload::Named(fields) => {
                for f in fields {
                    match declared.iter().find(|(n, _)| *n == f.name) {
                        Some((_, t)) => {
                            let place = format!("for field `{}` of `{}`", f.name, path);
                            self.expect(f.value, t, &place);
                        }
                        None => {
                            self.value(f.value);
                        }
                    }
                }
                if def.ok {
                    let noun = "a variant's payload";
                    if let Some(d) = self.field_order(span, (&path, noun), fields, &declared) {
                        self.errors.push(d);
                    }
                }
            }
            Payload::Positional(args) => {
                for (k, a) in args.iter().enumerate() {
                    match declared.get(k).filter(|_| args.len() == declared.len()) {
                        Some((n, t)) => {
                            let place = format!("for field `{}` of `{}`", n, path);
                            self.expect(a.value, t, &place);
                        }
                        None => {
                            self.value(a.value);
                        }
                    }
                }
                let order = declared
                    .iter()
                    .map(|(n, _)| format!("`{}`", n))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut d = Diagnostic::error(
                    "E0226",
                    span,
                    format!(
                        "a variant's payload is built with its fields named, in the order of its declaration, as in `{}({}: ...)`: {}",
                        path, declared[0].0, order
                    ),
                );
                if args.len() == declared.len() && !args.iter().any(|a| a.is_mut) {
                    d.fixes.push(Fix {
                        message: "name each field".into(),
                        applicability: Applicability::MachineApplicable,
                        edits: args
                            .iter()
                            .zip(&declared)
                            .map(|(a, (n, _))| Edit {
                                span: insertion_point(self.span(a.value)),
                                replacement: format!("{}: ", n),
                            })
                            .collect(),
                    });
                }
                self.errors.push(d);
            }
        }
        ty
    }

    /// `E0215` for a variant `name` that enum `def` lacks, with the closest variant suggested.
    fn no_such_variant(&self, def: &crate::structs::EnumDef, name: &str, at: Span) -> Diagnostic {
        let names = def.variant_names();
        let close = closest(name, &names);
        let message = match &close {
            Some(c) => format!(
                "`{}` has no variant `{}`; did you mean `{}`?",
                def.name, name, c
            ),
            None => format!(
                "`{}` has no variant `{}`; its variants are {}",
                def.name,
                name,
                names
                    .iter()
                    .map(|n| format!("`{}`", n))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let mut d = Diagnostic::error("E0215", at, message);
        if let Some(c) = close {
            d = d.with_fix(Applicability::Suggestion, format!("write `{}`", c), at, c);
        }
        d
    }

    /// `match scrutinee { arm ... }` (`core-calculus.md`, item 14): it reads an enum, a `bool`
    /// or an integer; each pattern is one of what it reads, its bindings taking their fields'
    /// types; every value is matched (`E0229`), no arm is one no value reaches (`E0230`), and
    /// `_` stands only on an integer (`E0231`). Its arms share the type of its value, as an
    /// `if`'s branches do, when it gives one.
    fn match_expr(&mut self, at: ExprId, scrutinee: ExprId, arms: &[Arm], used: bool) -> Ty {
        let found = self.value(scrutinee);
        let read = match &found {
            Ty::Enum(n) => match self.structs.get_enum(n) {
                Some(def) if def.ok => Read::Enum(def),
                _ => Read::Other,
            },
            Ty::Bool => Read::Bool,
            Ty::Int(i) => {
                let (lo, hi, text) = self.int_read(scrutinee, *i);
                Read::Int(lo, hi, text)
            }
            Ty::Any => Read::Other,
            other => {
                self.mismatch(
                    scrutinee,
                    format!(
                        "`match` reads an enum, a `bool` or an integer, but this is {}",
                        other.describe()
                    ),
                );
                Read::Other
            }
        };
        let mut clean = !matches!(read, Read::Other);
        let mut values: Vec<Option<Value>> = Vec::new();
        for (k, arm) in arms.iter().enumerate() {
            let v = self.pattern(at, k, arm, &read);
            clean &= v.is_some();
            values.push(v.flatten());
        }
        if clean {
            self.coverage(at, arms, &read, &values);
        }
        self.freeze(at, scrutinee, arms, used);
        // The arms, each checked once its pattern's bindings have their types.
        let mut types = Vec::new();
        for arm in arms {
            let t = match (arm.body, used) {
                (ArmBody::Expr(x), true) => self.value(x),
                (ArmBody::Expr(x), false) => {
                    self.discarded(x, true);
                    Ty::Unit
                }
                (ArmBody::Block(b), used) => self.block(b, used),
            };
            // An arm that leaves has any type.
            types.push(if arm_ends(self.tree, arm.body) {
                t
            } else {
                Ty::Any
            });
        }
        if !used {
            return Ty::Unit;
        }
        let mut joined = Ty::Any;
        for (arm, t) in arms.iter().zip(&types) {
            match joined.join(t) {
                Some(j) => joined = j,
                None => {
                    self.errors.push(Diagnostic::error(
                        "E0203",
                        arm.span,
                        format!(
                            "the arms of this `match` produce its value, so they have one type, but this one is {} after {}",
                            t.describe(),
                            joined.describe()
                        ),
                    ));
                    return Ty::Any;
                }
            }
        }
        joined
    }

    /// The range of integers an integer scrutinee takes: its declared refinement's when both
    /// bounds are literals, else its type's; with the type as a message writes it.
    fn int_read(&self, scrutinee: ExprId, i: Int) -> (Exact, Exact, String) {
        if let Some((tree, t)) = self.declared_type(scrutinee) {
            if let TypeKind::Int {
                refinement: Some(r),
                ..
            } = &tree.ty(t).kind
            {
                if let (
                    Bound::Int {
                        negative: ln,
                        value: lv,
                    },
                    Bound::Int {
                        negative: hn,
                        value: hv,
                    },
                ) = (&r.lo, &r.hi)
                {
                    let hi = Exact::new(*hn, *hv);
                    let hi = if r.inclusive { Some(hi) } else { hi.plus(-1) };
                    if let Some(hi) = hi {
                        return (Exact::new(*ln, *lv), hi, tree.render_type(t));
                    }
                }
            }
        }
        let t = match i {
            Int::One(t) | Int::Mixed(Some(t)) => t,
            Int::Literal => crate::types::I64,
            Int::Mixed(None) => {
                return (
                    Exact::new(true, 1 << 127),
                    Exact::new(false, u128::MAX),
                    String::from("integer"),
                )
            }
        };
        (
            Exact::new(true, t.min_magnitude()),
            Exact::new(false, t.max()),
            t.name.to_string(),
        )
    }

    /// The declared type of `e`, where it has one with its refinement: a binding's, a field's, a
    /// pattern binding's payload field's, and an element's of one of those.
    fn declared_type(&self, e: ExprId) -> Option<(&Tree, TypeId)> {
        match &self.tree.expr(e).kind {
            ExprKind::Name(_) => {
                let id = self.resolution.names[e.0 as usize]?;
                if let Some(&decl) = self.pattern_decls.get(&id) {
                    return Some(decl);
                }
                let t = self.resolution.bindings[id.0 as usize].ty?;
                Some((self.tree, t))
            }
            ExprKind::Field { base, name } => match &self.typing.exprs[base.0 as usize] {
                Ty::Struct(s) => {
                    let def = self.structs.get(s)?;
                    let (_, f) = def.field(name)?;
                    Some((def.tree(), f.decl))
                }
                _ => None,
            },
            ExprKind::Index { base, .. } => {
                let (tree, t) = self.declared_type(*base)?;
                match &tree.ty(t).kind {
                    TypeKind::Array { elem, .. } => Some((tree, *elem)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Checks the pattern of arm `k` of the `match` at `m` against what the `match` reads, and
    /// gives its bindings their fields' types: `None` after reporting a pattern of another kind
    /// or a variant the enum lacks; otherwise the value it matches, `None` for `_`.
    fn pattern(
        &mut self,
        m: ExprId,
        k: usize,
        arm: &Arm,
        read: &Read<'a>,
    ) -> Option<Option<Value>> {
        let p = &arm.pattern;
        let kind = match read {
            Read::Enum(def) => format!("`{}`", def.name),
            Read::Bool => String::from("a `bool`"),
            Read::Int(_, _, text) => format!("an integer, `{}`", text),
            Read::Other => String::from("a value of no type it reads"),
        };
        let wrong = |what: &str| {
            Diagnostic::error(
                "E0203",
                p.span,
                format!("this pattern is {}, but the `match` reads {}", what, kind),
            )
        };
        match (&p.kind, read) {
            (
                PatternKind::Variant {
                    enum_name,
                    name,
                    name_span,
                    bindings,
                },
                read,
            ) => {
                let Some(edef) = self.structs.get_enum(enum_name) else {
                    // Name resolution reported an enum the module lacks.
                    return None;
                };
                let Read::Enum(def) = read else {
                    if !matches!(read, Read::Other) {
                        self.errors
                            .push(wrong(&format!("a variant of `{}`", enum_name)));
                    }
                    return None;
                };
                if def.name != edef.name {
                    self.errors
                        .push(wrong(&format!("a variant of `{}`", enum_name)));
                    return None;
                }
                let Some((tag, vdef)) = def.variant(name) else {
                    self.errors
                        .push(self.no_such_variant(def, name, *name_span));
                    return None;
                };
                let tree = def.tree();
                let decls: Vec<TypeId> = vdef.fields.iter().map(|f| f.decl).collect();
                let path = format!("{}.{}", def.name, name);
                // Each binding takes the type of the field it names, or, written by position,
                // of the field at its place.
                for (i, b) in bindings.iter().flatten().enumerate() {
                    let field = match &b.field {
                        Some((f, _)) => vdef.field(f).map(|(j, _)| j),
                        None => Some(i).filter(|&j| j < vdef.fields.len()),
                    };
                    if let (Some(j), Some(&id)) = (field, self.resolution.patterns.get(&(m, k, i)))
                    {
                        self.typing.bindings[id.0 as usize] = vdef.fields[j].ty.clone();
                        self.pattern_decls.insert(id, (tree, decls[j]));
                    }
                }
                if let Some(d) = self.pattern_fields(p.span, *name_span, &path, vdef, bindings) {
                    self.errors.push(d);
                }
                Some(Some(Value::Variant(tag)))
            }
            (PatternKind::Wildcard, Read::Int(..) | Read::Other) => Some(None),
            (PatternKind::Wildcard, Read::Enum(_) | Read::Bool) => {
                self.errors.push(self.wildcard(m, k, read));
                None
            }
            (PatternKind::Bool(b), Read::Bool) => Some(Some(Value::Bool(*b))),
            (PatternKind::Bool(_), Read::Other) => None,
            (PatternKind::Bool(_), _) => {
                self.errors.push(wrong("a `bool`"));
                None
            }
            (
                PatternKind::Int {
                    negative,
                    value,
                    suffix,
                },
                read,
            ) => {
                // A pattern's literal takes the type of what the `match` reads, so it has no
                // suffix (`core-calculus.md`, item 14); the fix removes it.
                if let Some(suffix) = suffix {
                    let text = self.src.get(p.span.start..p.span.end).unwrap_or("");
                    let cut = text
                        .strip_suffix(suffix.as_str())
                        .map(|t| t.strip_suffix('_').unwrap_or(t).len());
                    let mut d = Diagnostic::error(
                        "E0221",
                        p.span,
                        format!(
                            "a pattern's literal takes the type of what the `match` reads, so it has no suffix: `{}`",
                            text.get(..cut.unwrap_or(text.len())).unwrap_or(text)
                        ),
                    );
                    if let Some(cut) = cut {
                        d = d.with_fix(
                            Applicability::MachineApplicable,
                            format!("remove the suffix `{}`", text.get(cut..).unwrap_or("")),
                            Span {
                                start: p.span.start + cut,
                                end: p.span.end,
                                ..p.span
                            },
                            "",
                        );
                    }
                    self.errors.push(d);
                    return None;
                }
                match read {
                    Read::Int(..) => Some(Some(Value::Int(Exact::new(*negative, *value)))),
                    Read::Other => None,
                    _ => {
                        self.errors.push(wrong("an integer"));
                        None
                    }
                }
            }
        }
    }

    /// `E0226` for a variant pattern that does not name every field of its payload once, in
    /// order, each bound to a name or `_`: with the fix that names them where it can.
    fn pattern_fields(
        &self,
        span: Span,
        name_span: Span,
        path: &str,
        vdef: &crate::structs::VariantDef,
        bindings: &Option<Vec<PatBinding>>,
    ) -> Option<Diagnostic> {
        let names: Vec<String> = vdef.fields.iter().map(|f| f.name.clone()).collect();
        let order = names
            .iter()
            .map(|n| format!("`{}`", n))
            .collect::<Vec<_>>()
            .join(", ");
        let bound = names
            .iter()
            .map(|n| format!("{}: _", n))
            .collect::<Vec<_>>()
            .join(", ");
        let after_name = Span {
            start: name_span.end,
            end: name_span.end,
            ..name_span
        };
        let Some(bs) = bindings else {
            if names.is_empty() {
                return None;
            }
            return Some(
                Diagnostic::error(
                    "E0226",
                    span,
                    format!(
                        "`{}` holds a payload, and a pattern names every field of it in order, binding each to a name or to `_`: `{}({})`",
                        path, path, bound
                    ),
                )
                .with_fix(
                    Applicability::MachineApplicable,
                    "bind each field to `_`",
                    after_name,
                    format!("({})", bound),
                ),
            );
        };
        if names.is_empty() {
            let parens = Span {
                start: name_span.end,
                ..span
            };
            return Some(
                Diagnostic::error(
                    "E0226",
                    span,
                    format!(
                        "`{}` holds nothing, so its pattern has no parentheses: `{}`",
                        path, path
                    ),
                )
                .with_fix(
                    Applicability::MachineApplicable,
                    "remove the parentheses",
                    parens,
                    "",
                ),
            );
        }
        // Bindings written by position: the fix names their fields.
        if bs.iter().any(|b| b.field.is_none()) {
            let mut d = Diagnostic::error(
                "E0226",
                span,
                format!(
                    "a pattern names each field of `{}` it binds, in the order of the declaration: {}",
                    path, order
                ),
            );
            if bs.len() == names.len() && bs.iter().all(|b| b.field.is_none()) {
                d.fixes.push(Fix {
                    message: "name each field".into(),
                    applicability: Applicability::MachineApplicable,
                    edits: bs
                        .iter()
                        .zip(&names)
                        .map(|(b, n)| Edit {
                            span: insertion_point(b.name_span),
                            replacement: format!("{}: ", n),
                        })
                        .collect(),
                });
            }
            return Some(d);
        }
        let written: Vec<(&String, Span)> = bs
            .iter()
            .filter_map(|b| b.field.as_ref().map(|(f, s)| (f, *s)))
            .collect();
        for (k, (f, fspan)) in written.iter().enumerate() {
            if !names.contains(f) {
                let close = closest(f, &names);
                let message = match &close {
                    Some(c) => format!("`{}` has no field `{}`; did you mean `{}`?", path, f, c),
                    None => format!("`{}` has no field `{}`; its fields are {}", path, f, order),
                };
                let mut d = Diagnostic::error("E0226", *fspan, message);
                if let Some(c) = close {
                    d = d.with_fix(
                        Applicability::Suggestion,
                        format!("write `{}`", c),
                        *fspan,
                        c,
                    );
                }
                return Some(d);
            }
            if written[..k].iter().any(|(g, _)| g == f) {
                return Some(Diagnostic::error(
                    "E0226",
                    *fspan,
                    format!(
                        "field `{}` of `{}` is named twice; a pattern names each field once",
                        f, path
                    ),
                ));
            }
        }
        let missing: Vec<String> = names
            .iter()
            .filter(|n| !written.iter().any(|(w, _)| w == n))
            .map(|n| format!("`{}`", n))
            .collect();
        if !missing.is_empty() {
            return Some(Diagnostic::error(
                "E0226",
                span,
                format!(
                    "this pattern leaves out {}; it names every field of `{}`, binding one it does not need to `_`: {}",
                    missing.join(", "),
                    path,
                    order
                ),
            ));
        }
        if written.iter().zip(&names).all(|((w, _), n)| *w == n) {
            return None;
        }
        Some(Diagnostic::error(
            "E0226",
            span,
            format!(
                "a pattern names the fields of `{}` in the order of its declaration: {}",
                path, order
            ),
        ))
    }

    /// `E0231` for `_` in arm `k`, on an enum or a `bool`: `_` never covers a variant, so that
    /// a variant added to an enum makes every `match` on it name it. The fix writes an arm for
    /// each value no arm before it matches, with `_`'s body.
    fn wildcard(&self, m: ExprId, k: usize, read: &Read) -> Diagnostic {
        let ExprKind::Match { arms, .. } = &self.tree.expr(m).kind else {
            unreachable!("a pattern of a `match`")
        };
        let arm = &arms[k];
        let before: Vec<&PatternKind> = arms[..k].iter().map(|a| &a.pattern.kind).collect();
        let left: Vec<String> = match read {
            Read::Enum(def) => def
                .variants
                .iter()
                .filter(|v| {
                    !before
                        .iter()
                        .any(|p| matches!(p, PatternKind::Variant { name, .. } if *name == v.name))
                })
                .map(|v| variant_pattern(&def.name, v))
                .collect(),
            _ => [true, false]
                .iter()
                .filter(|b| !before.contains(&&PatternKind::Bool(**b)))
                .map(|b| b.to_string())
                .collect(),
        };
        let message = match read {
            Read::Enum(def) => format!(
                "`_` stands only on an integer, but this `match` reads an enum, `{}`: name each variant, so that one added later must be named too",
                def.name
            ),
            _ => String::from(
                "`_` stands only on an integer, but this `match` reads a `bool`: name both values, `true` and `false`",
            ),
        };
        let mut d = Diagnostic::error("E0231", arm.pattern.span, message);
        let body_span = match arm.body {
            ArmBody::Expr(x) => self.span(x),
            ArmBody::Block(b) => self.tree.block(b).span,
        };
        let body = self.src.get(body_span.start..body_span.end);
        if let (Some(body), false) = (body, left.is_empty()) {
            let indent = " ".repeat(arm.span.col.saturating_sub(1) as usize);
            let text = left
                .iter()
                .map(|p| format!("{} => {}", p, body))
                .collect::<Vec<_>>()
                .join(&format!("\n{}", indent));
            d = d.with_fix(
                Applicability::Suggestion,
                "write an arm for each value left",
                arm.span,
                text,
            );
        }
        d
    }

    /// The arms no value reaches (`E0230`), behind an earlier arm or outside what the `match`
    /// reads, and the values no arm matches (`E0229`), whose fix adds an arm for each.
    fn coverage(&mut self, m: ExprId, arms: &[Arm], read: &Read, values: &[Option<Value>]) {
        let mut seen: BTreeSet<Value> = BTreeSet::new();
        let mut first: Vec<(Value, Span)> = Vec::new();
        let mut wild: Option<Span> = None;
        for (arm, v) in arms.iter().zip(values) {
            let unreached = |why: String| {
                Diagnostic::error(
                    "E0230",
                    arm.pattern.span,
                    format!("no value reaches this arm: {}; remove it", why),
                )
                .with_fix(
                    Applicability::MachineApplicable,
                    "remove the arm",
                    self.arm_line(arm),
                    "",
                )
            };
            if let Some(w) = wild {
                self.errors.push(
                    unreached("`_` before it matches every value left".into())
                        .with_related(w, "`_` is here"),
                );
                continue;
            }
            match v {
                None => {
                    if let Read::Int(lo, hi, _) = read {
                        if covered(&seen, *lo, *hi).is_none() {
                            self.errors
                                .push(unreached("the arms before it match every value".into()));
                            continue;
                        }
                    }
                    wild = Some(arm.pattern.span);
                }
                Some(v) => {
                    if let (Value::Int(x), Read::Int(lo, hi, text)) = (v, read) {
                        if x < lo || x > hi {
                            // A refinement holds what the `match` reads; a plain type holds it
                            // since the `match` narrows its scrutinee into the type
                            // ([`match-scrutinee-narrowing`]).
                            let why = if text.contains('[') {
                                format!(
                                    "{} lies outside `{}`, the values of what the `match` reads",
                                    x, text
                                )
                            } else {
                                format!(
                                    "the `match` narrows what it reads into `{}`, and {} lies outside it",
                                    text, x
                                )
                            };
                            self.errors.push(unreached(why));
                            continue;
                        }
                    }
                    if let Some((_, at)) = first.iter().find(|(f, _)| f == v) {
                        let at = *at;
                        self.errors.push(
                            unreached("an arm before it matches the same value".into())
                                .with_related(at, "matched first here"),
                        );
                        continue;
                    }
                    seen.insert(*v);
                    first.push((*v, arm.pattern.span));
                }
            }
        }
        if wild.is_some() {
            return;
        }
        let missing: Vec<String> = match read {
            Read::Enum(def) => def
                .variants
                .iter()
                .enumerate()
                .filter(|(t, _)| !seen.contains(&Value::Variant(*t)))
                .map(|(_, v)| variant_pattern(&def.name, v))
                .collect(),
            Read::Bool => [true, false]
                .iter()
                .filter(|b| !seen.contains(&Value::Bool(**b)))
                .map(|b| b.to_string())
                .collect(),
            Read::Int(lo, hi, _) => covered(&seen, *lo, *hi)
                .map(|x| vec![x.to_string()])
                .unwrap_or_default(),
            Read::Other => Vec::new(),
        };
        if missing.is_empty() {
            return;
        }
        let span = self.span(m);
        let message = match read {
            Read::Enum(def) => format!(
                "this `match` does not match {}; it has an arm for every variant of `{}`",
                missing
                    .iter()
                    .map(|p| format!("`{}`", p))
                    .collect::<Vec<_>>()
                    .join(", "),
                def.name
            ),
            Read::Bool => format!(
                "this `match` does not match `{}`; it has an arm for both values of `bool`",
                missing[0]
            ),
            Read::Int(_, _, text) => format!(
                "this `match` does not match {}, among others, a value of `{}`; add an arm for each value, or `_` after the arms for every value left",
                missing[0], text
            ),
            Read::Other => String::new(),
        };
        let mut d = Diagnostic::error("E0229", span, message);
        let new_arms: Vec<String> = match read {
            Read::Int(..) => vec![String::from("_")],
            _ => missing,
        };
        if let Some(last) = arms.last() {
            let indent = " ".repeat(last.span.col.saturating_sub(1) as usize);
            let text: String = new_arms
                .iter()
                .map(|p| {
                    format!(
                        "\n{}{} => todo(\"{}\")",
                        indent,
                        p,
                        p.replace('\\', "\\\\").replace('"', "\\\"")
                    )
                })
                .collect();
            d = d.with_fix(
                Applicability::Suggestion,
                "add an arm for each value left",
                insertion_after(last.span),
                text,
            );
        }
        self.errors.push(d);
    }

    /// An arm's whole line, its newline included, for the fix that removes it; or its span, when
    /// another arm shares its line.
    fn arm_line(&self, arm: &Arm) -> Span {
        let src = self.src;
        let start = src
            .get(..arm.span.start)
            .and_then(|s| s.rfind('\n'))
            .map(|i| i + 1);
        let end = src
            .get(arm.span.end..)
            .and_then(|s| s.find('\n'))
            .map(|i| arm.span.end + i + 1);
        match (start, end) {
            (Some(s), Some(e))
                if src[s..arm.span.start].trim().is_empty()
                    && src[arm.span.end..e].trim().is_empty() =>
            {
                Span {
                    start: s,
                    end: e,
                    line: arm.span.line,
                    col: 1,
                }
            }
            _ => arm.span,
        }
    }

    /// A store or a `mut` argument that reaches the scrutinee's place in an arm whose pattern
    /// binds a part of it (`E0232`): the binding reads the place where it lies, which the arm
    /// keeps as it is (`core-calculus.md`, item 14).
    fn freeze(&mut self, m: ExprId, scrutinee: ExprId, arms: &[Arm], used: bool) {
        if !is_place(self.tree, scrutinee) {
            return;
        }
        let Some(root) = self.root(scrutinee) else {
            return;
        };
        // A place that cannot change is reported where it is changed, by `E0205` or `E0206`.
        let kind = self.resolution.bindings[root.0 as usize].kind;
        if !matches!(kind, BindingKind::Var | BindingKind::Param { is_mut: true }) {
            return;
        }
        let frozen = self.path(scrutinee);
        let text = self
            .src
            .get(self.span(scrutinee).start..self.span(scrutinee).end)
            .unwrap_or("the scrutinee")
            .to_string();
        // The places each arm changes that the scrutinee's place holds or is part of.
        let mut found: Vec<Vec<ExprId>> = Vec::new();
        for arm in arms {
            let mut changed = Vec::new();
            match arm.body {
                ArmBody::Expr(x) => changed_places(self.tree, x, &mut changed),
                ArmBody::Block(b) => changed_places_block(self.tree, b, &mut changed),
            }
            changed.retain(|&place| {
                self.root(place) == Some(root) && overlaps(&frozen, &self.path(place))
            });
            found.push(changed);
        }
        if found.iter().all(Vec::is_empty) {
            return;
        }
        let transition = if used {
            None
        } else {
            self.transition(m, scrutinee, arms, &found, &text)
        };
        for place in found.into_iter().flatten() {
            let mut d = Diagnostic::error(
                "E0232",
                self.span(place),
                format!(
                    "the `match` reads `{}` before an arm runs, and its place stays frozen in every arm, so nothing in an arm changes `{}`; to give it a new value, write `{} = match {} {{ ... }}`, each arm giving the new value",
                    text, text, text, text
                ),
            )
            .with_related(self.span(scrutinee), "matched here");
            if let Some(fix) = &transition {
                d.fixes.push(fix.clone());
            }
            self.errors.push(d);
        }
    }

    /// The fix that writes a `match` statement whose arms change its scrutinee as a
    /// transition, `state = match state { ... }`: each arm a block that ends by assigning the
    /// whole place, which then gives the value instead, or that changes nothing of it, which
    /// then gives the place's value as it is. `None` for any other shape, whose message names
    /// the form.
    fn transition(
        &self,
        m: ExprId,
        scrutinee: ExprId,
        arms: &[Arm],
        found: &[Vec<ExprId>],
        text: &str,
    ) -> Option<Fix> {
        let at = self.span(m);
        let frozen = self.path(scrutinee);
        let root = self.root(scrutinee)?;
        let mut edits = vec![Edit {
            span: Span {
                end: at.start,
                ..at
            },
            replacement: format!("{} = ", text),
        }];
        for (arm, changed) in arms.iter().zip(found) {
            let ArmBody::Block(b) = arm.body else {
                return None;
            };
            let block = self.tree.block(b);
            if changed.is_empty() {
                // The arm gives the place's value as it is, on a line of its own.
                let edit = match block.stmts.last() {
                    Some(&last) => {
                        let s = self.tree.stmt(last).span;
                        let line_start = self.src.get(..s.start)?.rfind('\n').map_or(0, |i| i + 1);
                        let indent: String = self
                            .src
                            .get(line_start..s.start)?
                            .chars()
                            .take_while(|c| *c == ' ' || *c == '\t')
                            .collect();
                        Edit {
                            span: Span { start: s.end, ..s },
                            replacement: format!("\n{}{}", indent, text),
                        }
                    }
                    None => Edit {
                        span: block.span,
                        replacement: format!("{{ {} }}", text),
                    },
                };
                edits.push(edit);
                continue;
            }
            // Else the arm's last statement assigns the whole place, and nothing before it
            // changes any of it.
            let &last = block.stmts.last()?;
            let StmtKind::Assign {
                place,
                op: AssignOp::Set,
                value,
            } = &self.tree.stmt(last).kind
            else {
                return None;
            };
            if changed.as_slice() != [*place]
                || self.root(*place) != Some(root)
                || self.path(*place) != frozen
            {
                return None;
            }
            let value = self.span(*value);
            edits.push(Edit {
                span: self.tree.stmt(last).span,
                replacement: self.src.get(value.start..value.end)?.to_string(),
            });
        }
        Some(Fix {
            message: format!(
                "write it as a transition, `{} = match {} {{ ... }}`",
                text, text
            ),
            applicability: Applicability::Suggestion,
            edits,
        })
    }

    /// A place's steps from its root: each field by its name, each index as any element.
    fn path(&self, place: ExprId) -> Vec<Option<String>> {
        match &self.tree.expr(place).kind {
            ExprKind::Index { base, .. } => {
                let mut p = self.path(*base);
                p.push(None);
                p
            }
            ExprKind::Field { base, name } => {
                let mut p = self.path(*base);
                p.push(Some(name.clone()));
                p
            }
            _ => Vec::new(),
        }
    }
}

/// Whether two places of one root binding may overlap: one is a part of the other, an index
/// counting as any element.
fn overlaps(a: &[Option<String>], b: &[Option<String>]) -> bool {
    a.iter().zip(b).all(|(x, y)| match (x, y) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    })
}

/// The first value of `lo ..= hi` no value of `seen` covers, or `None` when they cover it all.
fn covered(seen: &BTreeSet<Value>, lo: Exact, hi: Exact) -> Option<Exact> {
    let mut x = lo;
    while x <= hi {
        if !seen.contains(&Value::Int(x)) {
            return Some(x);
        }
        x = x.plus(1)?;
    }
    None
}

/// A pattern for variant `v` of enum `e`, each field bound to `_`.
fn variant_pattern(e: &str, v: &crate::structs::VariantDef) -> String {
    if v.fields.is_empty() {
        return format!("{}.{}", e, v.name);
    }
    let fields: Vec<String> = v.fields.iter().map(|f| format!("{}: _", f.name)).collect();
    format!("{}.{}({})", e, v.name, fields.join(", "))
}

/// The point right after `span`.
fn insertion_after(span: Span) -> Span {
    Span {
        start: span.end,
        end: span.end,
        ..span
    }
}

/// Every place an expression changes: the place of each assignment, and of each `mut`
/// argument, in it and in every block it holds.
pub fn changed_places(tree: &Tree, e: ExprId, out: &mut Vec<ExprId>) {
    match &tree.expr(e).kind {
        ExprKind::Call { args, .. } => {
            for a in args {
                if a.is_mut {
                    out.push(a.value);
                }
                changed_places(tree, a.value, out);
            }
        }
        ExprKind::Variant { payload, .. } => match payload {
            Payload::None => {}
            Payload::Named(fields) => {
                for f in fields {
                    changed_places(tree, f.value, out);
                }
            }
            Payload::Positional(args) => {
                for a in args {
                    if a.is_mut {
                        out.push(a.value);
                    }
                    changed_places(tree, a.value, out);
                }
            }
        },
        ExprKind::Construct { fields, .. } => {
            for f in fields {
                changed_places(tree, f.value, out);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            changed_places(tree, *scrutinee, out);
            for a in arms {
                match a.body {
                    ArmBody::Expr(x) => changed_places(tree, x, out),
                    ArmBody::Block(b) => changed_places_block(tree, b, out),
                }
            }
        }
        ExprKind::Unary { operand, .. } => changed_places(tree, *operand, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            changed_places(tree, *lhs, out);
            changed_places(tree, *rhs, out);
        }
        ExprKind::Index { base, index } => {
            changed_places(tree, *base, out);
            changed_places(tree, *index, out);
        }
        ExprKind::Field { base, .. } => changed_places(tree, *base, out),
        ExprKind::Array(items) => {
            for &i in items {
                changed_places(tree, i, out);
            }
        }
        ExprKind::Repeat { value, .. } => changed_places(tree, *value, out),
        ExprKind::If {
            cond,
            then,
            otherwise,
        } => {
            changed_places(tree, *cond, out);
            changed_places_block(tree, *then, out);
            match otherwise {
                Some(Else::Block(b)) => changed_places_block(tree, *b, out),
                Some(Else::If(i)) => changed_places(tree, *i, out),
                None => {}
            }
        }
        ExprKind::Int { .. }
        | ExprKind::Bool(_)
        | ExprKind::Name(_)
        | ExprKind::Todo(_)
        | ExprKind::Unreachable(_)
        | ExprKind::Error => {}
    }
}

/// [`changed_places`] of every statement of a block.
pub fn changed_places_block(tree: &Tree, b: BlockId, out: &mut Vec<ExprId>) {
    for &s in &tree.block(b).stmts {
        match &tree.stmt(s).kind {
            StmtKind::Assign { place, value, .. } => {
                out.push(*place);
                changed_places(tree, *place, out);
                changed_places(tree, *value, out);
            }
            StmtKind::Binding { init: e, .. }
            | StmtKind::Discard(e)
            | StmtKind::Expr(e)
            | StmtKind::Return(Some(e)) => changed_places(tree, *e, out),
            StmtKind::While {
                cond,
                invariants,
                decreases,
                body,
            } => {
                changed_places(tree, *cond, out);
                for &x in invariants.iter().chain(decreases) {
                    changed_places(tree, x, out);
                }
                changed_places_block(tree, *body, out);
            }
            StmtKind::Guard { cond, otherwise } => {
                changed_places(tree, *cond, out);
                changed_places_block(tree, *otherwise, out);
            }
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
        }
    }
}

/// The literals with a suffix that are the operand of a unary `-`.
fn negated_literals(tree: &Tree) -> HashSet<ExprId> {
    tree.exprs
        .iter()
        .filter_map(|e| match e.kind {
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => Some(operand),
            _ => None,
        })
        .collect()
}

/// Whether `e` names a place: a binding, or an element or a field of one.
pub fn is_place(tree: &Tree, e: ExprId) -> bool {
    match &tree.expr(e).kind {
        ExprKind::Name(_) => true,
        ExprKind::Index { base, .. } | ExprKind::Field { base, .. } => is_place(tree, *base),
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
            | StmtKind::Discard(e)
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
        // A `match` is exhaustive, so its end is reached only through an arm that ends.
        ExprKind::Match { arms, .. } => arms.iter().any(|a| arm_ends(tree, a.body)),
        _ => true,
    }
}

/// Whether control can reach the end of an arm.
pub fn arm_ends(tree: &Tree, body: ArmBody) -> bool {
    match body {
        ArmBody::Expr(x) => expr_ends(tree, x),
        ArmBody::Block(b) => block_ends(tree, b),
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

    /// Every diagnostic of the module.
    fn all_diagnostics(src: &str) -> Vec<Diagnostic> {
        check_source(&module(src)).err().unwrap_or_default()
    }

    /// The diagnostics of the module but unused values and bindings (`E0217`, `E0218`): these
    /// programs bind and compute values only to check them, and those codes have tests of
    /// their own.
    fn diagnostics(src: &str) -> Vec<Diagnostic> {
        all_diagnostics(src)
            .into_iter()
            .filter(|d| !matches!(d.code, "E0217" | "E0218"))
            .collect()
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
        let parsed = crate::parse_qualified(&text, &module);
        let names: Vec<String> = parsed.iter().map(|p| p.name.clone()).collect();
        let signatures: Vec<Option<Signature>> = parsed
            .iter()
            .map(|p| p.syntax.as_ref().map(Signature::of))
            .collect();
        let Some(f) = parsed.last().and_then(|p| p.syntax.clone()) else {
            return Vec::new();
        };
        let parsed_structs = mente_syntax::parse_structs(&text, &module);
        let parsed_enums = mente_syntax::parse_enums(&text, &module);
        let (structs, _) = crate::structs::declare(&module, &parsed_structs, &parsed_enums);
        let (resolution, _) = crate::names::resolve(&f, &names, &structs);
        let (typing, errors) = check(&f, &resolution, &signatures, &structs, &text);
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
            codes("@checked\npub fn f(x: u8, c: bool)\n  requires x > 0 and c\n  decreases x\n{\n  var i = x\n  while i > 0\n    invariant i <= x\n    decreases i\n  {\n    i -= 1\n  }\n  guard c else {\n    return\n  }\n}"),
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

    /// A refinement's bound has the type it reads: `name` an integer, `name.len` an array
    /// (the fuzzer's finding 15, which checked and could not be lowered).
    #[test]
    fn test_a_refinements_bound_has_the_type_it_reads() {
        let d = all_diagnostics("pub fn f(xs: [8]u8, n: usize[xs..=8]) -> u8 = 0");
        assert_eq!(d.len(), 1, "{:?}", d);
        assert_eq!(
            (d[0].code, d[0].message.as_str()),
            (
                "E0203",
                "a refinement's bound is an integer, but `xs` is `[8]u8`"
            )
        );
        assert_eq!(
            all_codes("pub fn f(k: u8, n: usize[0..k.len]) -> u8 = 0"),
            ["E0203"]
        );
        assert_eq!(
            all_codes("pub fn f(b: bool, n: usize[0..b]) -> u8 = 0"),
            ["E0203"]
        );
        // A binding of its type, a parameter or a `val`, is a bound.
        assert!(
            all_codes("pub fn f(xs: [8]u8, k: usize, n: usize[k..xs.len]) -> u8 = 0").is_empty()
        );
        assert!(all_codes(
            "pub fn f(xs: [8]u8) -> u8 {\n  val k: usize = 2\n  val n: usize[0..k] = 1\n  xs[n]\n}"
        )
        .is_empty());
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
            ["E0213", "E0203", "E0203"]
        );
        // A declaration without a body cannot be called, and no more is checked of the call
        // than its arguments.
        assert_eq!(codes("pub fn f() = declared(true, 1)"), ["E0224"]);
        assert_eq!(
            messages("pub fn f() = declared(true, 1)"),
            ["`declared` has no body, so this call has nothing to run; M2 calls only a function with a body"]
        );
        assert_eq!(
            codes("pub fn f() -> u8 = declared(not 1)"),
            ["E0224", "E0203"]
        );
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
        let missing = diagnostics("pub fn f() {\n  var v: u8 = 0\n  v = 1\n  touch(v)\n}");
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

    const PAIR: &str = "pub struct Pair {
  i: u8
  j: u16
}
pub struct Box2 {
  p: Pair
  xs: [2]Pair
}
";

    /// A struct is built with every field once, in the order of its declaration, each value
    /// flowing into its field's type (`E0226`, `E0203`); a function takes no named
    /// arguments (`E0227`). Positional arguments of the right number get a fix that names them.
    #[test]
    fn test_structs_are_built_with_every_field_once_in_order() {
        let with = |f: &str| format!("{}{}", PAIR, f);
        assert_eq!(codes(&with("pub fn f() -> Pair = Pair(i: 0, j: 1)")), NONE);
        assert_eq!(
            binding_types(&with(
                "pub fn f() -> u16 {
  val p = Pair(i: 0, j: 1)
  val b = Box2(p: p, xs: [p, p])
  b.xs[1].j
}"
            )),
            [("p", "Pair"), ("b", "Box2")].map(|(n, t)| (n.to_string(), t.to_string()))
        );
        for (src, message) in [
            ("pub fn f() -> Pair = Pair(i: 0)", "this `Pair` leaves out `j`; a field has no default, so a struct is built with every field, in the order of its declaration: `i`, `j`"),
            ("pub fn f() -> Pair = Pair(j: 1, i: 0)", "the fields of `Pair` are named in the order of its declaration, which is the order their values are evaluated in: `i`, `j`"),
            ("pub fn f() -> Pair = Pair(i: 0, zz: 1)", "`Pair` has no field `zz`; its fields are `i`, `j`"),
            ("pub fn f() -> Pair = Pair(i: 0, jj: 1)", "`Pair` has no field `jj`; did you mean `j`?"),
            ("pub fn f() -> Pair = Pair(i: 0, i: 1, j: 2)", "field `i` of `Pair` is named twice; a struct is built with each field once"),
            ("pub fn f() -> Pair = Pair(0, 1)", "a struct is built with its fields named, in the order of its declaration, as in `Pair(i: ...)`: `i`, `j`"),
        ] {
            assert_eq!(codes(&with(src)), ["E0226"], "{}", src);
            assert_eq!(messages(&with(src)), [message], "{}", src);
        }
        let positional = with("pub fn f() -> Pair = Pair(0, 1)");
        let d = &diagnostics(
            "pub struct Pair {
  i: u8
  j: u16
}
pub fn f() -> Pair = Pair(0, 1)",
        )[0];
        assert_eq!(
            mente_syntax::diagnostics::apply_fix(
                &module(
                    "pub struct Pair {
  i: u8
  j: u16
}
pub fn f() -> Pair = Pair(0, 1)"
                ),
                &d.fixes[0]
            )
            .unwrap(),
            module(
                "pub struct Pair {
  i: u8
  j: u16
}
pub fn f() -> Pair = Pair(i: 0, j: 1)"
            )
        );
        assert_eq!(codes(&positional), ["E0226"]);
        assert_eq!(
            codes(&with("pub fn f() -> Pair = Pair(i: true, j: 1)")),
            ["E0203"]
        );
        assert_eq!(
            codes(&with(
                "pub fn f() -> Box2 = Box2(p: Pair(i: 0, j: 1), xs: [1, 2])"
            )),
            ["E0203"]
        );
        assert_eq!(codes("pub fn f() -> u8 = id(x: 1)"), ["E0227"]);
        assert_eq!(
            messages("pub fn f() -> u8 = id(x: 1)"),
            ["`id` is a function, whose arguments are positional; only the arguments that build a struct are named"]
        );
        // Positional arguments of the right number still flow into their fields' types; of
        // another number, into none. The fix that names them is given only for the right
        // number and no `mut`.
        assert_eq!(
            codes(&with("pub fn f() -> Pair = Pair(true, 1)")),
            ["E0226", "E0203"]
        );
        assert_eq!(codes(&with("pub fn f() -> Pair = Pair(true)")), ["E0226"]);
        assert_eq!(
            codes(&with("pub fn f() -> Pair = Pair(true, 1, 2)")),
            ["E0226"]
        );
        let fixes = |src: &str| {
            diagnostics(&with(src))
                .iter()
                .find(|d| d.code == "E0226")
                .map(|d| d.fixes.len())
        };
        assert_eq!(fixes("pub fn f() -> Pair = Pair(0, 1)"), Some(1));
        assert_eq!(fixes("pub fn f() -> Pair = Pair(0)"), Some(0));
        assert_eq!(
            fixes("pub fn f(x: u8) -> Pair {\n  var y = x\n  Pair(mut y, 1)\n}"),
            Some(0)
        );
    }

    /// `a.name` reads an array's `len` or a struct's field (`E0215` otherwise, with the closest
    /// field suggested); a field of a `var` or a `mut` parameter is a place, assigned and passed
    /// as `mut` (`E0205`, `E0206`), whose root binding no other argument names (`E0207`), a
    /// field of another argument too, and through an `if`; `==` on structs is a type error.
    #[test]
    fn test_fields_are_read_and_stored_through_places() {
        let with = |f: &str| format!("{}{}", PAIR, f);
        assert_eq!(
            codes(&with(
                "pub fn f(b: Box2, k: usize) -> u16 = b.p.i + b.xs[k].j + b.xs.len"
            )),
            NONE
        );
        assert_eq!(codes(&with("pub fn f(b: Box2) -> u8 = b.q")), ["E0215"]);
        assert_eq!(
            messages(&with("pub fn f(p: Pair) -> u8 = p.ii")),
            ["`Pair` has no field `ii`; did you mean `i`?"]
        );
        // The error lies at the field's name, its line and column too, and so does its fix.
        let src = module(&with("pub fn f(b: Box2) -> u8 = b.xs[0].jj"));
        let d = check_source(&src).err().unwrap_or_default();
        let d = d.iter().find(|d| d.code == "E0215").unwrap();
        let at = src.rfind("jj").unwrap();
        let line_start = src[..at].rfind('\n').map_or(0, |k| k + 1);
        assert_eq!((d.primary.start, d.primary.end), (at, at + 2));
        assert_eq!(d.primary.col as usize, at - line_start + 1);
        assert_eq!(d.primary.line as usize, src[..at].matches('\n').count() + 1);
        assert_eq!(d.fixes[0].edits[0].span, d.primary);
        assert_eq!(
            messages(&with("pub fn f(p: Pair) -> u8 = p.len")),
            ["`Pair` has no field `len`; its fields are `i`, `j`"]
        );
        assert_eq!(
            messages("pub fn f(x: u8) -> u8 = x.size"),
            ["`.size` reads a field of a struct, but this is `u8`"]
        );
        assert_eq!(messages("pub fn f(x: [2]u8) -> u8 = x.size"), ["an array has one field, `.len`, its length, but not `.size`; an element is read with an index, as in `a[i]`"]);
        assert_eq!(
            codes(&with(
                "pub fn f(mut b: Box2, k: usize) {
  b.p.i = 1
  b.xs[k].j += 2
  b.xs[0] = b.p
}"
            )),
            NONE
        );
        assert_eq!(
            codes(&with(
                "pub fn f(b: Box2) {
  b.p.i = 1
}"
            )),
            ["E0205"]
        );
        assert_eq!(
            codes(&with(
                "pub fn f() -> u8 {
  val p = Pair(i: 0, j: 1)
  touch(mut p.i)
  p.i
}"
            )),
            ["E0206"]
        );
        assert_eq!(
            codes(&with(
                "pub fn f(mut p: Pair) {
  touch(mut p.i)
}"
            )),
            NONE
        );
        assert_eq!(
            codes(&with(
                "fn two(mut a: u8, mut b: u16) {
  a = 1
  b = 2
}
pub fn f(mut p: Pair) {
  two(mut p.i, mut p.j)
}"
            )),
            ["E0207"]
        );
        assert_eq!(
            codes(&with(
                "fn put(mut a: u8, b: u16) {
  a = 1
}
pub fn f(mut p: Pair) {
  put(mut p.i, p.j)
}"
            )),
            ["E0207"]
        );
        assert_eq!(
            codes(&with(
                "fn put(mut a: u8, b: u8) {
  a = b
}
pub fn f(mut p: Pair, c: bool) {
  put(mut p.i, if c { p.i } else { 0 })
}"
            )),
            ["E0207"]
        );
        // And inside a struct being built.
        assert_eq!(
            codes(&with(
                "fn keep(mut a: u8, q: Pair) {
  a = q.i
}
pub fn f(mut p: Pair) {
  keep(mut p.i, Pair(i: p.i, j: 0))
}"
            )),
            ["E0207"]
        );
        // `mut` field arguments have the parameter's type exactly.
        assert_eq!(
            codes(&with(
                "pub fn f(mut p: Pair) {
  touch(mut p.j)
}"
            )),
            ["E0203"]
        );
        assert_eq!(
            codes(&with("pub fn f(a: Pair, b: Pair) -> bool = a == b")),
            ["E0109"]
        );
    }

    #[test]
    fn test_an_array_holds_at_most_4_gib() {
        let d = all_diagnostics("pub fn f(x: [4294967296]u8) -> u8 = 0");
        assert_eq!(d.len(), 1, "{:?}", d);
        assert_eq!(d[0].code, "E0223");
        assert_eq!(
            d[0].message,
            "an array of type `[4294967296]u8` holds 4294967296 bytes, more than the 4294967295 an array, a value kept in one piece, may hold"
        );
        // The bytes count, not the elements: a `u32` takes four.
        assert_eq!(
            all_codes("pub fn f(x: [1073741824]u32) -> u8 = 0"),
            ["E0223"]
        );
        assert!(all_codes("pub fn f(x: [1073741823]u32) -> u8 = 0").is_empty());
        assert!(all_codes("pub fn f(x: [4294967295]u8) -> u8 = 0").is_empty());
        // A `bool` takes one.
        assert_eq!(
            all_codes("pub fn f(x: [4294967296]bool) -> u8 = 0"),
            ["E0223"]
        );
        // A literal of exactly the most bytes is not too large either.
        assert!(all_codes("pub fn f() -> u8 {\n  _ = [0_u8; 4294967295]\n  0\n}").is_empty());
        // Literals alone may be kept in sixteen bytes each, `u128`'s, and count so.
        assert_eq!(
            all_codes("pub fn f() -> u8 {\n  _ = [18446744073709551616; 268435456]\n  0\n}"),
            ["E0223"]
        );
        assert!(
            all_codes("pub fn f() -> u8 {\n  _ = [18446744073709551616; 268435455]\n  0\n}")
                .is_empty()
        );
        // The innermost type that is too large is reported once, and a literal without a
        // written type is reported itself.
        assert_eq!(
            all_codes("pub fn f(x: [2][4294967296]u8) -> u8 = 0"),
            ["E0223"]
        );
        assert_eq!(
            all_codes("pub fn f() -> u8 {\n  _ = [0_u8; 4294967296]\n  0\n}"),
            ["E0223"]
        );
        // A literal whose written type is reported is not reported again, and a value that is
        // no literal, such as a call's result, is reported where its type is written only.
        assert_eq!(
            all_codes("pub fn f() -> u8 {\n  val a: [4294967296]u8 = [0; 4294967296]\n  a[0]\n}"),
            ["E0223"]
        );
        assert_eq!(
            all_codes("pub fn g() -> [4294967296]u8 = todo(\"x\")\npub fn f() -> u8 = g()[0]"),
            ["E0223"]
        );
    }

    /// An array holds at most 4,294,967,295 elements, whatever their size, wherever its type is
    /// written or its literal built.
    #[test]
    fn test_an_array_holds_at_most_4294967295_elements() {
        let d = all_diagnostics("pub fn f(x: [4294967296][0]u8) -> u8 = 0");
        assert_eq!(d.len(), 1, "{:?}", d);
        assert_eq!(d[0].code, "E0223");
        assert_eq!(
            d[0].message,
            "an array of type `[4294967296][0]u8` holds 4294967296 elements, more than the 4294967295 an array may hold, whatever their size"
        );
        assert!(all_codes("pub fn f(x: [4294967295][0]u8) -> u8 = 0").is_empty());
        assert_eq!(
            all_codes("pub fn f() -> [4294967296][0]bool = todo(\"x\")"),
            ["E0223"]
        );
        assert_eq!(
            all_codes(
                "pub fn f() -> usize {\n  val xs: [1][4294967296][0]u8 = todo(\"x\")\n  xs.len\n}"
            ),
            ["E0223"]
        );
        // A literal without a written type, and one of exactly the most elements.
        assert_eq!(
            all_codes("pub fn f() -> u8 {\n  _ = [[0_u8; 0]; 4294967296]\n  0\n}"),
            ["E0223"]
        );
        assert!(all_codes("pub fn f() -> u8 {\n  _ = [[0_u8; 0]; 4294967295]\n  0\n}").is_empty());
        // Of the module's own struct of no bytes.
        assert_eq!(
            all_codes("pub struct Z {\n  a: [0]u8\n}\npub fn f(zs: [4294967296]Z) -> u8 = 0"),
            ["E0223"]
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
        assert_eq!(codes("pub fn f() -> [2]u16 = [1, 300]"), NONE);
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
        assert_eq!(
            codes("pub fn f(v: u8) {\n  declared(mut v)\n}"),
            ["E0224", "E0206"]
        );
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
            // Not every body assigns the bindings these programs declare.
            .filter(|d| d.code != "E0219")
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
        assert_eq!(codes_of("  declared(mut v)"), ["E0224"]);
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

    fn all_codes(src: &str) -> Vec<&'static str> {
        all_diagnostics(src).iter().map(|d| d.code).collect()
    }

    #[test]
    fn test_a_value_nobody_uses_is_discarded_on_purpose() {
        let d = all_diagnostics("pub fn f(a: u8) {\n  id(a)\n}");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0217");
        assert_eq!(
            d[0].message,
            "this value, `u8`, is never used; use it, or discard it on purpose with `_ = ...`"
        );
        assert_eq!((d[0].primary.line, d[0].primary.col), (10, 3));
        let fix = &d[0].fixes[0];
        assert_eq!(fix.message, "discard it on purpose");
        assert_eq!(fix.applicability, Applicability::Suggestion);
        assert_eq!(fix.edits[0].replacement, "_ = ");
        assert_eq!(fix.edits[0].span.start, fix.edits[0].span.end);
        assert_eq!(fix.edits[0].span.start, d[0].primary.start);
        assert_eq!(all_codes("pub fn f(a: u8) {\n  a + 1\n}"), ["E0217"]);
        assert_eq!(all_codes("pub fn f(a: u8) = a + 1"), ["E0217"]);
        assert_eq!(all_codes("pub fn f(a: u8) {\n  a\n}"), ["E0217"]);
        assert_eq!(
            all_codes("pub fn f(c: bool) {\n  if c {\n    id(1)\n  } else {\n    2\n  }\n}"),
            ["E0217", "E0217"]
        );
        assert_eq!(
            all_codes("pub fn f(c: bool) {\n  while c {\n    c\n  }\n}"),
            ["E0217"]
        );
        for ok in [
            "pub fn f(a: u8) {\n  _ = id(a)\n}",
            "pub fn f(a: u8) {\n  _ = a + 1\n  nothing()\n}",
            "pub fn f() {\n  todo(\"later\")\n}",
            "pub fn f(c: bool) {\n  if c {\n    nothing()\n  }\n}",
            "pub fn f() = nothing()",
        ] {
            assert_eq!(all_codes(ok), NONE, "{}", ok);
        }
        assert_eq!(all_codes("pub fn f() {\n  _ = nothing()\n}"), ["E0203"]);
        assert_eq!(
            messages("pub fn f() {\n  _ = nothing()\n}"),
            ["`_ =` discards a value, but this produces none; `nothing` has no result type"]
        );
        assert_eq!(all_codes("pub fn f() {\n  _ = zz\n}"), ["E0201"]);
        assert_eq!(
            all_codes("pub fn f(c: bool) {\n  guard c else {\n    _ = unreachable(\"c\")\n  }\n}"),
            NONE
        );
    }

    #[test]
    fn test_a_literal_lies_in_its_suffix_type() {
        let d = all_diagnostics("pub fn f() -> u8 = 300_u8");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0221");
        assert_eq!(
            d[0].message,
            "the literal 300_u8 lies outside `u8`, which holds 0 to 255"
        );
        assert_eq!((d[0].primary.line, d[0].primary.col), (9, 20));
        assert_eq!(all_codes("pub fn f() -> u8 = 255_u8"), NONE);
        assert_eq!(all_codes("pub fn f() -> i8 = 128_i8"), ["E0221"]);
        assert_eq!(all_codes("pub fn f() -> i8 = 127_i8"), NONE);
        assert_eq!(all_codes("pub fn f() -> i8 = -128_i8"), NONE);
        assert_eq!(all_codes("pub fn f() -> i8 = -129_i8"), ["E0221"]);
        assert_eq!(all_codes("pub fn f() -> i64 = -255_u8"), NONE);
        assert_eq!(all_codes("pub fn f() -> u8 = -256_u8"), ["E0221"]);
        assert_eq!(all_codes("pub fn f() -> i64 = 1 - 255_u8"), NONE);
        assert_eq!(all_codes("pub fn f() -> u8 = 1 - 256_u8"), ["E0221"]);
        assert_eq!(all_codes("pub fn f() -> i64 = -(1 + 128_i8)"), ["E0221"]);
        assert_eq!(
            all_codes("pub fn f() -> u128 = 340282366920938463463374607431768211455_u128"),
            NONE
        );
    }

    /// The enums the tests of variants and `match` read.
    const SHAPES: &str = "pub enum Shape {
  Dot
  Rect(w: u8, h: u16)
}
pub enum Light {
  Red
  Amber
  Green
}
";

    /// Applies the first fix of the first diagnostic of `src`, a module's text, and returns the
    /// fixed text with what checking it reports.
    fn fixed_and_checked(src: &str) -> (String, Vec<&'static str>) {
        let d = &check_source(src).err().unwrap()[0];
        let fixed = mente_syntax::diagnostics::apply_fix(src, &d.fixes[0]).unwrap();
        let codes = check_source(&fixed)
            .err()
            .unwrap_or_default()
            .iter()
            .map(|d| d.code)
            .collect();
        (fixed, codes)
    }

    /// A variant is named with its enum and built as a struct is: every field of its payload
    /// once, in order, each value flowing into its field's type, and none for a variant that
    /// holds nothing; the fixes name fields given by position and remove empty parentheses.
    #[test]
    fn test_variants_are_built_with_their_payloads() {
        let with = |f: &str| format!("{}{}", SHAPES, f);
        assert_eq!(
            codes(&with("pub fn f() -> Shape = Shape.Rect(w: 1, h: 2)")),
            NONE
        );
        assert_eq!(codes(&with("pub fn f() -> Shape = Shape.Dot")), NONE);
        assert_eq!(
            binding_types(&with(
                "pub fn f() -> u8 {\n  val s = Shape.Dot\n  val l = Light.Red\n  0\n}"
            )),
            [("s", "Shape"), ("l", "Light")].map(|(n, t)| (n.to_string(), t.to_string()))
        );
        for (src, code, message) in [
            ("pub fn f() -> Shape = Shape.Rect(w: 1)", "E0226", "this `Shape.Rect` leaves out `h`; a field has no default, so a variant's payload is built with every field, in the order of its declaration: `w`, `h`"),
            ("pub fn f() -> Shape = Shape.Rect(h: 1, w: 2)", "E0226", "the fields of `Shape.Rect` are named in the order of its declaration, which is the order their values are evaluated in: `w`, `h`"),
            ("pub fn f() -> Shape = Shape.Rect(w: 1, hh: 2)", "E0226", "`Shape.Rect` has no field `hh`; did you mean `h`?"),
            ("pub fn f() -> Shape = Shape.Rect(1, 2)", "E0226", "a variant's payload is built with its fields named, in the order of its declaration, as in `Shape.Rect(w: ...)`: `w`, `h`"),
            ("pub fn f() -> Shape = Shape.Rect", "E0226", "`Shape.Rect` holds a payload, built with each field named, in the order of its declaration: `Shape.Rect(w: ..., h: ...)`"),
            ("pub fn f() -> Shape = Shape.Dot()", "E0226", "`Shape.Dot` holds nothing, so it is written without parentheses: `Shape.Dot`"),
            ("pub fn f() -> Shape = Shape.Dto", "E0215", "`Shape` has no variant `Dto`; did you mean `Dot`?"),
            ("pub fn f() -> Shape = Shape.Rect(w: true, h: 2)", "E0203", "expected `u8` for field `w` of `Shape.Rect`, found `bool`"),
            ("pub fn f() -> Light = Shape.Dot", "E0203", "expected `Light` as the result of `f`, found `Shape`"),
            ("pub fn f(s: Shape) -> bool = s == Shape.Dot", "E0109", "comparing structs and enums with `==` comes later in M2; the compiler does not have it yet"),
        ] {
            assert_eq!(codes(&with(src)), [code], "{}", src);
            assert_eq!(messages(&with(src)), [message], "{}", src);
        }
        // A call on a binding is a construct of a later milestone, and an unknown enum a name
        // nobody declared.
        assert_eq!(codes(&with("pub fn f(x: u8) -> u8 = x.max(1)")), ["E0109"]);
        assert_eq!(codes(&with("pub fn f() -> Shape = Shpe.Dot(1)")), ["E0201"]);
        assert_eq!(codes(&with("pub fn f() -> Shape = Shape(w: 1)")), ["E0204"]);
        // The fixes apply, and what they write checks.
        for (src, want) in [
            (
                "pub fn f() -> Shape = Shape.Rect(1, 2)",
                "pub fn f() -> Shape = Shape.Rect(w: 1, h: 2)",
            ),
            (
                "pub fn f() -> Shape = Shape.Dot()",
                "pub fn f() -> Shape = Shape.Dot",
            ),
        ] {
            let (fixed, codes) = fixed_and_checked(&module(&with(src)));
            assert!(fixed.contains(want), "{}", fixed);
            assert!(codes.is_empty(), "{}: {:?}", fixed, codes);
        }
    }

    /// A `match` reads an enum, a `bool` or an integer, its patterns of what it reads; its arms
    /// share its value's type; its bindings take their fields' types; and each pattern names
    /// every field of its payload once, in order.
    #[test]
    fn test_patterns_read_what_the_match_reads() {
        let with = |f: &str| format!("{}{}", SHAPES, f);
        assert_eq!(
            binding_types(&with(
                "pub fn f(s: Shape) -> u16 = match s {\n  Shape.Rect(w: a, h: b) => a + b\n  Shape.Dot => 0\n}"
            )),
            [("s", "Shape"), ("a", "u8"), ("b", "u16")]
                .map(|(n, t)| (n.to_string(), t.to_string()))
        );
        for (src, message) in [
            ("pub fn f(xs: [2]u8) -> u8 = match xs {\n  _ => 0\n}", "`match` reads an enum, a `bool` or an integer, but this is `[2]u8`"),
            ("pub fn f(b: bool) -> u8 = match b {\n  true => 1\n  0 => 2\n}", "this pattern is an integer, but the `match` reads a `bool`"),
            ("pub fn f(x: u8) -> u8 = match x {\n  true => 1\n  _ => 2\n}", "this pattern is a `bool`, but the `match` reads an integer, `u8`"),
            ("pub fn f(l: Light) -> u8 = match l {\n  Shape.Dot => 1\n  Light.Red => 2\n}", "this pattern is a variant of `Shape`, but the `match` reads `Light`"),
            ("pub fn f(x: u8) -> u8 = match x {\n  Light.Red => 1\n  _ => 2\n}", "this pattern is a variant of `Light`, but the `match` reads an integer, `u8`"),
            ("pub fn f(c: bool) -> u8 = match c {\n  true => 1\n  false => false\n}", "the arms of this `match` produce its value, so they have one type, but this one is `bool` after an integer"),
            ("pub fn f(x: u8) -> u8 = match x {\n  300_u8 => 1\n  _ => 2\n}", "a pattern's literal takes the type of what the `match` reads, so it has no suffix: `300`"),
            ("pub fn f(x: u8) -> u8 = match x {\n  0_u8 => 1\n  _ => 2\n}", "a pattern's literal takes the type of what the `match` reads, so it has no suffix: `0`"),
            ("pub fn f(x: i8) -> u8 = match x {\n  -1_i64 => 1\n  _ => 2\n}", "a pattern's literal takes the type of what the `match` reads, so it has no suffix: `-1`"),
        ] {
            assert_eq!(messages(&with(src)), [message], "{}", src);
        }
        // The fix removes the suffix, and the module then checks.
        let src = module(&with(
            "pub fn f(x: u8) -> u8 = match x {\n  0x1F_u8 => 1\n  _ => 2\n}",
        ));
        let (fixed, left) = fixed_and_checked(&src);
        assert!(fixed.contains("  0x1F => 1"), "{}", fixed);
        assert!(left.is_empty(), "{:?}", left);
        // An arm that leaves gives no value, and a `match` statement's arms none.
        assert_eq!(
            codes(&with(
                "pub fn f(l: Light) -> u8 {\n  val n: u8 = match l {\n    Light.Red => {\n      return 1\n    }\n    Light.Amber => todo(\"later\")\n    Light.Green => 3\n  }\n  n\n}"
            )),
            NONE
        );
        assert_eq!(
            codes(&with(
                "pub fn f(c: bool, mut n: u8) {\n  match c {\n    true => {\n      n = 1\n    }\n    false => {}\n  }\n}"
            )),
            NONE
        );
        for (arm, message) in [
            ("Shape.Rect(h: _, w: a) => a", "a pattern names the fields of `Shape.Rect` in the order of its declaration: `w`, `h`"),
            ("Shape.Rect(w: a) => a", "this pattern leaves out `h`; it names every field of `Shape.Rect`, binding one it does not need to `_`: `w`, `h`"),
            ("Shape.Rect(w: a, hh: _) => a", "`Shape.Rect` has no field `hh`; did you mean `h`?"),
            ("Shape.Rect(w: a, w: _) => a", "field `w` of `Shape.Rect` is named twice; a pattern names each field once"),
            ("Shape.Rect(a, _) => a", "a pattern names each field of `Shape.Rect` it binds, in the order of the declaration: `w`, `h`"),
            ("Shape.Rect => 1", "`Shape.Rect` holds a payload, and a pattern names every field of it in order, binding each to a name or to `_`: `Shape.Rect(w: _, h: _)`"),
        ] {
            let src = with(&format!(
                "pub fn f(s: Shape) -> u8 = match s {{\n  {}\n  Shape.Dot => 0\n}}",
                arm
            ));
            assert_eq!(messages(&src), [message], "{}", src);
        }
        let src = module(&with(
            "pub fn f(s: Shape) -> u8 = match s {\n  Shape.Dot() => 0\n  Shape.Rect(a, _) => a\n}",
        ));
        let (fixed, codes) = fixed_and_checked(&src);
        assert!(fixed.contains("Shape.Dot => 0"), "{}", fixed);
        assert_eq!(codes, ["E0226"]);
        let (fixed, codes) = fixed_and_checked(&fixed);
        assert!(fixed.contains("Shape.Rect(w: a, h: _) => a"), "{}", fixed);
        assert!(codes.is_empty(), "{:?}", codes);
        let src = module(&with(
            "pub fn f(s: Shape) -> u8 = match s {\n  Shape.Rect => 1\n  Shape.Dot => 0\n}",
        ));
        let (fixed, codes) = fixed_and_checked(&src);
        assert!(fixed.contains("Shape.Rect(w: _, h: _) => 1"), "{}", fixed);
        assert!(codes.is_empty(), "{:?}", codes);
        // An arm of a `match` statement whose value nobody uses is `E0217`, whose fix discards
        // it in a block, since an arm holds an expression or a block and a discard is a
        // statement; the fixed module checks.
        let src = module(
            "fn g(x: u8) -> u8 = x\npub fn f(c: bool, x: u8) -> u8 {\n  match c {\n    true => g(x)\n    false => {}\n  }\n  x\n}",
        );
        let (fixed, codes) = fixed_and_checked(&src);
        assert!(fixed.contains("    true => { _ = g(x) }"), "{}", fixed);
        assert!(codes.is_empty(), "{:?}", codes);
    }

    /// Every value a `match` reads is matched by an arm: every variant, both `bool`s, and every
    /// integer of the scrutinee's refinement, or of its type, which `_` covers; no arm is one no
    /// value reaches; and `_` stands on an integer only. Each fix applies and checks.
    #[test]
    fn test_a_match_covers_every_value_once() {
        let with = |f: &str| format!("{}{}", SHAPES, f);
        for src in [
            "pub fn f(l: Light) -> u8 = match l {\n  Light.Green => 0\n  Light.Red => 1\n  Light.Amber => 2\n}",
            "pub fn f(x: u8[0..3]) -> u8 = match x {\n  2 => 0\n  0 => 1\n  1 => 2\n}",
            "pub fn f(x: i8[-1..=1]) -> u8 = match x {\n  -1 => 0\n  0 => 1\n  1 => 2\n}",
            "pub fn f(x: u8) -> u8 = match x {\n  0 => 0\n  255 => 1\n  _ => 2\n}",
            "pub fn f(x: i128) -> u8 = match x {\n  -170141183460469231731687303715884105728 => 0\n  170141183460469231731687303715884105727 => 1\n  _ => 2\n}",
            "pub fn f(xs: [2]u8[0..2]) -> u8 = match xs[1] {\n  0 => 0\n  1 => 1\n}",
            "pub fn f(c: bool) -> u8 = match c {\n  false => 0\n  true => 1\n}",
        ] {
            assert_eq!(all_codes(&with(src)), NONE, "{}", src);
        }
        for (src, code, message) in [
            ("pub fn f(l: Light) -> u8 = match l {\n  Light.Green => 0\n  Light.Red => 1\n}", "E0229", "this `match` does not match `Light.Amber`; it has an arm for every variant of `Light`"),
            ("pub fn f(c: bool) -> u8 = match c {\n  true => 0\n}", "E0229", "this `match` does not match `false`; it has an arm for both values of `bool`"),
            ("pub fn f(x: u8) -> u8 = match x {\n  0 => 0\n  1 => 1\n}", "E0229", "this `match` does not match 2, among others, a value of `u8`; add an arm for each value, or `_` after the arms for every value left"),
            ("pub fn f(x: u8[0..3]) -> u8 = match x {\n  0 => 0\n  2 => 1\n}", "E0229", "this `match` does not match 1, among others, a value of `u8[0..3]`; add an arm for each value, or `_` after the arms for every value left"),
            ("pub fn f(l: Light) -> u8 = match l {\n  Light.Green => 0\n  Light.Red => 1\n  Light.Amber => 2\n  Light.Red => 3\n}", "E0230", "no value reaches this arm: an arm before it matches the same value; remove it"),
            ("pub fn f(x: u8[0..3]) -> u8 = match x {\n  0 => 0\n  1 => 1\n  2 => 2\n  3 => 3\n}", "E0230", "no value reaches this arm: 3 lies outside `u8[0..3]`, the values of what the `match` reads; remove it"),
            ("pub fn f(x: u8) -> u8 = match x {\n  -1 => 0\n  _ => 1\n}", "E0230", "no value reaches this arm: the `match` narrows what it reads into `u8`, and -1 lies outside it; remove it"),
            ("pub fn f(x: u8) -> u8 = match x - 1 {\n  -1 => 100\n  _ => 0\n}", "E0230", "no value reaches this arm: the `match` narrows what it reads into `u8`, and -1 lies outside it; remove it"),
            ("pub fn f(x: u8) -> u8 = match x {\n  _ => 0\n  1 => 1\n}", "E0230", "no value reaches this arm: `_` before it matches every value left; remove it"),
            ("pub fn f(x: u8[0..2]) -> u8 = match x {\n  0 => 0\n  1 => 1\n  _ => 2\n}", "E0230", "no value reaches this arm: the arms before it match every value; remove it"),
            ("pub fn f(l: Light) -> u8 = match l {\n  Light.Red => 0\n  _ => 1\n}", "E0231", "`_` stands only on an integer, but this `match` reads an enum, `Light`: name each variant, so that one added later must be named too"),
            ("pub fn f(c: bool) -> u8 = match c {\n  true => 0\n  _ => 1\n}", "E0231", "`_` stands only on an integer, but this `match` reads a `bool`: name both values, `true` and `false`"),
        ] {
            assert_eq!(codes(&with(src)), [code], "{}", src);
            assert_eq!(messages(&with(src)), [message], "{}", src);
        }
        // E0229's fix adds an arm for each value left, and the module then checks.
        for (src, added) in [
            (
                "pub fn f(l: Light) -> u8 = match l {\n  Light.Green => 0\n}",
                "  Light.Red => todo(\"Light.Red\")\n  Light.Amber => todo(\"Light.Amber\")",
            ),
            (
                "pub fn f(s: Shape) -> u8 = match s {\n  Shape.Dot => 0\n}",
                "  Shape.Rect(w: _, h: _) => todo(\"Shape.Rect(w: _, h: _)\")",
            ),
            (
                "pub fn f(x: u8) -> u8 = match x {\n  0 => 0\n}",
                "  _ => todo(\"_\")",
            ),
        ] {
            let (fixed, codes) = fixed_and_checked(&module(&with(src)));
            assert!(fixed.contains(added), "{}", fixed);
            assert!(codes.is_empty(), "{}: {:?}", fixed, codes);
        }
        // E0230's fix removes the arm; E0231's writes an arm for each value left.
        let (fixed, codes) = fixed_and_checked(&module(&with(
            "pub fn f(x: u8) -> u8 = match x {\n  1 => 0\n  1 => 2\n  _ => 1\n}",
        )));
        assert!(
            fixed.contains("match x {\n  1 => 0\n  _ => 1\n}"),
            "{}",
            fixed
        );
        assert!(codes.is_empty(), "{:?}", codes);
        let (fixed, codes) = fixed_and_checked(&module(&with(
            "pub fn f(l: Light) -> u8 = match l {\n  Light.Red => 0\n  _ => 1\n}",
        )));
        assert!(
            fixed.contains("  Light.Amber => 1\n  Light.Green => 1"),
            "{}",
            fixed
        );
        assert!(codes.is_empty(), "{:?}", codes);
    }

    /// In every arm, whether its pattern binds a name or not, a store or a `mut` argument that
    /// reaches the scrutinee's place is `E0232`, whose fix writes a `match` statement that
    /// gives the place a new value as a transition, `s = match s { ... }`, where its arms allow;
    /// another field of the same binding and a place that cannot change (`E0205`) are not.
    #[test]
    fn test_a_matched_place_stays_frozen_in_every_arm() {
        let base = format!(
            "{}pub struct Box2 {{\n  s: Shape\n  n: u8\n}}\nfn clear(mut s: Shape) {{\n  s = Shape.Dot\n}}\nfn bump(mut n: u8) {{\n  n = 1\n}}\n",
            SHAPES
        );
        let with = |f: &str| format!("{}{}", base, f);
        let arm = |body: &str| {
            with(&format!(
                "pub fn f(mut b: Box2, mut xs: [2]Shape, i: usize) -> u8 {{\n  match b.s {{\n    Shape.Rect(w: w, h: _) => {{\n      {}\n      w\n    }}\n    Shape.Dot => 0\n  }}\n}}",
                body
            ))
        };
        for body in [
            "b.s = Shape.Dot",
            "b = Box2(s: Shape.Dot, n: 0)",
            "clear(mut b.s)",
        ] {
            assert_eq!(codes(&arm(body)), ["E0232"], "{}", body);
        }
        for body in ["b.n = 3", "bump(mut b.n)", "xs[0] = Shape.Dot"] {
            assert_eq!(codes(&arm(body)), NONE, "{}", body);
        }
        // Through an index, any element counts.
        let src = with(
            "pub fn f(mut xs: [2]Shape, i: usize, j: usize) -> u8 = match xs[i] {\n  Shape.Rect(w: w, h: _) => {\n    xs[j] = Shape.Dot\n    w\n  }\n  Shape.Dot => 0\n}",
        );
        assert_eq!(codes(&src), ["E0232"]);
        let d = &diagnostics(&src)[0];
        assert_eq!(d.related[0].message, "matched here");
        // An arm that binds nothing keeps it frozen too ([`match-arm-freeze`]).
        let src = with(
            "pub fn f(mut s: Shape) -> u8 = match s {\n  Shape.Rect(w: _, h: _) => {\n    s = Shape.Dot\n    0\n  }\n  Shape.Dot => 1\n}",
        );
        assert_eq!(codes(&src), ["E0232"]);
        let d = &diagnostics(&src)[0];
        assert_eq!(
            d.message,
            "the `match` reads `s` before an arm runs, and its place stays frozen in every arm, so nothing in an arm changes `s`; to give it a new value, write `s = match s { ... }`, each arm giving the new value"
        );
        // A `match` that gives a value has no transition to offer, its message naming one.
        assert!(d.fixes.is_empty());
        // A `match` statement whose arms end by assigning the place, or leave it as it is, is
        // offered as a transition, and the fixed module checks.
        let src = module(&with(
            "pub fn f(mut s: Shape, mut k: u8) {\n  match s {\n    Shape.Rect(w: _, h: _) => {\n      k = 1\n      s = Shape.Dot\n    }\n    Shape.Dot => {}\n  }\n  k += 1\n}",
        ));
        let (fixed, left) = fixed_and_checked(&src);
        assert!(
            fixed.contains("  s = match s {\n    Shape.Rect(w: _, h: _) => {\n      k = 1\n      Shape.Dot\n    }\n    Shape.Dot => { s }\n  }"),
            "{}",
            fixed
        );
        assert!(left.is_empty(), "{:?}", left);
        let src = module(&with(
            "pub fn f(mut s: Shape) {\n  match s {\n    Shape.Rect(w: _, h: _) => {\n      s = Shape.Dot\n    }\n    Shape.Dot => {\n      s = Shape.Rect(w: 1, h: 2)\n    }\n  }\n}",
        ));
        let d = &check_source(&src).unwrap_err();
        assert_eq!(
            d.iter().map(|d| d.code).collect::<Vec<_>>(),
            ["E0232", "E0232"]
        );
        let (fixed, left) = fixed_and_checked(&src);
        assert!(
            fixed.contains("  s = match s {\n    Shape.Rect(w: _, h: _) => {\n      Shape.Dot\n    }\n    Shape.Dot => {\n      Shape.Rect(w: 1, h: 2)\n    }\n  }"),
            "{}",
            fixed
        );
        assert!(left.is_empty(), "{:?}", left);
        // An assignment that is not the arm's last statement, or a `mut` argument, has no
        // transition to offer.
        for arm in [
            "{\n      s = Shape.Dot\n      k = 1\n    }",
            "{\n      clear(mut s)\n    }",
        ] {
            let src = with(&format!(
                "pub fn f(mut s: Shape, mut k: u8) {{\n  match s {{\n    Shape.Rect(w: _, h: _) => {}\n    Shape.Dot => {{}}\n  }}\n  k += 1\n}}",
                arm
            ));
            let d = diagnostics(&src);
            assert_eq!(
                d.iter().map(|d| d.code).collect::<Vec<_>>(),
                ["E0232"],
                "{}",
                src
            );
            assert!(d[0].fixes.is_empty(), "{}", src);
        }
        // A read-only parameter is reported as one, once.
        assert_eq!(
            codes(&with(
                "pub fn f(s: Shape) -> u8 = match s {\n  Shape.Rect(w: w, h: _) => {\n    s = Shape.Dot\n    w\n  }\n  Shape.Dot => 1\n}"
            )),
            ["E0205"]
        );
        // A pattern's binding is read-only.
        assert_eq!(
            codes(&with(
                "pub fn f(s: Shape) -> u8 = match s {\n  Shape.Rect(w: w, h: _) => {\n    bump(mut w)\n    w\n  }\n  Shape.Dot => 1\n}"
            )),
            ["E0206"]
        );
    }
}
