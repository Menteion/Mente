//! The mutation gate on lowered code (`spec/mir-and-codegen.md` item 4).
//!
//! For each program and its calls, the gate lowers the program into MIR, makes every mutant
//! the operators of `mente_mir::cfg_mutate` make of it, and runs each mutant on the calls with
//! the MIR interpreter. A mutant is killed when, for some call, what it prints, a value, what a
//! `mut` argument is left holding, or a trap, differs from what the reference interpreter
//! prints; it survives otherwise. The gate passes when every survivor of a gated program is
//! recorded in `equivalent.txt`, one line per mutant, with the reason no input can tell it
//! apart from the original.
//!
//! A run of the original that differs from the reference interpreter is a lowering bug, and
//! fails the gate on its own. Every run fills new stack slots with [`POISON`] rather than
//! zeros, so a read of a slot nothing wrote shows. A mutant whose run the MIR interpreter stops
//! as broken MIR, an exact operation leaving its type or an access leaving its slot, is killed;
//! so is one that runs more than four times the original's instructions, which a mutated loop
//! exit does.
//!
//! The programs come from sources that each say whether their calls can grow:
//!
//! - the gate's own corpus, `tests/mutants/*.mn`, is gated: a survivor without a reason is a
//!   gap, closed by adding a call that kills it. Each function's first call also runs with
//!   each integer argument at its type's smallest and largest value;
//! - the construct corpus of `mir_run.rs`, with the calls its tests make, and the benchmark's
//!   agent programs, run on their tasks' examples and hidden tests, have fixed calls and are
//!   reported: their survivors are counted, not failed on. `mn test --mutate --reported` adds
//!   them.
//!
//! Any other source, such as a program generator, plugs in as a list of [`Program`]s given to
//! [`gate`]: a generated program whose calls can grow is gated, one whose calls are fixed is
//! reported.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use mente_codegen::lower::{self, Lowered, Shape};
use mente_codegen::native;
use mente_interp::Value;
use mente_mir::cfg::{self, FuncId, Loc, Memory, Num, Ty, Val};
use mente_mir::cfg_mutate::{self, Family};
use mente_sema::CheckedModule;

use crate::mir_run::{self, Printed};

/// The gate's own files, from the repository's root: its programs and `equivalent.txt`.
pub const DIR: &str = "compiler/mente-cli/tests/mutants";

/// A mutant may run this many times the original's instructions, plus [`STEP_ALLOWANCE`],
/// before it counts as running on forever.
const STEP_FACTOR: u64 = 4;
const STEP_ALLOWANCE: u64 = 10_000;

/// What each byte of a new stack slot holds until written, in every run the gate makes: native
/// code's slots hold whatever was there, so a mutant, or a lowering, that reads a slot before
/// writing it shows up, where memory filled with zeros would hide a missing `var i = 0`.
const POISON: u8 = 0xA5;

/// A program the gate runs, with the calls whose outputs tell its mutants apart.
#[derive(Clone, Debug)]
pub struct Program {
    /// Unique among the programs the gate runs, and the start of each of its mutants' names:
    /// a path from the repository's root.
    pub name: String,
    pub source: String,
    /// Each call as `mn run --call` takes it: `clamp(5, 0, 10)`, `bump(mut [1, 2])`.
    pub calls: Vec<String>,
    /// Whether Tier 1 runs first, leaving out of MIR each check it proves. Without it, a
    /// program may break its refinements and contracts, and its calls reach their traps.
    pub proofs: bool,
    /// Whether a survivor without a written reason fails the gate. A source whose calls are a
    /// fixed test set has its survivors reported instead.
    pub gated: bool,
}

/// How a mutant was told apart from the original.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kill {
    /// A call printed another value, left a `mut` argument holding another, or trapped
    /// elsewhere or not at all.
    Output,
    /// The MIR interpreter stopped the run as broken MIR.
    Broken,
    /// The run took more than its share of instructions.
    Timeout,
}

/// One mutant of one program, and what became of it.
#[derive(Clone, Debug)]
pub struct Outcome {
    /// `program::function/block.site/operator`, the name `equivalent.txt` records.
    pub id: String,
    pub family: Family,
    /// The line and column in the program of what the mutant changes.
    pub loc: Loc,
    /// How it was killed, and by which call; `None` when it survived every call.
    pub killed: Option<(Kill, String)>,
}

/// What the gate found for one program.
#[derive(Clone, Debug, Default)]
pub struct ProgramRun {
    pub name: String,
    pub gated: bool,
    pub outcomes: Vec<Outcome>,
    /// Mutants that break MIR's rules, which `cfg::verify` rejects, by family.
    pub skipped: Vec<(String, Family)>,
    /// Why the program, or one of its calls, could not take part: it does not check or lower,
    /// a call does not parse, or MIR does not print what the reference interpreter prints.
    pub problems: Vec<String>,
    /// Calls that reached MIR and so could tell mutants apart.
    pub calls: usize,
}

/// A call ready to run on any mutant: its entry, its arguments, the memory they live in, how
/// to read back what it prints, what the reference interpreter printed, and its step limit.
struct Prepared {
    text: String,
    func: FuncId,
    values: Vec<Val>,
    memory: Memory,
    sret: Option<(u64, Shape)>,
    places: Vec<(u64, Shape)>,
    expected: Printed,
    steps: u64,
}

/// Runs every program on its calls, in parallel, and returns what was found for each, in the
/// programs' order.
pub fn run_programs(programs: &[Program]) -> Vec<ProgramRun> {
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(programs.len().max(1));
    let mut found: Vec<(usize, ProgramRun)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(p) = programs.get(k) else {
                            return mine;
                        };
                        mine.push((k, run_program(p)));
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| {
                h.join().unwrap_or_else(|_| {
                    vec![(
                        usize::MAX,
                        ProgramRun {
                            name: "a worker".into(),
                            gated: true,
                            problems: vec!["a worker thread stopped unexpectedly".into()],
                            ..ProgramRun::default()
                        },
                    )]
                })
            })
            .collect()
    });
    found.sort_by_key(|(k, _)| *k);
    found.into_iter().map(|(_, r)| r).collect()
}

