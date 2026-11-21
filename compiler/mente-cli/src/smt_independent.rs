//! `mn smt --independent` and `mn smt --confirm`: the independent SMT cross-check of Tier 1
//! ([`smt-independent-encoding`](../../../docs/design-review.md#smt-independent-encoding)).
//!
//! `mn smt --independent` hands each checked function to `mente_smt::encode`, which reads only
//! its syntax and the types the checker gives it, and writes a query for each check Tier 1
//! erased: each obligation every record of which Tier 1 proved, and each trap its failure
//! raises, the rounds of its loops and the elements of its arrays joined in one query. A solver
//! must answer `unsat`, which means no input fails the check within the encoding's bounds.
//!
//! `mn smt --confirm` reads the solver's answers. Each `sat` is a model: the inputs of a run the
//! encoding says fails a check Tier 1 proved. The model is checked against its query by
//! `mente_smt::eval`, then run on the reference interpreter: a trap of the query's kind at its
//! place confirms a soundness bug of Tier 1, and the model reproduces it; any other outcome is
//! a defect of the encoding. Either fails the job, unless a file given with `--expect` names
//! the confirmed bug as known (a line `// smt: <function> <trap> at <line>:<col>`).

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use mente_interp::{Int, Limits, Stop, Value};
use mente_sema::tier1::{Obligation, ObligationKind, Tag as Tier1Tag};
use mente_sema::types::{Int as IntKind, Ty};
use mente_sema::{CheckedFn, CheckedModule};
use mente_smt::eval;
use mente_smt::num::Big;
use mente_smt::{Encoding, Facts, Function, IntType, Kind, Module, Shape, Tag};
use mente_syntax::Span;

/// The shape of a type the checker gives.
pub fn shape_of(t: &Ty) -> Shape {
    match t {
        Ty::Int(IntKind::One(t) | IntKind::Mixed(Some(t))) => {
            IntType::named(t.name).map_or(Shape::Untyped, Shape::Int)
        }
        Ty::Int(IntKind::Literal | IntKind::Mixed(None)) => Shape::Untyped,
        Ty::Bool => Shape::Bool,
        Ty::Array(n, e) => Shape::Array(*n, Box::new(shape_of(e))),
        Ty::Struct(_) | Ty::Enum(_) | Ty::Unit | Ty::Any => Shape::Other,
    }
}

/// What the checker knows of a function, as the encoding reads it.
pub fn facts(f: &CheckedFn) -> Facts {
    Facts {
        exprs: f.typing.exprs.iter().map(shape_of).collect(),
        literals: f
            .typing
            .literals
            .iter()
            .map(|t| t.as_ref().and_then(|t| IntType::named(t.ty.name)))
            .collect(),
        bindings: f
            .resolution
            .stmts
            .iter()
            .map(|b| {
                b.and_then(|id| f.typing.bindings.get(id.0 as usize))
                    .map(shape_of)
            })
            .collect(),
        mixed: f
            .typing
            .exprs
            .iter()
            .map(|t| matches!(t, Ty::Int(IntKind::Mixed(_))))
            .collect(),
    }
}

/// A checked module as the encoding reads it.
pub fn module_of(m: &CheckedModule) -> Module<'_> {
    Module {
        functions: m
            .functions
            .iter()
            .map(|f| {
                f.as_ref().map(|f| Function {
                    syntax: &f.syntax,
                    facts: facts(f),
                })
            })
            .collect(),
    }
}

fn kind_of(k: ObligationKind) -> Kind {
    match k {
        ObligationKind::Refinement => Kind::Refinement,
        ObligationKind::Range => Kind::Range,
        ObligationKind::Index => Kind::Index,
        ObligationKind::Divisor => Kind::Divisor,
        ObligationKind::Shift => Kind::Shift,
        ObligationKind::Requires => Kind::Requires,
        ObligationKind::Ensures => Kind::Ensures,
        ObligationKind::Invariant => Kind::Invariant,
        ObligationKind::Decreases => Kind::Decreases,
    }
}

fn tag_of(t: Tier1Tag) -> Tag {
    match t {
        Tier1Tag::None => Tag::None,
        Tier1Tag::Into(name) => Tag::Into(name),
        Tier1Tag::Clause(s) => Tag::Clause(s),
        Tier1Tag::Smaller => Tag::Smaller,
        Tier1Tag::Back => Tag::Back,
    }
}

/// An obligation's key: its kind, its place and its tag.
pub type Key = (Kind, Span, Tag);

/// The key of each of Tier 1's obligations.
pub fn key_of(o: &Obligation) -> Key {
    (kind_of(o.kind), o.span, tag_of(o.tag))
}

/// The checks Tier 1 erased: each key every record of which it proved, as the lowering leaves
/// a check out.
pub fn erased(obligations: &[Obligation]) -> Vec<Key> {
    let mut keys: Vec<(Key, bool)> = Vec::new();
    for o in obligations {
        let key = key_of(o);
        match keys.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 &= o.proven,
            None => keys.push((key, o.proven)),
        }
    }
    keys.into_iter().filter(|e| e.1).map(|e| e.0).collect()
}

/// The constructs whose absence leaves a function with no site at all: a parameter the encoding
/// lacks, before which nothing is encoded, and a function cut off as too large.
const WHOLE: [&str; 3] = [
    "a parameter of a struct or an enum",
    "an array of more elements than an encoding holds",
    "a function too large to encode",
];

/// The obligations Tier 1 records in a function that its encoding has no site for, each a defect
/// of the encoding: a check Tier 1 may erase would have no query. Matched in every function; the
/// obligations within a construct the encoding lacks are left out, which have no site by design,
/// and so is every obligation of a function a parameter it lacks, or its size, leaves unencoded.
pub fn unmatched<'o>(obligations: &'o [Obligation], enc: &Encoding) -> Vec<&'o Obligation> {
    let whole = enc.sites.is_empty() && enc.unencoded.iter().any(|(_, what)| WHOLE.contains(what));
    let exhausted = enc
        .unencoded
        .iter()
        .any(|(_, what)| *what == "a function too large to encode");
    if whole || exhausted {
        return Vec::new();
    }
    let within = |o: &Obligation| {
        enc.unencoded
            .iter()
            .any(|(at, _)| at.start <= o.span.start && o.span.end <= at.end)
    };
    let keys: Vec<Key> = enc.sites.iter().map(|s| s.key()).collect();
    obligations
        .iter()
        .filter(|o| !within(o) && !keys.contains(&key_of(o)))
        .collect()
}

/// The sites of an encoding grouped by obligation and trap, in the order they first appear,
/// for the obligations `keep` keeps.
pub fn groups(enc: &Encoding, keep: impl Fn(&Key) -> bool) -> Vec<Vec<usize>> {
    let mut out: Vec<(Key, mente_smt::Trap, Vec<usize>)> = Vec::new();
    for (k, s) in enc.sites.iter().enumerate() {
        let key = s.key();
        if !keep(&key) {
            continue;
        }
        match out.iter_mut().find(|(g, t, _)| *g == key && *t == s.trap) {
            Some(g) => g.2.push(k),
            None => out.push((key, s.trap, vec![k])),
        }
    }
    out.into_iter().map(|g| g.2).collect()
}

/// Why a check Tier 1 erased has no query: no input reaches it on any path the encoding follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unqueried {
    /// No path was cut off before it: no input reaches it at all.
    Unreachable,
    /// A path was cut off past the bounds before it, which may reach it.
    BeyondBound,
    /// A path was cut off at a construct the encoding lacks before it, which may reach it.
    BeyondLacked,
}

/// Why the sites `g` of one obligation have no query, where none does.
pub fn unqueried(enc: &Encoding, g: &[usize]) -> Unqueried {
    let sites = || g.iter().map(|&k| &enc.sites[k]);
    if sites().any(|s| s.beyond_bound) {
        Unqueried::BeyondBound
    } else if sites().any(|s| s.beyond_lacked) {
        Unqueried::BeyondLacked
    } else {
        Unqueried::Unreachable
    }
}

