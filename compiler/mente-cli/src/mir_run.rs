//! Running a call through MIR: the second execution path of M1's checkpoint (c), which prints
//! what `mn run` prints, so that it can be compared with the reference interpreter line for
//! line.

use mente_codegen::lower::{self, Lowered, Shape};
use mente_interp::Value;
use mente_mir::cfg::{self, Memory, Num, Ty, Val};
use mente_sema::CheckedModule;

/// What a run printed: its value and what each `mut` argument was left holding, or why it
/// stopped.
pub type Printed = Result<Vec<String>, String>;

/// Calls `name` on `args`, each marked `mut` or not, through MIR, and prints as `mn run` does.
pub fn run(module: &CheckedModule, name: &str, args: Vec<(bool, Value)>) -> Printed {
    let lowered = lower::lower(module).map_err(|e| format!("not lowered: {}", e))?;
    run_lowered(&lowered, name, args)
}

/// Calls `name` on `args` in a module already lowered.
pub fn run_lowered(lowered: &Lowered, name: &str, args: Vec<(bool, Value)>) -> Printed {
    let entry = lowered
        .entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("no function `{}` with a body to run", name))?;
    if args.len() != entry.params.len() {
        return Err(format!(
            "`{}` takes {} arguments, but {} were given",
            name,
            entry.params.len(),
            args.len()
        ));
    }
    let mut memory = Memory::default();
    let mut values = Vec::new();
    let sret = match &entry.result {
        Shape::Array(..) => {
            let at = memory.alloc(entry.result.size());
            values.push(Val::Addr(at));
            Some(at)
        }
        _ => None,
    };
    let mut places = Vec::new();
    for ((is_mut, shape), (_, value)) in entry.params.iter().zip(&args) {
        match (is_mut, shape) {
            (false, Shape::Int(_) | Shape::Bool) => values.push(scalar(value)?),
            (_, shape) => {
                let at = memory.alloc(shape.size());
                write(&mut memory, at, shape, value)?;
                values.push(Val::Addr(at));
                if *is_mut {
                    places.push((at, shape.clone()));
                }
            }
        }
    }
    let limits = cfg::Limits::default();
    let result = cfg::run(&lowered.module, entry.func, &values, &mut memory, limits)
        .map_err(|stop| stop.to_string())?;
    let mut printed = vec![match (sret, result) {
        (Some(at), _) => read(&memory, at, &entry.result)?,
        (None, v) => show(v),
    }];
    for (at, shape) in places {
        printed.push(format!("leaves {}", read(&memory, at, &shape)?));
    }
    Ok(printed)
}

/// Calls `name` on `args` through the reference interpreter, and prints as `mn run` does.
#[cfg(test)]
pub fn reference(module: &CheckedModule, name: &str, args: Vec<(bool, Value)>) -> Printed {
    let marked: Vec<bool> = args.iter().map(|(m, _)| *m).collect();
    let values = args.into_iter().map(|(_, v)| v).collect();
    let limits = mente_interp::Limits::default();
    let (v, finals) =
        mente_interp::call_leaving(module, name, values, limits).map_err(|s| s.to_string())?;
    let mut printed = vec![v.to_string()];
    for (_, last) in marked.iter().zip(finals).filter(|(m, _)| **m) {
        printed.push(format!("leaves {}", last));
    }
    Ok(printed)
}

fn num(value: &Value) -> Result<Num, String> {
    match value {
        Value::Int(i) => Ok(Num::new(i.is_negative(), i.magnitude())),
        _ => Err(format!("`{}` is not an integer", value)),
    }
}

fn scalar(value: &Value) -> Result<Val, String> {
    match value {
        Value::Bool(b) => Ok(Val::Bool(*b)),
        v => num(v).map(Val::Int),
    }
}