/// Lowers one program, prepares its calls, and runs each of its mutants on them.
pub fn run_program(p: &Program) -> ProgramRun {
    let mut run = ProgramRun {
        name: p.name.clone(),
        gated: p.gated,
        ..ProgramRun::default()
    };
    let checked = if p.proofs {
        mente_sema::check_program(&p.source)
    } else {
        mente_sema::check_program_without_proofs(&p.source)
    };
    let module = match checked {
        Ok(m) => m,
        Err(d) => {
            let first = d.first().map(|d| d.message.clone()).unwrap_or_default();
            run.problems.push(format!("does not check: {}", first));
            return run;
        }
    };
    let lowered = match lower::lower(&module) {
        Ok(l) => l,
        Err(e) => {
            run.problems.push(format!("does not lower: {}", e));
            return run;
        }
    };
    let mut calls = p.calls.clone();
    if p.gated {
        for c in edge_calls(&lowered, &p.calls) {
            if !calls.contains(&c) {
                calls.push(c);
            }
        }
    }
    let mut prepared = Vec::new();
    for call in &calls {
        match prepare(&module, &lowered, call) {
            Ok(Some(c)) => prepared.push(c),
            Ok(None) => {}
            Err(e) => run.problems.push(format!("{}: {}", call, e)),
        }
    }
    // Cheap calls first: most mutants die on the first call that reaches them.
    prepared.sort_by_key(|c| c.steps);
    run.calls = prepared.len();
    // A function's own name, or `enter f` for the entry the lowering gives `f`.
    let id = |m: &cfg_mutate::Mutant| {
        let func = lowered
            .module
            .functions
            .get(m.func.0 as usize)
            .map_or("", |f| f.name.as_str());
        format!("{}::{}", p.name, m.name(func))
    };
    let (kept, skipped) = cfg_mutate::split(&lowered.module);
    for (m, _) in skipped {
        run.skipped.push((id(&m), m.family));
    }
    for m in kept {
        let mutant = m.apply(&lowered.module);
        let killed = prepared.iter().find_map(|c| {
            let (printed, _) = execute(&mutant, c, limit(c.steps));
            (printed != c.expected).then(|| (kill_kind(&printed), c.text.clone()))
        });
        run.outcomes.push(Outcome {
            id: id(&m),
            family: m.family,
            loc: m.loc(&lowered.module),
            killed,
        });
    }
    run
}

/// The calls the gate adds to a gated program's own: each function's first written call again,
/// with each integer argument moved in turn to its type's smallest and largest value, where an
/// operation, a refinement or a bound on it is most often wrong. An argument outside its type
/// traps before the entry runs, on every path, so no edge beyond these reaches MIR.
fn edge_calls(lowered: &Lowered, written: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for entry in &lowered.entries {
        let Some(args) = written.iter().find_map(|c| {
            mente_interp::parse_call_modes(c)
                .filter(|(f, a)| *f == entry.name && a.len() == entry.params.len())
                .map(|(_, a)| a)
        }) else {
            continue;
        };
        for (j, (_, shape)) in entry.params.iter().enumerate() {
            let Shape::Int(t) = shape else {
                continue;
            };
            let t = lower::mir(*t);
            for edge in [t.min(), t.max()] {
                let shown: Vec<String> = args
                    .iter()
                    .enumerate()
                    .map(|(k, (m, v))| {
                        let v = if k == j {
                            edge.to_string()
                        } else {
                            v.to_string()
                        };
                        if *m {
                            format!("mut {}", v)
                        } else {
                            v
                        }
                    })
                    .collect();
                out.push(format!("{}({})", entry.name, shown.join(", ")));
            }
        }
    }
    out
}

/// The instructions a mutant may run on a call the original ran in `steps`.
fn limit(steps: u64) -> u64 {
    steps
        .saturating_mul(STEP_FACTOR)
        .saturating_add(STEP_ALLOWANCE)
}

fn kill_kind(printed: &Printed) -> Kill {
    match printed {
        Err(s) if s.starts_with("broken MIR") => Kill::Broken,
        Err(s) if s == &cfg::Stop::OutOfSteps.to_string() => Kill::Timeout,
        _ => Kill::Output,
    }
}

/// What the reference interpreter prints for `name` on `args`, as `mn run` prints it.
fn reference(module: &CheckedModule, name: &str, args: Vec<(bool, Value)>) -> Printed {
    let marked: Vec<bool> = args.iter().map(|(m, _)| *m).collect();
    let values = args.into_iter().map(|(_, v)| v).collect();
    let (v, finals) =
        mente_interp::call_leaving(module, name, values, mente_interp::Limits::default())
            .map_err(|s| s.to_string())?;
    let mut printed = vec![v.to_string()];
    for (_, last) in marked.iter().zip(finals).filter(|(m, _)| **m) {
        printed.push(format!("leaves {}", last));
    }
    Ok(printed)
}

/// Prepares `call` to run on mutants. A call MIR never runs, of a function without a body or
/// with arguments of the wrong shape, cannot tell mutants apart: `None`. MIR printing what the
/// reference interpreter does not is an error.
fn prepare(
    module: &CheckedModule,
    lowered: &Lowered,
    call: &str,
) -> Result<Option<Prepared>, String> {
    let (name, args) = mente_interp::parse_call_modes(call).ok_or("not a call")?;
    let expected = reference(module, &name, args.clone());
    let through_mir = mir_run::run_lowered(lowered, &name, args.clone());
    if through_mir != expected {
        return Err(format!(
            "MIR prints {:?} where the reference interpreter prints {:?}",
            through_mir, expected
        ));
    }
    let Some(entry) = lowered.entries.iter().find(|e| e.name == name) else {
        return Ok(None);
    };
    if args.len() != entry.params.len() {
        return Ok(None);
    }
    // An argument outside its parameter's type traps before the entry runs, on every path.
    let outside = args
        .iter()
        .map(|(_, v)| native_arg(v))
        .collect::<Option<Vec<_>>>()
        .is_none_or(|native_args| native::refusal(entry, &native_args).is_some());
    if outside {
        return Ok(None);
    }
    let mut memory = Memory::filled(POISON);
    let mut values = Vec::new();
    let sret = if entry.result.in_memory() {
        let at = memory.alloc(entry.result.size());
        values.push(Val::Addr(at));
        Some((at, entry.result.clone()))
    } else {
        None
    };
    let mut places = Vec::new();
    for ((is_mut, shape), (_, value)) in entry.params.iter().zip(&args) {
        match (is_mut, shape) {
            (false, Shape::Int(_) | Shape::Bool) => match scalar(value) {
                Some(v) => values.push(v),
                None => return Ok(None),
            },
            (_, shape) => {
                let at = memory.alloc(shape.size());
                if write(&mut memory, at, shape, value).is_none() {
                    return Ok(None);
                }
                values.push(Val::Addr(at));
                if *is_mut {
                    places.push((at, shape.clone()));
                }
            }
        }
    }
    let mut c = Prepared {
        text: call.to_string(),
        func: entry.func,
        values,
        memory,
        sret,
        places,
        expected,
        steps: 0,
    };
    let (printed, steps) = execute(&lowered.module, &c, cfg::Limits::default().max_steps);
    if printed != c.expected {
        // `mn run --mir` agreed with the reference interpreter on zeroed memory: the lowered
        // code reads a slot before it writes it.
        return Err(format!(
            "with new stack slots holding {:#04x}, MIR prints {:?} where it prints {:?} on zeroed ones",
            POISON, printed, c.expected
        ));
    }
    c.steps = steps;
    Ok(Some(c))
}

