//! The shapes a call can stand in, which hold the reference interpreter's figures of a call and
//! of a level of nesting (`mente_interp::host_figures`), on which its bound on host memory rests
//! (`mente_interp::host_bound`). Each shape is a public `q` that calls itself inside one construct,
//! nested to two depths; each call of it must take no more host stack than the figures give it
//! for its nesting.
//!
//! The constructs come from the tables of the generator's coverage (`coverage.rs`): every
//! construct of an expression, an operator, an argument, a statement, an assignment and an arm,
//! each compound assignment, and each place an assignment writes, must stand on the path from a
//! shape's body to its recursive call, unless it holds no expression or no checked program has it
//! ([`HOLDS_NO_CALL`], `coverage::NOT_GENERATED`). So a new construct is measured, or a test fails,
//! without anyone remembering to add it, and each operator and compound assignment is a shape of
//! its own, written from its table.

use std::collections::BTreeSet;

use mente_interp::{Int, Limits, Stop, TrapKind, Value};
use mente_sema::CheckedModule;
use mente_syntax::tree::{AssignOp, ExprKind, StmtKind, Tree};
use mente_syntax::Span;

use super::coverage;

/// The constructs of the tables that hold no expression, so that no call can stand in them.
pub(super) const HOLDS_NO_CALL: &[(&str, &str)] = &[
    ("integer literal", "a literal holds no expression"),
    ("suffixed integer literal", "a literal holds no expression"),
    ("true", "a literal holds no expression"),
    ("false", "a literal holds no expression"),
    ("name", "a name holds no expression"),
    ("todo", "`todo` holds a string, no expression"),
    ("unreachable", "`unreachable` holds a string, no expression"),
    ("variant", "a variant without a payload holds no expression"),
    ("return", "a `return` without a value holds no expression"),
    ("break", "`break` holds no expression"),
    ("continue", "`continue` holds no expression"),
];

/// The places an assignment writes, by the construct its place is.
fn place(kind: &str) -> Option<&'static str> {
    match kind {
        "name" => Some("assignment to a name"),
        "index" => Some("assignment to an index"),
        "field read" => Some("assignment to a field"),
        _ => None,
    }
}

/// The places an assignment can write.
const PLACES: [&str; 3] = [
    "assignment to a name",
    "assignment to an index",
    "assignment to a field",
];

/// A shape: its name, the source of its module with its call `k` levels deep, and the arguments
/// `q` is first called on.
struct Shape {
    name: String,
    text: Box<dyn Fn(usize) -> String>,
    args: Box<dyn Fn(usize) -> Vec<Value>>,
}

/// The first statement of every shape's body, which widens `q`'s frame: the array it discards is
/// held to the end of the statement, 4 KiB as item 25 counts it, so that the budget holds about
/// 250 calls of `q` and a run to its trap takes a moment. It costs a run next to nothing, since
/// a repeat narrows its one element, and the host stack a call takes does not grow with it, since
/// the interpreter keeps a value's elements off the host's stack. It nests no deeper than any
/// shape's call at the first depth, so the module's nesting stays the shape's.
const PAD: &str = "_ = [0_u128; 128]\n";

/// The count `q` is first called on, which no budget reaches.
fn count() -> Vec<Value> {
    vec![Value::Int(Int::from_u128(u128::from(u32::MAX)))]
}

/// `q(n: u32) -> u32`, whose body is [`PAD`], then `open(i)` for each level `i` from the outside
/// in, the call `q(n - 1)`, and `close(i)` from the inside out, after the declarations `before`.
fn nested(
    name: &str,
    before: &'static str,
    open: impl Fn(usize) -> String + 'static,
    close: impl Fn(usize) -> String + 'static,
) -> Shape {
    Shape {
        name: name.to_string(),
        text: Box::new(move |k| {
            let opened: String = (0..k).map(&open).collect();
            let closed: String = (0..k).rev().map(&close).collect();
            format!(
                "module m\n{}pub fn q(n: u32) -> u32 {{\n{}{}q(n - 1){}\n}}\n",
                before, PAD, opened, closed
            )
        }),
        args: Box::new(|_| count()),
    }
}