/// Writes `value` of `shape` at `at`; an integer outside its type is refused.
fn write(memory: &mut Memory, at: u64, shape: &Shape, value: &Value) -> Result<(), String> {
    match (shape, value) {
        (Shape::Int(t), v) => {
            let n = num(v)?;
            let t = lower::mir(*t);
            if !t.holds(n) {
                return Err(format!("the argument {} lies outside `{}`", n, t));
            }
            memory
                .store(at, Val::Int(n), Ty::Int(t))
                .map_err(|s| s.to_string())
        }
        (Shape::Bool, Value::Bool(b)) => memory
            .store(at, Val::Bool(*b), Ty::Bool)
            .map_err(|s| s.to_string()),
        (Shape::Array(len, elem), Value::Array(items)) if items.len() == *len as usize => {
            for (k, item) in items.iter().enumerate() {
                write(memory, at + u64::from(elem.size()) * k as u64, elem, item)?;
            }
            Ok(())
        }
        _ => Err(format!("`{}` does not have its parameter's shape", value)),
    }
}

/// The value of `shape` at `at`, as the reference interpreter prints it.
fn read(memory: &Memory, at: u64, shape: &Shape) -> Result<String, String> {
    match shape {
        Shape::Int(t) => memory
            .load(at, Ty::Int(lower::mir(*t)))
            .map(show)
            .map_err(|s| s.to_string()),
        Shape::Bool => memory
            .load(at, Ty::Bool)
            .map(show)
            .map_err(|s| s.to_string()),
        Shape::Array(len, elem) => {
            let items = (0..u64::from(*len))
                .map(|k| read(memory, at + u64::from(elem.size()) * k, elem))
                .collect::<Result<Vec<_>, String>>()?;
            Ok(format!("[{}]", items.join(", ")))
        }
        Shape::Unit => Ok("()".into()),
    }
}

