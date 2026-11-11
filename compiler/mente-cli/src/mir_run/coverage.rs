//! What of the grammar the program generator reaches (`docs/design-review.md`,
//! `quality-per-change`: new syntax extends the generator in the same pull request): M1's, and
//! M2's structs.
//!
//! Each table below names the constructs of one part of a function's syntax tree, and its
//! patterns name every variant and every field, without a wildcard. A construct added to the
//! grammar therefore does not compile until a table names it, and the table's list of names,
//! which the same macro writes, makes the test require that generated programs reach it, or
//! that [`NOT_GENERATED`] give the reason they cannot.

use std::collections::BTreeSet;

use mente_sema::types::INT_TYPES;
use mente_syntax::tree::{
    Arg, AssignOp, BinaryOp, Body, Bound, Contract, ContractKind, Else, ExprId, ExprKind,
    FieldSyntax, FnSyntax, Param, Refinement, StmtKind, StructSyntax, Tree, TypeId, TypeKind,
    UnaryOp,
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
    TypeKind::Error => "type error",
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
    "field read through an index that calls with mut",
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
];

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
    params.iter().for_each(|p| mark(param_kind(p)));
    contracts.iter().for_each(|c| mark(contract_kind(c)));
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
            TypeKind::Int {
                refinement: Some(_),
                ..
            } => mark("refined field"),
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

/// The functions of `src`, which must parse.
fn functions(src: &str) -> Vec<FnSyntax> {
    let module = mente_syntax::parse_module(src).unwrap_or_else(|d| panic!("{:?}\n{}", d, src));
    let (fns, diags) = mente_syntax::parse_functions(src, &module);
    assert!(diags.is_empty(), "{:?}\n{}", diags, src);
    fns
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
            p.syntax
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

/// The generated programs that pass every check, which the three paths run, reach every
/// construct of M1's grammar but those [`NOT_GENERATED`] names, and those never.
#[test]
fn test_generated_programs_reach_every_construct_of_the_grammar() {
    let grammar = grammar();
    let mut seen = BTreeSet::new();
    for seed in 0..SEEDS {
        let p = generate::program(seed);
        if mente_sema::check_program(&p.text).is_ok() {
            functions(&p.text)
                .iter()
                .for_each(|f| reached(f, &mut seen));
            structs(&p.text)
                .iter()
                .for_each(|s| reached_struct(s, &mut seen));
        }
    }
    for name in &seen {
        assert!(grammar.contains(name), "`{}` is not in the grammar", name);
    }
    for (name, reason) in NOT_GENERATED {
        assert!(grammar.contains(*name), "`{}` is not in the grammar", name);
        assert!(
            !seen.contains(*name),
            "`{}` is generated now, though excluded because {}",
            name,
            reason
        );
    }
    let missing: Vec<&String> = grammar
        .iter()
        .filter(|n| !seen.contains(*n) && NOT_GENERATED.iter().all(|(x, _)| x != n))
        .collect();
    assert!(
        missing.is_empty(),
        "no checked program of seeds 0 to {} reaches {:?}",
        SEEDS - 1,
        missing
    );
}

/// The seeds the test reads: the first programs of each mode.
const SEEDS: u64 = 300;