/// [`nested`] in an `if n > 0` of its own at each level, its block opening with \`head(i)\` and
/// closing with \`tail(i)\` around the call's level.
fn in_blocks(
    name: &str,
    before: &'static str,
    head: impl Fn(usize) -> String + 'static,
    tail: impl Fn(usize) -> String + 'static,
) -> Shape {
    nested(
        name,
        before,
        move |i| format!("if n > 0 {{\n{}", head(i)),
        move |i| format!("{}\n}} else {{\n  0\n}}", tail(i)),
    )
}

const STRUCT: &str = "struct S {\n  x: u32\n}\n";
const ENUM: &str = "enum E {\n  V(a: u32)\n  W\n}\n";
const BUMP: &str = "fn bump(mut x: u32) {\n  x = 0\n}\n";
const G: &str = "fn g(x: u32) -> u32 = x\n";

/// Every shape: the constructs of expressions, statements and arms, each operator and each
/// compound assignment of the tables, a parameter whose type nests, and a call alone.
fn shapes() -> Vec<Shape> {
    let mut out = vec![
        // A call that passes its parameters on, alone at the first depth.
        Shape {
            name: "bare call".into(),
            text: Box::new(|k| {
                let open: String = (1..k).map(|i| format!("if c{} {{\n", i)).collect();
                let params: String = (1..k).map(|i| format!(", c{}: bool", i)).collect();
                let args: String = (1..k).map(|i| format!(", c{}", i)).collect();
                format!(
                    "module m\npub fn q(n: u32{}) -> u32 {{\n{}{}q(n{}){}\n}}\n",
                    params,
                    PAD,
                    open,
                    args,
                    "\n} else {\n  0\n}".repeat(k - 1)
                )
            }),
            args: Box::new(|k| {
                let mut a = vec![Value::Int(Int::from_u128(1))];
                a.extend((1..k).map(|_| Value::Bool(true)));
                a
            }),
        },
        nested(
            "if with else",
            "",
            |_| "if n > 0 {\n".into(),
            |_| "\n} else {\n  0\n}".into(),
        ),
        nested(
            "else if",
            "",
            |_| "if n == 0 {\n  0\n} else if n > 0 {\n".into(),
            |_| "\n} else {\n  0\n}".into(),
        ),
        Shape {
            name: "if without else".into(),
            text: Box::new(|k| {
                format!(
                    "module m\npub fn q(n: u32) -> u32 {{\n{}{}_ = q(n - 1){}\n0\n}}\n",
                    PAD,
                    "if n > 0 {\n".repeat(k),
                    "\n}".repeat(k)
                )
            }),
            args: Box::new(|_| count()),
        },
        nested(
            "match arm",
            "",
            |_| "match n {\n0 => 0\n_ => ".into(),
            |_| "\n}".into(),
        ),
        nested(
            "block arm",
            "",
            |_| "match n {\n0 => 0\n_ => {\n".into(),
            |_| "\n}\n}".into(),
        ),
        nested(
            "match scrutinee",
            "",
            |_| "match (".into(),
            |_| ") {\n0 => 1\n_ => 0\n}".into(),
        ),
        nested("call", G, |_| "g(".into(), |_| ")".into()),
        nested("array literal", "", |_| "[".into(), |_| "][0]".into()),
        nested("repeated array", "", |_| "[".into(), |_| "; 1][0]".into()),
        nested("index", "", |_| "[1, 2][(".into(), |_| ") & 0]".into()),
        nested("struct", STRUCT, |_| "S(x: ".into(), |_| ").x".into()),
        nested(
            "variant",
            ENUM,
            |_| "match E.V(a: ".into(),
            |i| format!(") {{\nE.V(a: v{}) => v{}\nE.W => 0\n}}", i, i),
        ),
        in_blocks(
            "mut argument",
            BUMP,
            |i| format!("var a{}: [1]u32 = [0]\nbump(mut a{}[(", i, i),
            |i| format!(") & 0])\na{}[0]", i),
        ),
        in_blocks(
            "val",
            "",
            |i| format!("val a{} = ", i),
            |i| format!("\na{}", i),
        ),
        in_blocks(
            "val with a type",
            "",
            |i| format!("val a{}: u32 = ", i),
            |i| format!("\na{}", i),
        ),
        in_blocks(
            "var",
            "",
            |i| format!("var a{} = ", i),
            |i| format!("\na{} = a{}\na{}", i, i, i),
        ),
        in_blocks(
            "var with a type",
            "",
            |i| format!("var a{}: u32 = ", i),
            |i| format!("\na{} = a{}\na{}", i, i, i),
        ),
        in_blocks(
            "assignment to a name",
            "",
            |i| format!("var a{}: u32 = 0\na{} = ", i, i),
            |i| format!("\na{}", i),
        ),
        in_blocks(
            "assignment to an index",
            "",
            |i| format!("var a{}: [1]u32 = [0]\na{}[0] = ", i, i),
            |i| format!("\na{}[0]", i),
        ),
        in_blocks(
            "assignment to a field",
            STRUCT,
            |i| format!("var s{}: S = S(x: 0)\ns{}.x = ", i, i),
            |i| format!("\ns{}.x", i),
        ),
        in_blocks(
            "compound assignment to a name",
            "",
            |i| format!("var a{}: u32 = 0\na{} += ", i, i),
            |i| format!("\na{}", i),
        ),
        in_blocks(
            "compound assignment to a field",
            STRUCT,
            |i| format!("var s{}: S = S(x: 0)\ns{}.x += ", i, i),
            |i| format!("\ns{}.x", i),
        ),
        in_blocks("discard", "", |_| "_ = ".into(), |_| "\n0".into()),
        nested(
            "return with a value",
            "",
            |_| "if n > 0 {\nreturn ".into(),
            |_| "\n} else {\n  0\n}".into(),
        ),
        in_blocks(
            "guard",
            "",
            |_| "guard (".into(),
            |_| ") > 0 else {\nreturn 0\n}\n1".into(),
        ),
        in_blocks(
            "while",
            "",
            |i| format!("var w{}: u32 = 0\nwhile w{} < (", i, i),
            |i| format!(") {{\nw{} = w{} + 1\n}}\nw{}", i, i, i),
        ),
        nested("~x", "", |_| "~(".into(), |_| ")".into()),
        nested(
            "not x",
            "",
            |_| "if not ((".into(),
            |_| ") > 0) { 1 } else { 0 }".into(),
        ),
        Shape {
            name: "-x".into(),
            text: Box::new(|k| {
                format!(
                    "module m\npub fn q(n: i64) -> i64 {{\n{}{}q(n - 1){}\n}}\n",
                    PAD,
                    "-(".repeat(k),
                    ")".repeat(k)
                )
            }),
            args: Box::new(|_| vec![Value::Int(Int::from_u128(1 << 40))]),
        },
        Shape {
            name: "narrowing".into(),
            text: Box::new(|k| {
                let ty = format!("{}u32", "[1]".repeat(k));
                format!(
                    "module m\npub fn q(n: u32, a: {}) -> u32 {{\n{}q(n - 1, a)\n}}\n",
                    ty, PAD
                )
            }),
            args: Box::new(|k| {
                let mut a = Value::Int(Int::from_u128(1));
                for _ in 0..k {
                    a = Value::Array(vec![a]);
                }
                vec![Value::Int(Int::from_u128(u128::from(u32::MAX))), a]
            }),
        },
    ];
    // Each operator of the table, its operand the call: a comparison and a logical operator in
    // the condition of an `if`, which gives the shape's integer.
    for &op in coverage::BINARIES {
        let shape = match op {
            "==" | "!=" | "<" | "<=" | ">" | ">=" => nested(
                op,
                "",
                |_| "if (".into(),
                move |_| format!(") {} 1 {{ 1 }} else {{ 0 }}", op),
            ),
            "and" => nested(
                op,
                "",
                |_| "if n > 0 and (".into(),
                |_| ") > 0 { 1 } else { 0 }".into(),
            ),
            "or" => nested(
                op,
                "",
                |_| "if n == 0 or (".into(),
                |_| ") > 0 { 1 } else { 0 }".into(),
            ),
            _ => nested(op, "", |_| "(".into(), move |_| format!(") {} 1", op)),
        };
        out.push(shape);
    }
    // Each compound assignment of the table, to an element, whose place is read and written.
    for op in coverage::compound_assignments() {
        let name = format!("compound assignment {}", op);
        out.push(in_blocks(
            &name,
            "",
            move |i| format!("var a{}: [1]u32 = [1]\na{}[0] {} ", i, i, op),
            |i| format!("\na{}[0]", i),
        ));
    }
    out
}