/// What `mn smt --independent` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub queries: usize,
    pub functions: usize,
    pub files: usize,
    /// Checks Tier 1 erased that no input reaches, no path being cut off before them.
    pub unreachable: usize,
    /// Checks Tier 1 erased that no input reaches within the bounds, a path being cut off past
    /// them before the check.
    pub beyond_bound: usize,
    /// Checks Tier 1 erased that no input reaches, a path being cut off at a construct the
    /// encoding lacks before the check.
    pub beyond_lacked: usize,
    /// Places of constructs the encoding lacks.
    pub unencoded: usize,
    /// Files whose module does not check.
    pub unchecked: usize,
    /// Obligations of Tier 1 with no site.
    pub unmatched: usize,
}

/// The path of a file below `out`, as `mn smt` places a file's queries: its path without `.mn`,
/// its parts joined below `out`.
fn below(out: &Path, file: &Path) -> PathBuf {
    let relative: PathBuf = file
        .with_extension("")
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(part) => Some(part),
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::CurDir
            | std::path::Component::ParentDir => None,
        })
        .collect();
    out.join(relative)
}

/// Writes the independent queries of `files` below `out`, each in a file of its own,
/// `<function>-<line>-<kind>[-n].smt2`, whose first line is `; source: <file>:<line>:<col>`,
/// the obligation's place; prints each construct the encoding lacks, each obligation with no
/// site and each file that does not check, and counts each check with no query by why.
pub fn independent(files: &[PathBuf], out: &Path) -> Result<Written, String> {
    let mut w = Written {
        files: files.len(),
        ..Written::default()
    };
    for file in files {
        let shown = file.display().to_string();
        let src =
            fs::read_to_string(file).map_err(|e| format!("cannot read `{}`: {}", shown, e))?;
        let Ok(checked) = mente_sema::check_program(&src) else {
            println!(
                "unchecked: {}: the module does not check, so it has no queries",
                shown
            );
            w.unchecked += 1;
            continue;
        };
        let module = module_of(&checked);
        let dir = below(out, file);
        let mut seen: HashMap<String, usize> = HashMap::new();
        for (i, f) in checked.functions.iter().enumerate() {
            let Some(cf) = f else {
                continue;
            };
            w.functions += 1;
            let keys = erased(&cf.obligations);
            let mut enc = mente_smt::encode(&module, i);
            for o in unmatched(&cf.obligations, &enc) {
                println!(
                    "unmatched: {}:{}:{}: the {} obligation of `{}` Tier 1 records has no site in the encoding, a defect of the encoding",
                    shown,
                    o.span.line,
                    o.span.col,
                    o.kind.name(),
                    cf.syntax.name
                );
                w.unmatched += 1;
            }
            for (span, what) in &enc.unencoded {
                println!(
                    "unencoded: {}:{}:{}: {} in `{}`",
                    shown, span.line, span.col, what, cf.syntax.name
                );
                w.unencoded += 1;
            }
            for g in groups(&enc, |k| keys.contains(k)) {
                let site = &enc.sites[g[0]];
                let (kind, span) = (site.kind, site.span);
                let Some(text) = enc.query(&g) else {
                    match unqueried(&enc, &g) {
                        Unqueried::Unreachable => w.unreachable += 1,
                        Unqueried::BeyondBound => w.beyond_bound += 1,
                        Unqueried::BeyondLacked => w.beyond_lacked += 1,
                    }
                    continue;
                };
                let base = format!("{}-{}-{}", cf.syntax.name, span.line, kind.name());
                let n = seen.entry(base.clone()).or_insert(0);
                *n += 1;
                let path = match *n {
                    1 => dir.join(format!("{}.smt2", base)),
                    n => dir.join(format!("{}-{}.smt2", base, n)),
                };
                fs::create_dir_all(&dir)
                    .map_err(|e| format!("cannot create `{}`: {}", dir.display(), e))?;
                let place = format!("{}:{}:{}", shown, span.line, span.col);
                fs::write(&path, format!("; source: {}\n{}", place, text))
                    .map_err(|e| format!("cannot write `{}`: {}", path.display(), e))?;
                w.queries += 1;
            }
        }
    }
    Ok(w)
}

/// The counts of the CI job, recorded in [`BASELINE`], which each run is held to: the queries it
/// writes may fall below the record by at most a hundredth, the checks cut off by the bound may
/// rise above it by at most a hundredth, and the undecided answers may pass it by at most
/// [`UNDECIDED_SLACK`], since a timeout depends on the runner. A change that moves a count past
/// its margin records the new count, with why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Baseline {
    pub queries: usize,
    pub beyond_bound: usize,
    pub undecided: usize,
}

/// Where the baseline is kept, from the repository's root.
pub const BASELINE: &str = "compiler/mente-cli/tests/smt-baseline.txt";

/// How many undecided answers past the record a run may give.
pub const UNDECIDED_SLACK: usize = 5;

impl Baseline {
    /// The baseline a file records: its fields `queries`, `beyond-bound` and `undecided`, each
    /// once, as decimal numbers; `#` lines are comments.
    pub fn parse(text: &str) -> Result<Baseline, String> {
        let mut fields: HashMap<&str, usize> = HashMap::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| format!("`{}` is no `name: count` line", line))?;
            let name = name.trim();
            if !["queries", "beyond-bound", "undecided"].contains(&name) {
                return Err(format!("`{}` is no field of the baseline", name));
            }
            let value = value.trim();
            let n = value
                .parse()
                .ok()
                .filter(|_| value.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| format!("`{}: {}` is no decimal count", name, value))?;
            if fields.insert(name, n).is_some() {
                return Err(format!("`{}` is given twice", name));
            }
        }
        let field = |name: &str| {
            fields
                .get(name)
                .copied()
                .ok_or_else(|| format!("the baseline has no `{}`", name))
        };
        Ok(Baseline {
            queries: field("queries")?,
            beyond_bound: field("beyond-bound")?,
            undecided: field("undecided")?,
        })
    }

    /// Why an export falls outside the baseline, if it does.
    pub fn export_problems(&self, w: &Written) -> Vec<String> {
        let mut out = Vec::new();
        let floor = self.queries - self.queries / 100;
        if w.queries < floor {
            out.push(format!(
                "{} queries were written, below the floor of {}, a hundredth under the {} recorded in {}: the encoding lost checks, or the corpus lost programs",
                w.queries, floor, self.queries, BASELINE
            ));
        }
        let ceiling = self.beyond_bound + self.beyond_bound / 100;
        if w.beyond_bound > ceiling {
            out.push(format!(
                "{} checks Tier 1 erased are cut off by the bound, above the ceiling of {}, a hundredth over the {} recorded in {}: the encoding cuts off more paths than it did",
                w.beyond_bound, ceiling, self.beyond_bound, BASELINE
            ));
        }
        out
    }

    /// Why a count of undecided answers falls outside the baseline, if it does.
    pub fn undecided_problem(&self, undecided: usize) -> Option<String> {
        let cap = self.undecided + UNDECIDED_SLACK;
        (undecided > cap).then(|| {
            format!(
                "{} answers are undecided, above the cap of {}, the {} recorded in {} and {} more",
                undecided, cap, self.undecided, BASELINE, UNDECIDED_SLACK
            )
        })
    }
}

/// What the run of a model shows of the query it satisfies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The run traps where the query says: a check Tier 1 proved fails. The fields are the bug
    /// it confirms, its source file, its function and its obligation, and the report.
    Confirmed { bug: Bug, report: String },
    /// The run does not trap where the query says: a defect of the encoding.
    Unconfirmed(String),
}

/// A soundness bug of Tier 1: the source file, the function, and the obligation as a query
/// names it, its kind, its place and its tag, such as `range at 43:24 into `u8``, so that bugs
/// that share one trap are told apart.
pub type Bug = (String, String, String);

