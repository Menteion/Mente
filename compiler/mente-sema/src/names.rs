//! Name resolution for M1's functions (`spec/core-calculus.md`, items 6 and 7).
//!
//! [`resolve`] walks one parsed function and binds each name to its declaration: a parameter,
//! a `val` or `var`, or `result` in an `ensures` clause. A name is declared once in a function,
//! its inner blocks included (no shadowing); it is visible from its declaration to the end of
//! its block, and no binding takes the name of a function of its module. It also checks what
//! follows from the names alone: `break` and `continue` stand in a loop, a refinement's bound
//! names a binding that cannot change, every `val` and `var` is read, and every `var` is
//! assigned. The resolution is kept in tables indexed like the tree.

use std::collections::{HashMap, HashSet};

use mente_syntax::diagnostics::{Applicability, Diagnostic, Edit, Fix};
use mente_syntax::tree::{
    ArmBody, BlockId, Body, Bound, ContractKind, Else, ExprId, ExprKind, FnSyntax, Pattern,
    PatternKind, Payload, StmtKind, Tree, TypeId, TypeKind,
};
use mente_syntax::Span;

use crate::structs::{unknown_type, Structs};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BindingId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    Param {
        is_mut: bool,
    },
    Val,
    Var,
    /// `result` in an `ensures` clause.
    Result,
    /// A name a `match` pattern binds to a field of a variant's payload: a read-only projection
    /// of the value matched (`core-calculus.md`, item 14).
    Pattern,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    /// Where the name is declared.
    pub span: Span,
    pub kind: BindingKind,
    /// The type it was declared with, when it states one.
    pub ty: Option<TypeId>,
}

/// What each name of one function refers to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolution {
    pub bindings: Vec<Binding>,
    /// For each expression of the tree, the binding a name refers to.
    pub names: Vec<Option<BindingId>>,
    /// For each expression of the tree, the function a call calls, by its index in the
    /// module's list of functions.
    pub calls: Vec<Option<usize>>,
    /// For each statement of the tree, the binding a `val` or `var` declares.
    pub stmts: Vec<Option<BindingId>>,
    /// Each refinement bound that names a binding, for the type checker to check its type.
    pub bounds: Vec<NamedBound>,
    /// The binding each name of a `match` pattern declares, by the `match`, the arm and the
    /// binding's position in the pattern.
    pub patterns: HashMap<(ExprId, usize, usize), BindingId>,
}

/// A refinement's bound that names a binding: `name`, or `name.len`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedBound {
    /// The refined type the bound belongs to.
    pub ty: TypeId,
    /// Whether it reads the binding's length, `name.len`.
    pub len: bool,
    pub binding: BindingId,
}

/// Resolves the names of `f`, whose module declares `functions`, in order, and `structs`.
pub fn resolve(
    f: &FnSyntax,
    functions: &[String],
    structs: &Structs,
) -> (Resolution, Vec<Diagnostic>) {
    let count = f.tree.exprs.len();
    let mut r = Resolver {
        structs,
        functions: functions
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i))
            .collect(),
        resolution: Resolution {
            bindings: Vec::new(),
            names: vec![None; count],
            calls: vec![None; count],
            stmts: vec![None; f.tree.stmts.len()],
            bounds: Vec::new(),
            patterns: HashMap::new(),
        },
        scopes: vec![Vec::new()],
        declared: HashMap::new(),
        duplicates: Vec::new(),
        clashing: HashSet::new(),
        reads: Vec::new(),
        writes: Vec::new(),
        loops: 0,
        errors: Vec::new(),
    };
    r.function(f);
    r.unused(&f.tree);
    r.rename_duplicates(&f.tree);
    (r.resolution, r.errors)
}

impl Resolver<'_> {
    /// What type of the module `name` names, if it names one: a struct, or a type whose values
    /// come later.
    fn type_kind(&self, name: &str) -> Option<&'static str> {
        if self.structs.contains(name) {
            Some("a struct")
        } else if self.structs.is_enum(name) {
            Some("an enum")
        } else {
            self.structs.later.get(name).map(|l| l.kind)
        }
    }
}

struct Resolver<'a> {
    structs: &'a Structs,
    functions: HashMap<String, usize>,
    resolution: Resolution,
    /// The bindings visible at each open block, innermost last.
    scopes: Vec<Vec<BindingId>>,
    /// Every name declared in the function, with its first declaration.
    declared: HashMap<String, BindingId>,
    /// A declaration that needs a new name, and the index of its diagnostic.
    duplicates: Vec<(BindingId, usize)>,
    /// The names declared twice, or named like a function: their uses tell nothing.
    clashing: HashSet<String>,
    /// For each binding, how often it is read and how often it is written.
    reads: Vec<u32>,
    writes: Vec<u32>,
    /// How many loops enclose the statement being read.
    loops: u32,
    errors: Vec<Diagnostic>,
}

