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

use mente_interp::{Event, Stop, Trap, TrapKind};
use mente_sema::check;
use mente_sema::literals::From;
use mente_sema::octagon::{INF, RELATIONAL_LIMIT};
use mente_sema::tier1::Routing;
use mente_sema::types::{Int, Ty, INT_TYPES};
use mente_sema::wide::{self, Interval};
use mente_sema::{CheckedFn, CheckedModule};
use mente_syntax::tree::{
    Arg, ArmBody, AssignOp, BinaryOp, BlockId, Body, Bound, Contract, ContractKind, Else,
    EnumSyntax, ExprId, ExprKind, FieldSyntax, FnSyntax, Param, PatBinding, PatternKind, Payload,
    Refinement, StmtKind, StructSyntax, Tree, TypeId, TypeKind, UnaryOp,
};

use super::generate;

/// Defines `$name`, the construct a value of `$t` is, and `$all`, every name it gives, from one
/// table.
macro_rules! constructs {
    ($name:ident, $all:ident: $t:ty { $($p:pat => $n:literal,)+ }) => {
        pub(super) const $all: &[&str] = &[$($n),+];
        pub(super) fn $name(x: &$t) -> &'static str {
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

/// What a function holds of the edge of Tier 1's domain
/// (`ghost-contracts-and-totality.md`, item 4): a value whose static range lies wholly past
/// the limit, of which Tier 1 holds only a bound weakened to the nearest it has, under the
/// operations that carry that bound on, and the index and the narrowing computed from it.
const EDGES: &[&str] = &[
    "product past Tier 1's domain",
    "quotient of a value past Tier 1's domain",
    "quotient by a value past Tier 1's domain",
    "difference with a value past Tier 1's domain",
    "remainder with a value past Tier 1's domain",
    "operand below Tier 1's domain",
    "index computed from a value past Tier 1's domain",
    "narrowing of a value computed from one past Tier 1's domain",
];

/// What a call's arguments are to two parameters its callee relates.
const RELATIONS: &[&str] = &["call that relates two names"];

/// The operations written to trap whenever they run, by what the checker's static ranges and
/// types say of their operands: the check of each stays in MIR and in native code, since
/// Tier 1 proves none of them, and the three paths must trap alike.
const TRAPS: &[&str] = &[
    "divisor that is always zero",
    "index at its array's length",
    "shift by its type's width or more",
];

/// A loop's candidate that can trap (`ghost-contracts-and-totality.md`, item 2, "Facts at a
/// loop head"): a remainder whose divisor's static range holds 0, in a loop's condition or in a
/// guard of its body. Tier 1 keeps it only where the divisor is proven not 0 without the facts of
/// its own evaluation, which once proved it (the nightly's wrong proofs, 2026-11-16 to 11-21).
/// And a conjunct or a guard that is no candidate by its syntax (`loop-candidates-by-syntax`),
/// of each form the reviews of pull request 182 found Tier 1 holding at a loop's head: one that
/// may leave, by `break`, `continue`, `return`, `todo` or `unreachable`, a call whose `ensures`
/// bounds its argument, and a `break` in the condition of a loop inside a loop, which leaves the
/// loop around it. The review's form that assigns is a condition that changes state, which
/// [`CONDITIONS`] names.
const CANDIDATES: &[&str] = &[
    "remainder that can trap in a loop's condition",
    "remainder that can trap in a guard of a loop's body",
    "conjunct of a loop's condition that may leave",
    "guard of a loop's body whose condition may leave",
    "conjunct of a loop's condition that calls a function whose ensures bounds its argument",
    "break in the condition of a loop inside a loop",
    "guard of a loop's body that reads, after an and, a remainder of a binding the round assigned",
];

/// A condition that assigns or passes a place `mut`, anywhere inside it, which the checker
/// refuses (`E0236`): the condition of a `while`, a `guard` or an `if`, a `bool` scrutinee, a
/// contract clause or an operand of `and` or `or` changes nothing (`core-calculus.md`, item 10),
/// so no generated program holds one ([`NOT_GENERATED`]).
const CONDITIONS: &[&str] = &["condition that assigns or passes mut"];

/// A nested array literal that narrows its integers by itself (`core-calculus.md`, item 4,
/// Norman's decision of 2026-11-21): a row whose elements have one integer type, inside a literal
/// whose rows have no one type together, which keeps the others exact. The nightly runs found MIR
/// building such a row in the outer literal's type, where the reference interpreter narrowed it.
/// And branches, or arms, that give arrays whose integers meet past 64 bits, in an `i128` or a
/// `u128`, the edge E0222 holds them to (the review of PR 189 found lowering refusing arrays one
/// bit past it, which the checker let through). And a branch or an arm of no type beside two that
/// give values, through an `else if` too, which holds no integer of their hull (the final review
/// of PR 189 found one dropping the whole hull, and MIR meeting the others in `i64`).
const ROWS: &[&str] = &[
    "row of one integer type beside rows of others",
    "branches of arrays meeting in a 128-bit machine type",
    "branch of no type beside two that give values",
];

/// Where a bit operation, a shift or `~` over literals alone takes the type it works in
/// (`core-calculus.md`, item 5), by what the checker's table says of it: beside an operand
/// of a type, which gives it; and alone, an `i64`, either narrowed into a place, which gives
/// it none, or not.
const LITERALS: &[&str] = &[
    "bit operation on literals alone, beside an operand of a type",
    "bit operation on literals alone, narrowed into a place",
    "bit operation on literals alone, of no type but i64",
];

/// A literal whose type comes from an operand beyond the operation it stands in, by what the
/// checker's table says of it (`core-calculus.md`, item 5): it takes the type of every operand of
/// its expression, so beside `a` in `a + 3 + b` it takes the type that holds `a`'s and `b`'s.
const EXPRESSIONS: &[&str] = &[
    "literal that takes a type from beyond its own operation",
    "literal of a compound assignment that takes its place's wider type",
];

/// What a comparison's operands are, by what the checker's table says of its literals
/// (`core-calculus.md`, item 5): a literal takes the type of the other operand, on the right
/// or on the left; two expressions of literals alone are each an `i64`; operands of two
/// types, a suffixed literal among them, are compared exactly; and so is a literal beside
/// integers no standard type holds together, which takes no type.
const COMPARISONS: &[&str] = &[
    "literal compared with an operand of a type",
    "literal on the left of a comparison",
    "literals alone compared",
    "operands of two types compared",
    "literal compared exactly beside integers no type holds",
];

/// A right shift whose amount is valid for the type and reaches past the bits its value is
/// kept in: a value of a 128-bit type whose static range fits 64 bits, shifted by 64 or more.
/// Native code shifts within the width of the value it is given, and must still give 0, or -1
/// for a negative value (`core-calculus.md`, item 4).
const SHIFTS: &[&str] = &["right shift past the bits its value is kept in"];

/// A clause of a function marked `@checked` or `@prove` that holds an `if` or a `match` with a
/// block arm, by its kind: a `requires`, a loop's invariant and its measure, which Tier 1 walks
/// where it decides them, and an `ensures`, which it walks nowhere though a run evaluates it, so
/// that the containment oracle claims nothing inside it (`ghost-contracts-and-totality.md`,
/// item 2). The oracle's first version reported a false alarm inside such an `ensures`, which no
/// generated program held (`tier1-containment`).
const ROUTED_CLAUSES: &[&str] = &["requires", "ensures", "invariant", "loop decreases"];

/// The forms of a clause [`ROUTED_CLAUSES`] names: an `if`, and a `match` with a block arm.
const ROUTED_FORMS: &[&str] = &["an if", "a match with a block arm"];

/// The attributes that route a function's clauses outside the Tier-1 fragment, each its own
/// path through the checker (`tier1::Routing`): `@checked` makes them run-time checks, and
/// `@prove` trusted axioms until `mn prove` runs, which Tier 1 lists.
const ROUTES: &[&str] = &["@checked", "@prove"];

/// The row of a clause of kind `clause` and form `form` in a function marked `route`.
fn routed_row(clause: &str, form: &str, route: &str) -> String {
    format!("{} holding {}, in a {} function", clause, form, route)
}

/// The ways of each form of [`ROUTED_FORMS`]: an `if`'s two branches, and a `match`'s arms by
/// what they hold.
const ROUTED_WAYS: &[(&str, &[&str])] = &[
    ("an if", &["its then branch", "its else branch"]),
    (
        "a match with a block arm",
        &["an expression arm", "a block arm"],
    ),
];

/// The row of a way of a clause of kind `clause` and form `form`, routed either way, that a run
/// of its program takes, evaluating the value of that branch or arm ([`reached_routed_runs`]):
/// each way of each kind and form, taken in programs enough, so that no generated clause is split
/// where no value its runs give falls on one side.
fn way_row(clause: &str, form: &str, way: &str) -> String {
    format!(
        "{} holding {} in a routed function, {} taken",
        clause, form, way
    )
}

/// A `requires` of a `@checked` function that holds a block and that a call breaks, so that
/// the run-time check such a function writes traps it: a lowering that dropped the check would
/// run that call on where the reference interpreter traps. At the entry of the routed function
/// called from outside, and inside the program's call of the one the program calls, whose trap
/// the caller passes back, each of which a lowering keeps apart.
const TRAPPING_REQUIRES: &str =
    "requires holding a block that traps a call at its function's entry, in a @checked function";
const TRAPPING_CALLED_REQUIRES: &str =
    "requires holding a block that traps a call the program makes, in a @checked function";

/// Every row the clauses of routed functions give: each kind, form and attribute, each way of
/// each kind and form taken, and the `requires` that traps.
fn routed_rows() -> Vec<String> {
    let mut rows = Vec::new();
    for clause in ROUTED_CLAUSES {
        for form in ROUTED_FORMS {
            rows.extend(ROUTES.iter().map(|route| routed_row(clause, form, route)));
        }
        for (form, ways) in ROUTED_WAYS {
            rows.extend(ways.iter().map(|way| way_row(clause, form, way)));
        }
    }
    rows.push(TRAPPING_REQUIRES.to_string());
    rows.push(TRAPPING_CALLED_REQUIRES.to_string());
    rows
}

/// What generated programs never hold, each with its reason.
pub(super) const NOT_GENERATED: &[(&str, &str)] = &[
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
    (
        "condition that assigns or passes mut",
        "a condition changes nothing, and the checker refuses one that does (E0236)",
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
        header: _,
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

/// A clause of a function that is an `if` or a `match` with a block arm: its kind, its form, the
/// clause, and each of its ways, a branch of the `if` or an arm of the `match`, named as
/// [`ROUTED_WAYS`] names it, with the expression a run evaluates when it takes that way, its value.
struct RoutedClause {
    kind: &'static str,
    form: &'static str,
    clause: ExprId,
    ways: Vec<(&'static str, Option<ExprId>)>,
    /// Whether it is a clause of the loop a routed function begins with, after its counter's
    /// binding (`generate::Gen::leading_loop`).
    first_loop: bool,
}

/// Each clause of `f`, its loops' invariants and measures among them, that is an `if` or a
/// `match` with a block arm.
fn routed_clauses(f: &FnSyntax) -> Vec<RoutedClause> {
    let tree = &f.tree;
    // A block's value, its last statement's expression.
    let value = |b: BlockId| {
        let last = *tree.block(b).stmts.last()?;
        match tree.stmt(last).kind {
            StmtKind::Expr(e) => Some(e),
            _ => None,
        }
    };
    let mut clauses: Vec<(&'static str, ExprId, bool)> = f
        .contracts
        .iter()
        .map(|c| (contract_kind(c), c.expr, false))
        .collect();
    // The loop a routed function begins with is its body's second statement, after its
    // counter's binding.
    let first_loop = match &f.body {
        Some(Body::Block(b)) => tree.block(*b).stmts.get(1).map(|s| s.0 as usize),
        Some(Body::Expr(_)) | None => None,
    };
    for (k, s) in tree.stmts.iter().enumerate() {
        if let StmtKind::While {
            invariants,
            decreases,
            ..
        } = &s.kind
        {
            let first = first_loop == Some(k);
            clauses.extend(invariants.iter().map(|&i| ("invariant", i, first)));
            clauses.extend(decreases.iter().map(|&d| ("loop decreases", d, first)));
        }
    }
    let mut out = Vec::new();
    for (kind, clause, first_loop) in clauses {
        let (form, ways) = match &tree.expr(clause).kind {
            ExprKind::If {
                then,
                otherwise: Some(other),
                ..
            } => {
                let other = match other {
                    Else::Block(b) => value(*b),
                    Else::If(i) => Some(*i),
                };
                (
                    "an if",
                    vec![
                        ("its then branch", value(*then)),
                        ("its else branch", other),
                    ],
                )
            }
            ExprKind::Match { arms, .. }
                if arms.iter().any(|a| matches!(a.body, ArmBody::Block(_))) =>
            {
                let ways = arms
                    .iter()
                    .map(|a| match a.body {
                        ArmBody::Expr(x) => ("an expression arm", Some(x)),
                        ArmBody::Block(b) => ("a block arm", value(b)),
                    })
                    .collect();
                ("a match with a block arm", ways)
            }
            _ => continue,
        };
        out.push(RoutedClause {
            kind,
            form,
            clause,
            ways,
            first_loop,
        });
    }
    out
}

/// The rows of [`routed_row`] a function reaches, by each attribute it routes its clauses with.
fn reached_routed(f: &FnSyntax, routing: Routing, seen: &mut BTreeSet<String>) {
    let on = [routing.checked, routing.prove];
    for c in routed_clauses(f) {
        for (route, _) in ROUTES.iter().zip(on).filter(|(_, on)| *on) {
            seen.insert(routed_row(c.kind, c.form, route));
        }
    }
}

/// The classes of routed clauses, each held to a bar of its own on the share of those a run
/// evaluates that leave a way no run takes ([`UNTAKEN_BARS`]): by kind, by which routed function
/// holds it, the one called from outside or the one the program calls, and for a loop's clause,
/// by whether the loop is the function's first, which only steps its counter.
const ENTRY_REQUIRES: &str = "requires of the routed function called from outside";
const ENTRY_ENSURES: &str = "ensures of the routed function called from outside";
const ENTRY_FIRST_INVARIANT: &str =
    "invariant of the first loop of the routed function called from outside";
const ENTRY_FIRST_MEASURE: &str =
    "measure of the first loop of the routed function called from outside";
const CALLED_REQUIRES: &str = "requires of the routed function the program calls";
const CALLED_ENSURES: &str = "ensures of the routed function the program calls";
const CALLED_FIRST_INVARIANT: &str =
    "invariant of the first loop of the routed function the program calls";
const CALLED_FIRST_MEASURE: &str =
    "measure of the first loop of the routed function the program calls";
const LATER_INVARIANT: &str = "invariant of a later loop of a routed function";

/// Each class of routed clause with the most of every hundred a run evaluates that may leave a way
/// no run takes, and the few more any run may have.
///
/// Five classes take both ways by construction, so none may leave one: the routed function called
/// from outside writes no `requires` of the fragment and refines its parameters by literals alone,
/// so the call of every lowest value and the call of every highest exist (`function_calls`) and
/// take its `requires`' two ways, split between them (`Split::across`); the call of every highest
/// passes that `requires`, which fails on the lowest alone, runs the first loop's every round,
/// which only steps its counter, and calls the routed function the program calls with its own
/// highest values, which run that one's first loop the same way. Over seeds 0 to 2999 none did.
///
/// The others are measured over seeds 0 to 2999, the most per hundred their share there rounded
/// up, and the few more twice the standard deviation of the count at the coverage test's 300
/// seeds, at that share, rounded up, so that only a change that takes ways away crosses it:
/// - an `ensures` of the function called from outside: 124 of 523, 24 in a hundred, and 7 more for
///   about 52 a run evaluates there: it holds at each return, and the calls on one side of its
///   split may each trap in the body first;
/// - a `requires` of the function the program calls: 4 of 686, 1 in a hundred, and 2 more for about
///   69: its caller's call of every lowest value stops at the caller's own failing `requires` where
///   it has one, so the values below the split come from the random calls alone;
/// - an `ensures` of the function the program calls: 76 of 606, 13 in a hundred, and 6 more for
///   about 61, as for the other `ensures`;
/// - an invariant of a later loop, which leaves, traps or returns where its body says: 129 of 253,
///   51 in a hundred, and 5 more for about 25: a loop that goes round once takes both ways, and
///   one that leaves in its first round takes one; it cannot be made live by construction without
///   taking from its body what makes it a loop with a body.
pub(super) const UNTAKEN_BARS: &[(&str, usize, usize)] = &[
    (ENTRY_REQUIRES, 0, 0),
    (ENTRY_FIRST_INVARIANT, 0, 0),
    (ENTRY_FIRST_MEASURE, 0, 0),
    (CALLED_FIRST_INVARIANT, 0, 0),
    (CALLED_FIRST_MEASURE, 0, 0),
    (ENTRY_ENSURES, 24, 7),
    (CALLED_REQUIRES, 1, 2),
    (CALLED_ENSURES, 13, 6),
    (LATER_INVARIANT, 51, 5),
];

/// The class of a routed clause ([`UNTAKEN_BARS`]): of kind `kind`, held by the routed function
/// called from outside when `entry`, and of the function's first loop when `first_loop`.
fn class(kind: &str, entry: bool, first_loop: bool) -> &'static str {
    match (kind, entry, first_loop) {
        ("requires", true, _) => ENTRY_REQUIRES,
        ("requires", false, _) => CALLED_REQUIRES,
        ("ensures", true, _) => ENTRY_ENSURES,
        ("ensures", false, _) => CALLED_ENSURES,
        ("invariant", true, true) => ENTRY_FIRST_INVARIANT,
        ("invariant", false, true) => CALLED_FIRST_INVARIANT,
        ("loop decreases", true, true) => ENTRY_FIRST_MEASURE,
        ("loop decreases", false, true) => CALLED_FIRST_MEASURE,
        _ => LATER_INVARIANT,
    }
}

/// What the runs of a program take of one of its routed clauses ([`reached_routed_runs`]).
#[derive(Clone, Debug, Default)]
pub(super) struct ClauseRuns {
    /// The seed, the function and where the clause stands, for a problem to name.
    at: String,
    /// Its class, whose bar holds it ([`UNTAKEN_BARS`]).
    class: &'static str,
    /// Whether a run evaluated the clause.
    ran: bool,
    /// Each of its ways by name, with whether a run took it.
    taken: Vec<(&'static str, bool)>,
    /// Of the calls from outside that evaluated it, as a `requires`, those that stopped at it,
    /// false, and those it let through.
    broke: usize,
    held: usize,
}

/// What the runs of routed clauses show that none may: a `requires` that stops every call from
/// outside that reaches it, which hides its function from the comparison, as a recursion's would
/// whose every call passes the lowest value down; and, of each class, more clauses with a way no
/// run takes than its bar allows ([`UNTAKEN_BARS`]).
pub(super) fn routed_runs_problems(runs: &[ClauseRuns]) -> Vec<String> {
    let mut out: Vec<String> = runs
        .iter()
        .filter(|r| r.broke > 0 && r.held == 0)
        .map(|r| {
            format!(
                "{}: it stops each of the {} calls from outside that reach it, so the comparison \
                 sees none of the function past it",
                r.at, r.broke
            )
        })
        .collect();
    for &(class, per_hundred, room) in UNTAKEN_BARS {
        let ran: Vec<&ClauseRuns> = runs.iter().filter(|r| r.ran && r.class == class).collect();
        let untaken: Vec<&&ClauseRuns> = ran
            .iter()
            .filter(|r| r.taken.iter().any(|(_, taken)| !taken))
            .collect();
        let most = per_hundred * ran.len() / 100 + room;
        if untaken.len() <= most {
            continue;
        }
        let names: Vec<String> = untaken
            .iter()
            .map(|r| {
                let ways: Vec<&str> = r
                    .taken
                    .iter()
                    .filter(|(_, taken)| !taken)
                    .map(|(way, _)| *way)
                    .collect();
                format!("{} ({} untaken)", r.at, ways.join(", "))
            })
            .collect();
        out.push(format!(
            "{} of the {} {} clauses a run evaluates leave a way no run takes, above the {} that \
             may: {}",
            untaken.len(),
            ran.len(),
            class,
            most,
            names.join("; ")
        ));
    }
    out
}

/// A routed `requires` that stops every call from outside that reaches it is a problem
/// ([`routed_runs_problems`]): the second review of pull request 217 found the generator writing
/// one on a recursion's `n: u8[0..=depth]`, which every call passes down to its lowest value, so
/// that each of its calls trapped and the comparison saw none of the function past it. This is
/// that program; and the same `requires` on a function that does not recurse, which lets the calls
/// of the other values through, is none.
#[test]
fn test_a_requires_that_stops_every_call_that_reaches_it_is_a_problem() {
    let run = |text: &str| {
        let module = mente_sema::check_program(text).unwrap_or_else(|d| panic!("{:?}", d));
        let fns = functions(text);
        let routings: Vec<Routing> = fns.iter().map(|(_, r, _)| *r).collect();
        let public: Vec<bool> = fns.iter().map(|(_, _, p)| *p).collect();
        let mut seen = BTreeSet::new();
        let runs = reached_routed_runs(0, text, &module, &routings, &public, &mut seen);
        routed_runs_problems(&runs.unwrap_or_else(|e| panic!("{}", e)))
    };
    let clause = "  requires (if n > 1 { n > 1 } else { n != 0 })\n";
    let recursion = format!(
        "module m\n\n@checked\npub fn f(n: u8[0..=3]) -> u8\n{}  decreases n\n{{\n  guard n > 0 else {{\n    return 0\n  }}\n  f(n - 1)\n}}\n",
        clause
    );
    let problems = run(&recursion);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("it stops each of the 4 calls from outside")),
        "{:?}",
        problems
    );
    let once = format!(
        "module m\n\n@checked\npub fn f(n: u8[0..=3]) -> u8\n{}= n\n",
        clause
    );
    assert_eq!(run(&once), Vec::<String>::new());
}

/// Of a class of routed clause, more with a way no run takes than its bar allows are a problem,
/// and as many are none; a clause no run evaluates counts toward neither, and a class of clauses
/// that take both ways by construction allows none.
#[test]
fn test_routed_clauses_with_a_way_untaken_are_held_to_their_bar() {
    let clause = |class: &'static str, ran: bool, untaken: bool| ClauseRuns {
        at: "c".into(),
        class,
        ran,
        taken: vec![("its then branch", true), ("its else branch", !untaken)],
        ..ClauseRuns::default()
    };
    let bar = |class: &str| {
        UNTAKEN_BARS
            .iter()
            .find(|(c, _, _)| *c == class)
            .map_or(0, |(_, per_hundred, room)| per_hundred + room)
    };
    let runs = |class: &'static str, untaken: usize| -> Vec<ClauseRuns> {
        let mut runs: Vec<ClauseRuns> =
            (0..100).map(|k| clause(class, true, k < untaken)).collect();
        runs.extend((0..50).map(|_| clause(class, false, true)));
        runs
    };
    let most = bar(ENTRY_ENSURES);
    assert!(most > 0);
    assert_eq!(
        routed_runs_problems(&runs(ENTRY_ENSURES, most)),
        Vec::<String>::new()
    );
    let over = routed_runs_problems(&runs(ENTRY_ENSURES, most + 1));
    assert_eq!(over.len(), 1, "{:?}", over);
    assert!(
        over[0].starts_with(&format!(
            "{} of the 100 {} clauses a run evaluates leave a way no run takes, above the {}",
            most + 1,
            ENTRY_ENSURES,
            most
        )),
        "{}",
        over[0]
    );
    assert_eq!(bar(ENTRY_REQUIRES), 0);
    assert_eq!(routed_runs_problems(&runs(ENTRY_REQUIRES, 1)).len(), 1);
}

/// The rows of [`way_row`] and [`TRAPPING_REQUIRES`] the runs of a program reach, when it holds
/// a routed function, and what they take of each routed clause: each call the comparison makes
/// of it runs on the reference interpreter, traced, and a way of a clause is taken when a run
/// evaluates its value, and a `requires` stops a call when the call ends in its trap.
/// `routings` is each function's, as `module` has them. An error when the runs do not run.
fn reached_routed_runs(
    seed: u64,
    text: &str,
    module: &CheckedModule,
    routings: &[Routing],
    public: &[bool],
    seen: &mut BTreeSet<String>,
) -> Result<Vec<ClauseRuns>, String> {
    if !routings.iter().any(|r| r.checked || r.prove) {
        return Ok(Vec::new());
    }
    if routings.len() != module.functions.len() {
        return Err(format!(
            "seed {}: {} functions parse and {} check",
            seed,
            routings.len(),
            module.functions.len()
        ));
    }
    // Each routed clause, with its function's index and its text, as a `requires`'s trap says it.
    let mut clauses: Vec<(usize, RoutedClause, String)> = Vec::new();
    for (k, f) in module.functions.iter().enumerate() {
        let (Some(f), true) = (f, routings[k].checked || routings[k].prove) else {
            continue;
        };
        for c in routed_clauses(&f.syntax) {
            let text = f.syntax.tree.render_expr(c.clause);
            clauses.push((k, c, text));
        }
    }
    let mut runs: Vec<ClauseRuns> = clauses
        .iter()
        .map(|(k, c, _)| {
            let entry = public.get(*k).copied().unwrap_or(false);
            let span = module.functions[*k]
                .as_ref()
                .map(|f| f.syntax.tree.expr(c.clause).span);
            ClauseRuns {
                at: format!(
                    "seed {} `{}` {} at {}",
                    seed,
                    module.names[*k],
                    c.kind,
                    span.map_or(String::new(), |s| format!("{}:{}", s.line, s.col))
                ),
                class: class(c.kind, entry, c.first_loop),
                ..ClauseRuns::default()
            }
        })
        .collect();
    let calls = generate::calls(text, module, seed, super::differential::PLAN);
    let mut given: BTreeSet<(usize, u32)> = BTreeSet::new();
    let ran = mente_interp::with_calls(module, super::differential::STEPS, |interp| {
        for (name, args) in &calls {
            let values = args.iter().map(|(_, v)| v.clone()).collect();
            let mut this_call: BTreeSet<(usize, u32)> = BTreeSet::new();
            let outcome = interp.traced(name, values, &mut |e| {
                if let Event::Given { function, expr, .. } = e {
                    this_call.insert((function, expr.0));
                }
            });
            let stopped = match outcome {
                Err(Stop::Trap(Trap {
                    kind: TrapKind::Requires(clause),
                    ..
                })) => Some(clause),
                _ => None,
            };
            for ((k, c, text), run) in clauses.iter().zip(runs.iter_mut()) {
                if c.kind != "requires" || !this_call.contains(&(*k, c.clause.0)) {
                    continue;
                }
                if stopped.as_ref() == Some(text) {
                    run.broke += 1;
                } else {
                    run.held += 1;
                }
            }
            given.extend(this_call);
        }
    });
    if let Err(stop) = ran {
        return Err(format!(
            "seed {}: the runs of its routed clauses did not run: {}",
            seed, stop
        ));
    }
    for ((k, c, _), run) in clauses.iter().zip(runs.iter_mut()) {
        run.ran = given.contains(&(*k, c.clause.0));
        for (way, value) in &c.ways {
            let taken = value.is_some_and(|v| given.contains(&(*k, v.0)));
            run.taken.push((way, taken));
            if taken {
                seen.insert(way_row(c.kind, c.form, way));
            }
        }
        if c.kind == "requires" && routings[*k].checked && run.broke > 0 {
            let entry = public.get(*k).copied().unwrap_or(false);
            seen.insert(
                if entry {
                    TRAPPING_REQUIRES
                } else {
                    TRAPPING_CALLED_REQUIRES
                }
                .to_string(),
            );
        }
    }
    Ok(runs)
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

/// Whether `e` is a value `past` holds of, or arithmetic on one.
fn computed_from(tree: &Tree, e: ExprId, past: &dyn Fn(ExprId) -> bool) -> bool {
    past(e)
        || match &tree.expr(e).kind {
            ExprKind::Binary { lhs, rhs, .. } => {
                computed_from(tree, *lhs, past) || computed_from(tree, *rhs, past)
            }
            ExprKind::Unary { operand, .. } => computed_from(tree, *operand, past),
            _ => false,
        }
}

/// The row of [`SHIFTS`], when a checked function reaches it.
fn reached_shifts(f: &CheckedFn, seen: &mut BTreeSet<String>) {
    let ranges = wide::intervals(&f.syntax, &f.resolution, &f.typing);
    // Whether a bound lies within 64 bits, of either sign.
    let narrow = |(neg, mag): (bool, u128)| mag <= if neg { 1 << 63 } else { u128::from(u64::MAX) };
    for e in &f.syntax.tree.exprs {
        let ExprKind::Binary {
            op: BinaryOp::Shr,
            lhs,
            rhs,
        } = &e.kind
        else {
            continue;
        };
        let wide_type = matches!(
            &f.typing.exprs[lhs.0 as usize],
            Ty::Int(Int::One(t) | Int::Mixed(Some(t))) if t.bits == 128
        );
        let kept_narrow = ranges[lhs.0 as usize].is_some_and(|i| narrow(i.lo) && narrow(i.hi));
        let far = matches!(
            ranges[rhs.0 as usize],
            Some(Interval { lo: (false, k), .. }) if k >= 64
        );
        if wide_type && kept_narrow && far {
            seen.insert("right shift past the bits its value is kept in".to_string());
        }
    }
}

/// The rows of [`LITERALS`] a checked function reaches.
fn reached_literals(f: &CheckedFn, seen: &mut BTreeSet<String>) {
    for (k, e) in f.syntax.tree.exprs.iter().enumerate() {
        let bits = match &e.kind {
            ExprKind::Unary { op, .. } => *op == UnaryOp::BitNot,
            ExprKind::Binary { op, .. } => matches!(
                op,
                BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::Shl
                    | BinaryOp::Shr
            ),
            _ => false,
        };
        let Some(taken) = f.typing.literals.get(k).copied().flatten().filter(|_| bits) else {
            continue;
        };
        let narrowed = matches!(f.typing.places[k], Some(Ty::Int(_)));
        seen.insert(
            match (taken.from, narrowed) {
                (From::Operands, _) => {
                    "bit operation on literals alone, beside an operand of a type"
                }
                (From::Alone, true) => "bit operation on literals alone, narrowed into a place",
                (From::Alone, false) => "bit operation on literals alone, of no type but i64",
                // `E0212`: no checked program holds one.
                (From::Unheld, _) => {
                    "bit operation on literals alone, among integers no type holds"
                }
            }
            .to_string(),
        );
    }
}

/// The rows of [`EXPRESSIONS`] a checked function reaches: an operand of an arithmetic operation
/// that is literals alone and takes another type than the operation's own; and a literal in the
/// value of an arithmetic compound assignment, `m += a + ~3` with a `u32` `m` and a `u8` `a`,
/// that takes the type of the place and the value together where that is wider than the
/// value's own.
fn reached_expressions(f: &CheckedFn, seen: &mut BTreeSet<String>) {
    let typed = |e: ExprId| match &f.typing.exprs[e.0 as usize] {
        Ty::Int(Int::One(t) | Int::Mixed(Some(t))) => Some(*t),
        _ => None,
    };
    for (k, e) in f.syntax.tree.exprs.iter().enumerate() {
        let ExprKind::Binary { op, lhs, rhs } = &e.kind else {
            continue;
        };
        if !matches!(
            op,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem
        ) {
            continue;
        }
        let own = typed(ExprId(k as u32));
        for x in [*lhs, *rhs] {
            let taken = f.typing.literals.get(x.0 as usize).copied().flatten();
            if let (Some(t), Some(own)) = (taken, own) {
                if t.from == From::Operands && t.ty != own {
                    seen.insert(
                        "literal that takes a type from beyond its own operation".to_string(),
                    );
                }
            }
        }
    }
    let tree = &f.syntax.tree;
    for s in &tree.stmts {
        let StmtKind::Assign {
            place,
            op: AssignOp::Compound(op),
            value,
        } = &s.kind
        else {
            continue;
        };
        if !matches!(
            op,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem
        ) {
            continue;
        }
        let int = |e: &ExprId| match &f.typing.exprs[e.0 as usize] {
            Ty::Int(i) => Some(*i),
            _ => None,
        };
        let (Some(at), Some(own)) = (int(place), int(value)) else {
            continue;
        };
        let holder = |i: Int| match i {
            Int::One(t) | Int::Mixed(Some(t)) => Some(t),
            Int::Literal | Int::Mixed(None) => None,
        };
        let (Some(wide), Some(own)) = (holder(at.join(own)), holder(own)) else {
            continue;
        };
        let span = tree.expr(*value).span;
        let widened = tree.exprs.iter().enumerate().any(|(k, x)| {
            span.start <= x.span.start
                && x.span.end <= span.end
                && f.typing.literals[k].is_some_and(|t| t.from == From::Operands && t.ty == wide)
        });
        if wide != own && widened {
            seen.insert(
                "literal of a compound assignment that takes its place's wider type".to_string(),
            );
        }
    }
}

/// The rows of [`COMPARISONS`] a checked function reaches.
fn reached_comparisons(f: &CheckedFn, seen: &mut BTreeSet<String>) {
    let taken = |e: ExprId| f.typing.literals.get(e.0 as usize).copied().flatten();
    let typed = |e: ExprId| match &f.typing.exprs[e.0 as usize] {
        Ty::Int(Int::One(t) | Int::Mixed(Some(t))) => Some(*t),
        _ => None,
    };
    for e in &f.syntax.tree.exprs {
        let ExprKind::Binary { op, lhs, rhs } = &e.kind else {
            continue;
        };
        if !matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        ) {
            continue;
        }
        let mut reach = |name: &str| seen.insert(name.to_string());
        match (taken(*lhs), taken(*rhs)) {
            (Some(l), None) | (None, Some(l)) if l.from == From::Unheld => {
                reach("literal compared exactly beside integers no type holds");
            }
            (Some(l), Some(r)) if l.from == From::Alone && r.from == From::Alone => {
                reach("literals alone compared");
            }
            (Some(l), None) if l.from == From::Operands => {
                reach("literal compared with an operand of a type");
                reach("literal on the left of a comparison");
            }
            (None, Some(r)) if r.from == From::Operands => {
                reach("literal compared with an operand of a type");
            }
            (None, None) => {
                if let (Some(a), Some(b)) = (typed(*lhs), typed(*rhs)) {
                    if a != b {
                        reach("operands of two types compared");
                    }
                }
            }
            _ => {}
        }
    }
}

/// The rows of [`TRAPS`] a checked function reaches.
fn reached_traps(f: &CheckedFn, seen: &mut BTreeSet<String>) {
    let tree = &f.syntax.tree;
    let ranges = wide::intervals(&f.syntax, &f.resolution, &f.typing);
    // The one value an expression has, when its static range is that of a value never negative.
    let value = |e: ExprId| match ranges[e.0 as usize] {
        Some(Interval {
            lo: (false, lo),
            hi: (false, hi),
        }) if lo == hi => Some(lo),
        _ => None,
    };
    for e in &tree.exprs {
        match &e.kind {
            ExprKind::Binary {
                op: BinaryOp::Div | BinaryOp::Rem,
                rhs,
                ..
            } if value(*rhs) == Some(0) => {
                seen.insert("divisor that is always zero".to_string());
            }
            ExprKind::Binary {
                op: BinaryOp::Shl | BinaryOp::Shr,
                lhs,
                rhs,
            } => {
                let width = match &f.typing.exprs[lhs.0 as usize] {
                    Ty::Int(Int::One(t) | Int::Mixed(Some(t))) => Some(u128::from(t.bits)),
                    _ => None,
                };
                if width.zip(value(*rhs)).is_some_and(|(w, k)| k >= w) {
                    seen.insert("shift by its type's width or more".to_string());
                }
            }
            ExprKind::Index { base, index } => {
                if let Ty::Array(n, _) = &f.typing.exprs[base.0 as usize] {
                    if value(*index) == Some(*n) {
                        seen.insert("index at its array's length".to_string());
                    }
                }
            }
            _ => {}
        }
    }
}

/// The rows of [`CANDIDATES`] a checked function reaches: a remainder whose divisor's static
/// range holds 0, or has none, written inside a loop's condition or inside the condition of a
/// `guard` inside a loop's body, by their spans; and what leaves, assigns or calls there.
fn reached_candidates(module: &CheckedModule, f: &CheckedFn, seen: &mut BTreeSet<String>) {
    let tree = &f.syntax.tree;
    let ranges = wide::intervals(&f.syntax, &f.resolution, &f.typing);
    let holds_zero = |e: ExprId| match ranges[e.0 as usize] {
        Some(Interval { lo, hi }) => (lo.0 || lo.1 == 0) && (!hi.0 || hi.1 == 0),
        None => true,
    };
    let remainders: Vec<(usize, usize)> = tree
        .exprs
        .iter()
        .filter_map(|e| match &e.kind {
            ExprKind::Binary {
                op: BinaryOp::Rem,
                rhs,
                ..
            } if holds_zero(*rhs) => Some((e.span.start, e.span.end)),
            _ => None,
        })
        .collect();
    let inside =
        |s: mente_syntax::Span| remainders.iter().any(|&(a, b)| s.start <= a && b <= s.end);
    let bodies: Vec<mente_syntax::Span> = tree
        .stmts
        .iter()
        .filter_map(|s| match &s.kind {
            StmtKind::While { cond, body, .. } => {
                if inside(tree.expr(*cond).span) {
                    seen.insert("remainder that can trap in a loop's condition".to_string());
                }
                Some(tree.block(*body).span)
            }
            _ => None,
        })
        .collect();
    for s in &tree.stmts {
        if let StmtKind::Guard { cond, .. } = &s.kind {
            let c = tree.expr(*cond).span;
            if inside(c) && bodies.iter().any(|b| b.start <= c.start && c.end <= b.end) {
                seen.insert("remainder that can trap in a guard of a loop's body".to_string());
            }
        }
    }
    reached_no_candidates(module, f, &bodies, seen);
}

/// The rows of [`CANDIDATES`] for what is no candidate by its syntax, a checked function reaches
/// inside a loop's condition or a guard's condition inside `bodies`, its loops' bodies; and the
/// row of [`CONDITIONS`], which none reaches.
fn reached_no_candidates(
    module: &CheckedModule,
    f: &CheckedFn,
    bodies: &[mente_syntax::Span],
    seen: &mut BTreeSet<String>,
) {
    let tree = &f.syntax.tree;
    let within =
        |s: mente_syntax::Span, c: mente_syntax::Span| c.start <= s.start && s.end <= c.end;
    // What leaves, by a statement or at `todo` or `unreachable`, and what breaks.
    let mut leaving: Vec<mente_syntax::Span> = Vec::new();
    let mut breaking: Vec<mente_syntax::Span> = Vec::new();
    for s in &tree.stmts {
        match &s.kind {
            StmtKind::Break => {
                leaving.push(s.span);
                breaking.push(s.span);
            }
            StmtKind::Continue | StmtKind::Return(_) => leaving.push(s.span),
            _ => {}
        }
    }
    // A call of a function whose `ensures` names one of its parameters.
    let mut bounding: Vec<mente_syntax::Span> = Vec::new();
    for (i, e) in tree.exprs.iter().enumerate() {
        match &e.kind {
            ExprKind::Todo(_) | ExprKind::Unreachable(_) => leaving.push(e.span),
            ExprKind::Call { .. } => {
                let callee = f.resolution.calls[i].and_then(|c| module.functions[c].as_ref());
                if callee.is_some_and(|c| ensures_names_a_parameter(&c.syntax)) {
                    bounding.push(e.span);
                }
            }
            _ => {}
        }
    }
    let any_in =
        |found: &[mente_syntax::Span], c: mente_syntax::Span| found.iter().any(|&s| within(s, c));
    // A guard `p and q` in a loop's body whose `q` holds a remainder of a name the body assigns
    // before the guard.
    let assigned: Vec<(&str, mente_syntax::Span)> = tree
        .stmts
        .iter()
        .filter_map(|s| match &s.kind {
            StmtKind::Assign { place, .. } => match &tree.expr(*place).kind {
                ExprKind::Name(n) => Some((n.as_str(), s.span)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    let remainders_of: Vec<(&str, mente_syntax::Span)> = tree
        .exprs
        .iter()
        .filter_map(|e| match &e.kind {
            ExprKind::Binary {
                op: mente_syntax::tree::BinaryOp::Rem,
                lhs,
                ..
            } => match &tree.expr(*lhs).kind {
                ExprKind::Name(n) => Some((n.as_str(), e.span)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    for s in &tree.stmts {
        let StmtKind::Guard { cond, .. } = &s.kind else {
            continue;
        };
        let ExprKind::Binary {
            op: mente_syntax::tree::BinaryOp::And,
            rhs,
            ..
        } = &tree.expr(*cond).kind
        else {
            continue;
        };
        let right = tree.expr(*rhs).span;
        let Some(body) = bodies.iter().find(|&&b| within(s.span, b)) else {
            continue;
        };
        let stale = remainders_of.iter().any(|&(n, at)| {
            within(at, right)
                && assigned
                    .iter()
                    .any(|&(m, a)| m == n && within(a, *body) && a.end <= s.span.start)
        });
        if stale {
            seen.insert(
                "guard of a loop's body that reads, after an and, a remainder of a binding the round assigned"
                    .to_string(),
            );
        }
    }
    for s in &tree.stmts {
        match &s.kind {
            StmtKind::While { cond, .. } => {
                let c = tree.expr(*cond).span;
                if any_in(&leaving, c) {
                    seen.insert("conjunct of a loop's condition that may leave".to_string());
                }
                if any_in(&bounding, c) {
                    seen.insert(
                        "conjunct of a loop's condition that calls a function whose ensures bounds its argument"
                            .to_string(),
                    );
                }
                // Inside another loop's body, which its own body is not.
                let nested = bodies.iter().any(|&b| within(s.span, b));
                if nested && any_in(&breaking, c) {
                    seen.insert("break in the condition of a loop inside a loop".to_string());
                }
            }
            StmtKind::Guard { cond, .. } => {
                let c = tree.expr(*cond).span;
                if bodies.iter().any(|&b| within(c, b)) && any_in(&leaving, c) {
                    seen.insert("guard of a loop's body whose condition may leave".to_string());
                }
            }
            _ => {}
        }
    }
    if a_condition_changes_state(&f.syntax) {
        seen.insert("condition that assigns or passes mut".to_string());
    }
}

/// Whether a condition of `f` holds an assignment or a `mut` argument, by their spans: the row of
/// [`CONDITIONS`]. A condition is the condition of a `while`, a `guard` or an `if`, a contract
/// clause, an `and` or an `or`, and the scrutinee of a `match` on a `bool`, which this tells by
/// a `true` or `false` pattern among its arms.
fn a_condition_changes_state(f: &FnSyntax) -> bool {
    let tree = &f.tree;
    let mut changing: Vec<mente_syntax::Span> = tree
        .stmts
        .iter()
        .filter(|s| matches!(s.kind, StmtKind::Assign { .. }))
        .map(|s| s.span)
        .collect();
    for e in &tree.exprs {
        let args = match &e.kind {
            ExprKind::Call { args, .. }
            | ExprKind::Variant {
                payload: Payload::Positional(args),
                ..
            } => args.as_slice(),
            _ => &[],
        };
        changing.extend(
            args.iter()
                .filter(|a| a.is_mut)
                .map(|a| tree.expr(a.value).span),
        );
    }
    let mut conditions: Vec<ExprId> = f.contracts.iter().map(|c| c.expr).collect();
    for s in &tree.stmts {
        match &s.kind {
            StmtKind::While {
                cond,
                invariants,
                decreases,
                ..
            } => {
                conditions.push(*cond);
                conditions.extend(invariants.iter().chain(decreases));
            }
            StmtKind::Guard { cond, .. } => conditions.push(*cond),
            _ => {}
        }
    }
    for (i, e) in tree.exprs.iter().enumerate() {
        match &e.kind {
            ExprKind::If { cond, .. } => conditions.push(*cond),
            ExprKind::Binary {
                op: BinaryOp::And | BinaryOp::Or,
                ..
            } => conditions.push(ExprId(i as u32)),
            ExprKind::Match { scrutinee, arms }
                if arms
                    .iter()
                    .any(|a| matches!(a.pattern.kind, PatternKind::Bool(_))) =>
            {
                conditions.push(*scrutinee)
            }
            _ => {}
        }
    }
    conditions.into_iter().any(|c| {
        let at = tree.expr(c).span;
        changing
            .iter()
            .any(|s| at.start <= s.start && s.end <= at.end)
    })
}

/// The rows of [`ROWS`] a checked function reaches: an array literal whose elements are arrays
/// of integers of no one type, one of them a literal of one integer type; and an `if` or a
/// `match` that gives arrays whose integers, by the checker's hull of them, need more than 64
/// bits together; and an `if` or a `match` with a branch or an arm of no type beside two that
/// give values, an `else if` and a branch that is itself an `if` or a `match` counted as more
/// branches of the one join.
fn reached_rows(f: &CheckedFn, seen: &mut BTreeSet<String>) {
    let tree = &f.syntax.tree;
    let ty = |e: ExprId| &f.typing.exprs[e.0 as usize];
    let ranges = wide::intervals(&f.syntax, &f.resolution, &f.typing);
    for (k, e) in tree.exprs.iter().enumerate() {
        let join = matches!(e.kind, ExprKind::If { .. } | ExprKind::Match { .. });
        if !join || !matches!(ty(ExprId(k as u32)), Ty::Array(..)) {
            continue;
        }
        let Some(Interval { lo, hi }) = ranges[k] else {
            continue;
        };
        let above = |limit: u128| !hi.0 && hi.1 > limit;
        if (lo.0 && above(i64::MAX as u128)) || above(u64::MAX as u128) {
            seen.insert("branches of arrays meeting in a 128-bit machine type".to_string());
        }
    }
    // The values the branches of the join `e` give, through the joins among them.
    fn values(tree: &Tree, e: ExprId, out: &mut Vec<ExprId>) {
        let tail = |b| match &tree.stmt(*tree.block(b).stmts.last()?).kind {
            StmtKind::Expr(x) => Some(*x),
            _ => None,
        };
        let tails: Vec<Option<ExprId>> = match &tree.expr(e).kind {
            ExprKind::If {
                then, otherwise, ..
            } => vec![
                tail(*then),
                match otherwise {
                    Some(Else::Block(b)) => tail(*b),
                    Some(Else::If(i)) => Some(*i),
                    None => None,
                },
            ],
            ExprKind::Match { arms, .. } => arms
                .iter()
                .map(|a| match a.body {
                    ArmBody::Expr(x) => Some(x),
                    ArmBody::Block(b) => tail(b),
                })
                .collect(),
            _ => Vec::new(),
        };
        for t in tails.into_iter().flatten() {
            match tree.expr(t).kind {
                ExprKind::If { .. } | ExprKind::Match { .. } => values(tree, t, out),
                _ => out.push(t),
            }
        }
    }
    for (k, e) in tree.exprs.iter().enumerate() {
        if !matches!(e.kind, ExprKind::If { .. } | ExprKind::Match { .. }) {
            continue;
        }
        let mut given = Vec::new();
        values(tree, ExprId(k as u32), &mut given);
        let none = given.iter().filter(|v| ty(**v).has_no_value()).count();
        if none > 0 && given.len() - none >= 2 {
            seen.insert("branch of no type beside two that give values".to_string());
        }
    }
    // The integer type an array of integers holds, at its innermost depth.
    fn leaf(t: &Ty) -> Option<&Int> {
        match t {
            Ty::Array(_, e) => leaf(e),
            Ty::Int(i) => Some(i),
            _ => None,
        }
    }
    for (k, e) in tree.exprs.iter().enumerate() {
        let ExprKind::Array(items) = &e.kind else {
            continue;
        };
        let outer = ty(ExprId(k as u32));
        let mixed = matches!(outer, Ty::Array(_, inner) if matches!(**inner, Ty::Array(..)))
            && matches!(leaf(outer), Some(Int::Mixed(None)));
        let one = items.iter().any(|&i| {
            tree.expr(i).kind.is_array_literal() && matches!(leaf(ty(i)), Some(Int::One(_)))
        });
        if mixed && one {
            seen.insert("row of one integer type beside rows of others".to_string());
        }
    }
}

/// Whether a function's `ensures` names one of its parameters, which then bounds the argument a
/// call passes it, where the call returns.
fn ensures_names_a_parameter(f: &mente_syntax::tree::FnSyntax) -> bool {
    let params: Vec<&str> = f.params.iter().map(|p| p.name.as_str()).collect();
    f.contracts
        .iter()
        .filter(|c| c.kind == mente_syntax::tree::ContractKind::Ensures)
        .any(|c| {
            let at = f.tree.expr(c.expr).span;
            f.tree.exprs.iter().any(|e| {
                matches!(&e.kind, ExprKind::Name(n) if params.contains(&n.as_str()))
                    && at.start <= e.span.start
                    && e.span.end <= at.end
            })
        })
}

/// The rows of [`EDGES`] a checked function reaches. A value lies past the domain when its
/// static range, as `E0222` computes it, lies wholly at or beyond the limit on one side of
/// zero: no value of it is one Tier 1 holds exactly.
fn reached_edges(f: &CheckedFn, seen: &mut BTreeSet<String>) {
    let tree = &f.syntax.tree;
    let limit = INF.unsigned_abs();
    let ranges = wide::intervals(&f.syntax, &f.resolution, &f.typing);
    // 1 for a value wholly past the limit, -1 for one wholly below its negation, else 0.
    let side = |e: ExprId| -> i8 {
        match ranges[e.0 as usize] {
            Some(Interval { lo: (false, m), .. }) if m >= limit => 1,
            Some(Interval { hi: (true, m), .. }) if m >= limit => -1,
            _ => 0,
        }
    };
    let past = |e: ExprId| side(e) != 0;
    let mut mark = |name: &str| {
        seen.insert(name.to_string());
    };
    for (k, e) in tree.exprs.iter().enumerate() {
        match &e.kind {
            ExprKind::Binary { op, lhs, rhs } => {
                let (l, r) = (side(*lhs), side(*rhs));
                match op {
                    BinaryOp::Mul if l == 0 && r == 0 && past(ExprId(k as u32)) => {
                        mark("product past Tier 1's domain")
                    }
                    BinaryOp::Div => {
                        if l != 0 {
                            mark("quotient of a value past Tier 1's domain");
                        }
                        if r != 0 {
                            mark("quotient by a value past Tier 1's domain");
                        }
                    }
                    BinaryOp::Sub if l != 0 || r != 0 => {
                        mark("difference with a value past Tier 1's domain")
                    }
                    BinaryOp::Rem if l != 0 || r != 0 => {
                        mark("remainder with a value past Tier 1's domain")
                    }
                    _ => {}
                }
                if matches!(op, BinaryOp::Div | BinaryOp::Sub | BinaryOp::Rem) && (l < 0 || r < 0) {
                    mark("operand below Tier 1's domain");
                }
            }
            ExprKind::Index { index, .. } if computed_from(tree, *index, &past) => {
                mark("index computed from a value past Tier 1's domain")
            }
            _ => {}
        }
    }
    for s in &tree.stmts {
        let value = match &s.kind {
            StmtKind::Binding {
                ty: Some(_), init, ..
            } => Some(*init),
            StmtKind::Assign { value, .. } => Some(*value),
            _ => None,
        };
        // The value itself may fit its place: one wholly past the domain fits no type but a
        // 128-bit one.
        if value.is_some_and(|v| !past(v) && computed_from(tree, v, &past)) {
            mark("narrowing of a value computed from one past Tier 1's domain");
        }
    }
}

/// The pairs of parameters `f` relates, each with whether it is by their sum: a `requires`
/// of the form `a op b` or `a + b <= c`, and a refinement bound that names a parameter.
fn related_parameters(f: &FnSyntax) -> Vec<(usize, usize, bool)> {
    let tree = &f.tree;
    let index = |name: &str| f.params.iter().position(|p| p.name == name);
    let named = |e: ExprId| match &tree.expr(e).kind {
        ExprKind::Name(n) => index(n),
        _ => None,
    };
    let mut out = Vec::new();
    for c in &f.contracts {
        let (ContractKind::Requires, ExprKind::Binary { lhs, rhs, .. }) =
            (c.kind, &tree.expr(c.expr).kind)
        else {
            continue;
        };
        if let (Some(a), Some(b)) = (named(*lhs), named(*rhs)) {
            out.push((a, b, false));
        }
        if let ExprKind::Binary {
            op: BinaryOp::Add,
            lhs: a,
            rhs: b,
        } = &tree.expr(*lhs).kind
        {
            if let (Some(a), Some(b)) = (named(*a), named(*b)) {
                out.push((a, b, true));
            }
        }
    }
    for (k, p) in f.params.iter().enumerate() {
        if let TypeKind::Int {
            refinement: Some(Refinement { lo, hi, .. }),
            ..
        } = &tree.ty(p.ty).kind
        {
            for b in [lo, hi] {
                if let Bound::Name(n) = b {
                    out.extend(index(n).map(|i| (k, i, false)));
                }
            }
        }
    }
    out
}

/// The bindings an argument names, through the arithmetic it is written with.
fn named_in(tree: &Tree, e: ExprId, out: &mut BTreeSet<String>) {
    match &tree.expr(e).kind {
        ExprKind::Name(n) => {
            out.insert(n.clone());
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            named_in(tree, *lhs, out);
            named_in(tree, *rhs, out);
        }
        ExprKind::Unary { operand, .. } => named_in(tree, *operand, out),
        _ => {}
    }
}

/// What the calls of `f` ask Tier 1 to relate: the row of [`RELATIONS`] for a call whose
/// callee relates two parameters and whose arguments for them both name a binding; and, as
/// the lines returned, the calls Tier 1 cannot prove. It relates two names only in a function
/// of at most [`RELATIONAL_LIMIT`] variables, and holds each one's bounds alone in a function
/// of more, where a `guard` on two names proves nothing of them at the call. And it holds no
/// sum of a name with itself in any function.
fn reached_relations(
    module: &CheckedModule,
    f: &CheckedFn,
    seen: &mut BTreeSet<String>,
) -> Vec<String> {
    let tree = &f.syntax.tree;
    let mut wrong = Vec::new();
    for (i, e) in tree.exprs.iter().enumerate() {
        let ExprKind::Call { args, .. } = &e.kind else {
            continue;
        };
        let Some(callee) = f.resolution.calls[i].and_then(|c| module.functions[c].as_ref()) else {
            continue;
        };
        for (a, b, sum) in related_parameters(&callee.syntax) {
            let (mut first, mut second) = (BTreeSet::new(), BTreeSet::new());
            named_in(tree, args[a].value, &mut first);
            named_in(tree, args[b].value, &mut second);
            if first.is_empty() || second.is_empty() {
                continue;
            }
            seen.insert("call that relates two names".to_string());
            if f.variables > RELATIONAL_LIMIT {
                wrong.push(format!(
                    "`{}`, of {} variables where Tier 1 relates {}, passes two names to \
                     parameters `{}` relates",
                    f.syntax.name, f.variables, RELATIONAL_LIMIT, callee.syntax.name
                ));
            }
            if sum && !first.is_disjoint(&second) {
                wrong.push(format!(
                    "`{}` passes one name twice to parameters `{}` adds",
                    f.syntax.name, callee.syntax.name
                ));
            }
        }
    }
    wrong
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
fn functions(src: &str) -> Vec<(FnSyntax, Routing, bool)> {
    let module = mente_syntax::parse_module(src).unwrap_or_else(|d| panic!("{:?}\n{}", d, src));
    let parsed = mente_sema::parse_qualified(src, &module);
    let diags: Vec<_> = parsed.iter().flat_map(|p| p.errors.iter()).collect();
    assert!(diags.is_empty(), "{:?}\n{}", diags, src);
    // Each function's routing, from its attributes, as the checker reads it, and whether it is
    // public.
    let routings = module.items.iter().filter_map(|i| match i {
        mente_syntax::ItemDecl::Fn(f) => Some((Routing::of(&f.attrs), f.is_pub)),
        _ => None,
    });
    parsed
        .into_iter()
        .zip(routings)
        .filter_map(|(p, (r, public))| Some((p.syntax?, r, public)))
        .collect()
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
pub(super) fn compound_assignments() -> Vec<String> {
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
        EDGES,
        RELATIONS,
        TRAPS,
        CANDIDATES,
        CONDITIONS,
        ROWS,
        LITERALS,
        COMPARISONS,
        EXPRESSIONS,
        SHIFTS,
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
    for name in routed_rows() {
        assert!(all.insert(name.clone()), "`{}` is named twice", name);
    }
    let compound = compound_assignments();
    assert!(!compound.is_empty(), "no compound assignment parses");
    for name in compound {
        assert!(all.insert(name.clone()), "`{}` is named twice", name);
    }
    all
}

/// Every generated program passes every check and lowers, so that the three paths run it and
/// none is left out unseen: nothing refuses one but a narrowing Tier 1 proves always traps
/// (`E0508`), whose function is generated again once the reference interpreter has run the
/// claim ([`generate::checked`]), within the bars every run of generated programs is held to
/// ([`generate::beyond_the_bars`]). Some function is generated again, and its program comes
/// again from its seed and the attempt each function is at. At least
/// [`MIN_SEEDS_PER_ROW`] of the programs reach each construct of M1's grammar but those
/// [`NOT_GENERATED`] names, which none reaches.
#[test]
fn test_generated_programs_reach_every_construct_of_the_grammar() {
    let grammar = grammar();
    // The seeds whose program reaches each construct.
    let mut seeds: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut claims = Vec::new();
    let mut regenerated = 0;
    // Each seed's program is checked on its own, on as many threads as the machine has.
    let programs = mente_threads::parallel((0..SEEDS).collect(), |seed| {
        let c = generate::checked(seed);
        let mut seen = BTreeSet::new();
        let mut problems = c.problems;
        let mut routed = Vec::new();
        if let Some(module) = &c.module {
            let text = &c.program.text;
            let fns = functions(text);
            for (f, routing, _) in &fns {
                reached(f, &mut seen);
                reached_routed(f, *routing, &mut seen);
            }
            let routings: Vec<Routing> = fns.iter().map(|(_, r, _)| *r).collect();
            let public: Vec<bool> = fns.iter().map(|(_, _, p)| *p).collect();
            match reached_routed_runs(seed, text, module, &routings, &public, &mut seen) {
                Ok(runs) => routed = runs,
                Err(wrong) => problems.push(wrong),
            }
            structs(text)
                .iter()
                .for_each(|s| reached_struct(s, &mut seen));
            enums(text).iter().for_each(|e| reached_enum(e, &mut seen));
            for f in module.functions.iter().flatten() {
                reached_edges(f, &mut seen);
                reached_traps(f, &mut seen);
                reached_candidates(module, f, &mut seen);
                reached_rows(f, &mut seen);
                reached_literals(f, &mut seen);
                reached_comparisons(f, &mut seen);
                reached_expressions(f, &mut seen);
                reached_shifts(f, &mut seen);
                for wrong in reached_relations(module, f, &mut seen) {
                    problems.push(format!("seed {}: {}", seed, wrong));
                }
            }
            // What the checker accepts, the lowering lowers: a program it refused would run
            // on the reference interpreter alone.
            if let Err(why) = mente_codegen::lower::lower(module) {
                problems.push(format!(
                    "seed {}: the checker accepts the program and the lowering refuses it: {}",
                    seed, why
                ));
            }
        }
        // A program with a function generated again comes again from its seed and the attempt
        // each function is at, as the replay tests of `differential.rs` hold of their seeds.
        if c.regenerated > 0 && generate::generated(seed, &c.attempts).text != c.program.text {
            problems.push(format!(
                "seed {}: its functions generated again give another program from the same attempts",
                seed
            ));
        }
        let bits = c.program.bits.len();
        (
            seed,
            seen,
            c.claims,
            (c.regenerated, bits, routed),
            problems,
        )
    });
    let mut bits = 0;
    let mut routed = Vec::new();
    for (seed, seen, claimed, (again, written, clauses), wrong) in programs {
        problems.extend(wrong);
        routed.extend(clauses);
        claims.extend(claimed.into_iter().map(|c| (seed, c)));
        regenerated += again;
        bits += written;
        for name in seen {
            seeds.entry(name).or_default().push(seed);
        }
    }
    println!("{}", generate::claims_summary(regenerated, &claims));
    problems.extend(generate::beyond_the_bars(SEEDS, regenerated, &claims));
    // Of each class of routed clause, how many a run evaluates and how many of those leave a way
    // no run takes, which the derivation of its bar measures.
    for &(class, _, _) in UNTAKEN_BARS {
        let ran = routed.iter().filter(|r| r.ran && r.class == class).count();
        let untaken = routed
            .iter()
            .filter(|r| r.ran && r.class == class && r.taken.iter().any(|(_, taken)| !taken))
            .count();
        println!(
            "{}: {} evaluated by a run, {} with a way no run takes",
            class, ran, untaken
        );
    }
    problems.extend(routed_runs_problems(&routed));
    // Some function is generated again, so that a program replayed from its attempts is seen.
    if regenerated == 0 {
        problems.push("no function of these seeds is generated again".to_string());
    }
    // Some bit operation of literals alone is written, so that the check of the type each takes
    // (`generate::literals_as_written`, in `generate::checked`) checks something.
    if bits == 0 {
        problems
            .push("no program of these seeds writes a bit operation of literals alone".to_string());
    }
    assert!(
        problems.is_empty(),
        "generated programs the checker refuses, claims of Tier 1 a run contradicts, or more \
         of them than a run may have:\n{}",
        problems.join("\n")
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
    // The rows the fewest programs reach, with their seeds, so that a row about to be lost
    // shows before it is.
    let mut least: Vec<(usize, &String)> = seeds.iter().map(|(n, s)| (s.len(), n)).collect();
    least.sort();
    println!("the constructs the fewest programs reach:");
    for (count, name) in least.iter().take(RARE_ROWS_SHOWN) {
        println!("  `{}` {}: seeds {:?}", name, count, seeds[*name]);
    }
    println!(
        "the edge of Tier 1's domain, and what is written to trap: {}",
        EDGES
            .iter()
            .chain(TRAPS)
            .chain(LITERALS)
            .chain(SHIFTS)
            .map(|name| format!("`{}` {}", name, seeds.get(*name).map_or(0, |s| s.len())))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let few = too_few(&grammar, &seeds, MIN_SEEDS_PER_ROW);
    assert!(
        few.is_empty(),
        "fewer than {} of the programs of seeds 0 to {} reach each of these:\n{}",
        MIN_SEEDS_PER_ROW,
        SEEDS - 1,
        few.join("\n")
    );
}

/// The constructs of `grammar` that fewer than `bar` programs reach, each with the seeds
/// that do, those [`NOT_GENERATED`] names aside. A construct one or two programs hold is lost
/// by any change to the generator's choices, so each is asked of a few.
fn too_few(
    grammar: &BTreeSet<String>,
    seeds: &BTreeMap<String, Vec<u64>>,
    bar: usize,
) -> Vec<String> {
    grammar
        .iter()
        .filter(|n| NOT_GENERATED.iter().all(|(x, _)| x != n))
        .filter_map(|n| {
            let reach = seeds.get(n).map_or(&[][..], |s| s.as_slice());
            (reach.len() < bar).then(|| format!("`{}` in seeds {:?}", n, reach))
        })
        .collect()
}

/// The bar asks a few programs of every row: at least three, so that no row hangs on one
/// program or two, and a row below the bar is named with its seeds, one no program reaches
/// included; what [`NOT_GENERATED`] excuses is not asked for.
#[test]
fn test_each_row_is_asked_of_at_least_three_programs() {
    const { assert!(MIN_SEEDS_PER_ROW >= 3) };
    let excused = NOT_GENERATED[0].0;
    let grammar: BTreeSet<String> = ["val", "var", "while", excused]
        .iter()
        .map(|n| n.to_string())
        .collect();
    let seeds: BTreeMap<String, Vec<u64>> = [("val", vec![4, 9, 30]), ("var", vec![7, 8])]
        .into_iter()
        .map(|(n, s)| (n.to_string(), s))
        .collect();
    assert_eq!(
        too_few(&grammar, &seeds, 3),
        ["`var` in seeds [7, 8]", "`while` in seeds []"]
    );
    assert_eq!(too_few(&grammar, &seeds, 2), ["`while` in seeds []"]);
    assert_eq!(too_few(&grammar, &seeds, 0), [""; 0]);
    assert_eq!(too_few(&grammar, &seeds, 4).len(), 3);
}

/// The row of a condition that changes state, which no generated program holds since the
/// checker refuses it, is seen where a parsed function holds one: an assignment in a block of an
/// `if` inside a loop's condition, a `mut` argument in a `guard`'s or an `if`'s, in a loop's
/// `invariant`, in an operand of `and`, in the scrutinee of a `match` on a `bool`, and in a
/// function's `requires`; and not where the effect stands outside every condition.
#[test]
fn test_a_condition_that_changes_state_is_seen() {
    let function = |header: &str, body: &str| {
        let src = format!(
            "module t\nfn next(mut i: u8) -> u8 {{\n  i = i / 2\n  i\n}}\n{} {{\n  var i = n\n{}\n  i\n}}\n",
            header, body
        );
        let module = mente_syntax::parse_module(&src).unwrap();
        let parsed = mente_syntax::parse_bodies(&src, &module);
        parsed
            .into_iter()
            .filter_map(|p| p.syntax)
            .next_back()
            .unwrap()
    };
    let plain = "pub fn f(n: u8) -> u8";
    for (header, body) in [
        (
            plain,
            "  while (if n > 3 {\n    i = 0\n    0\n  } else {\n    0\n  }) == 0 {\n    i += 1\n  }",
        ),
        (plain, "  guard next(mut i) > 0 else {\n    return 0\n  }"),
        (
            plain,
            "  val k: u8 = if next(mut i) > 0 { 1 } else { 0 }\n  i += k",
        ),
        (
            plain,
            "  while i > 0 invariant next(mut i) >= 0 {\n    i -= 1\n  }",
        ),
        (plain, "  val c = n > 0 and next(mut i) > 0\n  _ = c"),
        (
            plain,
            "  val k: u8 = match next(mut i) > 0 {\n    true => 1\n    false => 0\n  }\n  i += k",
        ),
        (
            "pub fn f(mut n: u8) -> u8\n  requires next(mut n) > 0",
            "",
        ),
    ] {
        assert!(
            a_condition_changes_state(&function(header, body)),
            "{}",
            body
        );
    }
    for body in [
        "  val ok = next(mut i) > 0\n  if ok {\n    i = 0\n  }",
        "  while i > 0 {\n    _ = next(mut i)\n  }",
        "  val k = match next(mut i) {\n    0 => 1\n    _ => 0\n  }\n  i += k",
    ] {
        assert!(
            !a_condition_changes_state(&function(plain, body)),
            "{}",
            body
        );
    }
}

/// A join counts as "branch of no type beside two that give values" only with a branch or an arm
/// that gives none and two that give one, through an `else if` and a branch that is an `if`
/// itself: one value beside an `unreachable`, as a guard's `if` has, or two values and no branch
/// of no type, does not.
#[test]
fn test_a_branch_of_no_type_counts_beside_two_values_only() {
    let row = "branch of no type beside two that give values";
    let reaches = |body: &str| {
        let source = format!("module m\npub fn f(c: bool, d: bool, k: u8, x: u8) -> u8 = {body}\n");
        let module = mente_sema::check_program(&source).unwrap();
        let mut seen = BTreeSet::new();
        module
            .functions
            .iter()
            .flatten()
            .for_each(|f| reached_rows(f, &mut seen));
        seen.contains(row)
    };
    for body in [
        "if c { x } else if d { 3 } else { unreachable(\"u\") }",
        "if c { unreachable(\"u\") } else if d { x } else { 3 }",
        "if c { x } else {\n  if d { 3 } else { todo(\"t\") }\n}",
        "match k {\n  0 => x\n  1 => 3\n  _ => unreachable(\"u\")\n}",
    ] {
        assert!(reaches(body), "{}", body);
    }
    for body in [
        "if c { x } else { unreachable(\"u\") }",
        "if c { x } else if d { 3 } else { 4 }",
        "match k {\n  0 => x\n  _ => todo(\"t\")\n}",
    ] {
        assert!(!reaches(body), "{}", body);
    }
}

/// The seeds the test reads: the first programs of each mode.
const SEEDS: u64 = 300;

/// How many of the rows the fewest programs reach the test prints with their seeds.
const RARE_ROWS_SHOWN: usize = 12;

/// How many of those seeds' programs must reach each construct: a construct that hangs on one
/// program or two is lost by any change to what the generator chooses at random.
const MIN_SEEDS_PER_ROW: usize = 3;