/// The depths each shape is measured at.
const DEPTHS: [usize; 2] = [1, 6];

/// One shape measured at one depth: the module's nesting, and the most host stack one of its
/// calls took.
struct Measured {
    shape: String,
    nesting: u32,
    per_call: usize,
}

/// Each shape at each depth, run to the budget's trap in this build.
fn measure() -> Result<Vec<Measured>, String> {
    let mut out = Vec::new();
    for shape in shapes() {
        for k in DEPTHS {
            let text = (shape.text)(k);
            let m = mente_sema::check_program_without_proofs(&text).map_err(|d| {
                format!(
                    "the shape `{}` at {} does not check: {:?}\n{}",
                    shape.name,
                    k,
                    d.iter().map(|d| &d.message).collect::<Vec<_>>(),
                    text
                )
            })?;
            let (outcome, per_call) =
                mente_interp::host_per_call(&m, "q", (shape.args)(k), Limits::default());
            // Every shape recurses without end, so its run ends at the budget's trap.
            let ran_out = match &outcome {
                Err(Stop::Trap(t)) => t.kind == TrapKind::StackOverflow,
                Ok(_)
                | Err(
                    Stop::OutOfSteps
                    | Stop::TooLarge(_)
                    | Stop::NoFunction(_)
                    | Stop::BadArguments(_)
                    | Stop::Broken(_),
                ) => false,
            };
            if !ran_out {
                return Err(format!(
                    "the shape `{}` at {} ran to {:?}, not to the budget's trap",
                    shape.name, k, outcome
                ));
            }
            // The pad widens the frame alone: the module nests as deep without it.
            let nesting = mente_interp::nesting(&m);
            let unpadded = mente_sema::check_program_without_proofs(&text.replacen(PAD, "", 1))
                .map(|m| mente_interp::nesting(&m));
            if unpadded.as_ref().ok() != Some(&nesting) {
                return Err(format!(
                    "the shape `{}` at {} nests {} levels with its pad, and {:?} without",
                    shape.name,
                    k,
                    nesting,
                    unpadded.map_err(|d| d.len())
                ));
            }
            out.push(Measured {
                shape: shape.name.clone(),
                nesting,
                per_call,
            });
        }
    }
    Ok(out)
}