impl Resolver<'_> {
    fn binding(&self, id: BindingId) -> &Binding {
        &self.resolution.bindings[id.0 as usize]
    }

    fn function(&mut self, f: &FnSyntax) {
        let tree = &f.tree;
        for p in &f.params {
            self.declare(
                &p.name,
                p.name_span,
                BindingKind::Param { is_mut: p.is_mut },
                Some(p.ty),
            );
        }
        for p in &f.params {
            self.ty(tree, p.ty);
        }
        if let Some(t) = f.result {
            self.ty(tree, t);
        }
        for c in &f.contracts {
            if c.kind == ContractKind::Ensures {
                // `result` is visible in an `ensures` clause only.
                let id = self.push_binding(
                    "result",
                    tree.expr(c.expr).span,
                    BindingKind::Result,
                    f.result,
                );
                self.scopes.push(vec![id]);
                self.expr(tree, c.expr);
                self.scopes.pop();
            } else {
                self.expr(tree, c.expr);
            }
        }
        match f.body {
            Some(Body::Block(b)) => self.block(tree, b),
            Some(Body::Expr(e)) => self.expr(tree, e),
            None => {}
        }
    }

    fn push_binding(
        &mut self,
        name: &str,
        span: Span,
        kind: BindingKind,
        ty: Option<TypeId>,
    ) -> BindingId {
        self.resolution.bindings.push(Binding {
            name: name.to_string(),
            span,
            kind,
            ty,
        });
        self.reads.push(0);
        self.writes.push(0);
        BindingId(self.resolution.bindings.len() as u32 - 1)
    }

    /// Declares a name in the innermost block. A name of a function or a struct of the module is
    /// `E0220`, and a second declaration in the function is `E0202`.
    fn declare(
        &mut self,
        name: &str,
        span: Span,
        kind: BindingKind,
        ty: Option<TypeId>,
    ) -> BindingId {
        let id = self.push_binding(name, span, kind, ty);
        let declared_as = if self.functions.contains_key(name) {
            Some("a function")
        } else {
            self.type_kind(name)
        };
        if let Some(what) = declared_as {
            let d = Diagnostic::error(
                "E0220",
                span,
                format!(
                    "`{}` is the name of {} of this module; a name means one thing, so give this binding another",
                    name, what
                ),
            );
            self.clashing.insert(name.to_string());
            self.duplicates.push((id, self.errors.len()));
            self.errors.push(d);
            self.declared.entry(name.to_string()).or_insert(id);
        } else if let Some(&first) = self.declared.get(name) {
            let at = self.binding(first).span;
            let d = Diagnostic::error(
                "E0202",
                span,
                format!(
                    "`{}` is already declared in this function; Mente has no shadowing, so give this one a new name",
                    name
                ),
            )
            .with_related(at, "first declared here");
            self.clashing.insert(name.to_string());
            self.duplicates.push((id, self.errors.len()));
            self.errors.push(d);
        } else {
            self.declared.insert(name.to_string(), id);
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(id);
        }
        id
    }

    /// The binding a name refers to here, the innermost one first.
    fn lookup(&self, name: &str) -> Option<BindingId> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|s| s.iter().rev())
            .copied()
            .find(|&id| self.binding(id).name == name)
    }

    /// Every name visible here, for suggestions.
    fn visible(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .scopes
            .iter()
            .flatten()
            .map(|&id| self.binding(id).name.clone())
            .collect();
        names.extend(self.functions.keys().cloned());
        names
    }

    /// `E0201`, with the closest visible name as a suggestion.
    fn unknown(&mut self, name: &str, span: Span, what: &str, candidates: Vec<String>) {
        let close = closest(name, &candidates);
        let message = match &close {
            Some(c) => format!(
                "there is no {} named `{}` here; did you mean `{}`?",
                what, name, c
            ),
            None => format!("there is no {} named `{}` here", what, name),
        };
        let mut d = Diagnostic::error("E0201", span, message);
        if let Some(c) = close {
            d = d.with_fix(Applicability::Suggestion, format!("write `{}`", c), span, c);
        }
        self.errors.push(d);
    }

    fn block(&mut self, tree: &Tree, b: BlockId) {
        self.scopes.push(Vec::new());
        for &s in &tree.block(b).stmts {
            let stmt = tree.stmt(s);
            match &stmt.kind {
                StmtKind::Binding {
                    is_var,
                    name,
                    name_span,
                    ty,
                    init,
                } => {
                    // The new name is not yet visible in its own initializer.
                    self.expr(tree, *init);
                    if let Some(t) = ty {
                        self.ty(tree, *t);
                    }
                    let kind = if *is_var {
                        BindingKind::Var
                    } else {
                        BindingKind::Val
                    };
                    let id = self.declare(name, *name_span, kind, *ty);
                    self.resolution.stmts[s.0 as usize] = Some(id);
                }
                StmtKind::Assign { place, value, .. } => {
                    self.expr(tree, *value);
                    self.place(tree, *place, false);
                }
                StmtKind::Discard(e) => self.expr(tree, *e),
                StmtKind::While {
                    cond,
                    invariants,
                    decreases,
                    body,
                } => {
                    self.expr(tree, *cond);
                    for &e in invariants.iter().chain(decreases) {
                        self.expr(tree, e);
                    }
                    self.loops += 1;
                    self.block(tree, *body);
                    self.loops -= 1;
                }
                StmtKind::Guard { cond, otherwise } => {
                    self.expr(tree, *cond);
                    self.block(tree, *otherwise);
                }
                StmtKind::Return(value) => {
                    if let Some(e) = value {
                        self.expr(tree, *e);
                    }
                }
                StmtKind::Break | StmtKind::Continue => {
                    if self.loops == 0 {
                        let word = if stmt.kind == StmtKind::Break {
                            "break"
                        } else {
                            "continue"
                        };
                        self.errors.push(Diagnostic::error(
                            "E0208",
                            stmt.span,
                            format!("`{}` stands only inside a `while` loop", word),
                        ));
                    }
                }
                StmtKind::Expr(e) => self.expr(tree, *e),
            }
        }
        self.scopes.pop();
    }

    fn expr(&mut self, tree: &Tree, e: ExprId) {
        let span = tree.expr(e).span;
        match &tree.expr(e).kind {
            ExprKind::Name(n) => {
                if let Some(id) = self.name(e, n, span) {
                    self.reads[id.0 as usize] += 1;
                }
            }
            ExprKind::Call { callee, args } => {
                if let Some(&index) = self.functions.get(callee) {
                    self.resolution.calls[e.0 as usize] = Some(index);
                } else if self.structs.contains(callee) {
                    // A struct built with positional arguments, which the type checker reports.
                } else if self.structs.is_enum(callee) {
                    self.enum_called(callee, span);
                } else if let Some(later) = self.structs.later.get(callee) {
                    let name_span = Span {
                        end: span.start + callee.len(),
                        ..span
                    };
                    self.errors.push(later.diagnostic(callee, name_span));
                } else if self.lookup(callee).is_some() {
                    self.errors.push(Diagnostic::error(
                        "E0204",
                        span,
                        format!(
                            "`{}` is a value, not a function, so it cannot be called",
                            callee
                        ),
                    ));
                } else {
                    let candidates: Vec<String> = self.functions.keys().cloned().collect();
                    let name_span = Span {
                        end: span.start + callee.len(),
                        ..span
                    };
                    self.unknown(callee, name_span, "function", candidates);
                }
                for a in args {
                    if a.is_mut {
                        self.place(tree, a.value, true);
                    } else {
                        self.expr(tree, a.value);
                    }
                }
            }
            ExprKind::Construct { name, fields } => {
                if let Some(&index) = self.functions.get(name) {
                    // A function called with named arguments, which the type checker reports.
                    self.resolution.calls[e.0 as usize] = Some(index);
                } else if let Some(later) = self.structs.later.get(name) {
                    let name_span = Span {
                        end: span.start + name.len(),
                        ..span
                    };
                    self.errors.push(later.diagnostic(name, name_span));
                } else if self.structs.is_enum(name) {
                    self.enum_called(name, span);
                } else if !self.structs.contains(name) {
                    let mut candidates = self.structs.names();
                    candidates.sort();
                    candidates.dedup();
                    let name_span = Span {
                        end: span.start + name.len(),
                        ..span
                    };
                    self.unknown(name, name_span, "struct", candidates);
                }
                for f in fields {
                    self.expr(tree, f.value);
                }
            }
            ExprKind::Variant {
                enum_name,
                enum_span,
                name,
                name_span,
                payload,
            } => {
                self.variant(enum_name, *enum_span, name, *name_span, payload);
                match payload {
                    Payload::None => {}
                    Payload::Named(fields) => {
                        for f in fields {
                            self.expr(tree, f.value);
                        }
                    }
                    Payload::Positional(args) => {
                        for a in args {
                            if a.is_mut {
                                self.place(tree, a.value, true);
                            } else {
                                self.expr(tree, a.value);
                            }
                        }
                    }
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.expr(tree, *scrutinee);
                for (k, arm) in arms.iter().enumerate() {
                    // The names an arm's pattern binds are visible in that arm only.
                    self.scopes.push(Vec::new());
                    self.pattern(e, k, &arm.pattern);
                    match arm.body {
                        ArmBody::Expr(x) => self.expr(tree, x),
                        ArmBody::Block(b) => self.block(tree, b),
                    }
                    self.scopes.pop();
                }
            }
            ExprKind::Unary { operand, .. } => self.expr(tree, *operand),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(tree, *lhs);
                self.expr(tree, *rhs);
            }
            ExprKind::Index { base, index } => {
                self.expr(tree, *base);
                self.expr(tree, *index);
            }
            ExprKind::Field { base, .. } => self.expr(tree, *base),
            ExprKind::Array(items) => {
                for &i in items {
                    self.expr(tree, i);
                }
            }
            ExprKind::Repeat { value, .. } => self.expr(tree, *value),
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.expr(tree, *cond);
                self.block(tree, *then);
                match otherwise {
                    Some(Else::Block(b)) => self.block(tree, *b),
                    Some(Else::If(i)) => self.expr(tree, *i),
                    None => {}
                }
            }
            ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Error => {}
        }
    }

    /// `E0204` for an enum called or built as a struct is, at `span`: a value of an enum is one
    /// of its variants, named with it.
    fn enum_called(&mut self, name: &str, span: Span) {
        let first = self
            .structs
            .get_enum(name)
            .and_then(|d| d.variants.first())
            .map_or("A", |v| v.name.as_str());
        self.errors.push(Diagnostic::error(
            "E0204",
            span,
            format!(
                "`{}` is an enum, so a value of it is one of its variants, named with it, as in `{}.{}`",
                name, name, first
            ),
        ));
    }

    /// `a.b(...)` or `a.b`, `a` at `at`: a variant when `a` is an enum of the module, which the
    /// type checker reads further; a call on a receiver, a construct a later milestone brings,
    /// when `a` is a binding or a type of the language (`E0109`); else an unknown name
    /// (`E0201`), with the closest enum or binding suggested.
    fn variant(
        &mut self,
        enum_name: &str,
        at: Span,
        name: &str,
        name_span: Span,
        payload: &Payload,
    ) {
        if self.structs.is_enum(enum_name) {
            return;
        }
        let call = match payload {
            Payload::None => false,
            Payload::Named(_) | Payload::Positional(_) => true,
        };
        // A call on a type of the language, such as `List.new()`, is the parser's to report.
        let receiver = self.lookup(enum_name).is_some()
            || self.structs.is_type(enum_name)
            || self.structs.later.contains_key(enum_name);
        if receiver && call {
            let what = format!("a method call (`.{}(...)`)", name);
            let milestone = mente_syntax::body::later_type(enum_name).map_or("M2", |(m, _)| m);
            self.errors.push(Diagnostic::error(
                "E0109",
                name_span,
                mente_syntax::diagnostics::later_message(&what, Some(milestone)),
            ));
            return;
        }
        let mut candidates = self.visible();
        candidates.extend(self.structs.enums.iter().map(|e| e.name.clone()));
        self.unknown(enum_name, at, "enum or binding", candidates);
    }

    /// The names a pattern of arm `k` of the `match` at `e` binds, each declared in the arm's
    /// scope as a read-only projection; an enum the pattern names that is none of the module's
    /// is `E0201`.
    fn pattern(&mut self, e: ExprId, k: usize, pattern: &Pattern) {
        let PatternKind::Variant {
            enum_name,
            bindings,
            ..
        } = &pattern.kind
        else {
            return;
        };
        if !self.structs.is_enum(enum_name) {
            let span = Span {
                end: pattern.span.start + enum_name.len(),
                ..pattern.span
            };
            let candidates: Vec<String> =
                self.structs.enums.iter().map(|e| e.name.clone()).collect();
            self.unknown(enum_name, span, "enum", candidates);
        }
        for (i, b) in bindings.iter().flatten().enumerate() {
            let Some(name) = &b.name else {
                continue;
            };
            let id = self.declare(name, b.name_span, BindingKind::Pattern, None);
            self.resolution.patterns.insert((e, k, i), id);
        }
    }

    /// The binding `n`, at expression `e`, refers to: `E0204` when it names a function, and
    /// `E0201` when it names nothing.
    fn name(&mut self, e: ExprId, n: &str, span: Span) -> Option<BindingId> {
        if let Some(id) = self.lookup(n) {
            self.resolution.names[e.0 as usize] = Some(id);
            return Some(id);
        }
        if self.functions.contains_key(n) {
            self.errors.push(Diagnostic::error(
                "E0204",
                span,
                format!(
                    "`{}` is a function, which {} only calls: write `{}(...)`",
                    n,
                    mente_syntax::diagnostics::MILESTONE,
                    n
                ),
            ));
        } else if self.structs.contains(n) {
            self.errors.push(Diagnostic::error(
                "E0204",
                span,
                format!(
                    "`{}` is a struct, not a value: build one with its fields, as in `{}(field: value)`",
                    n, n
                ),
            ));
        } else if let Some(def) = self.structs.get_enum(n) {
            let first = def.variants.first().map_or("A", |v| v.name.as_str());
            self.errors.push(Diagnostic::error(
                "E0204",
                span,
                format!(
                    "`{}` is an enum, not a value: a value of it is one of its variants, named with it, as in `{}.{}`",
                    n, n, first
                ),
            ));
        } else if let Some(later) = self.structs.later.get(n) {
            self.errors.push(Diagnostic::error(
                "E0204",
                span,
                format!(
                    "`{}` is {} of this module, a type, not a value; {} comes in {}",
                    n, later.kind, later.what, later.milestone
                ),
            ));
        } else {
            let candidates = self.visible();
            self.unknown(n, span, "binding", candidates);
        }
        None
    }

    /// A place a statement assigns or a call changes: its root binding is written, and read
    /// too when the call copies it in.
    fn place(&mut self, tree: &Tree, e: ExprId, read: bool) {
        match &tree.expr(e).kind {
            ExprKind::Name(n) => {
                if let Some(id) = self.name(e, n, tree.expr(e).span) {
                    self.writes[id.0 as usize] += 1;
                    self.reads[id.0 as usize] += u32::from(read);
                }
            }
            ExprKind::Index { base, index } => {
                self.place(tree, *base, read);
                self.expr(tree, *index);
            }
            ExprKind::Field { base, .. } => self.place(tree, *base, read),
            _ => self.expr(tree, e),
        }
    }

    /// A `val` or `var` that is never read is `E0218`, and a `var` that is read but never
    /// assigned is `E0219`. Parameters and `result` are exempt: a signature is an interface.
    fn unused(&mut self, tree: &Tree) {
        for (s, declared) in self.resolution.stmts.iter().enumerate() {
            let Some(id) = *declared else {
                continue;
            };
            let b = self.binding(id);
            if self.clashing.contains(&b.name) {
                continue;
            }
            let stmt = tree.stmts[s].span;
            let d = if self.reads[id.0 as usize] == 0 {
                Diagnostic::error(
                    "E0218",
                    b.span,
                    format!(
                        "`{}` is never read; use it where it was meant to be used, or remove it",
                        b.name
                    ),
                )
                .with_fix(
                    Applicability::Suggestion,
                    format!("remove `{}`", b.name),
                    stmt,
                    "",
                )
            } else if b.kind == BindingKind::Var && self.writes[id.0 as usize] == 0 {
                Diagnostic::error(
                    "E0219",
                    b.span,
                    format!(
                        "`{}` is never assigned, so it is a `val`; a `var` says the value changes",
                        b.name
                    ),
                )
                .with_fix(
                    Applicability::MachineApplicable,
                    "declare it with `val`",
                    Span {
                        end: stmt.start + 3,
                        ..stmt
                    },
                    "val",
                )
            } else {
                continue;
            };
            self.errors.push(d);
        }
        // A pattern's binding nobody reads is `_`.
        let mut bound: Vec<BindingId> = self.resolution.patterns.values().copied().collect();
        bound.sort_by_key(|b| b.0);
        for id in bound {
            let b = self.binding(id);
            if self.reads[id.0 as usize] > 0 || self.clashing.contains(&b.name) {
                continue;
            }
            let d = Diagnostic::error(
                "E0218",
                b.span,
                format!(
                    "`{}` is never read; a field the arm does not need is bound to `_`",
                    b.name
                ),
            )
            .with_fix(
                Applicability::MachineApplicable,
                format!("bind `{}`'s field to `_`", b.name),
                b.span,
                "_",
            );
            self.errors.push(d);
        }
    }

    /// A refinement's bound names a binding that cannot change, or `name.len` (`E0209`).
    fn ty(&mut self, tree: &Tree, t: TypeId) {
        let span = tree.ty(t).span;
        match &tree.ty(t).kind {
            TypeKind::Int {
                refinement: Some(r),
                ..
            } => {
                for bound in [&r.lo, &r.hi] {
                    let (name, needs_fixed) = match bound {
                        Bound::Name(n) => (n, true),
                        Bound::Len(n) => (n, false),
                        Bound::Int { .. } => continue,
                    };
                    match self.lookup(name) {
                        None => {
                            let candidates = self.visible();
                            self.unknown(name, span, "binding", candidates);
                        }
                        Some(id) => {
                            self.reads[id.0 as usize] += 1;
                            self.resolution.bounds.push(NamedBound {
                                ty: t,
                                len: !needs_fixed,
                                binding: id,
                            });
                            let kind = self.binding(id).kind;
                            let changes = matches!(
                                kind,
                                BindingKind::Var | BindingKind::Param { is_mut: true }
                            );
                            if needs_fixed && changes {
                                self.errors.push(Diagnostic::error(
                                    "E0209",
                                    span,
                                    format!(
                                        "a refinement's bound names `{}`, which can change; a bound is a literal, a `val`, a read-only parameter, or `name.len`",
                                        name
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
            TypeKind::Array { elem, .. } => self.ty(tree, *elem),
            TypeKind::Named(n) | TypeKind::Enum(n) => {
                if let Some(d) = unknown_type(self.structs, n, span) {
                    self.errors.push(d);
                }
            }
            _ => {}
        }
    }

    /// Gives each `E0202` a fix that renames the second declaration and its uses.
    fn rename_duplicates(&mut self, tree: &Tree) {
        for (id, at) in std::mem::take(&mut self.duplicates) {
            let b = self.binding(id).clone();
            let fresh = (2..)
                .map(|n| format!("{}{}", b.name, n))
                .find(|n| !self.declared.contains_key(n) && !self.functions.contains_key(n))
                .unwrap_or_default();
            let mut edits = vec![Edit {
                span: b.span,
                replacement: fresh.clone(),
            }];
            for (i, name) in self.resolution.names.iter().enumerate() {
                if *name == Some(id) {
                    edits.push(Edit {
                        span: tree.exprs[i].span,
                        replacement: fresh.clone(),
                    });
                }
            }
            self.errors[at].fixes.push(Fix {
                message: format!("rename this `{}` and its uses to `{}`", b.name, fresh),
                applicability: Applicability::Suggestion,
                edits,
            });
        }
    }
}

/// The candidate closest to `name`, within two edits and not more than a third of its length.
pub(crate) fn closest(name: &str, candidates: &[String]) -> Option<String> {
    let limit = (name.len() / 3).clamp(1, 2);
    candidates
        .iter()
        .filter(|c| c.as_str() != name)
        .map(|c| (distance(name, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)))
        .map(|(_, c)| c.clone())
}

/// The edit distance between two words: insertions, deletions, substitutions and swaps of two
/// neighbouring characters, the commonest typing slips.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use mente_syntax::diagnostics::Diagnostic;

    use super::*;
    use crate::check_source;

    const NONE: [&str; 0] = [];

    fn module(src: &str) -> String {
        format!("module m\npub fn id(x: u8) -> u8 = x\n{}\n", src)
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

    #[test]
    fn test_an_unknown_name_suggests_the_closest_visible_one() {
        let d = diagnostics("pub fn f(count: u8) = cuont");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0201");
        assert_eq!(
            d[0].message,
            "there is no binding named `cuont` here; did you mean `count`?"
        );
        assert_eq!(d[0].fixes[0].message, "write `count`");
        assert_eq!(d[0].fixes[0].edits[0].replacement, "count");
        assert_eq!(d[0].fixes[0].edits[0].span, d[0].primary);
        assert_eq!(d[0].fixes[0].applicability, Applicability::Suggestion);
        let far = diagnostics("pub fn f(a: u8) = zebra");
        assert_eq!(far[0].message, "there is no binding named `zebra` here");
        assert!(far[0].fixes.is_empty());
        let call = diagnostics("pub fn f() = ix(1)");
        assert_eq!(
            call[0].message,
            "there is no function named `ix` here; did you mean `id`?"
        );
        assert_eq!(call[0].primary.end - call[0].primary.start, 2);
        assert_eq!(codes("pub fn f() = idd(1)"), ["E0201"]);
        assert_eq!(
            messages("pub fn f(id2: u8) = idd")[0],
            "there is no binding named `idd` here; did you mean `id`?"
        );
    }

    #[test]
    fn test_a_name_is_visible_from_its_declaration_to_the_end_of_its_block() {
        assert_eq!(codes("pub fn f() {\n  val x = x\n}"), ["E0201"]);
        assert_eq!(
            codes("pub fn f(c: bool) -> u8 {\n  if c {\n    val y = 1\n  }\n  y\n}"),
            ["E0201"]
        );
        assert_eq!(
            codes("pub fn f(c: bool) {\n  while c {\n    val y = 1\n  }\n  guard c else {\n    val z = 1\n    return\n  }\n  val w = y + z\n}"),
            ["E0201", "E0201"]
        );
        assert_eq!(
            codes("pub fn f(c: bool) -> u8 {\n  val x = 1\n  if c {\n    val y = x\n    return y\n  }\n  x\n}"),
            NONE
        );
        assert_eq!(codes("pub fn f() -> u8 {\n  result\n}"), ["E0201"]);
        assert_eq!(
            codes("pub fn f() -> u8\n  requires result > 0\n= 1"),
            ["E0201"]
        );
        assert_eq!(
            codes("pub fn f() -> u8\n  ensures result > 0\n= result"),
            ["E0201"]
        );
        assert_eq!(codes("pub fn f() -> u8\n  ensures result > 0\n= 1"), NONE);
        assert_eq!(codes("pub fn f(n: u8)\n  decreases n\n{\n}"), NONE);
    }

    #[test]
    fn test_a_name_is_declared_once_in_a_function() {
        let d = diagnostics("pub fn f(x: u8) {\n  val x = 1\n  val y = x + x\n}");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0202");
        assert_eq!(
            d[0].message,
            "`x` is already declared in this function; Mente has no shadowing, so give this one a new name"
        );
        assert_eq!(d[0].primary.line, 4);
        assert_eq!(d[0].related[0].span.line, 3);
        assert_eq!(d[0].related[0].message, "first declared here");
        let fix = &d[0].fixes[0];
        assert_eq!(fix.message, "rename this `x` and its uses to `x2`");
        assert_eq!(fix.edits.len(), 3);
        assert!(fix.edits.iter().all(|e| e.replacement == "x2"));
        assert_eq!(fix.edits[0].span, d[0].primary);
        assert_eq!(fix.edits[1].span.line, 5);
        let taken = diagnostics("pub fn f(x: u8, x2: u8) {\n  val x = 1\n}");
        assert_eq!(taken[0].fixes[0].edits[0].replacement, "x3");
        assert_eq!(
            codes("pub fn f(c: bool) {\n  val x = 1\n  if c {\n    val x = 2\n  }\n}"),
            ["E0202"]
        );
        assert_eq!(
            codes(
                "pub fn f(c: bool) {\n  if c {\n    val y = 1\n  } else {\n    val y = 2\n  }\n}"
            ),
            ["E0202"]
        );
        assert_eq!(codes("pub fn f(a: u8, a: u8) {\n}"), ["E0202"]);
        assert_eq!(
            codes("pub fn f() {\n  val a = 1\n  val a = 2\n  val a = 3\n}"),
            ["E0202", "E0202"]
        );
    }

    #[test]
    fn test_functions_are_called_and_values_are_not() {
        assert_eq!(codes("pub fn f() = id"), ["E0204"]);
        assert_eq!(
            messages("pub fn f() = id"),
            ["`id` is a function, which M2 only calls: write `id(...)`"]
        );
        assert_eq!(codes("pub fn f(x: u8) = x(1)"), ["E0204"]);
        assert_eq!(
            messages("pub fn f(x: u8) = x(1)"),
            ["`x` is a value, not a function, so it cannot be called"]
        );
        assert_eq!(codes("pub fn f(x: u8) = x(zz)"), ["E0204", "E0201"]);
        assert_eq!(codes("pub fn f(id: u8) -> u8 = id"), ["E0220"]);
    }

    #[test]
    fn test_break_and_continue_stand_in_a_loop() {
        assert_eq!(codes("pub fn f() {\n  break\n}"), ["E0208"]);
        assert_eq!(
            messages("pub fn f() {\n  break\n}"),
            ["`break` stands only inside a `while` loop"]
        );
        assert_eq!(
            messages("pub fn f() {\n  continue\n}"),
            ["`continue` stands only inside a `while` loop"]
        );
        assert_eq!(
            codes("pub fn f(c: bool) {\n  while c {\n  }\n  break\n}"),
            ["E0208"]
        );
        assert_eq!(
            codes("pub fn f(c: bool) {\n  while c {\n    while c {\n      break\n    }\n    if c {\n      continue\n    }\n    break\n  }\n}"),
            NONE
        );
    }

    #[test]
    fn test_a_refinement_bound_names_a_binding_that_cannot_change() {
        assert_eq!(
            codes("pub fn f(n: u8) {\n  var m: u8 = n\n  m += 1\n  val i: u8[0..m] = 0\n}"),
            ["E0209"]
        );
        assert_eq!(
            messages("pub fn f(n: u8) {\n  var m: u8 = n\n  m += 1\n  val i: u8[0..m] = 0\n}"),
            ["a refinement's bound names `m`, which can change; a bound is a literal, a `val`, a read-only parameter, or `name.len`"]
        );
        assert_eq!(codes("pub fn f(mut n: u8, i: u8[n..=9]) {\n}"), ["E0209"]);
        assert_eq!(codes("pub fn f(mut n: u8) -> u8[0..=n] = 0"), ["E0209"]);
        assert_eq!(
            codes("pub fn f(n: u8) {\n  var m: u8 = n\n  m += 1\n  val a: [2]u8[0..m] = [0, 0]\n}"),
            ["E0209"]
        );
        assert_eq!(codes("pub fn f(i: u8[0..k]) {\n}"), ["E0201"]);
        assert_eq!(codes("pub fn f(i: u8[k..=2]) {\n}"), ["E0201"]);
        assert_eq!(
            codes("pub fn f(n: u8, i: u8[0..n]) -> u8[0..=n] {\n  val k = n\n  var xs = [1, 2]\n  xs[0] = 3\n  val j: usize[0..xs.len] = 0\n  var l: u8[k..=n] = n\n  val m: i8[-4..4] = 0\n  0\n}"),
            NONE
        );
    }

    #[test]
    fn test_the_resolution_binds_names_and_calls_by_index() {
        let src = "module m\npub fn id(x: u8) -> u8 = x\npub fn g(y: u8) = id(y)\n";
        let Ok(module) = mente_syntax::parse_module(src) else {
            panic!("the module parses");
        };
        let parsed = mente_syntax::parse_bodies(src, &module);
        let names: Vec<String> = parsed.iter().map(|p| p.name.clone()).collect();
        let Some(g) = parsed[1].syntax.as_ref() else {
            panic!("g has a body");
        };
        let (r, errors) = resolve(g, &names, &Structs::default());
        assert_eq!(errors, Vec::new());
        assert_eq!(r.bindings.len(), 1);
        assert_eq!(r.bindings[0].name, "y");
        assert_eq!(r.bindings[0].kind, BindingKind::Param { is_mut: false });
        assert_eq!(r.names, [Some(BindingId(0)), None]);
        assert_eq!(r.calls, [None, Some(0)]);
    }

    #[test]
    fn test_suggestions_lie_within_a_third_of_the_name() {
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "ab"), 2);
        assert_eq!(distance("ab", ""), 2);
        assert_eq!(distance("abc", "abc"), 0);
        assert_eq!(distance("abc", "acb"), 1);
        assert_eq!(distance("ba", "ab"), 1);
        assert_eq!(distance("abcd", "badc"), 2);
        assert_eq!(distance("ab", "ba c"), 3);
        assert_eq!(distance("a", "b"), 1);
        let list = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(closest("ab", &list(&["ac", "aa"])), Some("aa".to_string()));
        assert_eq!(
            closest("abcdef", &list(&["abcdeg", "aacdeg"])),
            Some("abcdeg".to_string())
        );
        assert_eq!(closest("ab", &list(&["ab"])), None);
        assert_eq!(closest("abc", &list(&["abd"])), Some("abd".to_string()));
        assert_eq!(closest("abc", &list(&["axy"])), None);
        assert_eq!(
            closest("abcdef", &list(&["abcdxy"])),
            Some("abcdxy".to_string())
        );
        assert_eq!(closest("abcdefghi", &list(&["abcdefxyz"])), None);
        assert_eq!(
            closest("abcde", &list(&["abcxy", "abcdx"])),
            Some("abcdx".to_string())
        );
    }

    fn all_codes(src: &str) -> Vec<&'static str> {
        all_diagnostics(src).iter().map(|d| d.code).collect()
    }

    #[test]
    fn test_every_val_and_var_is_read() {
        let d = all_diagnostics("pub fn f(a: u8) -> u8 {\n  val clamped = a\n  a\n}");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0218");
        assert_eq!(
            d[0].message,
            "`clamped` is never read; use it where it was meant to be used, or remove it"
        );
        assert_eq!((d[0].primary.line, d[0].primary.col), (4, 7));
        let fix = &d[0].fixes[0];
        assert_eq!(fix.message, "remove `clamped`");
        assert_eq!(fix.applicability, Applicability::Suggestion);
        assert_eq!(fix.edits[0].replacement, "");
        assert_eq!((fix.edits[0].span.line, fix.edits[0].span.col), (4, 3));
        assert_eq!(fix.edits[0].span.end - fix.edits[0].span.start, 15);
        // Assigning a binding, or adding to it, does not read it.
        assert_eq!(
            all_codes("pub fn f() {\n  var x = 0\n  x = 1\n}"),
            ["E0218"]
        );
        assert_eq!(
            all_codes("pub fn f() {\n  var x = 0\n  x += 1\n}"),
            ["E0218"]
        );
        assert_eq!(
            all_codes("pub fn f() {\n  var xs = [0, 0]\n  xs[1] = 2\n}"),
            ["E0218"]
        );
        for ok in [
            "pub fn f(unused: u8) {\n}",
            "pub fn f(a: u8) -> u8 {\n  val b = a\n  b\n}",
            "pub fn f(i: usize) -> u8 {\n  var xs = [0, 0]\n  val j: usize = 1\n  xs[j] = 2\n  xs[i]\n}",
            "pub fn f() -> u8\n  ensures result > 0\n= 1",
        ] {
            assert_eq!(all_codes(ok), NONE, "{}", ok);
        }
        // A name declared twice reports that alone.
        assert_eq!(
            all_codes("pub fn f() -> u8 {\n  val a = 1\n  val a = 2\n  a\n}"),
            ["E0202"]
        );
    }

    #[test]
    fn test_every_var_is_assigned() {
        let d = all_diagnostics("pub fn f() -> u8 {\n  var total = 1\n  total\n}");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0219");
        assert_eq!(
            d[0].message,
            "`total` is never assigned, so it is a `val`; a `var` says the value changes"
        );
        let fix = &d[0].fixes[0];
        assert_eq!(fix.applicability, Applicability::MachineApplicable);
        assert_eq!(fix.message, "declare it with `val`");
        assert_eq!(fix.edits[0].replacement, "val");
        assert_eq!((fix.edits[0].span.line, fix.edits[0].span.col), (4, 3));
        assert_eq!(fix.edits[0].span.end - fix.edits[0].span.start, 3);
        let both = "pub fn bump(mut x: u8) {\n  x += 1\n}\n";
        for ok in [
            "pub fn f() -> u8 {\n  var total = 1\n  total += 1\n  total\n}",
            "pub fn f() -> u8 {\n  var xs = [1, 2]\n  xs[0] = 3\n  xs[1]\n}",
        ] {
            assert_eq!(all_codes(ok), NONE, "{}", ok);
        }
        assert_eq!(
            all_codes(&format!(
                "{}pub fn f() -> u8 {{\n  var v: u8 = 1\n  bump(mut v)\n  v\n}}",
                both
            )),
            NONE
        );
        assert_eq!(
            all_codes(&format!("{}pub fn f(i: usize) -> u8 {{\n  var xs: [2]u8 = [1, 2]\n  bump(mut xs[i])\n  xs[0]\n}}", both)),
            NONE
        );
    }

    /// A pattern's bindings are names of the function as every binding is: each declared once,
    /// none named like a type of its module, each read, or bound to `_`, which the fix writes;
    /// each visible in its arm only; and a name in a pattern is an enum of the module.
    #[test]
    fn test_pattern_bindings_are_named_once_and_read() {
        let with = |f: &str| format!("pub enum Shape {{\n  Dot\n  Rect(w: u8, h: u8)\n}}\n{}", f);
        let arm = |pattern: &str, body: &str| {
            with(&format!(
                "pub fn f(s: Shape, k: u8) -> u8 = match s {{\n  {} => {}\n  Shape.Dot => 0\n}}",
                pattern, body
            ))
        };
        assert_eq!(codes(&arm("Shape.Rect(w: a, h: b)", "a + b")), NONE);
        assert_eq!(codes(&arm("Shape.Rect(w: k, h: _)", "k")), ["E0202"]);
        assert_eq!(
            codes(&arm("Shape.Rect(w: Shape, h: _)", "Shape")),
            ["E0220"]
        );
        assert_eq!(codes(&arm("Shape.Rect(w: id, h: _)", "id")), ["E0220"]);
        // A binding is visible in its arm alone.
        assert_eq!(
            codes(&with(
                "pub fn f(s: Shape) -> u8 = match s {\n  Shape.Rect(w: a, h: _) => a\n  Shape.Dot => a\n}"
            )),
            ["E0201"]
        );
        // Two arms do not share a name, as two branches of an `if` do not.
        assert_eq!(
            codes(&with(
                "pub fn f(s: Shape, t: Shape) -> u8 = match s {\n  Shape.Rect(w: a, h: _) => a\n  Shape.Dot => match t {\n    Shape.Rect(w: a, h: _) => a\n    Shape.Dot => 0\n  }\n}"
            )),
            ["E0202"]
        );
        // An enum the pattern names is the module's; the error points at its name alone.
        let src = arm("Shpe.Rect(w: a, h: _)", "a");
        assert_eq!(
            messages(&src),
            ["there is no enum named `Shpe` here; did you mean `Shape`?"]
        );
        let text = module(&src);
        let d = &diagnostics(&src)[0];
        assert_eq!(&text[d.primary.start..d.primary.end], "Shpe");
        // A call on a struct's or a bitstruct's name, as on a receiver, comes later.
        for f in [
            "pub struct Pq {\n  a: u8\n}\npub fn f() -> u8 = Pq.make(1)",
            "pub bitstruct Bq : u8 {\n  @bits(0..8) pub raw: u8\n}\npub fn f() -> u8 = Bq.make(1)",
        ] {
            assert_eq!(codes(&with(f)), ["E0109"], "{}", f);
        }
        // A binding nobody reads is `_`, and the fix binds its field to `_`.
        let src = module(&arm("Shape.Rect(w: a, h: b)", "a"));
        let d: Vec<Diagnostic> = check_source(&src).err().unwrap_or_default();
        assert_eq!(d.iter().map(|d| d.code).collect::<Vec<_>>(), ["E0218"]);
        assert_eq!(
            d[0].message,
            "`b` is never read; a field the arm does not need is bound to `_`"
        );
        let fixed = mente_syntax::diagnostics::apply_fix(&src, &d[0].fixes[0]).unwrap();
        assert!(fixed.contains("Shape.Rect(w: a, h: _) => a"), "{}", fixed);
        assert_eq!(check_source(&fixed).err(), None);
        // An enum is no value, and is called or built as a struct is not.
        assert_eq!(
            codes(&with("pub fn f() -> u8 {\n  val s = Shape\n  0\n}")),
            ["E0204"]
        );
        assert_eq!(codes(&with("pub fn f() -> Shape = Shape(1)")), ["E0204"]);
        assert_eq!(codes(&with("pub fn f() -> Shape = Shape(w: 1)")), ["E0204"]);
    }

    /// A name means one thing in a module: no binding takes the name of a struct (`E0220`), a
    /// struct's name is no value (`E0204`), a struct built of a name that is none is `E0201`
    /// with the closest struct suggested, and a type that names nothing is `E0116`.
    #[test]
    fn test_no_binding_takes_the_name_of_a_type_of_its_module() {
        let pair = "pub struct Pair {
  i: u8
}
";
        let with = |f: &str| format!("{}{}", pair, f);
        assert_eq!(codes(&with("pub fn f(Pair: u8) -> u8 = Pair")), ["E0220"]);
        assert_eq!(
            messages(&with("pub fn f(a: u8) -> u8 {
  val Pair = a
  Pair
}")),
            ["`Pair` is the name of a struct of this module; a name means one thing, so give this binding another"]
        );
        assert_eq!(
            codes(&with(
                "pub fn f() -> u8 {
  val p = Pair
  0
}"
            )),
            ["E0204"]
        );
        assert_eq!(codes(&with("pub fn f() -> Pair = Piar(i: 0)")), ["E0201"]);
        assert_eq!(
            messages(&with("pub fn f() -> Pair = Piar(i: 0)")),
            ["there is no struct named `Piar` here; did you mean `Pair`?"]
        );
        // The error lies at the name alone.
        let src = module(&with("pub fn f() -> Pair = Piar(i: 0)"));
        let d = check_source(&src).err().unwrap_or_default();
        let d = d.iter().find(|d| d.code == "E0201").unwrap();
        let at = src.rfind("Piar").unwrap();
        assert_eq!((d.primary.start, d.primary.end), (at, at + 4));
        assert_eq!(codes(&with("pub fn f(p: Piar) -> u8 = 0")), ["E0116"]);
        assert_eq!(
            codes(&with(
                "pub fn f() -> u8 {
  val p: [2]Pear = todo(\"x\")
  0
}"
            )),
            ["E0116"]
        );
        assert_eq!(codes(&with("pub fn f(p: Pair) -> u8 = p.i")), NONE);
        // Every type the module declares counts, those whose values come later too: a
        // `@layout` struct, a bitstruct and an enum.
        let later = "@layout(size: 8, align: 8)
pub struct Stamp {
  @bytes(0..8) pub nanos: u64
}
pub bitstruct Tag : u8 {
  @bits(0..8) pub raw: u8
}
pub enum Mode {
  On
  Off
}
";
        let with = |f: &str| format!("{}{}", later, f);
        assert_eq!(
            messages(&with("pub fn f(a: u8) -> u8 {
  val Stamp = a
  Stamp
}")),
            ["`Stamp` is the name of a struct of this module; a name means one thing, so give this binding another"]
        );
        assert_eq!(
            messages(&with("pub fn f(Tag: u8) -> u8 = Tag")),
            ["`Tag` is the name of a bitstruct of this module; a name means one thing, so give this binding another"]
        );
        assert_eq!(
            messages(&with("pub fn f(a: u8) -> u8 {
  var Mode = a
  Mode = 1
  Mode
}")),
            ["`Mode` is the name of an enum of this module; a name means one thing, so give this binding another"]
        );
        // Building one, by name or by position, is a construct of its milestone, and its name is
        // no value.
        for build in ["Stamp(nanos: 1)", "Stamp(1)"] {
            assert_eq!(
                messages(&with(&format!(
                    "pub fn f() -> u8 {{
  _ = {}
  0
}}",
                    build
                ))),
                ["a `@layout` struct as a value (`Stamp`) comes in M3; M2 does not have it"],
                "{}",
                build
            );
        }
        assert_eq!(
            messages(&with(
                "pub fn f() -> u8 {
  _ = Tag(raw: 1)
  0
}"
            )),
            ["a bitstruct as a value (`Tag`) comes in M3; M2 does not have it"]
        );
        assert_eq!(
            messages(&with(
                "pub fn f() -> u8 {
  val m = Mode
  0
}"
            )),
            ["`Mode` is an enum, not a value: a value of it is one of its variants, named with it, as in `Mode.On`"]
        );
    }

    #[test]
    fn test_no_binding_takes_the_name_of_a_function() {
        let d = all_diagnostics("pub fn f(a: u8) -> u8 {\n  val id = a\n  id + 1\n}");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, "E0220");
        assert_eq!(
            d[0].message,
            "`id` is the name of a function of this module; a name means one thing, so give this binding another"
        );
        let fix = &d[0].fixes[0];
        assert_eq!(fix.message, "rename this `id` and its uses to `id2`");
        assert_eq!(fix.edits.len(), 2);
        assert_eq!(all_codes("pub fn f(f: u8) -> u8 = f"), ["E0220"]);
        assert_eq!(
            all_diagnostics("pub fn id2() {\n}\npub fn f(id: u8) -> u8 = id")[0].fixes[0].edits[0]
                .replacement,
            "id3"
        );
        assert_eq!(
            all_codes("pub fn f(a: u8) -> u8 {\n  val id = a\n  val id = 1\n  id(id)\n}"),
            ["E0220", "E0220"]
        );
    }
}