/// Runs a prepared call on `m` within `max_steps`, and prints as `mn run` does.
fn execute(m: &cfg::Module, c: &Prepared, max_steps: u64) -> (Printed, u64) {
    let mut memory = c.memory.clone();
    let limits = cfg::Limits {
        max_steps,
        ..cfg::Limits::default()
    };
    let (result, steps) = cfg::run_counting(m, c.func, &c.values, &mut memory, limits);
    let printed = result.map_err(|s| s.to_string()).and_then(|v| {
        let mut out = vec![match &c.sret {
            Some((at, shape)) => read(&memory, *at, shape)?,
            None => show(v),
        }];
        for (at, shape) in &c.places {
            out.push(format!("leaves {}", read(&memory, *at, shape)?));
        }
        Ok(out)
    });
    (printed, steps)
}

/// An argument as native code and `native::refusal` take it.
fn native_arg(value: &Value) -> Option<native::Arg> {
    Some(match value {
        Value::Bool(b) => native::Arg::Bool(*b),
        Value::Int(i) => native::Arg::Int(Num::new(i.is_negative(), i.magnitude())),
        Value::Array(items) => {
            native::Arg::Array(items.iter().map(native_arg).collect::<Option<Vec<_>>>()?)
        }
        Value::Struct(_, fields) => native::Arg::Struct(
            fields
                .iter()
                .map(|(_, v)| native_arg(v))
                .collect::<Option<Vec<_>>>()?,
        ),
        Value::Unit => return None,
    })
}

fn scalar(value: &Value) -> Option<Val> {
    match value {
        Value::Bool(b) => Some(Val::Bool(*b)),
        Value::Int(i) => Some(Val::Int(Num::new(i.is_negative(), i.magnitude()))),
        _ => None,
    }
}

