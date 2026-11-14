//! What of the grammar the program generator reaches (`docs/design-review.md`,
//! `quality-per-change`: new syntax extends the generator in the same pull request): M1's, and
//! M2's structs, enums and `match`.
//!
//! Each table below names the constructs of one part of a function's syntax tree, and its
//! patterns name every variant and every field, without a wildcard. A construct added to the
//! grammar therefore does not compile until a table names it, and the table's list of names,
//! which the same macro writes, makes the test require that generated programs reach it, or
//! that [`NOT_GENERATED`] give the reason they cannot.

use std::collections::{BTreeMap, BTreeSet};

use mente_sema::check;
use mente_sema::types::INT_TYPES;
use mente_syntax::tree::{
    Arg, ArmBody, AssignOp, BinaryOp, Body, Bound, Contract, ContractKind, Else, EnumSyntax,
    ExprId, ExprKind, FieldSyntax, FnSyntax, Param, PatBinding, PatternKind, Payload, Refinement,
    StmtKind, StructSyntax, Tree, TypeId, TypeKind, UnaryOp,
};

use super::generate;

/// Defines `$name`, the construct a value of `$t` is, and `$all`, every name it gives, from one
/// table.
macro_rules! constructs {
    ($name:ident, $all:ident: $t:ty { $($p:pat => $n:literal,)+ }) => {
        const $all: &[&str] = &[$($n),+];
        fn $name(x: &$t) -> &'static str {
            match x {
                $($p => $n,)+
            }
        }
    };
}

constructs!(expr_kind, EXPRS: ExprKind {
    ExprKind::Int { value: _, suffix: None } => "integer literal",
    ExprKind::Int { value: _, suffix: Some(_) } => "suffixed integer literal",
    ExprKind::Bool(true) => "true",
    ExprKind::Bool(false) => "false",
    ExprKind::Name(_) => "name",
    ExprKind::Todo(_) => "todo",
    ExprKind::Unreachable(_) => "unreachable",
    ExprKind::Unary { op: _, operand: _ } => "unary operation",
    ExprKind::Binary { op: _, lhs: _, rhs: _ } => "binary operation",
    ExprKind::Index { base: _, index: _ } => "index",
    ExprKind::Field { base: _, name: _ } => "field read",
    ExprKind::Call { callee: _, args: _ } => "call",
    ExprKind::Construct { name: _, fields: _ } => "struct construction",
    ExprKind::Variant { enum_name: _, enum_span: _, name: _, name_span: _, payload: Payload::None } => "variant",
    ExprKind::Variant { enum_name: _, enum_span: _, name: _, name_span: _, payload: Payload::Named(_) } => "variant with a payload",
    ExprKind::Variant { enum_name: _, enum_span: _, name: _, name_span: _, payload: Payload::Positional(_) } => "dotted call",
    ExprKind::Match { scrutinee: _, arms: _ } => "match",
    ExprKind::Array(_) => "array literal",
    ExprKind::Repeat { value: _, count: _ } => "repeated array",
    ExprKind::If { cond: _, then: _, otherwise: None } => "if without else",
    ExprKind::If { cond: _, then: _, otherwise: Some(Else::Block(_)) } => "if with else",
    ExprKind::If { cond: _, then: _, otherwise: Some(Else::If(_)) } => "else if",
    ExprKind::Error => "expression error",
});

constructs!(unary_kind, UNARIES: UnaryOp {
    UnaryOp::Neg => "-x",
    UnaryOp::BitNot => "~x",
    UnaryOp::Not => "not x",
});

constructs!(binary_kind, BINARIES: BinaryOp {
    BinaryOp::Or => "or",
    BinaryOp::And => "and",
    BinaryOp::Eq => "==",
    BinaryOp::Ne => "!=",
    BinaryOp::Lt => "<",
    BinaryOp::Le => "<=",
    BinaryOp::Gt => ">",
    BinaryOp::Ge => ">=",
    BinaryOp::BitOr => "|",
    BinaryOp::BitXor => "^",
    BinaryOp::BitAnd => "&",
    BinaryOp::Shl => "<<",
    BinaryOp::Shr => ">>",
    BinaryOp::Add => "+",
    BinaryOp::Sub => "-",
    BinaryOp::Mul => "*",
    BinaryOp::Div => "/",
    BinaryOp::Rem => "%",
});