/// The constructs on the path from `q`'s body to its call of itself: every expression and
/// statement whose span holds the call's, with the operator, the assignment, the place it writes,
/// the argument and the arm the call stands in.
fn constructs(m: &CheckedModule) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(q) = m.index("q").and_then(|i| m.functions[i].as_ref()) else {
        return out;
    };
    let tree: &Tree = &q.syntax.tree;
    let holds = |outer: Span, inner: Span| outer.start <= inner.start && inner.end <= outer.end;
    let calls: Vec<Span> = tree
        .exprs
        .iter()
        .filter(|e| matches!(&e.kind, ExprKind::Call { callee, .. } if callee == "q"))
        .map(|e| e.span)
        .collect();
    for call in calls {
        for e in tree.exprs.iter().filter(|e| holds(e.span, call)) {
            out.insert(coverage::expr_kind(&e.kind).to_string());
            match &e.kind {
                ExprKind::Binary { op, .. } => {
                    out.insert(coverage::binary_kind(op).to_string());
                }
                ExprKind::Unary { op, .. } => {
                    out.insert(coverage::unary_kind(op).to_string());
                }
                ExprKind::Call { args, .. } => {
                    for a in args.iter().filter(|a| holds(tree.expr(a.value).span, call)) {
                        out.insert(coverage::arg_kind(a).to_string());
                    }
                }
                ExprKind::Match { arms, .. } => {
                    for a in arms.iter().filter(|a| holds(a.span, call)) {
                        out.insert(coverage::arm_kind(&a.body).to_string());
                    }
                }
                _ => {}
            }
        }
        for s in tree.stmts.iter().filter(|s| holds(s.span, call)) {
            out.insert(coverage::stmt_kind(&s.kind).to_string());
            if let StmtKind::Assign { place: p, op, .. } = &s.kind {
                out.insert(coverage::assign_kind(op).to_string());
                if let AssignOp::Compound(b) = op {
                    out.insert(format!("{}=", b.symbol()));
                }
                if let Some(written) = place(coverage::expr_kind(&tree.expr(*p).kind)) {
                    out.insert(written.to_string());
                }
            }
        }
    }
    out
}