fn show(v: Val) -> String {
    match v {
        Val::Int(n) => n.to_string(),
        Val::Bool(b) => b.to_string(),
        Val::Addr(a) => format!("@{}", a),
        Val::Unit => "()".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    /// The calls of a benchmark task's sections: each line's call.
    fn calls(task: &str, sections: &[&str]) -> Vec<String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmark/tasks");
        let Ok(text) = fs::read_to_string(root.join(format!("{}.md", task))) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut inside = false;
        for line in text.lines() {
            if line.starts_with("## ") {
                inside = sections.contains(&line);
                continue;
            }
            let call = line.trim();
            if inside && line.starts_with("    ") && !call.is_empty() {
                let end = call
                    .find(" == ")
                    .or_else(|| call.find(" leaves "))
                    .unwrap_or(call.len());
                out.push(call[..end].to_string());
            }
        }
        out
    }

    /// The task a probe or preliminary program solves, from its file name.
    fn task_of(file: &str, tasks: &[String]) -> Option<String> {
        tasks
            .iter()
            .filter(|t| file.contains(t.as_str()))
            .max_by_key(|t| t.len())
            .cloned()
    }

    /// Runs every probe and preliminary program that checks on each call of `sections` of its
    /// task, through the reference interpreter and through MIR, and returns how many calls it
    /// compared and those whose outputs differ.
    fn compare(sections: &[&str]) -> (usize, Vec<String>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmark");
        let tasks: Vec<String> = fs::read_dir(root.join("tasks"))
            .unwrap()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_suffix(".md")
                    .map(String::from)
            })
            .collect();
        let mut compared = 0;
        let mut differences = Vec::new();
        for dir in ["probe/m1", "prelim/m1", "prelim/m1-non-core"] {
            for entry in fs::read_dir(root.join(dir)).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".mn") {
                    continue;
                }
                let src = fs::read_to_string(entry.path()).unwrap();
                let Ok(module) = mente_sema::check_program(&src) else {
                    continue;
                };
                let Some(task) = task_of(&name, &tasks) else {
                    continue;
                };
                let lowered = lower::lower(&module);
                for call in calls(&task, sections) {
                    let Some((f, args)) = mente_interp::parse_call_modes(&call) else {
                        continue;
                    };
                    let expected = reference(&module, &f, args.clone());
                    let got = match &lowered {
                        Ok(l) => run_lowered(l, &f, args),
                        Err(e) => Err(format!("not lowered: {}", e)),
                    };
                    compared += 1;
                    if got != expected {
                        differences.push(format!("{} {}: {:?} != {:?}", name, call, got, expected));
                    }
                }
            }
        }
        (compared, differences)
    }

    fn assert_same(compared: usize, differences: &[String], at_least: usize) {
        assert!(compared >= at_least, "only {} calls compared", compared);
        assert!(
            differences.is_empty(),
            "{} of {} differ:\n{}",
            differences.len(),
            compared,
            differences
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// Every probe and preliminary program that checks, run on each example of its task through
    /// the reference interpreter and through MIR: the two print the same.
    #[test]
    fn test_mir_runs_every_agent_program_as_the_reference_interpreter_does() {
        let (compared, differences) = compare(&["## Examples"]);
        assert_same(compared, &differences, 400);
    }

    /// The same on every example and hidden test, over 2,000 calls, too slow for a debug build:
    /// `cargo test --release -p mente-cli -- --ignored mir_matches_the_reference_interpreter`.
    #[test]
    #[ignore = "slow in a debug build; CI runs it in release"]
    fn mir_matches_the_reference_interpreter_on_every_hidden_test() {
        let (compared, differences) = compare(&["## Examples", "## Hidden tests"]);
        assert_same(compared, &differences, 2000);
    }

    /// Programs that reach each construct the lowering handles, each with calls whose output
    /// both paths must print alike: a value or a trap of every kind.
    const CONSTRUCTS: &[(&str, &[&str])] = &[
        // Narrowing at a binding, an argument and a return.
        (
            "pub fn bind(x: u16) -> u8 {\n  val y: u8 = x\n  y\n}\nfn take(x: u8) -> u8 = x\npub fn arg(x: u16) -> u8 = take(x)\npub fn back(x: u16) -> u8 = x",
            &["bind(200)", "bind(300)", "bind(70000)", "arg(300)", "back(256)", "back(255)"],
        ),
        // Refinements with literal and parameter bounds, inclusive or not.
        (
            "pub fn lit(i: usize) -> usize[0..=9] = i\npub fn named(n: usize, i: usize) -> usize {\n  val k: usize[0..n] = i\n  k\n}\npub fn neg(x: i8) -> i8[-3..3] = x",
            &["lit(9)", "lit(10)", "named(3, 2)", "named(3, 5)", "neg(-3)", "neg(3)", "neg(-4)"],
        ),
        // Contracts: requires at a call, ensures at a return.
        (
            "fn half(x: u8) -> u8\n  requires x > 3\n= x / 2\npub fn call(x: u8) -> u8 = half(x)\npub fn up(x: u8) -> u8\n  ensures result > x\n{\n  guard x < 10 else {\n    return x\n  }\n  x + 1\n}",
            &["call(9)", "call(1)", "up(3)", "up(12)"],
        ),
        // A loop with an invariant and a measure, and one whose measure stops falling.
        (
            "pub fn count(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n invariant i <= 200 decreases n - i {\n    i += 1\n  }\n  i\n}\npub fn stuck(n: u8, step: u8) -> u8 {\n  var i: u8 = 0\n  while i < n decreases n - i {\n    i += step\n  }\n  i\n}\npub fn broken(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n invariant i < 3 {\n    i += 1\n  }\n  i\n}",
            &["count(5)", "stuck(4, 2)", "stuck(4, 0)", "broken(2)", "broken(5)"],
        ),
        // A recursive function's measure, which must fall at each call.
        (
            "pub fn sum(n: u8) -> u16\n  decreases n\n{\n  guard n > 0 else {\n    return 0\n  }\n  val m: u8 = n - 1\n  m + sum(m)\n}\npub fn spin(n: u8) -> u8\n  decreases n\n{\n  guard n > 0 else {\n    return 0\n  }\n  spin(n)\n}",
            &["sum(10)", "spin(0)", "spin(2)"],
        ),
        // Division, remainder and their zero divisor.
        (
            "pub fn div(a: i32, b: i32) -> i32 = a / b\npub fn rem(a: i32, b: i32) -> i32 = a % b",
            &["div(-7, 2)", "rem(-7, 2)", "rem(7, -2)", "div(1, 0)", "rem(1, 0)"],
        ),
        // Indexes, read and written, nested, and outside their arrays.
        (
            "pub fn get(xs: [3]u8, i: usize) -> u8 = xs[i]\npub fn set(i: usize) -> [3]u8 {\n  var xs: [3]u8 = [1, 2, 3]\n  xs[i] = 9\n  xs\n}\npub fn grid(i: usize, j: usize) -> u8 {\n  var g: [2][2]u8 = [[1, 2], [3, 4]]\n  g[i][j] += 10\n  g[i][j]\n}\npub fn len(xs: [4]u8) -> usize = xs.len",
            &["get([1, 2, 3], 2)", "get([1, 2, 3], 3)", "set(1)", "set(5)", "grid(1, 0)", "grid(0, 2)", "len([1, 2, 3, 4])"],
        ),
        // `mut` arguments: a binding, an element, an array; what each is left holding.
        (
            "fn bump(mut x: u8) {\n  x += 1\n}\nfn swap(mut xs: [2]u8) {\n  val t: u8 = xs[0]\n  xs[0] = xs[1]\n  xs[1] = t\n}\npub fn one(mut x: u8) {\n  bump(mut x)\n}\npub fn element(mut xs: [3]u8, i: usize) {\n  bump(mut xs[i])\n}\npub fn pair(mut xs: [2]u8) {\n  swap(mut xs)\n}",
            &["one(mut 4)", "one(mut 255)", "element(mut [1, 2, 3], 1)", "element(mut [1, 2, 3], 3)", "pair(mut [5, 6])"],
        ),
        // A `mut` place whose refinement the callee's result breaks on the way back.
        (
            "fn twice(mut x: u8) {\n  x *= 2\n}\npub fn keep(mut k: u8[0..20]) {\n  twice(mut k)\n}",
            &["keep(mut 3)", "keep(mut 15)"],
        ),
        // Arrays: results, literals, repeats, a parameter's refinement, a copy that stays apart.
        (
            "pub fn fill(v: u8) -> [3]u8 = [v; 3]\npub fn wide(x: u16) -> [2]u8 = [1, x]\npub fn small(xs: [2]u8[0..10]) -> u8 = xs[0]\npub fn apart() -> [2]u8 {\n  var a: [2]u8 = [1, 2]\n  val b: [2]u8 = a\n  a[0] = 7\n  [a[0], b[0]]\n}",
            &["fill(4)", "wide(5)", "wide(300)", "small([1, 9])", "small([1, 12])", "apart()"],
        ),
        // Comparisons across types, a u128 against a signed integer among them.
        (
            "pub fn gt(a: u128, b: i64) -> bool = a > b\npub fn lt(a: i64, b: u128) -> bool = a < b\npub fn eq(a: u128, b: i8) -> bool = a == b\npub fn mix(a: u8, b: i64) -> bool = a <= b\npub fn same(a: bool, b: bool) -> bool = a == b",
            &["gt(5, -1)", "gt(0, 3)", "gt(7, 3)", "lt(-1, 0)", "lt(4, 4)", "eq(3, 3)", "eq(3, -3)", "mix(5, -2)", "mix(5, 9)", "same(true, false)"],
        ),
        // Shifts: amounts outside the width, a left shift that leaves its type, a floor.
        (
            "pub fn shl(x: u8, k: u8) -> u8 = x << k\npub fn shr(x: i8, k: i8) -> i8 = x >> k\npub fn big(x: i128, k: u8) -> i128 = x << k",
            &["shl(3, 2)", "shl(64, 2)", "shl(1, 8)", "shr(-7, 1)", "shr(-7, -1)", "big(1, 126)", "big(1, 127)", "big(3, 127)"],
        ),
        // Bit operations, whose operands narrow into one type first.
        (
            "pub fn both(a: u8, b: u8) -> u8 = a & b\npub fn mixed(a: u16, b: u16) -> u8 = a | b\npub fn flip(a: i8) -> i8 = ~a\npub fn sum_or(a: u8, b: u8) -> u8 = (a + b) ^ 1",
            &["both(12, 10)", "mixed(256, 1)", "flip(5)", "sum_or(200, 100)", "sum_or(2, 3)"],
        ),
        // Exact arithmetic in a wider type, narrowed where it lands, and checked 128-bit operations.
        (
            "pub fn add(a: u64, b: u64) -> u64 = a + b\npub fn diff(a: u64, b: u64) -> i64 = a - b\npub fn sub(a: u128, b: u128) -> u128 = a - b\npub fn mul(a: i128, b: i128) -> i128 = a * b\npub fn neg(a: i128) -> i128 = -a\npub fn sq(a: u64) -> u128 {\n  val p: u128 = a * a\n  p + 1\n}",
            &["add(18446744073709551615, 0)", "add(18446744073709551615, 1)", "diff(3, 5)", "diff(18446744073709551615, 0)", "sub(1, 2)", "sub(5, 2)", "mul(170141183460469231731687303715884105727, 2)", "neg(-170141183460469231731687303715884105728)", "neg(5)", "sq(18446744073709551615)"],
        ),
        // Control flow: an `if` value of two types, short circuits, `guard`, `break`, `continue`.
        (
            "pub fn pick(c: bool, a: u8, b: i16) -> i16 = if c { a } else { b }\npub fn first(xs: [3]u8, i: usize) -> bool = i < 3 and xs[i] > 1\npub fn either(xs: [3]u8, i: usize) -> bool = i >= 3 or xs[i] > 1\npub fn find(xs: [5]u8, v: u8) -> usize {\n  var i: usize = 0\n  while i < 5 {\n    if xs[i] == v {\n      break\n    }\n    i += 1\n  }\n  i\n}\npub fn odd(n: u8) -> u8 {\n  var i: u8 = 0\n  var s: u8 = 0\n  while i < n {\n    i += 1\n    guard i % 2 == 1 else {\n      continue\n    }\n    s += i\n  }\n  s\n}",
            &["pick(true, 200, -5)", "pick(false, 200, -5)", "first([1, 2, 3], 1)", "first([1, 2, 3], 7)", "either([1, 2, 3], 7)", "either([1, 2, 3], 0)", "find([4, 5, 6, 7, 8], 7)", "find([4, 5, 6, 7, 8], 9)", "odd(7)"],
        ),
        // A bound of an array's length, a measure of two clauses, and a measure below zero.
        (
            "pub fn at(xs: [3]u8, i: usize) -> u8 {
  val k: usize[0..xs.len] = i
  xs[k]
}
pub fn grid(n: u8) -> u16 {
  var i: u8 = 0
  var j: u8 = 0
  var s: u16 = 0
  while i < n decreases n - i decreases 3 - j {
    if j < 3 {
      j += 1
    } else {
      j = 0
      i += 1
    }
    s += 1
  }
  s
}
pub fn below(n: i8) -> i8 {
  var i: i8 = n
  while i < 0 decreases 0 - i - 2 {
    i += 1
  }
  i
}",
            &["at([1, 2, 3], 2)", "at([1, 2, 3], 3)", "grid(2)", "below(-5)"],
        ),
        // Arrays of other element types, two `ensures`, an element's refinement, a negative
        // value into an unsigned type, and booleans in memory.
        (
            "pub fn widen(xs: [2]u8) -> [2]u16 {\n  xs\n}\npub fn shrink(xs: [2]u16) -> [2]u8 {\n  xs\n}\nfn two(x: u8) -> u8\n  ensures result > 0\n  ensures result < 100\n= x\npub fn ends(x: u8) -> u8 = two(x)\npub fn put(i: usize, v: u8) -> u8 {\n  var xs: [2]u8[0..10] = [1, 2]\n  xs[i] = v\n  xs[i]\n}\npub fn tou(x: i16) -> u8 = x\npub fn flags(mut fs: [2]bool) {\n  fs[0] = not fs[0]\n}\npub fn flip(mut b: bool) {\n  b = not b\n}",
            &["widen([1, 2])", "shrink([1, 2])", "shrink([1, 300])", "ends(5)", "ends(0)", "ends(150)", "put(1, 9)", "put(1, 12)", "tou(-1)", "tou(7)", "flags(mut [true, false])", "flip(mut true)"],
        ),
        // `todo`, `unreachable` and a discarded result.
        (
            "pub fn later(x: u8) -> u8 {\n  guard x > 0 else {\n    todo(\"zero is not handled\")\n  }\n  x\n}\npub fn never(x: u8) -> u8 {\n  guard x < 10 else {\n    unreachable(\"x is a digit\")\n  }\n  x\n}\nfn noisy(x: u8) -> u8 = x + 1\npub fn drop(x: u8) -> u8 {\n  _ = noisy(x)\n  x\n}",
            &["later(0)", "later(2)", "never(12)", "never(3)", "drop(255)", "drop(4)"],
        ),
    ];

    /// An argument from outside the program must have its parameter's shape and fit its types.
    #[test]
    fn test_mir_runs_refuse_arguments_of_another_shape() {
        let text = "module m\npub fn get(xs: [3]u8, i: usize) -> u8 = xs[i]\n";
        let module = mente_sema::check_program(text).unwrap();
        let lowered = lower::lower(&module).unwrap();
        let call = |c: &str| {
            let (f, args) = mente_interp::parse_call_modes(c).expect("a call");
            run_lowered(&lowered, &f, args)
        };
        assert_eq!(call("get([1, 2, 3], 1)"), Ok(vec!["2".to_string()]));
        assert!(call("get([1, 2], 0)").unwrap_err().contains("shape"));
        assert!(call("get([1, true, 3], 0)").unwrap_err().contains("not an integer"));
        assert!(call("get([1, 2, 300], 0)")
            .unwrap_err()
            .contains("lies outside"));
        assert!(call("get([1, 2, 3])")
            .unwrap_err()
            .contains("takes 2 arguments"));
    }

    /// Each construct the lowering handles, run on calls that reach every path of it, prints
    /// alike through the reference interpreter and through MIR.
    #[test]
    fn test_mir_lowers_each_construct_as_the_reference_interpreter_runs_it() {
        let mut differences = Vec::new();
        let mut compared = 0;
        for (src, calls) in CONSTRUCTS {
            // Without Tier 1, which rejects the failing refinements and contracts at compile time:
            // the run-time checks are what is compared.
            let text = format!("module m\n{}\n", src);
            let module = match mente_sema::check_program_without_proofs(&text) {
                Ok(m) => m,
                Err(d) => panic!("{}\ndoes not check: {:?}", src, d),
            };
            let lowered = lower::lower(&module).unwrap_or_else(|e| panic!("{}: {}", src, e));
            for call in *calls {
                let (f, args) = mente_interp::parse_call_modes(call).expect("a call");
                let expected = reference(&module, &f, args.clone());
                let got = run_lowered(&lowered, &f, args);
                compared += 1;
                if got != expected {
                    differences.push(format!("{}: {:?} != {:?}", call, got, expected));
                }
            }
        }
        assert_same(compared, &differences, 90);
    }
}
