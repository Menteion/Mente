//! The measure the bound on the reference interpreter's host memory rests on ([`host_bound`]):
//! how much of the host's stack one call takes, from where it begins to the deepest point it
//! reaches before a call it makes begins, on each shape a call can stand in, nested to two
//! depths, in the build that runs it. Each call must take no more than this build's figures
//! give it for its nesting, [`host::CALLS`] and [`host::LEVELS`]; the bound is made of the larger
//! of the two builds' figures, so that it holds in either.
//!
//! [`host_bound`]: crate::host_bound

use crate::host;
use crate::{Int, Limits, Stop, Value};
use mente_sema::types::Ty;
use mente_sema::CheckedModule;
use mente_syntax::body::NESTING_LIMIT;
use mente_syntax::tree::Tree;

/// How deep a module nests, as the parser counts it (`NESTING_LIMIT`): the most levels of
/// expressions, blocks and types in any of its functions and type declarations, and the most
/// levels a value of a declared type nests through its fields.
pub fn nesting(m: &CheckedModule) -> u32 {
    let functions = m.functions.iter().flatten().map(|f| height(&f.syntax.tree));
    let structs = m.structs.defs.iter().map(|s| height(s.tree()));
    let enums = m.structs.enums.iter().map(|e| height(e.tree()));
    let fields = m
        .structs
        .defs
        .iter()
        .flat_map(|s| s.fields.iter().map(|f| &f.ty))
        .chain(m.structs.enums.iter().flat_map(|e| {
            e.variants
                .iter()
                .flat_map(|v| v.fields.iter().map(|f| &f.ty))
        }))
        .map(|t| depth(m, t, 0));
    functions
        .chain(structs)
        .chain(enums)
        .chain(fields)
        .max()
        .unwrap_or(0)
}