/// The file, line and column of a query's first line, `; source: <file>:<line>:<col>`.
fn source_of(query: &str) -> Option<(String, u32, u32)> {
    let line = query.lines().next()?.strip_prefix("; source: ")?;
    let mut parts = line.rsplitn(3, ':');
    let col = parts.next()?.parse().ok()?;
    let row = parts.next()?.parse().ok()?;
    Some((parts.next()?.to_string(), row, col))
}

/// The value of a parameter of shape `shape` from the model, its inputs read in order; an input
/// the model leaves out takes 0 or `false`, as any value satisfies the query there.
fn value_of(
    shape: &Shape,
    names: &mut std::slice::Iter<String>,
    model: &mut HashMap<String, eval::Value>,
) -> Value {
    match shape {
        Shape::Int(_) | Shape::Untyped => {
            let name = names.next().cloned().unwrap_or_default();
            let v = model
                .entry(name)
                .or_insert(eval::Value::Int(Big::zero()))
                .clone();
            match v {
                eval::Value::Int(b) => Value::Int(int_of(&b)),
                eval::Value::Bool(_) | eval::Value::Array(..) => Value::Int(Int::ZERO),
            }
        }
        Shape::Bool => {
            let name = names.next().cloned().unwrap_or_default();
            let v = model
                .entry(name)
                .or_insert(eval::Value::Bool(false))
                .clone();
            Value::Bool(v == eval::Value::Bool(true))
        }
        Shape::Array(n, e) => Value::Array((0..*n).map(|_| value_of(e, names, model)).collect()),
        Shape::Other => Value::Unit,
    }
}

/// An exact integer as the interpreter holds it; one past its window, which no parameter's type
/// holds, as 0.
fn int_of(b: &Big) -> Int {
    match (b.is_negative(), b.magnitude_u128()) {
        (false, Some(m)) => Int::from_u128(m),
        (true, Some(m)) => Int::from_u128(m).negate(),
        (_, None) => Int::ZERO,
    }
}

/// Confirms the `sat` answer `model` of the query at `path`, as described above.
pub fn confirm_one(path: &Path, model: &str) -> Result<Verdict, String> {
    let query =
        fs::read_to_string(path).map_err(|e| format!("cannot read `{}`: {}", path.display(), e))?;
    let (file, line, col) = source_of(&query)
        .ok_or_else(|| format!("`{}` names no source on its first line", path.display()))?;
    let header = mente_smt::header(&query)
        .ok_or_else(|| format!("`{}` names no function and trap", path.display()))?;
    let mut values = eval::model(model)?;
    let src = fs::read_to_string(&file).map_err(|e| format!("cannot read `{}`: {}", file, e))?;
    let checked =
        mente_sema::check_program(&src).map_err(|_| format!("`{}` no longer checks", file))?;
    let module = module_of(&checked);
    let index = checked
        .index(&header.function)
        .ok_or_else(|| format!("`{}` has no function `{}`", file, header.function))?;
    let enc = mente_smt::encode(&module, index);
    let args: Vec<Value> = enc
        .params
        .iter()
        .map(|p| value_of(&p.shape, &mut p.inputs.iter(), &mut values))
        .collect();
    let shown: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let call = format!("{}({})", header.function, shown.join(", "));
    let place = format!("{}:{}:{}", file, line, col);
    if !eval::holds(&query, &values)? {
        return Ok(Verdict::Unconfirmed(format!(
            "{}: the model {} does not satisfy its query, so the solver's answer or its reading is wrong",
            place, call
        )));
    }
    let outcome = mente_interp::call(&checked, &header.function, args, Limits::default());
    let trapped_here = matches!(&outcome, Err(Stop::Trap(t))
        if t.kind.name() == header.trap_kind
            && t.span.line == header.trap_line
            && t.span.col == header.trap_col);
    let ran = match &outcome {
        Ok(v) => format!("returns {}", v),
        Err(stop) => format!("stops: {}", stop),
    };
    if trapped_here {
        return Ok(Verdict::Confirmed {
            report: format!(
                "{}: Tier 1 proved the obligation `{}` of `{}`, and {} {}",
                place, header.obligation, header.function, call, ran
            ),
            bug: (file, header.function, header.obligation),
        });
    }
    Ok(Verdict::Unconfirmed(format!(
        "{}: the query of the obligation `{}` of `{}` names the trap {} at {}:{}, but {} {}: a defect of the encoding",
        place,
        header.obligation,
        header.function,
        header.trap_kind,
        header.trap_line,
        header.trap_col,
        call,
        ran
    )))
}

/// The lines of a file of `tests/smt-known` that begin with `prefix`, each a function and an
/// obligation as a query names it: `<prefix><function> <obligation>`.
fn named(file: &str, src: &str, prefix: &str) -> Vec<Bug> {
    src.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix(prefix)?;
            let (function, obligation) = rest.split_once(' ')?;
            Some((
                file.to_string(),
                function.to_string(),
                obligation.trim().to_string(),
            ))
        })
        .collect()
}

/// The known soundness bugs a file names: each line `// smt: <function> <obligation>`.
pub fn expected(file: &str, src: &str) -> Vec<Bug> {
    named(file, src, "// smt: ")
}

/// The checks a file names as known to lie past the bounds: each line
/// `// smt beyond the bound: <function> <obligation>`, which a test holds.
#[cfg(test)]
pub fn beyond(file: &str, src: &str) -> Vec<Bug> {
    named(file, src, "// smt beyond the bound: ")
}

/// What `mn smt --confirm` found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Confirmed {
    pub unsat: usize,
    pub undecided: Vec<String>,
    pub confirmed: Vec<String>,
    pub known: Vec<String>,
    pub unconfirmed: Vec<String>,
    /// Known bugs no `sat` answer confirmed.
    pub missing: Vec<String>,
    /// Answers that are none: a solver's error, an empty answer, an answer to no query or a
    /// second answer to one, and a query with no answer.
    pub errors: Vec<String>,
    /// The undecided answers past the baseline's cap.
    pub over: Option<String>,
    /// How many queries of each fragment were answered, and how many of them undecided.
    pub fragments: BTreeMap<&'static str, (usize, usize)>,
}

impl Confirmed {
    /// Whether the job passes: no bug it did not expect, no defect of the encoding, every bug it
    /// expects confirmed, every query answered once by the solver without an error, and no more
    /// undecided answers than the baseline allows.
    pub fn passes(&self) -> bool {
        self.confirmed.is_empty()
            && self.unconfirmed.is_empty()
            && self.missing.is_empty()
            && self.errors.is_empty()
            && self.over.is_none()
    }
}

/// Whether a confirmed bug is the known bug `e`: one file, as paths, one function and one
/// obligation.
fn is(e: &Bug, found: &Bug) -> bool {
    Path::new(&e.0) == Path::new(&found.0) && e.1 == found.1 && e.2 == found.2
}

/// What `confirm` holds the answers to: the known bugs, the queries that were asked, which each
/// must have one answer, and the baseline of undecided answers.
#[derive(Clone, Debug, Default)]
pub struct Gate {
    pub expect: Vec<Bug>,
    pub queries: Option<Vec<String>>,
    pub baseline: Option<Baseline>,
}