constructs!(arg_kind, ARGS: Arg {
    Arg { is_mut: false, value: _ } => "argument",
    Arg { is_mut: true, value: _ } => "mut argument",
});

constructs!(stmt_kind, STMTS: StmtKind {
    StmtKind::Binding { is_var: false, name: _, name_span: _, ty: None, init: _ } => "val",
    StmtKind::Binding { is_var: false, name: _, name_span: _, ty: Some(_), init: _ } => "val with a type",
    StmtKind::Binding { is_var: true, name: _, name_span: _, ty: None, init: _ } => "var",
    StmtKind::Binding { is_var: true, name: _, name_span: _, ty: Some(_), init: _ } => "var with a type",
    StmtKind::Assign { place: _, op: _, value: _ } => "assignment",
    StmtKind::While { cond: _, invariants: _, decreases: _, body: _ } => "while",
    StmtKind::Guard { cond: _, otherwise: _ } => "guard",
    StmtKind::Return(None) => "return",
    StmtKind::Return(Some(_)) => "return with a value",
    StmtKind::Break => "break",
    StmtKind::Continue => "continue",
    StmtKind::Discard(_) => "discard",
    StmtKind::Expr(_) => "expression statement",
});

constructs!(assign_kind, ASSIGNS: AssignOp {
    AssignOp::Set => "=",
    AssignOp::Compound(_) => "compound assignment",
});

constructs!(invariant_kind, INVARIANTS: bool {
    false => "loop without an invariant",
    true => "loop invariant",
});

constructs!(loop_decreases_kind, LOOP_DECREASES: bool {
    false => "loop without decreases",
    true => "loop decreases",
});

constructs!(type_kind, TYPES: TypeKind {
    TypeKind::Int { name: _, refinement: None } => "integer type",
    TypeKind::Int { name: _, refinement: Some(_) } => "refined integer type",
    TypeKind::Bool => "bool",
    TypeKind::Array { len: _, elem: _ } => "array type",
    TypeKind::Named(_) => "struct type",
    TypeKind::Enum(_) => "enum type",
    TypeKind::Error => "type error",
});

constructs!(pattern_kind, PATTERNS: PatternKind {
    PatternKind::Variant { enum_name: _, name: _, name_span: _, bindings: None } => "variant pattern",
    PatternKind::Variant { enum_name: _, name: _, name_span: _, bindings: Some(_) } => "variant pattern with bindings",
    PatternKind::Int { negative: false, value: _, suffix: None } => "literal pattern",
    PatternKind::Int { negative: false, value: _, suffix: Some(_) } => "suffixed literal pattern",
    PatternKind::Int { negative: true, value: _, suffix: _ } => "negative literal pattern",
    PatternKind::Bool(true) => "true pattern",
    PatternKind::Bool(false) => "false pattern",
    PatternKind::Wildcard => "_ pattern",
});

constructs!(binding_kind, BINDINGS: PatBinding {
    PatBinding { field: Some(_), name: Some(_), name_span: _ } => "pattern binding",
    PatBinding { field: Some(_), name: None, name_span: _ } => "field bound to _",
    PatBinding { field: None, name: _, name_span: _ } => "binding by position",
});

constructs!(arm_kind, ARMS: ArmBody {
    ArmBody::Expr(_) => "expression arm",
    ArmBody::Block(_) => "block arm",
});

constructs!(variant_kind, VARIANTS: bool {
    false => "variant declared without a payload",
    true => "variant declared with a payload",
});

constructs!(range_kind, RANGES: Refinement {
    Refinement { lo: _, hi: _, inclusive: false } => "..",
    Refinement { lo: _, hi: _, inclusive: true } => "..=",
});

constructs!(bound_kind, BOUNDS: Bound {
    Bound::Int { negative: false, value: _ } => "literal bound",
    Bound::Int { negative: true, value: _ } => "negative bound",
    Bound::Name(_) => "name bound",
    Bound::Len(_) => "length bound",
});