/// The height of `tree`: the levels of its deepest node, which the parser holds to
/// `NESTING_LIMIT`.
fn height(tree: &Tree) -> u32 {
    let (mut low, mut high) = (0, NESTING_LIMIT);
    // `too_deep(limit)` is `None` exactly when no node is higher than `limit`.
    if tree.too_deep(high).is_some() {
        return NESTING_LIMIT + 1;
    }
    while low < high {
        let mid = low + (high - low) / 2;
        if tree.too_deep(mid).is_some() {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

/// The levels a value of type `t` nests through its elements and fields; `NESTING_LIMIT + 1`
/// past the limit, where a declaration would name itself (`seen` declarations deep).
fn depth(m: &CheckedModule, t: &Ty, seen: u32) -> u32 {
    if seen > NESTING_LIMIT {
        return NESTING_LIMIT + 1;
    }
    let fields = |tys: Vec<&Ty>| tys.into_iter().map(|t| depth(m, t, seen + 1)).max();
    1 + match t {
        Ty::Array(_, elem) => depth(m, elem, seen),
        Ty::Struct(s) => m
            .structs
            .get(s)
            .and_then(|d| fields(d.fields.iter().map(|f| &f.ty).collect()))
            .unwrap_or(0),
        Ty::Enum(e) => m
            .structs
            .get_enum(e)
            .and_then(|d| {
                fields(
                    d.variants
                        .iter()
                        .flat_map(|v| v.fields.iter().map(|f| &f.ty))
                        .collect(),
                )
            })
            .unwrap_or(0),
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Any => 0,
    }
}

/// A shape a call can stand in: the source of a module whose public `q` calls itself `k` levels
/// of the shape deep, and the arguments `q` is first called on.
struct Shape {
    name: &'static str,
    text: fn(usize) -> String,
    args: fn(usize) -> Vec<Value>,
}

/// `q(n: u32) -> u32`, whose body is `open(i)` for each level `i` from the outside in, the call
/// `q(n - 1)`, and `close(i)` from the inside out, after the declarations `before`.
fn nest(k: usize, before: &str, open: fn(usize) -> String, close: fn(usize) -> String) -> String {
    let opened: String = (0..k).map(open).collect();
    let closed: String = (0..k).rev().map(close).collect();
    format!(
        "module m\n{}pub fn q(n: u32) -> u32 = {}q(n - 1){}\n",
        before, opened, closed
    )
}

/// The count `q` is first called on, which no budget reaches.
fn count(_: usize) -> Vec<Value> {
    vec![Value::Int(Int::from_u128(u128::from(u32::MAX)))]
}

/// The shapes the evaluation of an expression or a statement can hold a call in, each one level
/// of the parser's or more, and a parameter whose type nests, which each call narrows.
fn shapes() -> Vec<Shape> {
    vec![
        // A call that passes its parameters on, alone at the first depth and inside an `if` for
        // each depth after it.
        Shape {
            name: "bare call",
            text: |k| {
                let open: String = (1..k).map(|i| format!("if c{} {{\n", i)).collect();
                let params: String = (1..k).map(|i| format!(", c{}: bool", i)).collect();
                let args: String = (1..k).map(|i| format!(", c{}", i)).collect();
                format!(
                    "module m\npub fn q(n: u32{}) -> u32 = {}q(n{}){}\n",
                    params,
                    open,
                    args,
                    "\n} else {\n  0\n}".repeat(k - 1)
                )
            },
            args: |k| {
                let mut a = vec![Value::Int(Int::from_u128(1))];
                a.extend((1..k).map(|_| Value::Bool(true)));
                a
            },
        },
        Shape {
            name: "if",
            text: |k| {
                nest(
                    k,
                    "",
                    |_| "if n > 0 {\n".into(),
                    |_| "\n} else {\n  0\n}".into(),
                )
            },
            args: count,
        },
        Shape {
            name: "match",
            text: |k| {
                nest(
                    k,
                    "",
                    |_| "match n {\n0 => 0\n_ => ".into(),
                    |_| "\n}".into(),
                )
            },
            args: count,
        },
        Shape {
            name: "arithmetic",
            text: |k| nest(k, "", |_| "0 + (".into(), |_| ")".into()),
            args: count,
        },
        Shape {
            name: "call",
            text: |k| {
                nest(
                    k,
                    "fn g(x: u32) -> u32 = x\n",
                    |_| "g(".into(),
                    |_| ")".into(),
                )
            },
            args: count,
        },
        Shape {
            name: "array",
            text: |k| nest(k, "", |_| "[".into(), |_| "][0]".into()),
            args: count,
        },
        Shape {
            name: "repeat",
            text: |k| nest(k, "", |_| "[".into(), |_| "; 1][0]".into()),
            args: count,
        },
        Shape {
            name: "struct",
            text: |k| {
                nest(
                    k,
                    "struct S {\n  x: u32\n}\n",
                    |_| "S(x: ".into(),
                    |_| ").x".into(),
                )
            },
            args: count,
        },
        Shape {
            name: "variant",
            text: |k| {
                nest(
                    k,
                    "enum E {\n  V(a: u32)\n  W\n}\n",
                    |_| "match E.V(a: ".into(),
                    |i| format!(") {{\nE.V(a: v{}) => v{}\nE.W => 0\n}}", i, i),
                )
            },
            args: count,
        },
        Shape {
            name: "comparison",
            text: |k| {
                nest(
                    k,
                    "",
                    |_| "if (".into(),
                    |_| ") > 0 { 1 } else { 0 }".into(),
                )
            },
            args: count,
        },
        Shape {
            name: "and",
            text: |k| {
                nest(
                    k,
                    "",
                    |_| "if n > 0 and (".into(),
                    |_| ") > 0 { 1 } else { 0 }".into(),
                )
            },
            args: count,
        },
        Shape {
            name: "not",
            text: |k| {
                nest(
                    k,
                    "",
                    |_| "if not ((".into(),
                    |_| ") > 0) { 1 } else { 0 }".into(),
                )
            },
            args: count,
        },
        Shape {
            name: "binding",
            text: |k| {
                nest(
                    k,
                    "",
                    |i| format!("if n > 0 {{\nval a{}: u32 = ", i),
                    |i| format!("\na{}\n}} else {{\n  0\n}}", i),
                )
            },
            args: count,
        },
        Shape {
            name: "assignment",
            text: |k| {
                nest(
                    k,
                    "",
                    |i| format!("if n > 0 {{\nvar a{}: u32 = 0\na{} = ", i, i),
                    |i| format!("\na{}\n}} else {{\n  0\n}}", i),
                )
            },
            args: count,
        },
        Shape {
            name: "guard",
            text: |k| {
                nest(
                    k,
                    "",
                    |_| "if n > 0 {\nguard (".into(),
                    |_| ") > 0 else {\nreturn 0\n}\n1\n} else {\n  0\n}".into(),
                )
            },
            args: count,
        },
        Shape {
            name: "while",
            text: |k| {
                nest(
                    k,
                    "",
                    |i| format!("if n > 0 {{\nvar w{}: u32 = 0\nwhile w{} < (", i, i),
                    |i| format!(") {{\nw{} = w{} + 1\n}}\nw{}\n}} else {{\n  0\n}}", i, i, i),
                )
            },
            args: count,
        },
        Shape {
            name: "minus",
            text: |k| {
                format!(
                    "module m\npub fn q(n: i64) -> i64 = {}q(n - 1){}\n",
                    "-(".repeat(k),
                    ")".repeat(k)
                )
            },
            args: |_| vec![Value::Int(Int::from_u128(1 << 40))],
        },
        Shape {
            name: "narrowing",
            text: |k| {
                let ty = format!("{}u32", "[1]".repeat(k));
                format!(
                    "module m\npub fn q(n: u32, a: {}) -> u32 = q(n - 1, a)\n",
                    ty
                )
            },
            args: |k| {
                let mut a = Value::Int(Int::from_u128(1));
                for _ in 0..k {
                    a = Value::Array(vec![a]);
                }
                vec![Value::Int(Int::from_u128(u128::from(u32::MAX))), a]
            },
        },
    ]
}

/// The depths each shape is measured at.
const DEPTHS: [usize; 2] = [1, 8];

/// The steps each measured run may take: enough for many calls, far fewer than the budget's.
const STEPS: u64 = 40_000;

/// One shape measured at one depth: the module's nesting, and the most host stack one of its
/// calls took.
pub struct Measured {
    pub shape: &'static str,
    pub nesting: u32,
    pub per_call: usize,
}

/// Each shape at each depth, run in this build; an error where a shape does not check or does
/// not run.
pub fn measure() -> Result<Vec<Measured>, String> {
    let mut out = Vec::new();
    for shape in shapes() {
        for k in DEPTHS {
            let text = (shape.text)(k);
            let m = mente_sema::check_program_without_proofs(&text).map_err(|d| {
                format!(
                    "the shape `{}` at {} does not check: {:?}",
                    shape.name, k, d
                )
            })?;
            let limits = Limits {
                max_steps: STEPS,
                ..Limits::default()
            };
            let (outcome, per_call) = crate::host_per_call(&m, "q", (shape.args)(k), limits);
            // The run ends at its steps, or at the budget, never otherwise.
            let ended = match &outcome {
                Err(Stop::OutOfSteps) => true,
                Err(Stop::Trap(t)) => t.kind == crate::TrapKind::StackOverflow,
                Ok(_)
                | Err(
                    Stop::TooLarge(_)
                    | Stop::NoFunction(_)
                    | Stop::BadArguments(_)
                    | Stop::Broken(_),
                ) => false,
            };
            if !ended {
                return Err(format!(
                    "the shape `{}` at {} ran to {:?}, not to its steps' end",
                    shape.name, k, outcome
                ));
            }
            out.push(Measured {
                shape: shape.name,
                nesting: nesting(&m),
                per_call,
            });
        }
    }
    Ok(out)
}

/// Whether every call of every shape keeps within what this build's figures give it for its
/// nesting: a report of each shape's figures, or one that names each that does not.
pub fn host_figures_hold() -> Result<String, String> {
    let (call, level) = host::this_build();
    let measured = measure()?;
    let mut report = String::new();
    let mut over = Vec::new();
    for m in &measured {
        let allowed = call + level * m.nesting as usize;
        report.push_str(&format!(
            "{:12} nesting {:3}: {:7} bytes a call, {:7} allowed\n",
            m.shape, m.nesting, m.per_call, allowed
        ));
        if m.per_call > allowed {
            over.push(format!(
                "`{}` nesting {} takes {} bytes a call, more than the {} that {} + {} for each level allows",
                m.shape, m.nesting, m.per_call, allowed, call, level
            ));
        }
    }
    // The figures of each shape: what a level takes, between the two depths, and a call besides.
    for pair in measured.chunks(2) {
        if let [a, b] = pair {
            let levels = b.nesting.saturating_sub(a.nesting).max(1) as usize;
            let per_level = b.per_call.saturating_sub(a.per_call) / levels;
            report.push_str(&format!(
                "{:12} {:5} bytes a level, {:6} a call besides\n",
                a.shape,
                per_level,
                a.per_call.saturating_sub(per_level * a.nesting as usize)
            ));
        }
    }
    if over.is_empty() {
        Ok(report)
    } else {
        Err(format!("{}{}", report, over.join("\n")))
    }
}