/// Reads the answers, one a line, a solver's whole answer on one line, a tab, then the query's
/// path; and confirms each `sat`. An answer is `unsat`, `sat` with its model, or undecided,
/// `unknown` or `timeout`; anything else, a solver's `(error ...)` or nothing, is an error.
pub fn confirm(answers: &str, gate: &Gate) -> Confirmed {
    let mut c = Confirmed::default();
    let mut found = Vec::new();
    let mut answered: Vec<&str> = Vec::new();
    for line in answers.lines().filter(|l| !l.trim().is_empty()) {
        let (answer, path) = line.split_once('\t').unwrap_or((line, ""));
        if answered.contains(&path) {
            c.errors.push(format!("{}: answered twice", path));
            continue;
        }
        answered.push(path);
        let fragment = fs::read_to_string(path)
            .map(|q| mente_smt::fragment(&q))
            .unwrap_or("unread");
        let tally = c.fragments.entry(fragment).or_insert((0, 0));
        tally.0 += 1;
        let mut words = answer.splitn(2, char::is_whitespace);
        match words.next() {
            Some("unsat") => c.unsat += 1,
            Some("sat") => {
                let model = words.next().unwrap_or("");
                match confirm_one(Path::new(path), model) {
                    Ok(Verdict::Confirmed { bug, report }) => {
                        if gate.expect.iter().any(|e| is(e, &bug)) {
                            c.known.push(report);
                        } else {
                            c.confirmed.push(report);
                        }
                        found.push(bug);
                    }
                    Ok(Verdict::Unconfirmed(why)) => c.unconfirmed.push(why),
                    Err(why) => c.unconfirmed.push(format!("{}: {}", path, why)),
                }
            }
            Some("unknown" | "timeout") => {
                tally.1 += 1;
                c.undecided.push(format!("{}: {}", path, answer));
            }
            _ => c.errors.push(format!(
                "{}: the solver answered `{}`, which is neither `sat`, `unsat`, `unknown` nor `timeout`",
                path, answer
            )),
        }
    }
    if let Some(queries) = &gate.queries {
        for q in queries.iter().filter(|q| !answered.contains(&q.as_str())) {
            c.errors.push(format!("{}: asked and not answered", q));
        }
        for a in answered.iter().filter(|a| !queries.iter().any(|q| q == *a)) {
            c.errors.push(format!("{}: answered and not asked", a));
        }
    }
    for e in &gate.expect {
        if !found.iter().any(|f| is(e, f)) {
            c.missing.push(format!(
                "{}: no query of `{}` was confirmed at its obligation `{}`",
                e.0, e.1, e.2
            ));
        }
    }
    c.over = gate
        .baseline
        .and_then(|b| b.undecided_problem(c.undecided.len()));
    c
}

/// The baseline at `path`, or an exit with why it cannot be read.
fn baseline_at(path: &str) -> Baseline {
    match fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|t| Baseline::parse(&t))
    {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: the baseline `{}`: {}", path, e);
            std::process::exit(2);
        }
    }
}

/// `mn smt --independent <file.mn | dir>... --out <dir> [--baseline <file>]`: writes the
/// independent queries and prints a summary; exits 1 when an obligation of Tier 1 has no site,
/// or when the counts fall outside the baseline, and 2 when a file cannot be read or written.
pub fn cmd_independent(args: &[String]) {
    let mut out = None;
    let mut baseline = None;
    let mut inputs = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--out" {
            out = words.next().map(PathBuf::from);
        } else if word == "--baseline" {
            baseline = Some(baseline_at(words.next().map_or("", String::as_str)));
        } else {
            inputs.push(PathBuf::from(word));
        }
    }
    let (Some(out), false) = (out, inputs.is_empty()) else {
        eprintln!("Usage: mn smt --independent <file.mn | dir>... --out <dir> [--baseline <file>]");
        std::process::exit(2);
    };
    let files: Vec<PathBuf> = inputs
        .iter()
        .flat_map(|p| crate::collect_mn_files(p))
        .collect();
    match independent(&files, &out) {
        Ok(w) => {
            println!(
                "Wrote {} independent queries for {} functions of {} files to `{}`. Of the checks Tier 1 erased that no input reaches, {} are unreachable, {} lie past the bounds and {} past a construct the encoding lacks. {} places hold a construct the encoding lacks, {} files do not check, {} obligations have no site.",
                w.queries,
                w.functions,
                w.files,
                out.display(),
                w.unreachable,
                w.beyond_bound,
                w.beyond_lacked,
                w.unencoded,
                w.unchecked,
                w.unmatched
            );
            let problems = baseline.map_or_else(Vec::new, |b| b.export_problems(&w));
            for p in &problems {
                println!("baseline: {}", p);
            }
            if w.unmatched > 0 || !problems.is_empty() {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(2);
        }
    }
}