constructs!(param_kind, PARAMS: Param {
    Param { is_mut: false, name: _, name_span: _, ty: _ } => "parameter",
    Param { is_mut: true, name: _, name_span: _, ty: _ } => "mut parameter",
});

constructs!(field_kind, FIELDS: FieldSyntax {
    FieldSyntax { is_pub: false, name: _, name_span: _, ty: _ } => "field",
    FieldSyntax { is_pub: true, name: _, name_span: _, ty: _ } => "pub field",
});

/// What the expressions and statements of a function reach beyond their kinds: which field a
/// read names, and the places that hold a field.
const PLACES: &[&str] = &[
    "length",
    "struct field",
    "assignment to a field",
    "mut field argument",
    "array of structs",
    "struct of structs",
    "refined field",
    "refined array element in a field",
    "exclusive field range",
    "field read through an index that calls with mut",
    "struct of enums",
    "enum of structs",
    "enum of enums",
    "array in a payload",
    "refined payload field",
    "match on an enum",
    "match on a bool",
    "match on an integer",
    "match covering a refinement without _",
    "match covering an arithmetic scrutinee's type without _",
    "match in an arm",
    "match with a mut call in its scrutinee",
    "match reading an if",
    "return in an arm or a branch of an expression body",
    "value whose every branch leaves, as a binding's initializer",
    "value whose every branch leaves, as an element",
    "value whose every branch leaves, as an argument",
    "value whose every branch leaves, as an operand",
    "value whose every branch leaves, as an expression body",
    "array of no type, as a returned value",
    "array of no type, as an assigned value",
    "array of no type, as a struct field",
    "array of no type, as a variant field",
    "array of no type, as an element",
    "repeat of no copies of a value of no type",
    "branch that leaves by an operand",
];

constructs!(contract_kind, CONTRACTS: Contract {
    Contract { kind: ContractKind::Requires, expr: _ } => "requires",
    Contract { kind: ContractKind::Ensures, expr: _ } => "ensures",
    Contract { kind: ContractKind::Decreases, expr: _ } => "decreases",
});

constructs!(result_kind, RESULTS: Option<TypeId> {
    None => "no result type",
    Some(_) => "result type",
});

constructs!(body_kind, BODIES: Option<Body> {
    None => "no body",
    Some(Body::Block(_)) => "block body",
    Some(Body::Expr(_)) => "expression body",
});

/// What generated programs never hold, each with its reason.
const NOT_GENERATED: &[(&str, &str)] = &[
    (
        "expression error",
        "only a diagnostic puts it in a tree, and a generated program parses",
    ),
    (
        "type error",
        "only a diagnostic puts it in a tree, and a generated program parses",
    ),
    (
        "no body",
        "a function without a body is a declaration, which M1 refuses to call (E0224)",
    ),
    (
        "dotted call",
        "a variant built by position is an error (E0226), and a call on a receiver comes later in M2",
    ),
    (
        "binding by position",
        "a pattern names each field it binds (E0226)",
    ),
    (
        "suffixed literal pattern",
        "a pattern's literal takes the type of what the `match` reads, so it has no suffix (E0221)",
    ),
];

/// The positions the rows of a value whose every branch leaves count.
const LEAVING_POSITIONS: &[&str] = &[
    "a binding's initializer",
    "an element",
    "an argument",
    "an operand",
    "an expression body",
];

/// The positions the rows of an array of no type count.
const AGGREGATE_POSITIONS: &[&str] = &[
    "a returned value",
    "an assigned value",
    "a struct field",
    "a variant field",
    "an element",
];

/// Whether `e` gives no value, by its syntax: `todo`, `unreachable`, an `if` or a `match` whose
/// every branch leaves, or an array of at least one element of which each gives none
/// (`core-calculus.md`, item 6).
fn gives_nothing(tree: &Tree, e: ExprId) -> bool {
    match &tree.expr(e).kind {
        ExprKind::Array(items) => {
            !items.is_empty() && items.iter().all(|&i| gives_nothing(tree, i))
        }
        ExprKind::Repeat { value, count } => *count > 0 && gives_nothing(tree, *value),
        _ => check::value_leaves(tree, e),
    }
}

