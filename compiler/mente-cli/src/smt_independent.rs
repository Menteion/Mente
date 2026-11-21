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

use std::collections::HashMap;
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

/// The obligations Tier 1 records in a function that its encoding has no site for, where the
/// encoding holds the function whole; none where a construct it lacks cuts a path off. Each is a
/// defect of the encoding: a check Tier 1 may erase would have no query.
pub fn unmatched<'o>(obligations: &'o [Obligation], enc: &Encoding) -> Vec<&'o Obligation> {
    if !enc.unencoded.is_empty() {
        return Vec::new();
    }
    let keys: Vec<Key> = enc.sites.iter().map(|s| s.key()).collect();
    obligations
        .iter()
        .filter(|o| !keys.contains(&key_of(o)))
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

/// What `mn smt --independent` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub queries: usize,
    pub functions: usize,
    pub files: usize,
    /// Checks Tier 1 erased that no input reaches within the bounds.
    pub unreached: usize,
    /// Places of constructs the encoding lacks.
    pub unencoded: usize,
    /// Files whose module does not check.
    pub unchecked: usize,
    /// Obligations of Tier 1 with no site in a function the encoding holds whole.
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
/// the obligation's place; prints each construct the encoding lacks and each file that does not
/// check.
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
                    w.unreached += 1;
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

/// What the run of a model shows of the query it satisfies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The run traps where the query says: a check Tier 1 proved fails. The fields are the
    /// source file, the function, the trap's kind, line and column, and the report.
    Confirmed {
        file: String,
        function: String,
        trap: (String, u32, u32),
        report: String,
    },
    /// The run does not trap where the query says: a defect of the encoding.
    Unconfirmed(String),
}

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
            file,
            function: header.function.clone(),
            trap: (header.trap_kind, header.trap_line, header.trap_col),
            report: format!(
                "{}: Tier 1 proved the {} obligation of `{}`, and {} {}",
                place, header.obligation, header.function, call, ran
            ),
        });
    }
    Ok(Verdict::Unconfirmed(format!(
        "{}: the query of the {} obligation of `{}` names the trap {} at {}:{}, but {} {}: a defect of the encoding",
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

/// The known bugs a file names: each line `// smt: <function> <trap> at <line>:<col>`.
pub fn expected(file: &str, src: &str) -> Vec<(String, String, (String, u32, u32))> {
    src.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("// smt: ")?;
            let (function, rest) = rest.split_once(' ')?;
            let (trap, place) = rest.split_once(" at ")?;
            let (line, col) = place.trim().split_once(':')?;
            Some((
                file.to_string(),
                function.to_string(),
                (trap.to_string(), line.parse().ok()?, col.parse().ok()?),
            ))
        })
        .collect()
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
}

impl Confirmed {
    /// Whether the job passes: no bug it did not expect, no defect of the encoding, and every
    /// bug it expects confirmed.
    pub fn passes(&self) -> bool {
        self.confirmed.is_empty() && self.unconfirmed.is_empty() && self.missing.is_empty()
    }
}

/// Whether a confirmed bug is the known bug `e`: one file, as paths, one function and one trap.
fn is(
    e: &(String, String, (String, u32, u32)),
    found: &(String, String, (String, u32, u32)),
) -> bool {
    Path::new(&e.0) == Path::new(&found.0) && e.1 == found.1 && e.2 == found.2
}

/// Reads the answers, one a line, a solver's whole answer on one line, a tab, then the query's
/// path; and confirms each `sat`, the known bugs being those `expect` names.
pub fn confirm(answers: &str, expect: &[(String, String, (String, u32, u32))]) -> Confirmed {
    let mut c = Confirmed::default();
    let mut found = Vec::new();
    for line in answers.lines().filter(|l| !l.trim().is_empty()) {
        let (answer, path) = line.split_once('\t').unwrap_or((line, ""));
        let mut words = answer.splitn(2, char::is_whitespace);
        match words.next() {
            Some("unsat") => c.unsat += 1,
            Some("sat") => {
                let model = words.next().unwrap_or("");
                match confirm_one(Path::new(path), model) {
                    Ok(Verdict::Confirmed {
                        file,
                        function,
                        trap,
                        report,
                    }) => {
                        let key = (file, function, trap);
                        if expect.iter().any(|e| is(e, &key)) {
                            c.known.push(report);
                        } else {
                            c.confirmed.push(report);
                        }
                        found.push(key);
                    }
                    Ok(Verdict::Unconfirmed(why)) => c.unconfirmed.push(why),
                    Err(why) => c.unconfirmed.push(format!("{}: {}", path, why)),
                }
            }
            _ => c.undecided.push(format!("{}: {}", path, answer)),
        }
    }
    for e in expect {
        if !found.iter().any(|f| is(e, f)) {
            c.missing.push(format!(
                "{}: no query of `{}` was confirmed at its {} at {}:{}",
                e.0, e.1, e.2 .0, e.2 .1, e.2 .2
            ));
        }
    }
    c
}

/// `mn smt --independent <file.mn | dir>... --out <dir>`: writes the independent queries and
/// prints a summary; exits 1 when an obligation of Tier 1 has no site in a function the encoding
/// holds whole, and 2 when a file cannot be read or written.
pub fn cmd_independent(args: &[String]) {
    let mut out = None;
    let mut inputs = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--out" {
            out = words.next().map(PathBuf::from);
        } else {
            inputs.push(PathBuf::from(word));
        }
    }
    let (Some(out), false) = (out, inputs.is_empty()) else {
        eprintln!("Usage: mn smt --independent <file.mn | dir>... --out <dir>");
        std::process::exit(2);
    };
    let files: Vec<PathBuf> = inputs
        .iter()
        .flat_map(|p| crate::collect_mn_files(p))
        .collect();
    match independent(&files, &out) {
        Ok(w) => {
            println!(
                "Wrote {} independent queries for {} functions of {} files to `{}`; {} checks Tier 1 erased are reached by no input within the bounds, {} places hold a construct the encoding lacks, {} files do not check, {} obligations have no site.",
                w.queries,
                w.functions,
                w.files,
                out.display(),
                w.unreached,
                w.unencoded,
                w.unchecked,
                w.unmatched
            );
            if w.unmatched > 0 {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(2);
        }
    }
}