/// Every construct a call can stand in, from the generator's tables.
fn required() -> BTreeSet<String> {
    let mut all: BTreeSet<String> = [
        coverage::EXPRS,
        coverage::UNARIES,
        coverage::BINARIES,
        coverage::ARGS,
        coverage::STMTS,
        coverage::ASSIGNS,
        coverage::ARMS,
        &PLACES,
    ]
    .iter()
    .flat_map(|t| t.iter().map(|s| s.to_string()))
    .chain(coverage::compound_assignments())
    .collect();
    for (name, _) in HOLDS_NO_CALL.iter().chain(coverage::NOT_GENERATED) {
        all.remove(*name);
    }
    all
}

/// Whether every call of every shape keeps within this build's figures for its nesting, and takes
/// more host stack nested deeper: a report of each shape's figures, or one that names each shape
/// that does not.
pub(super) fn figures_hold() -> Result<String, String> {
    let (call, level) = mente_interp::host_figures();
    let measured = measure()?;
    let mut report = String::new();
    let mut over = Vec::new();
    let mut widest = (0, String::new());
    for pair in measured.chunks(2) {
        for m in pair {
            let allowed = call + level * m.nesting as usize;
            if m.per_call > allowed {
                over.push(format!(
                    "`{}` nesting {} takes {} bytes a call, more than the {} that {} + {} for each level allows",
                    m.shape, m.nesting, m.per_call, allowed, call, level
                ));
            }
        }
        if let [a, b] = pair {
            let levels = b.nesting.saturating_sub(a.nesting).max(1) as usize;
            let per_level = b.per_call.saturating_sub(a.per_call) / levels;
            // Each level of nesting adds to the host stack a call takes, so a measure that does
            // not grow with the nesting measures nothing.
            if per_level == 0 {
                over.push(format!(
                    "`{}` takes {} bytes a call at nesting {} and {} at {}: the measure does not grow with the nesting",
                    a.shape, a.per_call, a.nesting, b.per_call, b.nesting
                ));
            }
            if per_level > widest.0 {
                widest = (per_level, a.shape.clone());
            }
            report.push_str(&format!(
                "{:34} {:5} bytes a level; {:7} a call at nesting {:2}, {:7} allowed\n",
                a.shape,
                per_level,
                b.per_call,
                b.nesting,
                call + level * b.nesting as usize
            ));
        }
    }
    report.push_str(&format!(
        "the widest level: {} bytes, {}; the figures: {} a call, {} a level\n",
        widest.0, widest.1, call, level
    ));
    if over.is_empty() {
        Ok(report)
    } else {
        Err(format!("{}{}", report, over.join("\n")))
    }
}

/// Every call of every shape keeps within what this build's figures give it for its nesting, so
/// that the bound on the reference interpreter's host memory holds, which the harnesses derive
/// the nesting they keep to from: a step that grows a frame past its figure fails here first.
/// The generated programs' comparison holds a build with optimisation to its figures the same
/// way, on each platform CI runs it on.
#[test]
fn test_every_call_keeps_within_the_host_figures() {
    match figures_hold() {
        Ok(report) => println!("{}", report),
        Err(over) => panic!("{}", over),
    }
}

/// Every construct of the generator's tables that a call can stand in stands on the path from
/// some shape's body to its recursive call, so that a new construct is measured, or this fails.
#[test]
fn test_the_shapes_hold_a_call_in_every_construct_of_the_tables() {
    let mut reached = BTreeSet::new();
    for shape in shapes() {
        let text = (shape.text)(2);
        let m = mente_sema::check_program_without_proofs(&text)
            .unwrap_or_else(|d| panic!("`{}` does not check: {:?}", shape.name, d));
        reached.extend(constructs(&m));
    }
    let missing: Vec<String> = required().difference(&reached).cloned().collect();
    assert!(
        missing.is_empty(),
        "no shape holds a call in: {:?}; write a shape for each in host_shapes.rs, or list it in \
         HOLDS_NO_CALL with its reason",
        missing
    );
}