/// Where each expression of a function stands, by what holds it: a binding's initializer, an
/// element, an argument, an operand, the expression body, a returned value, an assigned value,
/// a struct's or a variant's field, or none of these.
fn positions(tree: &Tree, body: &Option<Body>) -> Vec<Option<&'static str>> {
    let mut at: Vec<Option<&'static str>> = vec![None; tree.exprs.len()];
    let mut put = |e: &ExprId, p: &'static str| at[e.0 as usize] = Some(p);
    for e in &tree.exprs {
        match &e.kind {
            ExprKind::Unary { operand, .. } => put(operand, "an operand"),
            ExprKind::Binary { lhs, rhs, .. } => {
                put(lhs, "an operand");
                put(rhs, "an operand");
            }
            ExprKind::Call { args, .. } => args.iter().for_each(|a| put(&a.value, "an argument")),
            ExprKind::Array(items) => items.iter().for_each(|i| put(i, "an element")),
            ExprKind::Repeat { value, .. } => put(value, "an element"),
            ExprKind::Construct { fields, .. } => {
                fields.iter().for_each(|f| put(&f.value, "a struct field"))
            }
            ExprKind::Variant {
                payload: Payload::Named(fields),
                ..
            } => fields.iter().for_each(|f| put(&f.value, "a variant field")),
            ExprKind::Variant {
                payload: Payload::None | Payload::Positional(_),
                ..
            }
            | ExprKind::Int { .. }
            | ExprKind::Bool(_)
            | ExprKind::Name(_)
            | ExprKind::Todo(_)
            | ExprKind::Unreachable(_)
            | ExprKind::Index { .. }
            | ExprKind::Field { .. }
            | ExprKind::Match { .. }
            | ExprKind::If { .. }
            | ExprKind::Error => {}
        }
    }
    for s in &tree.stmts {
        match &s.kind {
            StmtKind::Binding { init, .. } => put(init, "a binding's initializer"),
            StmtKind::Assign { value, .. } => put(value, "an assigned value"),
            StmtKind::Return(Some(e)) => put(e, "a returned value"),
            StmtKind::While { .. }
            | StmtKind::Guard { .. }
            | StmtKind::Return(None)
            | StmtKind::Break
            | StmtKind::Continue
            | StmtKind::Discard(_)
            | StmtKind::Expr(_) => {}
        }
    }
    match body {
        Some(Body::Expr(e)) => put(e, "an expression body"),
        Some(Body::Block(_)) | None => {}
    }
    at
}