/// `mn smt --confirm <answers> [--expect <file.mn>]...`: confirms each `sat` answer on the
/// reference interpreter and prints what it finds; exits 1 when a bug no `--expect` file names
/// is confirmed, when a model is not confirmed, or when an expected bug is not, and 2 when a file
/// cannot be read.
pub fn cmd_confirm(args: &[String]) {
    let mut answers = None;
    let mut expect = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--expect" {
            let Some(file) = words.next() else {
                answers = None;
                break;
            };
            match fs::read_to_string(file) {
                Ok(src) => expect.extend(expected(file, &src)),
                Err(e) => {
                    eprintln!("error: cannot read `{}`: {}", file, e);
                    std::process::exit(2);
                }
            }
        } else {
            answers = Some(word.clone());
        }
    }
    let Some(answers) = answers else {
        eprintln!("Usage: mn smt --confirm <answers> [--expect <file.mn>]...");
        std::process::exit(2);
    };
    let text = match fs::read_to_string(&answers) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {}", answers, e);
            std::process::exit(2);
        }
    };
    let c = confirm(&text, &expect);
    for l in &c.confirmed {
        println!("confirmed: {}", l);
    }
    for l in &c.known {
        println!("known: {}", l);
    }
    for l in &c.unconfirmed {
        println!("unconfirmed: {}", l);
    }
    for l in &c.missing {
        println!("missing: {}", l);
    }
    for l in &c.undecided {
        println!("undecided: {}", l);
    }
    println!(
        "{} unsat, {} undecided, {} confirmed soundness bugs of Tier 1, {} known, {} not confirmed, {} known bugs missing.",
        c.unsat,
        c.undecided.len(),
        c.confirmed.len(),
        c.known.len(),
        c.unconfirmed.len(),
        c.missing.len()
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

    /// Runs `name` on each of `inputs` on one thread of the interpreter's.
    fn run_all(
        module: &CheckedModule,
        name: &str,
        inputs: &[Vec<Value>],
    ) -> Vec<Result<Value, Stop>> {
        let limits = Limits {
            max_steps: 50_000,
            ..Limits::default()
        };
        mente_interp::with_calls(module, limits, |calls| {
            inputs
                .iter()
                .map(|args| {
                    let mut seen = |_: Span, _: Int, _: mente_interp::Shown| {};
                    calls
                        .observed(name, args.clone(), &mut seen)
                        .map(|(v, _)| v)
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

    /// Holds the encoding of every function of `src` against the interpreter, input by input:
    /// where the inputs are admitted and no path of theirs is cut off, the sites whose condition
    /// holds are exactly the trap the run raises, if a check raises it. Returns the runs
    /// compared and each disagreement.
    fn agreement(src: &str, extra: &[(String, Vec<Value>)]) -> (usize, Vec<String>) {
        let Ok(checked) = mente_sema::check_program_without_proofs(src) else {
            return (0, Vec::new());
        };
        let module = module_of(&checked);
        let mut compared = 0;
        let mut wrong = Vec::new();
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
            roots.extend([enc.admitted, enc.cut]);
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
            // it; only those are run.
            let mut kept = Vec::new();
            for args in &inputs {
                let Some(values) = inputs_of(&enc, args) else {
                    continue;
                };
                let (env, _) = eval::run(&parsed, &values).unwrap();
                let truth = |t: mente_smt::term::T| {
                    eval::eval(&eval::Sexp::Atom(enc.terms.atom(t)), &env).unwrap()
                        == eval::Value::Bool(true)
                };
                if !truth(enc.admitted) || truth(enc.cut) {
                    continue;
                }
                let holding: Vec<&mente_smt::Site> =
                    enc.sites.iter().filter(|s| truth(s.fails)).collect();
                kept.push((args.clone(), holding));
                if kept.len() == LIMIT {
                    break;
                }
            }
            let runs: Vec<Vec<Value>> = kept.iter().map(|k| k.0.clone()).collect();
            let outcomes = run_all(&checked, name, &runs);
            for ((args, holding), outcome) in kept.into_iter().zip(outcomes) {
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
                compared += 1;
                let traps: Vec<(&str, Span)> =
                    holding.iter().map(|s| (s.trap.kind, s.trap.span)).collect();
                let agrees = match expected {
                    Some(t) => traps.len() == 1 && traps[0] == t,
                    None => traps.is_empty(),
                };
                if !agrees {
                    wrong.push(format!(
                        "{}: the run gives {:?}, the encoding's sites hold for {:?}",
                        call,
                        outcome.as_ref().map_err(|e| e.to_string()),
                        holding
                            .iter()
                            .map(|s| format!(
                                "{} at {}:{} (trap {} at {}:{})",
                                s.kind.name(),
                                s.span.line,
                                s.span.col,
                                s.trap.kind,
                                s.trap.span.line,
                                s.trap.span.col
                            ))
                            .collect::<Vec<_>>()
                    ));
                }
            }
        }
        (compared, wrong)
    }

    /// The encoding of every construct program agrees with the reference interpreter on every
    /// input it admits and does not cut off: a query holds of an input exactly where the run of
    /// that input traps at the query's check.
    #[test]
    fn test_each_query_holds_exactly_of_the_inputs_whose_run_traps_there() {
        let mut compared = 0;
        let mut wrong = Vec::new();
        for (program, calls) in crate::mir_run::CONSTRUCTS {
            let src = format!("module m\n{}\n", program);
            let extra: Vec<(String, Vec<Value>)> = calls
                .iter()
                .filter_map(|c| mente_interp::parse_call(c))
                .collect();
            let (n, w) = agreement(&src, &extra);
            compared += n;
            wrong.extend(
                w.into_iter()
                    .map(|w| format!("{}\n  {}", program.lines().next().unwrap_or(""), w)),
            );
        }
        assert!(
            wrong.is_empty(),
            "{} disagreements of {} runs:\n{}",
            wrong.len(),
            compared,
            wrong
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(compared > 2000, "only {} runs compared", compared);
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

    /// Every obligation Tier 1 records in a function the encoding holds whole has a site of the
    /// same kind, place and tag: so each check Tier 1 erases has its query, and a site's key
    /// says truly whether Tier 1 proved it.
    #[test]
    fn test_every_obligation_tier1_records_is_a_site_of_the_encoding() {
        let mut missing = Vec::new();
        let (mut whole, mut lacking, mut obligations) = (0, 0, 0);
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
                if !enc.unencoded.is_empty() {
                    lacking += 1;
                    continue;
                }
                whole += 1;
                obligations += cf.obligations.len();
                for o in unmatched(&cf.obligations, &enc) {
                    {
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
        }
        assert!(
            missing.is_empty(),
            "{} of {} obligations in {} functions have no site ({} functions not held whole):
{}",
            missing.len(),
            obligations,
            whole,
            lacking,
            missing.iter().take(60).cloned().collect::<Vec<_>>().join(
                "
"
            )
        );
        assert!(
            whole > 400 && obligations > 1500,
            "only {} functions, {} obligations",
            whole,
            obligations
        );
        // A site taken away is reported; a function the encoding does not hold whole is not.
        let src = "module m
pub fn f(x: u8, d: u8[1..=9]) -> u8 = x / d
";
        let checked = mente_sema::check_program(src).unwrap();
        let module = module_of(&checked);
        let cf = checked.functions[0].as_ref().unwrap();
        let mut enc = mente_smt::encode(&module, 0);
        assert!(unmatched(&cf.obligations, &enc).is_empty());
        let divisor = enc
            .sites
            .iter()
            .position(|s| s.kind == Kind::Divisor)
            .unwrap();
        enc.sites.remove(divisor);
        let left = unmatched(&cf.obligations, &enc);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].kind, ObligationKind::Divisor);
        enc.unencoded.push((cf.syntax.name_span, "a struct"));
        assert!(unmatched(&cf.obligations, &enc).is_empty());
    }

    /// The encoding of every function of the conformance programs, the examples and the kernels
    /// agrees with the reference interpreter on every input it admits and does not cut off.
    #[test]
    fn test_each_query_of_the_corpus_holds_exactly_of_the_inputs_whose_run_traps_there() {
        let mut compared = 0;
        let mut wrong = Vec::new();
        for (file, src) in corpus() {
            if file.starts_with("construct ") {
                continue;
            }
            let (n, w) = agreement(&src, &[]);
            compared += n;
            wrong.extend(w.into_iter().map(|w| format!("{}: {}", file, w)));
        }
        assert!(
            wrong.is_empty(),
            "{} disagreements of {} runs:
{}",
            wrong.len(),
            compared,
            wrong.iter().take(40).cloned().collect::<Vec<_>>().join(
                "
"
            )
        );
        assert!(compared > 5000, "only {} runs compared", compared);
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
    /// and `confirm` reports each as known; without the file's `smt:` lines each fails the job.
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
        let mut answers = String::new();
        let mut wrong = String::new();
        for q in &queries {
            let text = fs::read_to_string(q).unwrap();
            let h = mente_smt::header(&text).unwrap();
            let known = expect.iter().any(|(_, f, t)| {
                *f == h.function && *t == (h.trap_kind.clone(), h.trap_line, h.trap_col)
            });
            let line = |answer: String| {
                format!(
                    "{}	{}
",
                    answer,
                    q.display()
                )
            };
            if known {
                answers.push_str(&line(format!("sat {}", model(&h.function))));
                if h.function == "rem" {
                    wrong.push_str(&line("sat ((p.d 1))".to_string()));
                }
            } else {
                answers.push_str(&line(
                    "unsat (error \"model is not available\")".to_string(),
                ));
            }
        }
        let c = confirm(&answers, &expect);
        assert!(c.passes() && c.known.len() == 4, "{:#?}", c);
        assert!(
            c.unsat + 4 == queries.len() && c.undecided.is_empty(),
            "{:#?}",
            c
        );
        let reported = c.known.join(
            "
",
        );
        for part in [
            "rem(0) stops: trap[division-by-zero] at 24:19",
            "always() stops: trap[narrowing] at 51:14",
            "narrow(6) stops: trap[narrowing] at 43:24",
            "guarded(0)",
        ] {
            assert!(
                reported.contains(part),
                "{} not in:
{}",
                part,
                reported
            );
        }
        // Not known, each is a soundness bug that fails the job; so is one known at another
        // place, or in another file, which is then missing too.
        let c = confirm(&answers, &[]);
        assert_eq!((c.confirmed.len(), c.passes()), (4, false), "{:#?}", c);
        let elsewhere: Vec<_> = expect
            .iter()
            .enumerate()
            .map(|(k, (f, n, (t, l, col)))| match k % 2 {
                0 => (f.clone(), n.clone(), (t.clone(), *l, col + 1)),
                _ => ("elsewhere.mn".to_string(), n.clone(), (t.clone(), *l, *col)),
            })
            .collect();
        let c = confirm(&answers, &elsewhere);
        assert_eq!(
            (c.known.len(), c.confirmed.len(), c.missing.len()),
            (0, 4, 4),
            "{:#?}",
            c
        );
        // A model that does not satisfy its query is a defect, and a known bug not confirmed
        // fails too; a timeout decides nothing.
        let c = confirm(
            &format!(
                "{}timeout	somewhere.smt2
",
                wrong
            ),
            &expect,
        );
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
        let _ = fs::remove_dir_all(&out);
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