/// `mn smt --confirm <answers> [--queries <list>] [--baseline <file>] [--expect <file.mn>]...`:
/// confirms each `sat` answer on the reference interpreter and prints what it finds, and how many
/// queries of each fragment were answered and undecided; exits 1 when a bug no `--expect` file
/// names is confirmed, when a model is not confirmed, when an expected bug is not, when an
/// answer is an error or a query of the list has none, or when the undecided answers pass the
/// baseline, and 2 when a file cannot be read.
pub fn cmd_confirm(args: &[String]) {
    let mut answers = None;
    let mut gate = Gate::default();
    let mut words = args.iter();
    let read = |file: &str| match fs::read_to_string(file) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {}", file, e);
            std::process::exit(2);
        }
    };
    let mut usage = false;
    while let Some(word) = words.next() {
        match word.as_str() {
            "--expect" | "--queries" | "--baseline" => {
                let Some(file) = words.next() else {
                    usage = true;
                    break;
                };
                match word.as_str() {
                    "--expect" => gate.expect.extend(expected(file, &read(file))),
                    "--queries" => {
                        gate.queries = Some(read(file).lines().map(str::to_string).collect())
                    }
                    _ => gate.baseline = Some(baseline_at(file)),
                }
            }
            _ => answers = Some(word.clone()),
        }
    }
    let (Some(answers), false) = (answers, usage) else {
        eprintln!("Usage: mn smt --confirm <answers> [--queries <list>] [--baseline <file>] [--expect <file.mn>]...");
        std::process::exit(2);
    };
    let c = confirm(&read(&answers), &gate);
    for (prefix, lines) in [
        ("confirmed", &c.confirmed),
        ("known", &c.known),
        ("unconfirmed", &c.unconfirmed),
        ("missing", &c.missing),
        ("error", &c.errors),
        ("undecided", &c.undecided),
    ] {
        for l in lines {
            println!("{}: {}", prefix, l);
        }
    }
    for (fragment, (n, undecided)) in &c.fragments {
        println!(
            "fragment: {} queries {}, {} of them undecided",
            n, fragment, undecided
        );
    }
    if let Some(over) = &c.over {
        println!("baseline: {}", over);
    }
    println!(
        "{} unsat, {} undecided, {} confirmed soundness bugs of Tier 1, {} known, {} not confirmed, {} known bugs missing, {} errors.",
        c.unsat,
        c.undecided.len(),
        c.confirmed.len(),
        c.known.len(),
        c.unconfirmed.len(),
        c.missing.len(),
        c.errors.len()
    );
    if !c.passes() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mente_sema::CheckedModule;

    /// The inputs of the encoding's parameters for the arguments `args`, by input name.
    fn inputs_of(enc: &Encoding, args: &[Value]) -> Option<HashMap<String, eval::Value>> {
        fn put(
            shape: &Shape,
            v: &Value,
            names: &mut std::slice::Iter<String>,
            out: &mut HashMap<String, eval::Value>,
        ) -> Option<()> {
            match (shape, v) {
                (Shape::Int(_) | Shape::Untyped, Value::Int(i)) => {
                    let m = Big::from_u128(i.magnitude());
                    let b = if i.is_negative() { m.neg() } else { m };
                    out.insert(names.next()?.clone(), eval::Value::Int(b));
                }
                (Shape::Bool, Value::Bool(b)) => {
                    out.insert(names.next()?.clone(), eval::Value::Bool(*b));
                }
                (Shape::Array(_, e), Value::Array(items)) => {
                    for item in items {
                        put(e, item, names, out)?;
                    }
                }
                _ => return None,
            }
            Some(())
        }
        let mut out = HashMap::new();
        if enc.params.len() != args.len() {
            return None;
        }
        for p in &enc.params {
            for n in &p.inputs {
                out.insert(n.clone(), eval::Value::Int(Big::zero()));
            }
        }
        for (p, a) in enc.params.iter().zip(args) {
            put(&p.shape, a, &mut p.inputs.iter(), &mut out)?;
        }
        Some(out)
    }

    /// Boundary values of an integer type, and a few between.
    fn samples(t: IntType) -> Vec<Value> {
        let lo = t.min();
        let hi = t.max();
        let mut out: Vec<Big> = vec![lo.clone(), hi.clone(), lo.add(&Big::from_u128(1))];
        out.push(hi.sub(&Big::from_u128(1)));
        for v in [
            0i128, 1, 2, 3, 5, 7, 9, 10, 100, 250, 255, 256, -1, -2, -128, 300, 70000,
        ] {
            out.push(Big::from_i128(v));
        }
        out.retain(|v| *v >= lo && *v <= hi);
        out.dedup();
        out.iter().map(|b| Value::Int(int_of(b))).collect()
    }

    /// Inputs for a function's parameters: every pair of samples for one or two integer and
    /// boolean parameters, a spread of them for more, and arrays of samples.
    fn generated(enc: &Encoding) -> Vec<Vec<Value>> {
        fn of(shape: &Shape, k: usize) -> Option<Vec<Value>> {
            match shape {
                Shape::Int(t) => Some(samples(*t)),
                Shape::Bool => Some(vec![Value::Bool(false), Value::Bool(true)]),
                Shape::Array(n, e) => {
                    let each = of(e, k)?;
                    let rows = (0..4)
                        .map(|r| {
                            Value::Array(
                                (0..*n as usize)
                                    .map(|i| each[(i * 7 + r * 3 + k) % each.len()].clone())
                                    .collect(),
                            )
                        })
                        .collect();
                    Some(rows)
                }
                Shape::Untyped | Shape::Other => None,
            }
        }
        let Some(choices) = enc
            .params
            .iter()
            .enumerate()
            .map(|(k, p)| of(&p.shape, k))
            .collect::<Option<Vec<_>>>()
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        match choices.as_slice() {
            [] => out.push(Vec::new()),
            [a] => out.extend(a.iter().map(|x| vec![x.clone()])),
            [a, b] => {
                for x in a {
                    for y in b {
                        out.push(vec![x.clone(), y.clone()]);
                    }
                }
            }
            many => {
                for r in 0..40usize {
                    out.push(
                        many.iter()
                            .enumerate()
                            .map(|(k, c)| c[(r * (2 * k + 3) + k) % c.len()].clone())
                            .collect(),
                    );
                }
            }
        }
        out
    }

    /// Runs `name` on each of `inputs` on one thread of the interpreter's: the value each run
    /// returns and its parameters' final values, or why it stops.
    fn run_all(
        module: &CheckedModule,
        name: &str,
        inputs: &[Vec<Value>],
    ) -> Vec<Result<(Value, Vec<Value>), Stop>> {
        let limits = Limits {
            max_steps: 50_000,
            ..Limits::default()
        };
        mente_interp::with_calls(module, limits, |calls| {
            inputs
                .iter()
                .map(|args| {
                    let mut seen = |_: Span, _: Int, _: mente_interp::Shown| {};
                    calls.observed(name, args.clone(), &mut seen)
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// The traps a check raises, by `TrapKind::name`.
    const CHECK_TRAPS: &[&str] = &[
        "narrowing",
        "refinement",
        "division-by-zero",
        "index-out-of-bounds",
        "shift-amount",
        "requires",
        "ensures",
        "invariant",
        "decreases",
    ];

    /// The most inputs of one function compared with the interpreter.
    const LIMIT: usize = 120;

    /// Whether the value of a term, `term`, is the value `v` the interpreter gives: an integer,
    /// a boolean, an array element by element; no term for no value.
    fn same(term: Option<&eval::Value>, v: &Value) -> bool {
        match (term, v) {
            (None, Value::Unit) => true,
            (Some(eval::Value::Int(b)), Value::Int(i)) => {
                let m = Big::from_u128(i.magnitude());
                *b == if i.is_negative() { m.neg() } else { m }
            }
            (Some(eval::Value::Bool(b)), Value::Bool(c)) => b == c,
            (Some(eval::Value::Array(default, set)), Value::Array(items)) => {
                items.iter().enumerate().all(|(k, item)| {
                    let at = set.get(&Big::from_u128(k as u128)).unwrap_or(default);
                    same(Some(at), item)
                })
            }
            _ => false,
        }
    }

    /// What holding an encoding against the interpreter found: the runs compared, the inputs
    /// left out because a path of theirs is cut off past the bounds, and each disagreement.
    #[derive(Debug, Default)]
    struct Runs {
        compared: usize,
        cut: usize,
        wrong: Vec<String>,
    }

    impl Runs {
        fn add(&mut self, other: Runs) {
            self.compared += other.compared;
            self.cut += other.cut;
            self.wrong.extend(other.wrong);
        }

        /// Whether the share of inputs cut off is at most the recorded one, `cut` of `total`,
        /// and half a hundredth: more cut off means the encoding follows fewer paths.
        fn cut_within(&self, (cut, total): (usize, usize)) -> bool {
            let mine = self.compared + self.cut;
            self.cut * total * 200 <= (cut * 200 + total) * mine
        }
    }

    /// Holds the encoding of every function of `src` against the interpreter, input by input,
    /// where the inputs are admitted and no path of theirs is cut off: the sites whose condition
    /// holds are exactly the trap the run raises, if a check raises it; and where the run
    /// returns, the encoding says it returns, the value it returns and the final value of each
    /// parameter, a `mut` one's copy back. An input cut off is counted.
    fn agreement(src: &str, extra: &[(String, Vec<Value>)]) -> Runs {
        let mut runs = Runs::default();
        let Ok(checked) = mente_sema::check_program_without_proofs(src) else {
            return runs;
        };
        let module = module_of(&checked);
        for (i, f) in checked.functions.iter().enumerate() {
            let Some(cf) = f else {
                continue;
            };
            let name = &cf.syntax.name;
            let enc = mente_smt::encode(&module, i);
            if !enc.unencoded.is_empty() {
                continue;
            }
            let mut inputs = generated(&enc);
            inputs.extend(
                extra
                    .iter()
                    .filter(|(n, _)| n == name)
                    .map(|(_, a)| a.clone()),
            );
            if inputs.is_empty() {
                continue;
            }
            let mut roots: Vec<mente_smt::term::T> = enc.sites.iter().map(|s| s.fails).collect();
            roots.extend([enc.admitted, enc.cut, enc.returns]);
            roots.extend(enc.result);
            roots.extend(enc.finals.iter().flatten());
            let mut text = String::new();
            enc.terms.write(&roots, &mut text);
            let parsed = eval::parse(&text).unwrap();
            let tree = &cf.syntax.tree;
            let end = tree
                .exprs
                .iter()
                .map(|e| e.span.end)
                .chain(tree.stmts.iter().map(|s| s.span.end))
                .chain(tree.blocks.iter().map(|b| b.span.end))
                .max()
                .unwrap_or(0);
            let start = cf.syntax.name_span.start;
            let within = |s: Span| s.start >= start && s.end <= end;
            // The inputs the encoding admits and follows whole, each with the sites that hold of
            // it and the outcome it gives; only those are run.
            let mut kept = Vec::new();
            for args in &inputs {
                let Some(values) = inputs_of(&enc, args) else {
                    continue;
                };
                let (env, _) = eval::run(&parsed, &values).unwrap();
                let value = |t: mente_smt::term::T| {
                    eval::eval(&eval::parse(&enc.terms.atom(t)).unwrap()[0], &env).unwrap()
                };
                let truth = |t: mente_smt::term::T| value(t) == eval::Value::Bool(true);
                if !truth(enc.admitted) {
                    continue;
                }
                if truth(enc.cut) {
                    runs.cut += 1;
                    continue;
                }
                let holding: Vec<&mente_smt::Site> =
                    enc.sites.iter().filter(|s| truth(s.fails)).collect();
                let result = enc.result.map(value);
                let finals: Vec<Option<eval::Value>> =
                    enc.finals.iter().map(|t| t.map(value)).collect();
                kept.push((args.clone(), holding, truth(enc.returns), result, finals));
                if kept.len() == LIMIT {
                    break;
                }
            }
            let calls: Vec<Vec<Value>> = kept.iter().map(|k| k.0.clone()).collect();
            let outcomes = run_all(&checked, name, &calls);
            for ((args, holding, returns, result, finals), outcome) in
                kept.into_iter().zip(outcomes)
            {
                let call = format!(
                    "{}({})",
                    name,
                    args.iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                // A check of a function the entry calls fails in that function's text, and is
                // none of the entry's sites.
                let expected = match &outcome {
                    Err(Stop::Trap(t))
                        if CHECK_TRAPS.contains(&t.kind.name())
                            && (within(t.span)
                                || enc.sites.iter().any(|s| {
                                    s.trap.kind == t.kind.name() && s.trap.span == t.span
                                })) =>
                    {
                        Some((t.kind.name(), t.span))
                    }
                    Ok(_) | Err(Stop::Trap(_)) => None,
                    Err(_) => continue,
                };
                runs.compared += 1;
                let traps: Vec<(&str, Span)> =
                    holding.iter().map(|s| (s.trap.kind, s.trap.span)).collect();
                let mut agrees = match expected {
                    Some(t) => traps.len() == 1 && traps[0] == t,
                    None => traps.is_empty(),
                };
                // What a run that returns gives back, and that one that traps returns nothing.
                let mut gives = String::new();
                match &outcome {
                    Ok((v, back)) => {
                        let back_agrees = back.len() == finals.len()
                            && back.iter().zip(&finals).all(|(b, f)| same(f.as_ref(), b));
                        if !returns || !same(result.as_ref(), v) || !back_agrees {
                            agrees = false;
                            gives = format!(
                                "; the encoding returns: {}, the value {:?} and the parameters {:?}",
                                returns, result, finals
                            );
                        }
                    }
                    Err(_) => {
                        if returns {
                            agrees = false;
                            gives = "; the encoding returns".to_string();
                        }
                    }
                }
                if !agrees {
                    runs.wrong.push(format!(
                        "{}: the run gives {:?}, the encoding's sites hold for {:?}{}",
                        call,
                        outcome.as_ref().map_err(|e| e.to_string()),
                        holding
                            .iter()
                            .map(|s| format!(
                                "{} (trap {} at {}:{})",
                                s.obligation(),
                                s.trap.kind,
                                s.trap.span.line,
                                s.trap.span.col
                            ))
                            .collect::<Vec<_>>(),
                        gives
                    ));
                }
            }
        }
        runs
    }

    /// The share of the construct programs' inputs cut off past the bounds when it was recorded:
    /// inputs cut off, of the inputs admitted.
    const CONSTRUCTS_CUT: (usize, usize) = (113, 5749);

    /// The encoding of every construct program agrees with the reference interpreter on every
    /// input it admits and does not cut off: a query holds of an input exactly where the run of
    /// that input traps at the query's check, and a run that returns gives what the encoding
    /// says. No larger a share of the inputs is cut off than was recorded.
    #[test]
    fn test_each_query_holds_exactly_of_the_inputs_whose_run_traps_there() {
        let mut runs = Runs::default();
        for (program, calls) in crate::mir_run::CONSTRUCTS {
            let src = format!("module m\n{}\n", program);
            let extra: Vec<(String, Vec<Value>)> = calls
                .iter()
                .filter_map(|c| mente_interp::parse_call(c))
                .collect();
            let mut found = agreement(&src, &extra);
            let first = program.lines().next().unwrap_or("").to_string();
            found.wrong = found
                .wrong
                .into_iter()
                .map(|w| format!("{}\n  {}", first, w))
                .collect();
            runs.add(found);
        }
        assert!(
            runs.wrong.is_empty(),
            "{} disagreements of {} runs:\n{}",
            runs.wrong.len(),
            runs.compared,
            runs.wrong
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(runs.compared > 2000, "only {} runs compared", runs.compared);
        assert!(
            runs.cut_within(CONSTRUCTS_CUT),
            "{} of {} inputs are cut off past the bounds, more than the {} of {} recorded",
            runs.cut,
            runs.cut + runs.compared,
            CONSTRUCTS_CUT.0,
            CONSTRUCTS_CUT.1
        );
    }

    /// The modules of the corpus the CI job runs on, as files, its agent programs aside: the
    /// conformance programs that must pass, the examples, the kernels and the construct
    /// programs. The job itself fails on an obligation with no site in any of its modules.
    fn corpus() -> Vec<(String, String)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut out = Vec::new();
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|e| e == "mn") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        for dir in ["conformance", "examples", "perf/kernels"] {
            walk(&root.join(dir), &mut files);
        }
        files.sort();
        for f in files {
            let src = fs::read_to_string(&f).unwrap();
            if f.starts_with(root.join("conformance")) && !src.starts_with("// expect: pass") {
                continue;
            }
            out.push((f.display().to_string(), src));
        }
        for (k, (program, _)) in crate::mir_run::CONSTRUCTS.iter().enumerate() {
            out.push((
                format!("construct {}", k),
                format!(
                    "module m
{}
",
                    program
                ),
            ));
        }
        out
    }

    /// Every obligation Tier 1 records has a site of the same kind, place and tag, in every
    /// function, those within a construct the encoding lacks aside: so each check Tier 1 erases
    /// has its query, and a site's key says truly whether Tier 1 proved it.
    #[test]
    fn test_every_obligation_tier1_records_is_a_site_of_the_encoding() {
        let mut missing = Vec::new();
        let (mut functions, mut lacking, mut obligations) = (0, 0, 0);
        for (file, src) in corpus() {
            let Ok(checked) = mente_sema::check_program(&src) else {
                continue;
            };
            let module = module_of(&checked);
            for (i, f) in checked.functions.iter().enumerate() {
                let Some(cf) = f else {
                    continue;
                };
                let enc = mente_smt::encode(&module, i);
                functions += 1;
                lacking += usize::from(!enc.unencoded.is_empty());
                obligations += cf.obligations.len();
                for o in unmatched(&cf.obligations, &enc) {
                    missing.push(format!(
                        "{}: {} of `{}` at {}:{} {:?} ({})",
                        file,
                        o.kind.name(),
                        cf.syntax.name,
                        o.span.line,
                        o.span.col,
                        o.tag,
                        o.detail
                    ));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "{} of {} obligations in {} functions have no site ({} functions hold a construct the encoding lacks):\n{}",
            missing.len(),
            obligations,
            functions,
            lacking,
            missing.iter().take(60).cloned().collect::<Vec<_>>().join("\n")
        );
        assert!(
            functions > 500 && lacking > 50 && obligations > 2500,
            "only {} functions, {} with a construct the encoding lacks, {} obligations",
            functions,
            lacking,
            obligations
        );
        // A site taken away is reported, in a function the encoding holds whole and in one that
        // holds a construct it lacks; an obligation within that construct is not.
        let src = "module m\npub struct P {\n  pub a: u8\n}\npub fn f(x: u8, d: u8[1..=9]) -> u16 {\n  val p = P(a: x / d)\n  p.a + x / d\n}\n";
        let checked = mente_sema::check_program(src).unwrap();
        let module = module_of(&checked);
        let index = checked.index("f").unwrap();
        let cf = checked.functions[index].as_ref().unwrap();
        let mut enc = mente_smt::encode(&module, index);
        assert!(!enc.unencoded.is_empty());
        assert!(unmatched(&cf.obligations, &enc).is_empty());
        let divisor = enc
            .sites
            .iter()
            .position(|s| s.kind == Kind::Divisor)
            .unwrap();
        let taken = enc.sites.remove(divisor);
        let left = unmatched(&cf.obligations, &enc);
        assert_eq!(left.len(), 1);
        assert_eq!(
            (left[0].kind, left[0].span),
            (ObligationKind::Divisor, taken.span)
        );
        let construct = enc.unencoded[0].0;
        assert!(cf
            .obligations
            .iter()
            .any(|o| construct.start <= o.span.start && o.span.end <= construct.end));
        let whole = vec![(cf.syntax.name_span, "a parameter of a struct or an enum")];
        enc.sites.clear();
        enc.unencoded = whole;
        assert!(unmatched(&cf.obligations, &enc).is_empty());
    }

    /// The share of the corpus's inputs cut off past the bounds when it was recorded: inputs cut
    /// off, of the inputs admitted.
    const CORPUS_CUT: (usize, usize) = (611, 13220);

    /// The encoding of every function of the conformance programs, the examples and the kernels
    /// agrees with the reference interpreter on every input it admits and does not cut off, its
    /// values included; no larger a share of the inputs is cut off than was recorded.
    #[test]
    fn test_each_query_of_the_corpus_holds_exactly_of_the_inputs_whose_run_traps_there() {
        let mut runs = Runs::default();
        for (file, src) in corpus() {
            if file.starts_with("construct ") {
                continue;
            }
            let mut found = agreement(&src, &[]);
            found.wrong = found
                .wrong
                .into_iter()
                .map(|w| format!("{}: {}", file, w))
                .collect();
            runs.add(found);
        }
        assert!(
            runs.wrong.is_empty(),
            "{} disagreements of {} runs:\n{}",
            runs.wrong.len(),
            runs.compared,
            runs.wrong
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(runs.compared > 5000, "only {} runs compared", runs.compared);
        assert!(
            runs.cut_within(CORPUS_CUT),
            "{} of {} inputs are cut off past the bounds, more than the {} of {} recorded",
            runs.cut,
            runs.cut + runs.compared,
            CORPUS_CUT.0,
            CORPUS_CUT.1
        );
    }

    /// A fresh scratch directory for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mente-smt-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The known soundness bugs of `tests/smt-known` are written as queries, the model of each,
    /// a failing input, satisfies its query and traps on the interpreter where the query says,
    /// and `confirm` reports each as known, by its obligation; without the file's `smt:` lines
    /// each fails the job, and so does a bug known by another obligation, a solver's error, an
    /// answer missing or given twice, and undecided answers past the baseline.
    #[test]
    fn test_the_known_soundness_bugs_are_confirmed_on_the_interpreter() {
        let file =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/smt-known/loop-candidate-facts.mn");
        let out = scratch("known");
        let w = independent(std::slice::from_ref(&file), &out).unwrap();
        assert_eq!((w.files, w.unchecked, w.unencoded), (1, 0, 0));
        let shown = file.display().to_string();
        let expect = expected(&shown, &fs::read_to_string(&file).unwrap());
        assert_eq!(expect.len(), 4, "{:?}", expect);
        let model = |function: &str| match function {
            "narrow" => "((p.d 6))",
            "always" => "",
            _ => "((p.d 0))",
        };
        let mut queries: Vec<PathBuf> = fs::read_dir(out.join(below(Path::new(""), &file)))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        queries.sort();
        let listed: Vec<String> = queries.iter().map(|q| q.display().to_string()).collect();
        let line = |answer: &str, q: &str| format!("{}\t{}\n", answer, q);
        let mut answers = String::new();
        let mut wrong = String::new();
        for (q, path) in queries.iter().zip(&listed) {
            let h = mente_smt::header(&fs::read_to_string(q).unwrap()).unwrap();
            if expect
                .iter()
                .any(|(_, f, o)| *f == h.function && *o == h.obligation)
            {
                answers.push_str(&line(&format!("sat {}", model(&h.function)), path));
                if h.function == "rem" {
                    wrong.push_str(&line("sat ((p.d 1))", path));
                }
            } else {
                answers.push_str(&line("unsat (error \"model is not available\")", path));
            }
        }
        let gate = Gate {
            expect: expect.clone(),
            queries: Some(listed.clone()),
            baseline: None,
        };
        let c = confirm(&answers, &gate);
        assert!(c.passes() && c.known.len() == 4, "{:#?}", c);
        assert!(
            c.unsat + 4 == queries.len() && c.undecided.is_empty(),
            "{:#?}",
            c
        );
        let reported = c.known.join("\n");
        for part in [
            "the obligation `divisor at 24:23` of `rem`, and rem(0) stops: trap[division-by-zero] at 24:19",
            "the obligation `range at 51:14 into `u8`` of `always`, and always() stops: trap[narrowing] at 51:14",
            "the obligation `range at 43:24 into `u8`` of `narrow`, and narrow(6) stops: trap[narrowing] at 43:24",
            "the obligation `divisor at 33:15` of `guarded`, and guarded(0)",
        ] {
            assert!(reported.contains(part), "{} not in:\n{}", part, reported);
        }
        // Not known, each is a soundness bug that fails the job; so is one known by another
        // obligation, or in another file, which is then missing too.
        let c = confirm(&answers, &Gate::default());
        assert_eq!((c.confirmed.len(), c.passes()), (4, false), "{:#?}", c);
        let elsewhere: Vec<Bug> = expect
            .iter()
            .enumerate()
            .map(|(k, (f, n, o))| match k % 2 {
                0 => (f.clone(), n.clone(), o.replace(" at ", " at 1")),
                _ => ("elsewhere.mn".to_string(), n.clone(), o.clone()),
            })
            .collect();
        let c = confirm(
            &answers,
            &Gate {
                expect: elsewhere,
                ..Gate::default()
            },
        );
        assert_eq!(
            (c.known.len(), c.confirmed.len(), c.missing.len()),
            (0, 4, 4),
            "{:#?}",
            c
        );
        // A model that does not satisfy its query is a defect, and a known bug not confirmed
        // fails too; a timeout decides nothing, but past the baseline it fails.
        let c = confirm(&format!("{}timeout\tsomewhere.smt2\n", wrong), &gate);
        assert_eq!(
            (c.unconfirmed.len(), c.missing.len(), c.undecided.len()),
            (1, 4, 1),
            "{:#?}",
            c
        );
        assert!(
            c.unconfirmed[0].contains("rem(1)") && c.unconfirmed[0].contains("does not satisfy"),
            "{:#?}",
            c
        );
        assert!(!c.passes());
        let tight = Gate {
            baseline: Some(Baseline {
                queries: 1,
                beyond_bound: 0,
                undecided: 0,
            }),
            ..Gate::default()
        };
        let many: String = (0..=UNDECIDED_SLACK)
            .map(|k| line("unknown", &format!("q{}.smt2", k)))
            .collect();
        let c = confirm(&many, &tight);
        assert!(c.over.is_some() && !c.passes(), "{:#?}", c);
        let fewer: String = (0..UNDECIDED_SLACK)
            .map(|k| line("timeout", &format!("q{}.smt2", k)))
            .collect();
        assert!(confirm(&fewer, &tight).passes());
        // A solver's error, an empty answer, an answer twice, and a query of the list with no
        // answer or an answer to none, each fails the job.
        let first = &listed[0];
        for (answers, says) in [
            (
                line("(error \"line 3: unknown constant\")", first),
                "the solver answered `(error",
            ),
            (line("", first), "the solver answered ``"),
            (
                format!("{}{}", line("unsat", first), line("unsat", first)),
                "answered twice",
            ),
            (line("unsat", "other.smt2"), "answered and not asked"),
        ] {
            let c = confirm(&answers, &gate);
            assert!(
                !c.passes() && c.errors.iter().any(|e| e.contains(says)),
                "{}: {:#?}",
                says,
                c
            );
        }
        let c = confirm(&line("unsat", first), &gate);
        assert!(
            c.errors
                .iter()
                .any(|e| e.ends_with(": asked and not answered")),
            "{:#?}",
            c
        );
        let _ = fs::remove_dir_all(&out);
    }

    /// The CI job holds its counts to the baseline it records: the queries it writes to a floor a
    /// hundredth under the record, the checks cut off by the bound to a ceiling a hundredth over
    /// it, and the undecided answers to the record and a few; and the baseline is read strictly.
    #[test]
    fn test_the_job_holds_its_counts_to_the_recorded_baseline() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let b = Baseline::parse(&fs::read_to_string(root.join(BASELINE)).unwrap()).unwrap();
        assert!(b.queries >= 1000, "{:?}", b);
        let workflow = fs::read_to_string(root.join(".github/workflows/smt.yml")).unwrap();
        assert_eq!(
            workflow
                .matches(&format!("--baseline {}", BASELINE))
                .count(),
            2,
            "smt.yml gives the baseline to the export and to the confirmation"
        );
        let at = |queries: usize, beyond_bound: usize| Written {
            queries,
            beyond_bound,
            ..Written::default()
        };
        let floor = b.queries - b.queries / 100;
        let ceiling = b.beyond_bound + b.beyond_bound / 100;
        assert!(b.export_problems(&at(floor, ceiling)).is_empty());
        let low = b.export_problems(&at(floor - 1, ceiling));
        assert!(
            low.len() == 1 && low[0].contains("below the floor"),
            "{:?}",
            low
        );
        let high = b.export_problems(&at(floor, ceiling + 1));
        assert!(
            high.len() == 1 && high[0].contains("cut off by the bound"),
            "{:?}",
            high
        );
        assert_eq!(b.undecided_problem(b.undecided + UNDECIDED_SLACK), None);
        assert!(b
            .undecided_problem(b.undecided + UNDECIDED_SLACK + 1)
            .is_some());
        let ok = "# a comment\nqueries: 10\nbeyond-bound: 2\nundecided: 0\n";
        assert_eq!(
            Baseline::parse(ok),
            Ok(Baseline {
                queries: 10,
                beyond_bound: 2,
                undecided: 0
            })
        );
        for (text, says) in [
            (
                "queries: 10\nbeyond-bound: 2\n",
                "the baseline has no `undecided`",
            ),
            (
                "queries: 10\nqueries: 11\nbeyond-bound: 2\nundecided: 0\n",
                "`queries` is given twice",
            ),
            (
                "queries: -1\nbeyond-bound: 2\nundecided: 0\n",
                "`queries: -1` is no decimal count",
            ),
            (
                "queries: 10\nbeyond-bound: 2\nundecided: 0\nunreached: 3\n",
                "`unreached` is no field of the baseline",
            ),
            ("queries 10\n", "`queries 10` is no `name: count` line"),
        ] {
            assert_eq!(Baseline::parse(text), Err(says.to_string()), "{}", text);
        }
    }

    /// The checks `tests/smt-known` names as lying past the bounds are erased by Tier 1, fail on
    /// some input, and have no query, the encoding naming them cut off by the bound: the known
    /// limit of K, which the job counts against its baseline.
    #[test]
    fn test_a_check_past_the_bound_is_named_cut_off_not_unreachable() {
        let file =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/smt-known/loop-past-the-bound.mn");
        let src = fs::read_to_string(&file).unwrap();
        let limits = beyond(&file.display().to_string(), &src);
        assert!(!limits.is_empty());
        let checked = mente_sema::check_program(&src).unwrap();
        let module = module_of(&checked);
        for (_, function, obligation) in &limits {
            let index = checked.index(function).unwrap();
            let cf = checked.functions[index].as_ref().unwrap();
            let mut enc = mente_smt::encode(&module, index);
            let keys = erased(&cf.obligations);
            let group = groups(&enc, |k| keys.contains(k))
                .into_iter()
                .find(|g| enc.sites[g[0]].obligation() == *obligation)
                .unwrap_or_else(|| panic!("{} erases no `{}`", function, obligation));
            let trap = enc.sites[group[0]].trap;
            assert_eq!(unqueried(&enc, &group), Unqueried::BeyondBound);
            assert_eq!(enc.query(&group), None);
            // An input past the bound fails the check.
            let fails = generated(&enc).into_iter().any(|args| {
                matches!(
                    mente_interp::call(&checked, function, args, Limits::default()),
                    Err(Stop::Trap(t)) if t.kind.name() == trap.kind && t.span == trap.span
                )
            });
            assert!(fails, "no input of `{}` fails `{}`", function, obligation);
        }
        // A check no path reaches, with none cut off, is unreachable.
        let src =
            "module m\npub fn f(x: u8, d: u8[1..=9]) -> u8 {\n  if 1 > 2 {\n    return x / d\n  }\n  x\n}\n";
        let checked = mente_sema::check_program_without_proofs(src).unwrap();
        let module = module_of(&checked);
        let enc = mente_smt::encode(&module, 0);
        let divisor = enc
            .sites
            .iter()
            .position(|s| s.kind == Kind::Divisor)
            .unwrap();
        assert_eq!(unqueried(&enc, &[divisor]), Unqueried::Unreachable);
    }

    /// Each construct the encoding lacks where a path reaches it is one of `UNENCODED`, and
    /// each of those is reached by some program, so the list neither misses one nor goes stale.
    #[test]
    fn test_each_construct_the_encoding_lacks_is_listed_and_reached() {
        let mut programs: Vec<String> = crate::mir_run::CONSTRUCTS
            .iter()
            .map(|(p, _)| {
                format!(
                    "module m
{}
",
                    p
                )
            })
            .collect();
        for p in [
            "pub struct P {
  pub a: u8
}
pub fn f(p: P) -> u8 = p.a
pub fn g(n: u8) -> u8 = P(a: n).a",
            "pub enum E {
  A
  B
}
pub fn h(n: u8) -> u8 {
  val e: E = if n > 3 { E.A } else { E.B }
  match e {
    E.A => 1
    E.B => 2
  }
}",
            "pub fn big(xs: [300]u8) -> u8 = xs[0]",
        ] {
            programs.push(format!(
                "module m
{}
",
                p
            ));
        }
        let mut seen: Vec<&'static str> = Vec::new();
        for src in &programs {
            let checked = mente_sema::check_program_without_proofs(src)
                .unwrap_or_else(|e| panic!("{}: {:?}", src, e));
            let module = module_of(&checked);
            for i in 0..module.functions.len() {
                if module.functions[i].is_some() {
                    seen.extend(mente_smt::encode(&module, i).unencoded.iter().map(|u| u.1));
                }
            }
        }
        // A function too large, and a call of a declaration without a body, which the checker
        // refuses (`E0224`) and the encoding still lists.
        let src = "module m
fn inner(x: u8) -> u8 = x + 1
pub fn outer(x: u8) -> u8 = inner(x)
";
        let checked = mente_sema::check_program_without_proofs(src).unwrap();
        let mut module = module_of(&checked);
        let outer = checked.index("outer").unwrap();
        seen.extend(
            mente_smt::encode_within(&module, outer, 5)
                .unencoded
                .iter()
                .map(|u| u.1),
        );
        assert!(mente_smt::encode(&module, outer).unencoded.is_empty());
        module.functions[checked.index("inner").unwrap()] = None;
        seen.extend(
            mente_smt::encode(&module, outer)
                .unencoded
                .iter()
                .map(|u| u.1),
        );
        for s in &seen {
            assert!(
                mente_smt::UNENCODED.contains(s),
                "`{}` is lacked but not listed",
                s
            );
        }
        for listed in mente_smt::UNENCODED {
            assert!(
                seen.contains(listed),
                "`{}` is listed but no program reaches it",
                listed
            );
        }
    }
}