/// The constructs `f` holds.
fn reached(f: &FnSyntax, seen: &mut BTreeSet<String>) {
    let FnSyntax {
        name: _,
        name_span: _,
        params,
        result,
        contracts,
        body,
        tree,
    } = f;
    let Tree {
        exprs,
        stmts,
        blocks: _,
        types,
    } = tree;
    let mut mark = |name: &str| {
        seen.insert(name.to_string());
    };
    mark(result_kind(result));
    mark(body_kind(body));
    // An expression body holds statements only in the blocks of its arms and branches.
    if matches!(body, Some(Body::Expr(_)))
        && stmts
            .iter()
            .any(|s| matches!(s.kind, StmtKind::Return(Some(_))))
    {
        mark("return in an arm or a branch of an expression body");
    }
    params.iter().for_each(|p| mark(param_kind(p)));
    contracts.iter().for_each(|c| mark(contract_kind(c)));
    // An `if` or a `match` whose every branch leaves gives no value; a branch that leaves only
    // through an operand keeps its type (`core-calculus.md`, item 6).
    let at = positions(tree, body);
    for (k, e) in exprs.iter().enumerate() {
        let branches: Vec<ArmBody> = match &e.kind {
            ExprKind::If {
                then,
                otherwise: Some(Else::Block(b)),
                ..
            } => vec![ArmBody::Block(*then), ArmBody::Block(*b)],
            ExprKind::If {
                then,
                otherwise: Some(Else::If(i)),
                ..
            } => vec![ArmBody::Block(*then), ArmBody::Expr(*i)],
            ExprKind::Match { arms, .. } => arms.iter().map(|a| a.body).collect(),
            _ => Vec::new(),
        };
        if !branches.is_empty() && check::value_leaves(tree, ExprId(k as u32)) {
            if let Some(p) = at[k].filter(|p| LEAVING_POSITIONS.contains(p)) {
                mark(&format!("value whose every branch leaves, as {}", p));
            }
        }
        if branches
            .iter()
            .any(|&b| !check::arm_ends(tree, b) && !check::arm_leaves(tree, b))
        {
            mark("branch that leaves by an operand");
        }
        if e.kind.is_array_literal() && gives_nothing(tree, ExprId(k as u32)) {
            if let Some(p) = at[k].filter(|p| AGGREGATE_POSITIONS.contains(p)) {
                mark(&format!("array of no type, as {}", p));
            }
        }
        if let ExprKind::Repeat { value, count: 0 } = &e.kind {
            if gives_nothing(tree, *value) {
                mark("repeat of no copies of a value of no type");
            }
        }
    }
    for e in exprs {
        mark(expr_kind(&e.kind));
        match &e.kind {
            ExprKind::Int {
                suffix: Some(s), ..
            } => mark(&format!("literal of {}", s)),
            ExprKind::Unary { op, .. } => mark(unary_kind(op)),
            ExprKind::Binary { op, .. } => mark(binary_kind(op)),
            ExprKind::Call { args, .. } => {
                for a in args {
                    mark(arg_kind(a));
                    if a.is_mut && holds_field(tree, a.value) {
                        mark("mut field argument");
                    }
                }
            }
            ExprKind::Field { name, .. } if name == "len" => mark("length"),
            ExprKind::Field { base, .. } => {
                mark("struct field");
                if indexed_by_a_mut_call(tree, *base) {
                    mark("field read through an index that calls with mut");
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                if calls_with_mut(tree, *scrutinee) {
                    mark("match with a mut call in its scrutinee");
                }
                if matches!(tree.expr(*scrutinee).kind, ExprKind::If { .. }) {
                    mark("match reading an if");
                }
                let mut wildcard = false;
                for arm in arms {
                    mark(pattern_kind(&arm.pattern.kind));
                    mark(arm_kind(&arm.body));
                    match &arm.pattern.kind {
                        PatternKind::Variant { bindings, .. } => {
                            mark("match on an enum");
                            for b in bindings.iter().flatten() {
                                mark(binding_kind(b));
                            }
                        }
                        PatternKind::Bool(_) => mark("match on a bool"),
                        PatternKind::Int { .. } => mark("match on an integer"),
                        PatternKind::Wildcard => wildcard = true,
                    }
                    let nested = match arm.body {
                        ArmBody::Expr(x) => matches!(tree.expr(x).kind, ExprKind::Match { .. }),
                        ArmBody::Block(b) => tree.block(b).stmts.iter().any(|&s| {
                            matches!(&tree.stmt(s).kind, StmtKind::Expr(x)
                                if matches!(tree.expr(*x).kind, ExprKind::Match { .. }))
                        }),
                    };
                    if nested {
                        mark("match in an arm");
                    }
                }
                let ints = arms
                    .iter()
                    .any(|a| matches!(a.pattern.kind, PatternKind::Int { .. }));
                if ints && !wildcard {
                    // An arithmetic scrutinee is narrowed into its type, which its arms then
                    // cover whole; a refined name's arms cover its refinement.
                    let arithmetic = matches!(
                        tree.expr(*scrutinee).kind,
                        ExprKind::Binary { .. } | ExprKind::Unary { .. }
                    );
                    mark(if arithmetic {
                        "match covering an arithmetic scrutinee's type without _"
                    } else {
                        "match covering a refinement without _"
                    });
                }
            }
            _ => {}
        }
    }
    for s in stmts {
        mark(stmt_kind(&s.kind));
        match &s.kind {
            StmtKind::Assign { op, place, .. } => {
                mark(assign_kind(op));
                if holds_field(tree, *place) {
                    mark("assignment to a field");
                }
                if let AssignOp::Compound(op) = op {
                    mark(&format!("{}=", op.symbol()));
                }
            }
            StmtKind::While {
                invariants,
                decreases,
                ..
            } => {
                mark(invariant_kind(&!invariants.is_empty()));
                mark(loop_decreases_kind(&!decreases.is_empty()));
            }
            _ => {}
        }
    }
    for t in types {
        mark(type_kind(&t.kind));
        match &t.kind {
            TypeKind::Int { name, refinement } => {
                mark(&format!("type {}", name));
                if let Some(r) = refinement {
                    mark(range_kind(r));
                    mark(bound_kind(&r.lo));
                    mark(bound_kind(&r.hi));
                }
            }
            TypeKind::Array { elem, .. } => match tree.types[elem.0 as usize].kind {
                TypeKind::Array { .. } => mark("array of arrays"),
                TypeKind::Named(_) => mark("array of structs"),
                _ => {}
            },
            _ => {}
        }
    }
}

/// Whether a place holds a field: `s.f`, `s.f[i]`, `xs[i].f`.
fn holds_field(tree: &Tree, e: ExprId) -> bool {
    match &tree.expr(e).kind {
        ExprKind::Field { .. } => true,
        ExprKind::Index { base, .. } => holds_field(tree, *base),
        _ => false,
    }
}

/// Whether a place's path holds an index whose expression calls with a `mut` argument.
fn indexed_by_a_mut_call(tree: &Tree, e: ExprId) -> bool {
    match &tree.expr(e).kind {
        ExprKind::Index { base, index } => {
            calls_with_mut(tree, *index) || indexed_by_a_mut_call(tree, *base)
        }
        ExprKind::Field { base, .. } => indexed_by_a_mut_call(tree, *base),
        _ => false,
    }
}

fn calls_with_mut(tree: &Tree, e: ExprId) -> bool {
    match &tree.expr(e).kind {
        ExprKind::Call { args, .. } => {
            args.iter().any(|a| a.is_mut) || args.iter().any(|a| calls_with_mut(tree, a.value))
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            calls_with_mut(tree, *lhs) || calls_with_mut(tree, *rhs)
        }
        ExprKind::Unary { operand, .. } => calls_with_mut(tree, *operand),
        _ => false,
    }
}

/// The constructs a struct declaration holds.
fn reached_struct(s: &StructSyntax, seen: &mut BTreeSet<String>) {
    let StructSyntax {
        name: _,
        name_span: _,
        fields,
        tree,
    } = s;
    let mut mark = |name: &str| {
        seen.insert(name.to_string());
    };
    for f in fields {
        mark(field_kind(f));
        match &tree.ty(f.ty).kind {
            TypeKind::Named(_) => mark("struct of structs"),
            TypeKind::Enum(_) => mark("struct of enums"),
            TypeKind::Int {
                refinement: Some(r),
                ..
            } => {
                mark("refined field");
                if !r.inclusive {
                    mark("exclusive field range");
                }
            }
            TypeKind::Array { elem, .. } => {
                if let TypeKind::Int {
                    refinement: Some(r),
                    ..
                } = &tree.ty(*elem).kind
                {
                    mark("refined array element in a field");
                    if !r.inclusive {
                        mark("exclusive field range");
                    }
                }
            }
            _ => {}
        }
    }
    for t in &tree.types {
        mark(type_kind(&t.kind));
        if let TypeKind::Int { name, refinement } = &t.kind {
            mark(&format!("type {}", name));
            if let Some(r) = refinement {
                mark(range_kind(r));
                mark(bound_kind(&r.lo));
                mark(bound_kind(&r.hi));
            }
        }
    }
}

/// The constructs an enum declaration holds.
fn reached_enum(e: &EnumSyntax, seen: &mut BTreeSet<String>) {
    let EnumSyntax {
        name: _,
        name_span: _,
        variants,
        tree,
    } = e;
    let mut mark = |name: &str| {
        seen.insert(name.to_string());
    };
    for v in variants {
        mark(variant_kind(&!v.fields.is_empty()));
        for f in &v.fields {
            match &tree.ty(f.ty).kind {
                TypeKind::Named(_) => mark("enum of structs"),
                TypeKind::Enum(_) => mark("enum of enums"),
                TypeKind::Array { .. } => mark("array in a payload"),
                TypeKind::Int {
                    refinement: Some(_),
                    ..
                } => mark("refined payload field"),
                _ => {}
            }
        }
    }
    for t in &tree.types {
        mark(type_kind(&t.kind));
    }
}

/// The functions of `src`, which must parse, their trees as name resolution qualifies them:
/// `a.b`, `a` an enum, is a variant.
fn functions(src: &str) -> Vec<FnSyntax> {
    let module = mente_syntax::parse_module(src).unwrap_or_else(|d| panic!("{:?}\n{}", d, src));
    let parsed = mente_sema::parse_qualified(src, &module);
    let diags: Vec<_> = parsed.iter().flat_map(|p| p.errors.iter()).collect();
    assert!(diags.is_empty(), "{:?}\n{}", diags, src);
    parsed.into_iter().filter_map(|p| p.syntax).collect()
}

/// The enums of `src`, which must parse.
fn enums(src: &str) -> Vec<EnumSyntax> {
    let module = mente_syntax::parse_module(src).unwrap_or_else(|d| panic!("{:?}\n{}", d, src));
    let names = mente_sema::structs::enum_names(&module);
    mente_syntax::parse_enums(src, &module)
        .into_iter()
        .map(|p| {
            assert!(p.errors.is_empty(), "{:?}\n{}", p.errors, src);
            let mut syntax = p.syntax;
            mente_sema::structs::qualify(&mut syntax.tree, &names);
            syntax
        })
        .collect()
}

/// The structs of `src`, which must parse.
fn structs(src: &str) -> Vec<StructSyntax> {
    let module = mente_syntax::parse_module(src).unwrap_or_else(|d| {
        panic!(
            "{:?}
{}",
            d, src
        )
    });
    let names = mente_sema::structs::enum_names(&module);
    mente_syntax::parse_structs(src, &module)
        .into_iter()
        .map(|p| {
            assert!(
                p.errors.is_empty(),
                "{:?}
{}",
                p.errors,
                src
            );
            let mut syntax = p.syntax;
            mente_sema::structs::qualify(&mut syntax.tree, &names);
            syntax
        })
        .collect()
}

/// The compound assignments the grammar has, as the parser reads them: each binary operator
/// followed by `=`, where that parses as an assignment.
fn compound_assignments() -> Vec<String> {
    BINARIES
        .iter()
        .filter_map(|symbol| {
            let src = format!("module m\nfn f(mut x: u8) {{\n    x {}= 1\n}}\n", symbol);
            let module = mente_syntax::parse_module(&src).ok()?;
            let (fns, diags) = mente_syntax::parse_functions(&src, &module);
            let compound = diags.is_empty()
                && fns.iter().flat_map(|f| &f.tree.stmts).any(|s| {
                    matches!(&s.kind, StmtKind::Assign { op: AssignOp::Compound(op), .. }
                        if op.symbol() == *symbol)
                });
            compound.then(|| format!("{}=", symbol))
        })
        .collect()
}

/// Every construct of M1's grammar a function's tree can hold.
fn grammar() -> BTreeSet<String> {
    let tables: &[&[&str]] = &[
        EXPRS,
        UNARIES,
        BINARIES,
        ARGS,
        STMTS,
        ASSIGNS,
        INVARIANTS,
        LOOP_DECREASES,
        TYPES,
        RANGES,
        BOUNDS,
        PARAMS,
        CONTRACTS,
        RESULTS,
        BODIES,
        FIELDS,
        PLACES,
        PATTERNS,
        BINDINGS,
        ARMS,
        VARIANTS,
        &["array of arrays"],
    ];
    let mut all = BTreeSet::new();
    for name in tables.iter().flat_map(|t| t.iter()) {
        assert!(all.insert(name.to_string()), "`{}` is named twice", name);
    }
    for t in INT_TYPES {
        assert!(all.insert(format!("type {}", t.name)));
        assert!(all.insert(format!("literal of {}", t.name)));
    }
    let compound = compound_assignments();
    assert!(!compound.is_empty(), "no compound assignment parses");
    for name in compound {
        assert!(all.insert(name.clone()), "`{}` is named twice", name);
    }
    all
}

/// Every generated program passes every check, so that the three paths run it and none is left
/// out unseen: nothing refuses one but a narrowing Tier 1 proves always traps (`E0508`), whose
/// function is generated again once the reference interpreter has run the claim
/// ([`generate::checked`]), at most [`MAX_REGENERATED`] times over these seeds. At least
/// [`MIN_SEEDS_PER_ROW`] of the programs reach each construct of M1's grammar but those
/// [`NOT_GENERATED`] names, which none reaches.
#[test]
fn test_generated_programs_reach_every_construct_of_the_grammar() {
    let grammar = grammar();
    // The seeds whose program reaches each construct.
    let mut seeds: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut regenerated = Vec::new();
    // Each seed's program is checked on its own, on as many threads as the machine has.
    let programs = super::differential::parallel((0..SEEDS).collect(), |seed| {
        let c = generate::checked(seed);
        let mut seen = BTreeSet::new();
        if c.module.is_some() {
            let text = &c.program.text;
            functions(text).iter().for_each(|f| reached(f, &mut seen));
            structs(text)
                .iter()
                .for_each(|s| reached_struct(s, &mut seen));
            enums(text).iter().for_each(|e| reached_enum(e, &mut seen));
        }
        (seed, seen, c.regenerated, c.problems)
    });
    for (seed, seen, again, wrong) in programs {
        problems.extend(wrong);
        regenerated.extend(again.into_iter().map(|r| (seed, r)));
        for name in seen {
            seeds.entry(name).or_default().push(seed);
        }
    }
    assert!(
        problems.is_empty(),
        "generated programs the checker refuses, or claims of Tier 1 a run contradicts:\n{}",
        problems.join("\n")
    );
    let checked = regenerated.iter().filter(|(_, r)| r.reached > 0).count();
    println!(
        "{} functions generated again for a narrowing that always traps, {} of them with the \
         claim seen to hold in a run",
        regenerated.len(),
        checked
    );
    assert!(
        regenerated.len() <= MAX_REGENERATED,
        "{} functions were generated again for a narrowing that always traps, above the bound \
         of {}: the generator writes more of them than it did: {:?}",
        regenerated.len(),
        MAX_REGENERATED,
        regenerated
            .iter()
            .map(|(seed, r)| format!(
                "seed {} {} {}:{}: {}",
                seed, r.function, r.line, r.col, r.message
            ))
            .collect::<Vec<_>>()
    );
    for name in seeds.keys() {
        assert!(grammar.contains(name), "`{}` is not in the grammar", name);
    }
    for (name, reason) in NOT_GENERATED {
        assert!(grammar.contains(*name), "`{}` is not in the grammar", name);
        assert!(
            !seeds.contains_key(*name),
            "`{}` is generated now, though excluded because {}",
            name,
            reason
        );
    }
    let mut least: Vec<(usize, &String)> = seeds.iter().map(|(n, s)| (s.len(), n)).collect();
    least.sort();
    println!(
        "the constructs the fewest programs reach: {}",
        least
            .iter()
            .take(8)
            .map(|(count, name)| format!("`{}` {}", name, count))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // A construct one or two programs hold is lost by any change to the generator's choices,
    // so each is asked of a few.
    let few: Vec<String> = grammar
        .iter()
        .filter(|n| NOT_GENERATED.iter().all(|(x, _)| x != n))
        .filter_map(|n| {
            let reach = seeds.get(n).map_or(&[][..], |s| s.as_slice());
            (reach.len() < MIN_SEEDS_PER_ROW).then(|| format!("`{}` in seeds {:?}", n, reach))
        })
        .collect();
    assert!(
        few.is_empty(),
        "fewer than {} of the programs of seeds 0 to {} reach each of these:\n{}",
        MIN_SEEDS_PER_ROW,
        SEEDS - 1,
        few.join("\n")
    );
}

/// The seeds the test reads: the first programs of each mode.
const SEEDS: u64 = 300;

/// How many of those seeds' programs must reach each construct: a construct that hangs on one
/// program or two is lost by any change to what the generator chooses at random.
const MIN_SEEDS_PER_ROW: usize = 3;

/// The most functions the programs of those seeds have generated again for a narrowing Tier 1
/// proves always traps (`E0508`). A generator that writes more of them than this has lost what it
/// knows of the values it narrows.
const MAX_REGENERATED: usize = 40;