/// Writes `value` of `shape` at `at`, as `mn run --mir` does; `None` when it does not fit.
fn write(memory: &mut Memory, at: u64, shape: &Shape, value: &Value) -> Option<()> {
    match (shape, value) {
        (Shape::Int(t), v) => {
            let n = scalar(v).and_then(|v| match v {
                Val::Int(n) => Some(n),
                _ => None,
            })?;
            let t = lower::mir(*t);
            t.holds(n).then_some(())?;
            memory.store(at, Val::Int(n), Ty::Int(t)).ok()
        }
        (Shape::Bool, Value::Bool(b)) => memory.store(at, Val::Bool(*b), Ty::Bool).ok(),
        (Shape::Array(len, elem), Value::Array(items)) if items.len() == *len as usize => {
            for (k, item) in items.iter().enumerate() {
                write(memory, at + u64::from(elem.size()) * k as u64, elem, item)?;
            }
            Some(())
        }
        (Shape::Struct(st), Value::Struct(_, fields)) if fields.len() == st.fields.len() => {
            for (f, (_, item)) in st.fields.iter().zip(fields) {
                write(memory, at + u64::from(f.offset), &f.shape, item)?;
            }
            Some(())
        }
        _ => None,
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
        Shape::Struct(st) => {
            let fields = st
                .fields
                .iter()
                .map(|f| {
                    let v = read(memory, at + u64::from(f.offset), &f.shape)?;
                    Ok(format!("{}: {}", f.name, v))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(format!("{}({})", st.name, fields.join(", ")))
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

// -------------------------------------------------------------------------------------------
// Sources of programs
// -------------------------------------------------------------------------------------------

/// The gate's own corpus: each `.mn` file of [`DIR`], with a call on each line that starts
/// `// call: `. A line `// without proofs` checks it without Tier 1, so that its calls reach
/// the traps Tier 1 would reject at compile time.
pub fn corpus(root: &Path) -> Result<Vec<Program>, String> {
    let dir = root.join(DIR);
    let mut files: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {}", dir.display(), e))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mn"))
        .collect();
    files.sort();
    let mut programs = Vec::new();
    for file in files {
        let source = fs::read_to_string(&file).map_err(|e| format!("{}: {}", file.display(), e))?;
        let calls = source
            .lines()
            .filter_map(|l| l.trim().strip_prefix("// call: "))
            .map(|c| c.trim().to_string())
            .collect();
        let name = format!(
            "{}/{}",
            DIR,
            file.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        programs.push(Program {
            name,
            proofs: !source.lines().any(|l| l.trim() == "// without proofs"),
            source,
            calls,
            gated: true,
        });
    }
    Ok(programs)
}

/// The construct corpus of `mir_run.rs`, with the calls its tests make, which are fixed there:
/// reported, not gated, and checked without Tier 1 as those tests check it, so that its calls
/// reach the traps Tier 1 would reject. Each is named after the functions it defines.
pub fn constructs() -> Vec<Program> {
    mir_run::CONSTRUCTS
        .iter()
        .map(|(src, calls)| Program {
            name: format!(
                "compiler/mente-cli/src/mir_run.rs#{}",
                functions(src).join("+")
            ),
            source: format!(
                "module m
{}
",
                src
            ),
            calls: calls.iter().map(|c| c.to_string()).collect(),
            proofs: false,
            gated: false,
        })
        .collect()
}

/// The names of the functions a source defines, in order: each word after a `fn` keyword.
fn functions(src: &str) -> Vec<&str> {
    src.match_indices("fn ")
        .filter(|(at, _)| {
            src[..*at]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace)
        })
        .filter_map(|(at, _)| {
            let rest = &src[at + 3..];
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
            (end > 0).then(|| &rest[..end])
        })
        .collect()
}

/// The benchmark's agent programs that check, each with the calls of its task's examples and
/// hidden tests, which are fixed: their survivors are reported, not gated.
pub fn agent_programs(root: &Path) -> Vec<Program> {
    let bench = root.join("benchmark");
    let tasks: Vec<String> = fs::read_dir(bench.join("tasks"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .strip_suffix(".md")
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut programs = Vec::new();
    for dir in [
        "probe/m1",
        "prelim/m1",
        "prelim/m1-non-core",
        "prelim/m1-harder/run-1",
        "prelim/m1-harder/run-2",
    ] {
        let Ok(entries) = fs::read_dir(bench.join(dir)) else {
            continue;
        };
        let mut files: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for file in files {
            let Some(name) = file.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            let (true, Ok(source)) = (name.ends_with(".mn"), fs::read_to_string(&file)) else {
                continue;
            };
            // The task a program solves is the longest task name its file name contains.
            let Some(task) = tasks
                .iter()
                .filter(|t| name.contains(t.as_str()))
                .max_by_key(|t| t.len())
            else {
                continue;
            };
            if mente_sema::check_program(&source).is_err() {
                continue;
            }
            programs.push(Program {
                name: format!("benchmark/{}/{}", dir, name),
                calls: task_calls(&bench.join("tasks").join(format!("{}.md", task))),
                source,
                proofs: true,
                gated: false,
            });
        }
    }
    programs
}

/// The calls of a task's examples and hidden tests: each indented line up to ` == ` or
/// ` leaves `.
fn task_calls(task: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(task) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("## ") {
            inside = line == "## Examples" || line == "## Hidden tests";
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

// -------------------------------------------------------------------------------------------
// Equivalences and the verdict
// -------------------------------------------------------------------------------------------

/// The survivors recorded as equivalent, each with its reason: `equivalent.txt`, one mutant a
/// line, `program::mutant -- reason`, with `#` starting a comment line.
#[derive(Clone, Debug, Default)]
pub struct Equivalences {
    pub reasons: BTreeMap<String, String>,
}

impl Equivalences {
    pub fn parse(text: &str) -> Result<Equivalences, String> {
        let mut reasons = BTreeMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((id, reason)) = line.split_once(" -- ") else {
                return Err(format!("line {}: not `program::mutant -- reason`", n + 1));
            };
            let (id, reason) = (id.trim(), reason.trim());
            if !id.contains("::") || reason.is_empty() {
                return Err(format!(
                    "line {}: a mutant and a written reason are both needed",
                    n + 1
                ));
            }
            if reasons.insert(id.to_string(), reason.to_string()).is_some() {
                return Err(format!("line {}: `{}` is recorded twice", n + 1, id));
            }
        }
        Ok(Equivalences { reasons })
    }

    pub fn read(root: &Path) -> Result<Equivalences, String> {
        let path = root.join(DIR).join("equivalent.txt");
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        Equivalences::parse(&text).map_err(|e| format!("{}: {}", path.display(), e))
    }
}

/// Counts of one family's mutants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub mutants: usize,
    pub killed: usize,
    pub broken: usize,
    pub timeout: usize,
    pub equivalent: usize,
    pub survived: usize,
    pub skipped: usize,
}

impl Counts {
    fn add(&mut self, o: Counts) {
        self.mutants += o.mutants;
        self.killed += o.killed;
        self.broken += o.broken;
        self.timeout += o.timeout;
        self.equivalent += o.equivalent;
        self.survived += o.survived;
        self.skipped += o.skipped;
    }
}

/// The gate's verdict on a run.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Counts by family, of the gated programs and of the reported ones.
    pub gated: BTreeMap<Family, Counts>,
    pub reported: BTreeMap<Family, Counts>,
    /// Survivors of gated programs with no written reason: each with its line and column.
    pub unexplained: Vec<(String, Loc)>,
    /// Survivors of reported programs with no written reason.
    pub unexplained_reported: Vec<(String, Loc)>,
    /// Mutants recorded as equivalent that a call kills, each with that call.
    pub refuted: Vec<(String, String)>,
    /// Records of programs that ran, naming a mutant they no longer have.
    pub stale: Vec<String>,
    /// Problems of gated programs: each fails the gate.
    pub problems: Vec<String>,
    /// Problems of reported programs, such as one that does not lower.
    pub problems_reported: Vec<String>,
    /// Programs run, gated and reported, and the calls that reached MIR.
    pub programs: (usize, usize),
    pub calls: usize,
}

impl Report {
    pub fn passes(&self) -> bool {
        self.unexplained.is_empty()
            && self.refuted.is_empty()
            && self.stale.is_empty()
            && self.problems.is_empty()
    }

    /// A table of counts by family, with a total row.
    pub fn table(counts: &BTreeMap<Family, Counts>) -> String {
        let mut out = format!(
            "  {:<9} {:>7} {:>7} {:>7} {:>7} {:>10} {:>8} {:>7}\n",
            "family", "mutants", "killed", "broken", "timeout", "equivalent", "survived", "skipped"
        );
        let mut total = Counts::default();
        let row = |name: &str, c: &Counts| {
            format!(
                "  {:<9} {:>7} {:>7} {:>7} {:>7} {:>10} {:>8} {:>7}\n",
                name, c.mutants, c.killed, c.broken, c.timeout, c.equivalent, c.survived, c.skipped
            )
        };
        for family in Family::ALL {
            let c = counts.get(&family).copied().unwrap_or_default();
            total.add(c);
            out.push_str(&row(family.name(), &c));
        }
        out.push_str(&row("total", &total));
        out
    }

    /// What `mn test --mutate` prints for this report, the run having taken `took`.
    pub fn render(&self, took: std::time::Duration) -> String {
        let mut out = format!(
            "MIR mutation gate on lowered code: {} gated programs, {} calls that reach MIR, in {:.1} s\n{}",
            self.programs.0,
            self.calls,
            took.as_secs_f64(),
            Report::table(&self.gated)
        );
        if self.programs.1 > 0 {
            out.push_str(&format!(
                "Reported programs, whose calls are fixed tests: {}\n{}",
                self.programs.1,
                Report::table(&self.reported)
            ));
            for p in &self.problems_reported {
                out.push_str(&format!("  reported problem: {}\n", p));
            }
            for (id, loc) in &self.unexplained_reported {
                out.push_str(&format!("  survives: {} at {}:{}\n", id, loc.line, loc.col));
            }
        }
        for p in &self.problems {
            out.push_str(&format!("  problem: {}\n", p));
        }
        for (id, loc) in &self.unexplained {
            out.push_str(&format!(
                "  survives with no written reason: {} at {}:{}\n",
                id, loc.line, loc.col
            ));
        }
        for (id, call) in &self.refuted {
            out.push_str(&format!(
                "  recorded as equivalent, but `{}` kills it: {}\n",
                call, id
            ));
        }
        for id in &self.stale {
            out.push_str(&format!(
                "  recorded, but the program has no such mutant: {}\n",
                id
            ));
        }
        out.push_str(if self.passes() {
            "[PASS] every mutant of the gated programs is killed or has a written reason\n"
        } else {
            "[FAIL] add a call that kills each survivor, or record why none can in equivalent.txt\n"
        });
        out
    }
}

/// Runs the gate on `programs`, against the recorded equivalences.
pub fn gate(programs: &[Program], eq: &Equivalences) -> Report {
    verdict(&run_programs(programs), eq)
}

/// The programs `mn test --mutate` runs: the gate's own corpus, and with `reported` the sources
/// whose calls are fixed, the construct corpus and the benchmark's agent programs, after it.
pub fn programs(root: &Path, reported: bool) -> Result<Vec<Program>, String> {
    let mut programs = corpus(root)?;
    if reported {
        programs.extend(constructs());
        programs.extend(agent_programs(root));
    }
    Ok(programs)
}

/// `mn test --mutate`'s gate on lowered code, on [`programs`]. Prints the counts by family and
/// every survivor of a gated program without a written reason, and returns whether the gate
/// passes.
pub fn command(root: &Path, reported: bool) -> bool {
    let started = std::time::Instant::now();
    let (programs, eq) = match (programs(root, reported), Equivalences::read(root)) {
        (Ok(p), Ok(e)) => (p, e),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("error: {}", e);
            return false;
        }
    };
    let report = gate(&programs, &eq);
    print!("{}", report.render(started.elapsed()));
    report.passes()
}

/// The verdict on runs already made: every survivor of a gated program must have a written
/// reason, no recorded mutant may be killed, and no record of a program that ran may name a
/// mutant it does not have.
pub fn verdict(runs: &[ProgramRun], eq: &Equivalences) -> Report {
    let mut report = Report::default();
    let mut seen = std::collections::BTreeSet::new();
    for run in runs {
        let problems = run.problems.iter().map(|p| format!("{}: {}", run.name, p));
        if run.gated {
            report.programs.0 += 1;
            report.problems.extend(problems);
        } else {
            report.programs.1 += 1;
            report.problems_reported.extend(problems);
        }
        report.calls += run.calls;
        let counts = if run.gated {
            &mut report.gated
        } else {
            &mut report.reported
        };
        for (_, family) in &run.skipped {
            counts.entry(*family).or_default().skipped += 1;
        }
        for o in &run.outcomes {
            seen.insert(o.id.clone());
            let c = counts.entry(o.family).or_default();
            c.mutants += 1;
            match (&o.killed, eq.reasons.contains_key(&o.id)) {
                (Some((kind, call)), recorded) => {
                    c.killed += 1;
                    match kind {
                        Kill::Broken => c.broken += 1,
                        Kill::Timeout => c.timeout += 1,
                        Kill::Output => {}
                    }
                    if recorded {
                        report.refuted.push((o.id.clone(), call.clone()));
                    }
                }
                (None, true) => c.equivalent += 1,
                (None, false) => {
                    c.survived += 1;
                    if run.gated {
                        report.unexplained.push((o.id.clone(), o.loc));
                    } else {
                        report.unexplained_reported.push((o.id.clone(), o.loc));
                    }
                }
            }
        }
    }
    let ran: std::collections::BTreeSet<&str> = runs.iter().map(|r| r.name.as_str()).collect();
    for id in eq.reasons.keys() {
        let program = id.split("::").next().unwrap_or_default();
        if ran.contains(program) && !seen.contains(id) {
            report.stale.push(id.clone());
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;

    fn program(source: &str, calls: &[&str]) -> Program {
        Program {
            name: "t.mn".into(),
            source: format!("module m\n{}\n", source),
            calls: calls.iter().map(|c| c.to_string()).collect(),
            proofs: false,
            gated: true,
        }
    }

    fn outcome<'a>(run: &'a ProgramRun, id: &str) -> &'a Outcome {
        run.outcomes.iter().find(|o| o.id == id).unwrap_or_else(|| {
            let all: Vec<&str> = run.outcomes.iter().map(|o| o.id.as_str()).collect();
            panic!("no mutant {} among {:?}", id, all)
        })
    }

    /// `if x < 0 { 0 - x } else { x }`: the comparison's negation is killed by the first call
    /// that is not zero, and `<` to `<=` by none, since at zero both arms give zero.
    #[test]
    fn test_a_mutant_of_a_known_function_is_killed_by_a_known_input() {
        let p = program(
            "pub fn abs(x: i8) -> i16 = if x < 0 { 0 - x } else { x }",
            &["abs(0)", "abs(5)", "abs(-3)"],
        );
        let run = run_program(&p);
        assert!(run.problems.is_empty(), "{:?}", run.problems);
        // The three written calls, and abs(x) at -128 and 127.
        assert_eq!(run.calls, 5);
        let negated = outcome(&run, "t.mn::abs/b0.i2/lt>ge");
        assert_eq!(negated.family, Family::Cmp);
        assert_eq!(negated.loc.line, 2);
        let (kind, by) = negated.killed.clone().expect("killed");
        assert_eq!(kind, Kill::Output);
        assert!(by == "abs(5)" || by == "abs(-3)", "{}", by);
        assert_eq!(outcome(&run, "t.mn::abs/b0.i2/lt>le").killed, None);
        // The entry of an unrefined function only calls it, and has nothing to mutate.
        assert!(run.outcomes.iter().all(|o| !o.id.contains("enter abs")));
    }

    /// A survivor fails the gate until `equivalent.txt` records it with a reason; a record of a
    /// mutant some call kills, or of one the program no longer has, fails it too.
    #[test]
    fn test_a_survivor_without_a_written_reason_fails_the_gate() {
        let p = program(
            "pub fn abs(x: i8) -> i16 = if x < 0 { 0 - x } else { x }",
            &["abs(0)", "abs(5)", "abs(-3)"],
        );
        let runs = run_programs(std::slice::from_ref(&p));
        let none = verdict(&runs, &Equivalences::default());
        assert!(!none.passes());
        let ids: Vec<&str> = none.unexplained.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"t.mn::abs/b0.i2/lt>le"), "{:?}", ids);

        let mut text = String::from("# reasons\n");
        for id in &ids {
            text.push_str(&format!("{} -- reasoned in the test\n", id));
        }
        let recorded = Equivalences::parse(&text).unwrap();
        let all = verdict(&runs, &recorded);
        assert!(all.passes(), "{:?}", all);
        let c = all.gated.get(&Family::Cmp).copied().unwrap_or_default();
        assert!(c.equivalent >= 1 && c.survived == 0, "{:?}", c);

        let refuted = Equivalences::parse(&format!(
            "{}t.mn::abs/b0.i2/lt>ge -- wrong: abs(5) tells it apart\n",
            text
        ))
        .unwrap();
        let wrong = verdict(&runs, &refuted);
        assert!(!wrong.passes());
        assert_eq!(wrong.refuted.len(), 1);

        let stale =
            Equivalences::parse(&format!("{}t.mn::abs/b9.i9/lt>le -- gone\n", text)).unwrap();
        assert_eq!(verdict(&runs, &stale).stale, ["t.mn::abs/b9.i9/lt>le"]);

        // A survivor of a program whose calls are fixed is reported, not failed on.
        let fixed = Program { gated: false, ..p };
        let report = verdict(&run_programs(&[fixed]), &Equivalences::default());
        assert!(report.passes(), "{:?}", report);
        assert!(!report.unexplained_reported.is_empty());

        for bad in [
            "t.mn::x",
            "t.mn::x -- ",
            "x -- why",
            "t.mn::x -- a\nt.mn::x -- b",
        ] {
            assert!(Equivalences::parse(bad).is_err(), "{}", bad);
        }
    }

    /// The verdict counts each outcome under its family, the gated programs apart from the
    /// reported ones, and every call and skipped mutant.
    #[test]
    fn test_the_verdict_counts_every_outcome() {
        let outcome = |id: &str, family, killed: Option<Kill>| Outcome {
            id: id.into(),
            family,
            loc: Loc::default(),
            killed: killed.map(|k| (k, "f(1)".to_string())),
        };
        let gated = ProgramRun {
            name: "g".into(),
            gated: true,
            outcomes: vec![
                outcome("g::f/b0.i0/lt>le", Family::Cmp, Some(Kill::Output)),
                outcome("g::f/b0.i1/lt>ge", Family::Cmp, Some(Kill::Broken)),
                outcome("g::f/b0.i2/int+1", Family::Const, Some(Kill::Timeout)),
                outcome("g::f/b0.i3/int-1", Family::Const, None),
                outcome("g::f/b0.i4/flip", Family::Const, None),
            ],
            skipped: vec![("g::f/b0.i5/shl>id".into(), Family::Identity)],
            problems: Vec::new(),
            calls: 3,
        };
        let reported = |name: &str| ProgramRun {
            name: name.into(),
            gated: false,
            outcomes: vec![outcome(
                &format!("{}::h/b1.term/swap", name),
                Family::Branch,
                None,
            )],
            skipped: vec![
                (format!("{}::h/b1.i0/shl>id", name), Family::Identity),
                (format!("{}::h/b1.i1/shl>id", name), Family::Identity),
            ],
            problems: Vec::new(),
            calls: 2,
        };
        let eq = Equivalences::parse("g::f/b0.i3/int-1 -- reasoned\n").unwrap();
        let report = verdict(&[gated, reported("r"), reported("s")], &eq);
        assert_eq!((report.programs, report.calls), ((1, 2), 7));
        let get = |m: &BTreeMap<Family, Counts>, f| m.get(&f).copied().unwrap_or_default();
        let counts = |mutants, killed, broken, timeout, equivalent, survived, skipped| Counts {
            mutants,
            killed,
            broken,
            timeout,
            equivalent,
            survived,
            skipped,
        };
        assert_eq!(get(&report.gated, Family::Cmp), counts(2, 2, 1, 0, 0, 0, 0));
        assert_eq!(
            get(&report.gated, Family::Const),
            counts(3, 1, 0, 1, 1, 1, 0)
        );
        assert_eq!(
            get(&report.gated, Family::Identity),
            counts(0, 0, 0, 0, 0, 0, 1)
        );
        assert_eq!(
            get(&report.reported, Family::Branch),
            counts(2, 0, 0, 0, 0, 2, 0)
        );
        assert_eq!(
            get(&report.reported, Family::Identity),
            counts(0, 0, 0, 0, 0, 0, 4)
        );
        assert_eq!(report.unexplained.len(), 1);
        assert_eq!(report.unexplained_reported.len(), 2);
    }

    /// A call whose array argument has another length is refused alike by the reference
    /// interpreter and MIR before anything runs, so it reaches no mutant and is no problem.
    #[test]
    fn test_calls_of_another_shape_are_left_out() {
        let p = program(
            "pub fn h(xs: [2]u8) -> u8 = xs[0]",
            &["h([1, 2])", "h([1])", "h([1, 2, 3])"],
        );
        let run = run_program(&p);
        assert!(run.problems.is_empty(), "{:?}", run.problems);
        assert_eq!(run.calls, 1);
    }

    /// A struct argument written into memory has each of its struct's fields: another number
    /// of fields is refused, whatever else checked the call before.
    #[test]
    fn test_a_struct_argument_is_written_only_with_each_field() {
        let text = "module m\npub struct P {\n  a: u8\n  b: u8\n}\npub fn h(p: P) -> u8 = p.a\n";
        let lowered = lower::lower(&mente_sema::check_program(text).unwrap()).unwrap();
        let shape = &lowered.entries[0].params[0].1;
        let value = |fields: &[&str]| {
            Value::Struct(
                "P".into(),
                fields
                    .iter()
                    .map(|f| (f.to_string(), Value::Int(mente_interp::Int::from_u128(1))))
                    .collect(),
            )
        };
        let mut memory = Memory::filled(POISON);
        let at = memory.alloc(shape.size());
        assert!(write(&mut memory, at, shape, &value(&["a", "b"])).is_some());
        assert!(write(&mut memory, at, shape, &value(&["a"])).is_none());
        assert!(write(&mut memory, at, shape, &value(&["a", "b", "c"])).is_none());
    }

    /// A program MIR runs differently from the reference interpreter, or one that does not
    /// check, is a problem of its own; a call MIR never reaches tells no mutant apart.
    #[test]
    fn test_programs_that_cannot_take_part_are_problems() {
        let broken = program("pub fn f(x: u8) -> u8 = y", &["f(1)"]);
        assert!(run_program(&broken).problems[0].starts_with("does not check"));
        let p = program(
            "pub fn f(x: u8) -> u8 = x",
            &["f(1)", "g(1)", "f(300)", "nonsense"],
        );
        let run = run_program(&p);
        // `f(300)` traps before the entry runs and `g(1)` names no function, so neither reaches
        // MIR; f(1) also comes back at 0 and 255.
        assert_eq!(run.calls, 3);
        assert_eq!(run.problems, ["nonsense: not a call"]);
        // A problem fails the gate for a gated program, and is only reported for another.
        let failing = verdict(&[run_program(&broken)], &Equivalences::default());
        assert!(!failing.passes() && failing.problems_reported.is_empty());
        assert!(failing.problems[0].starts_with("t.mn: does not check"));
        let fixed = Program {
            gated: false,
            ..broken
        };
        let report = verdict(&[run_program(&fixed)], &Equivalences::default());
        assert!(report.passes() && report.problems.is_empty());
        assert_eq!(report.problems_reported.len(), 1);
    }

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// A folder of its own under the system's temporary one, empty.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mente-mutants-{}-{}", std::process::id(), name));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// The gate's own corpus passes: every mutant of every program is killed or recorded, every
    /// family has mutants and kills, and every record names a mutant that survives.
    #[test]
    fn test_the_corpus_passes_the_gate() {
        let programs = corpus(&root()).unwrap();
        let eq = Equivalences::read(&root()).unwrap();
        let report = gate(&programs, &eq);
        assert!(report.passes(), "{}", report.render(Duration::ZERO));
        assert_eq!(report.programs, (programs.len(), 0));
        assert!(
            programs.len() >= 9 && report.calls >= 400,
            "{:?}",
            report.programs
        );
        let mut total = Counts::default();
        for family in Family::ALL {
            let c = report.gated.get(&family).copied().unwrap_or_default();
            assert!(c.mutants > 0 && c.killed > 0, "{}: {:?}", family.name(), c);
            total.add(c);
        }
        assert_eq!(total.mutants, total.killed + total.equivalent);
        assert_eq!(total.equivalent, eq.reasons.len());
        assert!(total.broken > 0 && total.timeout > 0, "{:?}", total);
        assert!(report.reported.is_empty());
    }

    /// A corpus program's calls are its `// call: ` lines, and `// without proofs` turns Tier 1
    /// off; files that are not `.mn` are left alone, and a missing folder is an error.
    #[test]
    fn test_corpus_programs_carry_their_calls() {
        let dir = scratch("corpus");
        assert!(corpus(&dir).is_err());
        put(
            &dir.join(DIR).join("b.mn"),
            "// without proofs\n// call: f(1)\n  // call:  f(2) \nmodule m\npub fn f(x: u8) -> u8 = x\n",
        );
        put(&dir.join(DIR).join("a.mn"), "// call: g(true)\nmodule m\n");
        put(&dir.join(DIR).join("notes.txt"), "// call: f(3)\n");
        let found = corpus(&dir).unwrap();
        let shown: Vec<(String, Vec<String>, bool, bool)> = found
            .into_iter()
            .map(|p| (p.name, p.calls, p.proofs, p.gated))
            .collect();
        assert_eq!(
            shown,
            [
                (
                    format!("{}/a.mn", DIR),
                    vec!["g(true)".to_string()],
                    true,
                    true
                ),
                (
                    format!("{}/b.mn", DIR),
                    vec!["f(1)".to_string(), "f(2)".to_string()],
                    false,
                    true
                ),
            ]
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// The construct corpus enters as reported programs, without Tier 1, each named apart after
    /// the functions it defines and carrying the calls `mir_run.rs` makes.
    #[test]
    fn test_the_construct_corpus_is_reported_under_its_function_names() {
        let found = constructs();
        assert_eq!(found.len(), mir_run::CONSTRUCTS.len());
        let mut names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names[0],
            "compiler/mente-cli/src/mir_run.rs#bind+take+arg+back"
        );
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), found.len(), "two constructs share a name");
        for (p, (src, calls)) in found.iter().zip(mir_run::CONSTRUCTS) {
            assert!(!p.gated && !p.proofs, "{}", p.name);
            assert_eq!(p.source, format!("module m\n{}\n", src));
            assert_eq!(p.calls, calls.to_vec());
        }
        assert_eq!(
            functions("pub fn a(x: u8) -> u8 = todo(\"fn x\")\nfn b_2() {}\n  fn c() {}\nfn (x)"),
            ["a", "b_2", "c"]
        );
    }

    /// An agent program that checks gets the calls of its task's examples and hidden tests, its
    /// task being the longest task name its file name holds; its survivors are reported.
    #[test]
    fn test_agent_programs_take_their_tasks_tests() {
        let dir = scratch("agents");
        let bench = dir.join("benchmark");
        put(
            &bench.join("tasks/pick.md"),
            "# Pick\n\n## Signature\n\n    pick(x: u8) -> u8\n\n## Examples\n\n    pick(1) == 1\nprose pick(9)\n\n## Hidden tests\n\n    bump(mut 2) leaves 3\n    pick(4)\n\n## Notes\n\n    pick(5) == 5\n",
        );
        put(
            &bench.join("tasks/pick-two.md"),
            "## Examples\n\n    pick(7) == 7\n",
        );
        let src = "module m\npub fn pick(x: u8) -> u8 = x\n";
        put(&bench.join("probe/m1/pick-1.mn"), src);
        put(&bench.join("prelim/m1/pick-two-1.mn"), src);
        put(
            &bench.join("prelim/m1/pick-2.mn"),
            "module m\npub fn pick(x: u8) -> u8 = y\n",
        );
        put(&bench.join("prelim/m1/other.mn"), src);
        put(&bench.join("prelim/m1/pick-3.txt"), src);
        let found = agent_programs(&dir);
        let shown: Vec<(String, Vec<String>, bool, bool)> = found
            .into_iter()
            .map(|p| (p.name, p.calls, p.proofs, p.gated))
            .collect();
        let calls = |cs: &[&str]| cs.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        assert_eq!(
            shown,
            [
                (
                    "benchmark/probe/m1/pick-1.mn".to_string(),
                    calls(&["pick(1)", "bump(mut 2)", "pick(4)"]),
                    true,
                    false
                ),
                (
                    "benchmark/prelim/m1/pick-two-1.mn".to_string(),
                    calls(&["pick(7)"]),
                    true,
                    false
                ),
            ]
        );
        assert_eq!(
            task_calls(&bench.join("tasks/missing.md")),
            Vec::<String>::new()
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// Each function's first written call comes back with each integer argument, `mut` or not,
    /// at its type's smallest and largest value; arrays and booleans stay as written.
    #[test]
    fn test_edge_calls_move_each_integer_to_its_types_edges() {
        let src = "module m\npub fn f(a: u8, mut b: u8, c: i128, xs: [2]u8) -> [2]u8 {\n  b = a\n  [xs[0], c]\n}\npub fn g(n: i8) -> i8 = n\n";
        let module = mente_sema::check_program_without_proofs(src).unwrap();
        let lowered = lower::lower(&module).unwrap();
        let written = [
            "f(1)",
            "f(1, mut 2, 3, [4, 5])",
            "f(9, mut 9, 9, [9, 9])",
            "h(1)",
        ];
        let written: Vec<String> = written.iter().map(|c| c.to_string()).collect();
        let max = "170141183460469231731687303715884105727";
        assert_eq!(
            edge_calls(&lowered, &written),
            [
                "f(0, mut 2, 3, [4, 5])".to_string(),
                "f(255, mut 2, 3, [4, 5])".into(),
                "f(1, mut 0, 3, [4, 5])".into(),
                "f(1, mut 255, 3, [4, 5])".into(),
                format!(
                    "f(1, mut 2, -{}, [4, 5])",
                    "170141183460469231731687303715884105728"
                ),
                format!("f(1, mut 2, {}, [4, 5])", max),
            ]
        );
        assert_eq!(
            edge_calls(&lowered, &["g(0)".to_string()]),
            ["g(-128)", "g(127)"]
        );
    }

    /// The counts by family print as a table with a total, and a report prints each kind of
    /// finding and its verdict.
    #[test]
    fn test_reports_print_counts_and_findings() {
        let mut counts = BTreeMap::new();
        let one = Counts {
            mutants: 9,
            killed: 5,
            broken: 2,
            timeout: 1,
            equivalent: 3,
            survived: 1,
            skipped: 4,
        };
        counts.insert(Family::Cmp, one);
        counts.insert(Family::Trap, one);
        let table = Report::table(&counts);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 10);
        assert_eq!(
            lines[0],
            "  family    mutants  killed  broken timeout equivalent survived skipped"
        );
        assert_eq!(
            lines[1],
            "  arith           0       0       0       0          0        0       0"
        );
        assert_eq!(
            lines[3],
            "  cmp             9       5       2       1          3        1       4"
        );
        assert_eq!(
            lines[9],
            "  total          18      10       4       2          6        2       8"
        );
        let names: Vec<&str> = lines[1..9]
            .iter()
            .map(|l| l.split_whitespace().next().unwrap_or(""))
            .collect();
        assert_eq!(
            names,
            ["arith", "bits", "cmp", "const", "branch", "trap", "identity", "memory"]
        );

        let clean = Report {
            programs: (2, 0),
            calls: 7,
            ..Report::default()
        };
        let shown = clean.render(Duration::from_millis(1500));
        assert!(shown.starts_with("MIR mutation gate on lowered code: 2 gated programs, 7 calls that reach MIR, in 1.5 s\n  family"), "{}", shown);
        assert!(shown.ends_with(
            "[PASS] every mutant of the gated programs is killed or has a written reason\n"
        ));
        assert!(!shown.contains("Reported programs"));

        let loc = Loc { line: 3, col: 4 };
        let found = Report {
            programs: (1, 2),
            reported: counts.clone(),
            unexplained: vec![("p::f/b0.i1/lt>le".into(), loc)],
            unexplained_reported: vec![("q::g/b1.term/swap".into(), loc)],
            refuted: vec![("p::f/b0.i2/flip".into(), "f(1)".into())],
            stale: vec!["p::f/b9.i9/flip".into()],
            problems: vec!["p: does not check".into()],
            problems_reported: vec!["q: does not lower".into()],
            ..Report::default()
        };
        let shown = found.render(Duration::ZERO);
        for line in [
            "Reported programs, whose calls are fixed tests: 2\n",
            "  total          18      10       4       2          6        2       8\n",
            "  reported problem: q: does not lower\n",
            "  survives: q::g/b1.term/swap at 3:4\n",
            "  problem: p: does not check\n",
            "  survives with no written reason: p::f/b0.i1/lt>le at 3:4\n",
            "  recorded as equivalent, but `f(1)` kills it: p::f/b0.i2/flip\n",
            "  recorded, but the program has no such mutant: p::f/b9.i9/flip\n",
            "[FAIL] add a call that kills each survivor, or record why none can in equivalent.txt\n",
        ] {
            assert!(shown.contains(line), "no `{}` in:\n{}", line, shown);
        }
        for (r, passes) in [
            (
                Report {
                    unexplained: found.unexplained.clone(),
                    ..Report::default()
                },
                false,
            ),
            (
                Report {
                    refuted: found.refuted.clone(),
                    ..Report::default()
                },
                false,
            ),
            (
                Report {
                    stale: found.stale.clone(),
                    ..Report::default()
                },
                false,
            ),
            (
                Report {
                    problems: found.problems.clone(),
                    ..Report::default()
                },
                false,
            ),
            (
                Report {
                    unexplained_reported: found.unexplained_reported.clone(),
                    problems_reported: found.problems_reported.clone(),
                    ..Report::default()
                },
                true,
            ),
        ] {
            assert_eq!(r.passes(), passes, "{:?}", r);
        }
    }

    /// A mutant may run four times the original's instructions and ten thousand more; a run
    /// stopped as broken MIR or for its steps is told apart from one that printed otherwise.
    #[test]
    fn test_mutant_runs_are_bounded_and_their_kills_named() {
        assert_eq!(limit(0), 10_000);
        assert_eq!(limit(250), 11_000);
        assert_eq!(limit(u64::MAX), u64::MAX);
        assert_eq!(
            kill_kind(&Err("broken MIR: v3 divides by zero unchecked".into())),
            Kill::Broken
        );
        assert_eq!(
            kill_kind(&Err(cfg::Stop::OutOfSteps.to_string())),
            Kill::Timeout
        );
        assert_eq!(
            kill_kind(&Err("trap[narrowing] at 1:2: 300 does not fit".into())),
            Kill::Output
        );
        assert_eq!(kill_kind(&Ok(vec!["5".into()])), Kill::Output);
    }

    /// `mn test --mutate`'s gate passes on a corpus whose mutants are all killed and fails on an
    /// unexplained survivor or a missing file; `--reported` puts the construct corpus and the
    /// agent programs after the gated corpus.
    #[test]
    fn test_the_command_passes_or_fails_on_the_corpus_it_finds() {
        let dir = scratch("command");
        assert!(!command(&dir, false));
        assert!(programs(&dir, true).is_err());
        put(&dir.join(DIR).join("equivalent.txt"), "# none\n");
        put(
            &dir.join(DIR).join("id.mn"),
            "// call: id(true)\nmodule m\npub fn id(x: bool) -> bool = x\n",
        );
        put(
            &dir.join("benchmark/tasks/id.md"),
            "## Examples\n\n    id(false) == false\n",
        );
        put(
            &dir.join("benchmark/probe/m1/id-1.mn"),
            "module m\npub fn id(x: bool) -> bool = if x { false } else { x }\n",
        );
        assert!(command(&dir, false));
        let names = |reported| -> Vec<String> {
            programs(&dir, reported)
                .unwrap()
                .into_iter()
                .map(|p| p.name)
                .collect()
        };
        let gated = format!("{}/id.mn", DIR);
        assert_eq!(names(false), std::slice::from_ref(&gated));
        let mut all = vec![gated];
        all.extend(constructs().into_iter().map(|p| p.name));
        all.push("benchmark/probe/m1/id-1.mn".into());
        assert_eq!(names(true), all);
        put(
            &dir.join(DIR).join("pick.mn"),
            "// call: pick(true)\nmodule m\npub fn pick(c: bool) -> u8 = if c { 1 } else { 2 }\n",
        );
        assert!(!command(&dir, false));
        put(&dir.join(DIR).join("equivalent.txt"), "not a record\n");
        assert!(!command(&dir, false));
        fs::remove_dir_all(&dir).ok();
    }
}
