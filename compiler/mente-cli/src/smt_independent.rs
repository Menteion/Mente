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

/// Why a check Tier 1 erased has no query: no input fails it on any path the encoding follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unqueried {
    /// No path of the function was cut off, before the check or after it: no input reaches a
    /// failure of it at all.
    Unreachable,
    /// A path was cut off past the bounds, which may reach it: before it, or after it, where a
    /// later round may fail it.
    BeyondBound,
    /// A path was cut off at a construct the encoding lacks, before it or after it.
    BeyondLacked,
}

/// Why the sites `g` of one obligation have no query, where none does: a cut before one of
/// them first, then a cut anywhere in the function.
pub fn unqueried(enc: &Encoding, g: &[usize]) -> Unqueried {
    let sites = || g.iter().map(|&k| &enc.sites[k]);
    if sites().any(|s| s.beyond_bound) {
        Unqueried::BeyondBound
    } else if sites().any(|s| s.beyond_lacked) {
        Unqueried::BeyondLacked
    } else if enc.cut_past_bound {
        Unqueried::BeyondBound
    } else if enc.cut_at_lacked {
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
    /// Checks Tier 1 erased that have no site: within a construct the encoding lacks, or in a
    /// function it does not encode.
    pub no_site: usize,
    /// Places of constructs the encoding lacks.
    pub unencoded: usize,
    /// Files whose module does not check.
    pub unchecked: usize,
    /// Obligations of Tier 1 with no site.
    pub unmatched: usize,
    /// Each file read and each check Tier 1 erased, for the comparison with the base.
    pub export: Export,
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
    independent_with(files, out, &mente_smt::encode)
}

/// [`independent`], with the encoder given, so that a test can take sites away from it. Every
/// check Tier 1 erased is exported, whether or not the encoding has a site for it, so that a
/// check is missing from an export only where Tier 1 did not erase it.
fn independent_with(
    files: &[PathBuf],
    out: &Path,
    encode: &dyn for<'m> Fn(&'m Module<'m>, usize) -> Encoding,
) -> Result<Written, String> {
    let mut w = Written {
        files: files.len(),
        ..Written::default()
    };
    for file in files {
        let shown = file.display().to_string();
        let src =
            fs::read_to_string(file).map_err(|e| format!("cannot read `{}`: {}", shown, e))?;
        let hash = text_hash(&src);
        let checked = mente_sema::check_program(&src);
        w.export.files.insert(
            shown.clone(),
            Program {
                hash: hash.clone(),
                checks: checked.is_ok(),
            },
        );
        let Ok(checked) = checked else {
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
            let mut enc = encode(&module, i);
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
            // What the encoding made of each trap of each erased check, in Tier 1's order.
            let mut classes: Vec<(Key, Vec<Class>)> =
                keys.iter().map(|k| (*k, Vec::new())).collect();
            for g in groups(&enc, |k| keys.contains(k)) {
                let site = &enc.sites[g[0]];
                let (key, kind, span) = (site.key(), site.kind, site.span);
                let class = match enc.query(&g) {
                    None => match unqueried(&enc, &g) {
                        Unqueried::Unreachable => {
                            w.unreachable += 1;
                            Class::Unreachable
                        }
                        Unqueried::BeyondBound => {
                            w.beyond_bound += 1;
                            Class::BeyondBound
                        }
                        Unqueried::BeyondLacked => {
                            w.beyond_lacked += 1;
                            Class::BeyondLacked
                        }
                    },
                    Some(text) => {
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
                        Class::Query(path.display().to_string())
                    }
                };
                if let Some(entry) = classes.iter_mut().find(|(k, _)| *k == key) {
                    entry.1.push(class);
                }
            }
            for (key, mut found) in classes {
                if found.is_empty() {
                    w.no_site += 1;
                    found.push(Class::NoSite);
                }
                w.export.checks.push(Erased {
                    file: shown.clone(),
                    hash: hash.clone(),
                    function: cf.syntax.name.clone(),
                    obligation: mente_smt::obligation_text(key),
                    classes: found,
                });
            }
        }
    }
    Ok(w)
}

/// What the encoding made of one trap of a check Tier 1 erased.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Class {
    /// A query, at its path.
    Query(String),
    Unreachable,
    BeyondBound,
    BeyondLacked,
    /// No site at all: the check lies within a construct the encoding lacks, or in a function
    /// it does not encode.
    NoSite,
}

impl Class {
    fn text(&self) -> String {
        match self {
            Class::Query(path) => format!("query {}", path),
            Class::Unreachable => "unreachable".to_string(),
            Class::BeyondBound => "beyond-bound".to_string(),
            Class::BeyondLacked => "beyond-lacked".to_string(),
            Class::NoSite => "no-site".to_string(),
        }
    }

    /// The class, its query's path aside, as two runs are compared.
    fn kind(&self) -> &'static str {
        match self {
            Class::Query(_) => "a query",
            Class::Unreachable => "unreachable",
            Class::BeyondBound => "cut off by the bound",
            Class::BeyondLacked => "cut off by a construct the encoding lacks",
            Class::NoSite => "without a site in the encoding",
        }
    }

    /// How much of the check is verified: a query, which the solver answers, more than the
    /// encoding's own finding that no input reaches the check, more than nothing, past a cut or
    /// with no site.
    fn rank(&self) -> u8 {
        match self {
            Class::Query(_) => 2,
            Class::Unreachable => 1,
            Class::BeyondBound | Class::BeyondLacked | Class::NoSite => 0,
        }
    }

    fn read(text: &str) -> Option<Class> {
        Some(match text {
            "unreachable" => Class::Unreachable,
            "beyond-bound" => Class::BeyondBound,
            "beyond-lacked" => Class::BeyondLacked,
            "no-site" => Class::NoSite,
            _ => Class::Query(text.strip_prefix("query ")?.to_string()),
        })
    }
}

/// A check Tier 1 erased, as the comparison with the base reads it: its program's file, the hash
/// of that file's text, its function, its obligation, and what the encoding made of each trap
/// the check raises, in the order the encoding meets them. A check with no site has one class,
/// [`Class::NoSite`]. Two runs tell their checks apart by function and obligation alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Erased {
    pub file: String,
    pub hash: String,
    pub function: String,
    pub obligation: String,
    pub classes: Vec<Class>,
}

impl Erased {
    /// The check as a line of `--keys`: `check`, then its fields and each class, each after a
    /// tab.
    pub fn line(&self) -> String {
        let mut fields = vec![
            "check".to_string(),
            self.file.clone(),
            self.hash.clone(),
            self.function.clone(),
            self.obligation.clone(),
        ];
        fields.extend(self.classes.iter().map(Class::text));
        fields.join("\t")
    }

    fn read(fields: &[&str]) -> Result<Erased, String> {
        let [file, hash, function, obligation, classes @ ..] = fields else {
            return Err(format!(
                "`{}` is no check: a file, a hash, a function, an obligation and its classes",
                fields.join(" ")
            ));
        };
        if classes.is_empty() {
            return Err(format!("`{}` is a check with no class", fields.join(" ")));
        }
        Ok(Erased {
            file: file.to_string(),
            hash: hash.to_string(),
            function: function.to_string(),
            obligation: obligation.to_string(),
            classes: classes
                .iter()
                .map(|c| Class::read(c).ok_or_else(|| format!("`{}` is no class", c)))
                .collect::<Result<_, _>>()?,
        })
    }

    /// What tells it from the other checks of its program: its function and its obligation.
    fn key(&self) -> (&str, &str) {
        (&self.function, &self.obligation)
    }

    /// How much of the check is verified: as much as its least verified trap ([`Class::rank`]).
    fn rank(&self) -> u8 {
        self.classes.iter().map(Class::rank).min().unwrap_or(0)
    }

    /// The paths of its queries.
    fn queries(&self) -> Vec<&str> {
        self.classes
            .iter()
            .filter_map(|c| match c {
                Class::Query(path) => Some(path.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Its classes, their queries' paths aside, as a summary writes them.
    fn kinds(&self) -> String {
        let kinds: Vec<&str> = self.classes.iter().map(Class::kind).collect();
        kinds.join(", ")
    }

    /// Whether this run's check is verified less than the base's `base`: unless its traps,
    /// best first, are each verified at least as well as the base's in the same place, with no
    /// fewer of them; so one trap lost, or one verified less, is less.
    fn less_than(&self, base: &Erased) -> bool {
        let ranks = |e: &Erased| {
            let mut r: Vec<u8> = e.classes.iter().map(Class::rank).collect();
            r.sort_unstable_by(|a, b| b.cmp(a));
            r
        };
        let (mine, theirs) = (ranks(self), ranks(base));
        mine.len() < theirs.len() || mine.iter().zip(&theirs).any(|(m, t)| m < t)
    }
}

/// A file an export read: the hash of its text, and whether its module checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub hash: String,
    pub checks: bool,
}

/// What an export wrote of each program: each file it read, one that does not check included,
/// and each check Tier 1 erased in them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Export {
    pub files: BTreeMap<String, Program>,
    pub checks: Vec<Erased>,
}

impl Export {
    /// The export as `--keys` writes it: for each file a line `file`, its path, its hash and
    /// `checked` or `unchecked`, then a line for each check ([`Erased::line`]), each field after
    /// a tab.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (file, p) in &self.files {
            let checks = if p.checks { "checked" } else { "unchecked" };
            out.push_str(&["file", file, &p.hash, checks].join("\t"));
            out.push('\n');
        }
        for e in &self.checks {
            out.push_str(&e.line());
            out.push('\n');
        }
        out
    }

    pub fn read(text: &str) -> Result<Export, String> {
        let mut export = Export::default();
        for line in text.lines().filter(|l| !l.is_empty()) {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.split_first() {
                Some((&"file", [file, hash, checks @ ("checked" | "unchecked")])) => {
                    let program = Program {
                        hash: hash.to_string(),
                        checks: *checks == "checked",
                    };
                    export.files.insert(file.to_string(), program);
                }
                Some((&"check", rest)) => export.checks.push(Erased::read(rest)?),
                _ => return Err(format!("`{}` is neither a file nor a check", line)),
            }
        }
        Ok(export)
    }

    fn of(&self, file: &str) -> Vec<&Erased> {
        self.checks.iter().filter(|e| e.file == file).collect()
    }

    /// The files it read, by the hash of their text, each hash's files in order.
    fn by_hash(&self) -> BTreeMap<&str, Vec<&str>> {
        let mut out: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (file, p) in &self.files {
            out.entry(p.hash.as_str()).or_default().push(file.as_str());
        }
        out
    }
}

/// The hash of a program's text that tells whether two trees hold it unchanged: 64-bit FNV-1a.
pub fn text_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:016x}", h)
}

/// How many undecided answers more than the base's, over the checks both runs query, a run may
/// give, since a timeout depends on the runner.
pub const UNDECIDED_SLACK: usize = 5;

/// The share of a run's queries that may be undecided, over all of them: at most one in
/// `UNDECIDED_SHARE`, so that the slack above cannot accumulate from one pull request to the
/// next, nor hide in new programs.
pub const UNDECIDED_SHARE: usize = 200;

/// The trailer of a commit message that lets one check be verified less than on the base:
/// `Smt-Drop: <file> <function> <obligation>: <reason>`, the file and the obligation exactly as
/// the comparison names them. It holds for the commits between the base and the run, and so
/// expires with its pull request.
pub const WAIVER_TRAILER: &str = "Smt-Drop";

/// A check a commit lets the run verify less than its base does, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiver {
    pub file: String,
    pub function: String,
    pub obligation: String,
    pub reason: String,
}

impl Waiver {
    /// Reads a trailer's value, `<file> <function> <obligation>: <reason>`: the file and the
    /// function are words, the obligation runs to the first `: `, and the reason is not empty.
    /// The value is trimmed first, so its file is never empty, and neither is a reason after
    /// the first `: `, which a trimmed value cannot end with; a doubled space can still leave
    /// the function empty, and a `: ` right after it the obligation.
    fn read(value: &str) -> Result<Waiver, String> {
        let bad = || {
            format!(
                "`{}: {}` is no waiver: write `{}: <file> <function> <obligation>: <reason>`",
                WAIVER_TRAILER, value, WAIVER_TRAILER
            )
        };
        let mut words = value.trim().splitn(3, ' ');
        let (Some(file), Some(function), Some(rest)) = (words.next(), words.next(), words.next())
        else {
            return Err(bad());
        };
        let (obligation, reason) = rest.split_once(": ").ok_or_else(bad)?;
        if function.is_empty() || obligation.is_empty() {
            return Err(bad());
        }
        Ok(Waiver {
            file: file.to_string(),
            function: function.to_string(),
            obligation: obligation.to_string(),
            reason: reason.trim().to_string(),
        })
    }

    fn text(&self) -> String {
        format!(
            "{}: {} {} {}: {}",
            WAIVER_TRAILER, self.file, self.function, self.obligation, self.reason
        )
    }
}

/// The waivers of the trailers' values, one a line, as `git log --format=%(trailers:...)` prints
/// them, blank lines aside; and each line that is no waiver.
pub fn waivers_of(text: &str) -> (Vec<Waiver>, Vec<String>) {
    let mut waivers = Vec::new();
    let mut bad = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match Waiver::read(line) {
            Ok(w) => waivers.push(w),
            Err(e) => bad.push(e),
        }
    }
    (waivers, bad)
}

/// What the comparison of a run with its base found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comparison {
    /// Checks of programs both trees hold unchanged that the run verifies less than its base
    /// ([`Erased::less_than`]), no waiver naming them: a check both erase that fell, and a check
    /// Tier 1 erases in the run alone that the encoding cuts off or has no site for. Each fails
    /// the run, since only the encoder or the checker can have changed them.
    pub dropped: Vec<String>,
    /// The checks a waiver let through, each with its reason.
    pub waived: Vec<String>,
    /// Waivers no dropped check used, and trailers that are no waiver; each fails the run.
    pub unused: Vec<String>,
    /// Programs both trees hold unchanged that one export did not read, the run's or the base's,
    /// which fails the run: a floor derived from the trees, so that an export that lost its
    /// programs cannot pass as a comparison of fewer.
    pub lost: Vec<String>,
    /// Programs both trees hold unchanged whose module the base checks and the run does not,
    /// which fails it.
    pub unchecked: Vec<String>,
    /// The undecided answers over the checks both runs query: the base's and the run's.
    pub undecided: (usize, usize),
    /// The run's undecided answers over all its queries, and its queries.
    pub total: (usize, usize),
    /// Checks of programs both hold unchanged that the run verifies no less, but otherwise: one
    /// whose classes changed, and one Tier 1 erases in one run alone that is still verified.
    pub moved: Vec<String>,
    /// Programs added, changed, renamed or deleted, with their counts.
    pub programs: Vec<String>,
}

impl Comparison {
    pub fn passes(&self) -> bool {
        self.dropped.is_empty()
            && self.unused.is_empty()
            && self.lost.is_empty()
            && self.unchecked.is_empty()
            && self.undecided.1 <= self.undecided.0 + UNDECIDED_SLACK
            && self.total.0 * UNDECIDED_SHARE <= self.total.1
    }
}

/// Whether the answer to the query at `path` decides nothing.
fn undecided(answers: &HashMap<String, String>, path: &str) -> bool {
    answers.get(path).is_some_and(|a| {
        matches!(
            a.split_whitespace().next(),
            Some("unknown" | "timeout") | None
        )
    })
}

/// The answers of a file of answers, by their queries' paths.
pub fn answers_of(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(a, p)| (p.to_string(), a.to_string()))
        .collect()
}

/// The counts of one program's checks, as a summary lists them, by their least verified trap.
fn counts(checks: &[&Erased]) -> String {
    let n = |c: &dyn Fn(&Class) -> bool| checks.iter().filter(|e| e.classes.iter().any(c)).count();
    format!(
        "{} checks: {} with a query, {} unreachable, {} cut off by the bound, {} by a construct the encoding lacks, {} without a site",
        checks.len(),
        n(&|c| matches!(c, Class::Query(_))),
        n(&|c| *c == Class::Unreachable),
        n(&|c| *c == Class::BeyondBound),
        n(&|c| *c == Class::BeyondLacked),
        n(&|c| *c == Class::NoSite)
    )
}

/// Compares a run with its base, check by check over the programs both hold unchanged: a
/// program is unchanged when one hash of its text is in both, whatever its path. `here` tells
/// whether a file of the base lies unchanged in the run's tree, its text hashing to the hash
/// given, and `there` whether a file of the run lies unchanged in the base's. A check the run
/// verifies less is let through by a waiver that names it by the run's path, its function and
/// its obligation exactly.
pub fn compare(
    base: &Export,
    base_answers: &HashMap<String, String>,
    head: &Export,
    head_answers: &HashMap<String, String>,
    here: &dyn Fn(&str, &str) -> bool,
    there: &dyn Fn(&str, &str) -> bool,
    waivers: &[Waiver],
) -> Comparison {
    let mut c = Comparison::default();
    let mut used = vec![false; waivers.len()];
    let mut less = |c: &mut Comparison, file: &str, e: &Erased, line: String| match waivers
        .iter()
        .position(|w| w.file == file && w.function == e.function && w.obligation == e.obligation)
    {
        Some(k) => {
            used[k] = true;
            c.waived.push(format!("{}; {}", line, waivers[k].reason));
        }
        None => c.dropped.push(line),
    };
    let (base_hashes, head_hashes) = (base.by_hash(), head.by_hash());
    for (hash, base_files) in &base_hashes {
        let Some(head_files) = head_hashes.get(hash) else {
            continue;
        };
        let (bf, hf) = (base_files[0], head_files[0]);
        if bf != hf {
            c.programs.push(format!("renamed: {} to {}", bf, hf));
        }
        if base.files[bf].checks && !head.files[hf].checks {
            c.unchecked.push(format!(
                "{}: the base's checker accepts its module and this run's does not",
                hf
            ));
            continue;
        }
        let (base_checks, head_checks) = (base.of(bf), head.of(hf));
        for be in &base_checks {
            let Some(he) = head_checks.iter().find(|he| he.key() == be.key()) else {
                c.moved.push(format!(
                    "{}: `{}`'s obligation `{}` is erased by Tier 1 on the base alone",
                    hf, be.function, be.obligation
                ));
                continue;
            };
            let line = format!(
                "{}: `{}`'s obligation `{}` is {} on the base and {} here",
                hf,
                be.function,
                be.obligation,
                be.kinds(),
                he.kinds()
            );
            if he.less_than(be) {
                less(&mut c, hf, he, line);
            } else if he.kinds() != be.kinds() {
                c.moved.push(line);
            }
            if !be.queries().is_empty() && !he.queries().is_empty() {
                c.undecided.0 += be
                    .queries()
                    .iter()
                    .filter(|p| undecided(base_answers, p))
                    .count();
                c.undecided.1 += he
                    .queries()
                    .iter()
                    .filter(|p| undecided(head_answers, p))
                    .count();
            }
        }
        for he in head_checks
            .iter()
            .filter(|he| !base_checks.iter().any(|be| be.key() == he.key()))
        {
            let line = format!(
                "{}: `{}`'s obligation `{}` is erased by Tier 1 here alone, and {}",
                hf,
                he.function,
                he.obligation,
                he.kinds()
            );
            if he.rank() == 0 {
                less(&mut c, hf, he, line);
            } else {
                c.moved.push(line);
            }
        }
    }
    for (file, p) in base
        .files
        .iter()
        .filter(|(_, p)| !head_hashes.contains_key(p.hash.as_str()))
    {
        if head.files.contains_key(file) {
            c.programs.push(format!(
                "changed: {}: {} on the base; {} here",
                file,
                counts(&base.of(file)),
                counts(&head.of(file))
            ));
        } else if here(file, &p.hash) {
            c.lost.push(format!(
                "{}: both trees hold it unchanged, but this export did not read it; the base's gave {}",
                file,
                counts(&base.of(file))
            ));
        } else {
            c.programs.push(format!(
                "deleted: {}: {} on the base",
                file,
                counts(&base.of(file))
            ));
        }
    }
    for (file, p) in head
        .files
        .iter()
        .filter(|(f, p)| !base_hashes.contains_key(p.hash.as_str()) && !base.files.contains_key(*f))
    {
        if there(file, &p.hash) {
            c.lost.push(format!(
                "{}: both trees hold it unchanged, but the base's export did not read it; this one gave {}",
                file,
                counts(&head.of(file))
            ));
        } else {
            c.programs
                .push(format!("added: {}: {}", file, counts(&head.of(file))));
        }
    }
    for (w, used) in waivers.iter().zip(used) {
        if !used {
            c.unused.push(format!(
                "{}: no check of a program both trees hold unchanged that this run verifies less is named so",
                w.text()
            ));
        }
    }
    let queries: Vec<&str> = head.checks.iter().flat_map(|e| e.queries()).collect();
    c.total = (
        queries
            .iter()
            .filter(|p| undecided(head_answers, p))
            .count(),
        queries.len(),
    );
    c
}

/// Whether the file at `path`, below `tree`, hashes to `hash`; a path that is absolute names a
/// file outside both trees, which both exports read.
fn unchanged_in(tree: &Path, path: &str, hash: &str) -> bool {
    fs::read_to_string(tree.join(path)).is_ok_and(|t| text_hash(&t) == hash)
}

/// The package the independent encoding is: a commit whose workspace holds it has an export to
/// compare with.
pub const ENCODING_PACKAGE: &str = "mente-smt";

/// Whether the workspace at `tree` holds [`ENCODING_PACKAGE`]: one of the members its
/// `Cargo.toml` lists names that package, wherever it lies.
pub fn encodes(tree: &Path) -> Result<bool, String> {
    let manifest = tree.join("Cargo.toml");
    let root = fs::read_to_string(&manifest)
        .map_err(|e| format!("cannot read `{}`: {}", manifest.display(), e))?;
    let Some(start) = root.find("members = [") else {
        return Err(format!("`{}` lists no members", manifest.display()));
    };
    let list = &root[start + "members = [".len()..];
    let list = &list[..list.find(']').unwrap_or(list.len())];
    let wanted = format!("name = \"{}\"", ENCODING_PACKAGE);
    for member in list.split('"').skip(1).step_by(2) {
        let text = fs::read_to_string(tree.join(member).join("Cargo.toml")).unwrap_or_default();
        let package = text
            .lines()
            .skip_while(|l| l.trim() != "[package]")
            .skip(1)
            .take_while(|l| !l.trim_start().starts_with('['));
        if package.into_iter().any(|l| l.trim() == wanted) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `mn smt --encodes <dir>`: exits 0 when the workspace at `<dir>` holds the independent
/// encoding ([`encodes`]), 1 when it does not, and 2 when its manifest cannot be read.
pub fn cmd_encodes(args: &[String]) {
    let [tree] = args else {
        eprintln!("Usage: mn smt --encodes <dir>");
        std::process::exit(2);
    };
    match encodes(Path::new(tree)) {
        Ok(true) => println!("`{}` holds {}.", tree, ENCODING_PACKAGE),
        Ok(false) => {
            println!("`{}` does not hold {}.", tree, ENCODING_PACKAGE);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(2);
        }
    }
}

/// `mn smt --compare --base-tree <dir> --base <keys> <answers> --head <keys> <answers> --waivers
/// <file>`, run in the run's own tree: compares a run of the independent cross-check with its
/// base's ([`compare`]), the base's commit checked out at `<dir>`, under the waivers of
/// `<file>` ([`waivers_of`]), and prints what it finds. A base whose workspace lacks the
/// encoding ([`encodes`]) has no export, and every program is new to the comparison. Exits 1
/// when the run verifies less of a check of a program both trees hold unchanged and no waiver
/// names it, when a waiver is unused or no waiver, when either export did not read such a
/// program, when the run no longer checks one, or when its undecided answers pass the base's by
/// more than [`UNDECIDED_SLACK`] or one in [`UNDECIDED_SHARE`] of its queries; and 2 when a
/// file cannot be read.
pub fn cmd_compare(args: &[String]) {
    let read = |file: &str| match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {}", file, e);
            std::process::exit(2);
        }
    };
    let export = |file: &str| match Export::read(&read(file)) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: `{}`: {}", file, e);
            std::process::exit(2);
        }
    };
    let (tree, base, head, waivers) = match args {
        [t, tree, b, bk, ba, h, hk, ha, w, waivers]
            if t == "--base-tree" && b == "--base" && h == "--head" && w == "--waivers" =>
        {
            (Path::new(tree), (bk, ba), (hk, ha), waivers)
        }
        _ => {
            eprintln!("Usage: mn smt --compare --base-tree <dir> --base <keys> <answers> --head <keys> <answers> --waivers <file>");
            std::process::exit(2);
        }
    };
    let encodes = match encodes(tree) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(2);
        }
    };
    if !encodes {
        println!(
            "The base at `{}` does not hold {}, so every program is new to the comparison.",
            tree.display(),
            ENCODING_PACKAGE
        );
    }
    let (waivers, bad) = waivers_of(&read(waivers));
    let mut c = compare(
        &export(base.0),
        &answers_of(&read(base.1)),
        &export(head.0),
        &answers_of(&read(head.1)),
        &|path, hash| unchanged_in(Path::new(""), path, hash),
        &|path, hash| encodes && unchanged_in(tree, path, hash),
        &waivers,
    );
    c.unused.extend(bad);
    for (prefix, lines) in [
        ("dropped", &c.dropped),
        ("waived", &c.waived),
        ("unused", &c.unused),
        ("lost", &c.lost),
        ("unchecked", &c.unchecked),
        ("moved", &c.moved),
        ("program", &c.programs),
    ] {
        for l in lines {
            println!("{}: {}", prefix, l);
        }
    }
    println!(
        "Of the programs both trees hold unchanged, {} checks are verified less than on the base and {} more waived, {} programs were not read and {} no longer check, and {} checks moved otherwise; {} waivers are unused or malformed; {} programs were added, changed, renamed or deleted. Undecided: {} on the base and {} here over the checks both query, {} of this run's {} queries.",
        c.dropped.len(),
        c.waived.len(),
        c.lost.len(),
        c.unchecked.len(),
        c.moved.len(),
        c.unused.len(),
        c.programs.len(),
        c.undecided.0,
        c.undecided.1,
        c.total.0,
        c.total.1
    );
    if !c.passes() {
        std::process::exit(1);
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
    let clause = header
        .obligation
        .rsplit_once(" of the clause at ")
        .and_then(|(_, at)| at.split_once(':'))
        .and_then(|(l, c)| Some((l.parse().ok()?, c.parse().ok()?)))
        .and_then(|(l, c)| clause_at(&checked, l, c));
    let trapped_here = matches!(&outcome, Err(Stop::Trap(t))
        if t.kind.name() == header.trap_kind
            && t.span.line == header.trap_line
            && t.span.col == header.trap_col
            && tag_agrees(&header.obligation, &t.kind, clause.as_deref()));
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

/// The clause of `checked` whose expression starts at `line:col`, a contract's, a loop's
/// invariant or a loop's measure, written as the interpreter writes it in a trap.
fn clause_at(checked: &CheckedModule, line: u32, col: u32) -> Option<String> {
    for f in checked.functions.iter().flatten() {
        let tree = &f.syntax.tree;
        let mut clauses: Vec<mente_syntax::tree::ExprId> =
            f.syntax.contracts.iter().map(|c| c.expr).collect();
        for s in &tree.stmts {
            if let mente_syntax::tree::StmtKind::While {
                invariants,
                decreases,
                ..
            } = &s.kind
            {
                clauses.extend(invariants.iter().chain(decreases));
            }
        }
        for e in clauses {
            let at = tree.expr(e).span;
            if (at.line, at.col) == (line, col) {
                return Some(tree.render_expr(e));
            }
        }
    }
    None
}

/// Whether a trap tells the obligation it is, beside its kind and its place, where it carries
/// enough to: a narrowing names the type it narrows into, which an obligation's tag ``into `T``
/// names; a clause's trap writes the clause, which the tag `of the clause at L:C` places and
/// `clause` is. Any other trap, an index's, a divisor's, a shift amount's or a refinement's, carries
/// its kind and its place alone, which two obligations can share: such a trap agrees with any.
pub fn tag_agrees(obligation: &str, trap: &mente_interp::TrapKind, clause: Option<&str>) -> bool {
    use mente_interp::TrapKind as K;
    if let Some((_, ty)) = obligation.split_once(" into `") {
        let ty = ty.trim_end_matches('`');
        return matches!(trap, K::Narrowing { into, .. } if *into == ty);
    }
    if obligation.contains(" of the clause at ") {
        return match trap {
            K::Requires(text) | K::Ensures(text) | K::Invariant(text) | K::Decreases(text) => {
                clause == Some(text.as_str())
            }
            K::Narrowing { .. }
            | K::Refinement { .. }
            | K::DivisionByZero
            | K::IndexOutOfBounds { .. }
            | K::ShiftAmount { .. }
            | K::Todo(_)
            | K::Unreachable(_)
            | K::StackOverflow
            | K::IntermediateOverflow => false,
        };
    }
    true
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
    /// How many queries of each fragment were answered, and how many of them undecided.
    pub fragments: BTreeMap<&'static str, (usize, usize)>,
}

impl Confirmed {
    /// Whether the job passes: no bug it did not expect, no defect of the encoding, every bug it
    /// expects confirmed, and every query answered once by the solver without an error. How many
    /// answers are undecided, the comparison with the base holds ([`compare`]).
    pub fn passes(&self) -> bool {
        self.confirmed.is_empty()
            && self.unconfirmed.is_empty()
            && self.missing.is_empty()
            && self.errors.is_empty()
    }
}

/// Whether a confirmed bug is the known bug `e`: one file, as paths, one function and one
/// obligation.
fn is(e: &Bug, found: &Bug) -> bool {
    Path::new(&e.0) == Path::new(&found.0) && e.1 == found.1 && e.2 == found.2
}

/// What `confirm` holds the answers to: the known bugs, and the queries that were asked, which
/// each must have one answer.
#[derive(Clone, Debug, Default)]
pub struct Gate {
    pub expect: Vec<Bug>,
    pub queries: Option<Vec<String>>,
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
    c
}

/// What `mn smt --independent` exits with once it wrote everything: 1 when an obligation of Tier
/// 1 has no site where it should, a defect of the encoding, and 0 otherwise.
fn independent_status(w: &Written) -> i32 {
    i32::from(w.unmatched > 0)
}

/// `mn smt --independent <file.mn | dir>... --out <dir> [--keys <file>]`: writes the independent
/// queries and prints a summary, and with `--keys` what it made of each check Tier 1 erased, for
/// the comparison with the base ([`Export`]); exits as [`independent_status`] says, and 2 when a
/// file cannot be read or written.
pub fn cmd_independent(args: &[String]) {
    let mut out = None;
    let mut keys = None;
    let mut inputs = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--out" {
            out = words.next().map(PathBuf::from);
        } else if word == "--keys" {
            keys = words.next().map(PathBuf::from);
        } else {
            inputs.push(PathBuf::from(word));
        }
    }
    let (Some(out), false) = (out, inputs.is_empty()) else {
        eprintln!("Usage: mn smt --independent <file.mn | dir>... --out <dir> [--keys <file>]");
        std::process::exit(2);
    };
    let files: Vec<PathBuf> = inputs
        .iter()
        .flat_map(|p| crate::collect_mn_files(p))
        .collect();
    match independent(&files, &out) {
        Ok(w) => {
            println!(
                "Wrote {} independent queries for {} functions of {} files to `{}`. Of the checks Tier 1 erased that no input reaches, {} are unreachable, {} lie past the bounds and {} past a construct the encoding lacks; {} have no site, lying within such a construct. {} places hold a construct the encoding lacks, {} files do not check, {} obligations have no site where they should.",
                w.queries,
                w.functions,
                w.files,
                out.display(),
                w.unreachable,
                w.beyond_bound,
                w.beyond_lacked,
                w.no_site,
                w.unencoded,
                w.unchecked,
                w.unmatched
            );
            if let Some(keys) = keys {
                if let Err(e) = fs::write(&keys, w.export.text()) {
                    eprintln!("error: cannot write `{}`: {}", keys.display(), e);
                    std::process::exit(2);
                }
            }
            let status = independent_status(&w);
            if status != 0 {
                std::process::exit(status);
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(2);
        }
    }
}

/// `mn smt --confirm <answers> [--queries <list>] [--expect <file.mn | dir>]...`:
/// confirms each `sat` answer on the reference interpreter and prints what it finds, and how many
/// queries of each fragment were answered and undecided; exits 1 when a bug no `--expect` file,
/// or file of an `--expect` directory, names is confirmed, when a model is not confirmed, when an expected bug is not, when an
/// answer is an error or a query of the list has none, and 2 when a file cannot be read.
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
            "--expect" | "--queries" => {
                let Some(file) = words.next() else {
                    usage = true;
                    break;
                };
                match word.as_str() {
                    "--expect" => {
                        if !Path::new(file).is_dir() {
                            read(file);
                        }
                        for each in crate::collect_mn_files(Path::new(file)) {
                            let shown = each.display().to_string();
                            gate.expect.extend(expected(&shown, &read(&shown)));
                        }
                    }
                    _ => gate.queries = Some(read(file).lines().map(str::to_string).collect()),
                }
            }
            _ => answers = Some(word.clone()),
        }
    }
    let (Some(answers), false) = (answers, usage) else {
        eprintln!(
            "Usage: mn smt --confirm <answers> [--queries <list>] [--expect <file.mn | dir>]..."
        );
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

    /// The encoding of every function of the conformance programs, the examples and the kernels
    /// agrees with the reference interpreter on every input it admits and does not cut off, its
    /// values included. How many of the corpus's checks are cut off is held by the CI job's
    /// comparison with its base, program by program, since every pull request may add programs
    /// to the corpus and a recorded share would be edited by each.
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
    }

    /// A fresh scratch directory for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mente-smt-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A bug a file names as known (`// smt:` lines, read by `expected`) is confirmed on the
    /// interpreter and reported as known, by its file, its function and its obligation; not
    /// known, each is a soundness bug that fails the job, and so is one known by another
    /// obligation or in another file, which is then missing too. A model that does not satisfy
    /// its query is a defect, and so is a solver's error, an answer missing or given twice. Tier
    /// 1 has no known soundness bug since pull request 182 fixed root cause 1, so the bugs here
    /// are queries written by hand for checks it keeps, beside the export's own.
    #[test]
    fn test_a_bug_named_as_known_is_confirmed_and_reported_by_its_obligation() {
        let dir = scratch("known");
        let file = dir.join("bugs.mn");
        fs::write(
            &file,
            "module m\n// smt: rem divisor at 4:31\n// smt: narrow range at 5:30 into `u8`\npub fn rem(d: u8) -> u8 = 7 % d\npub fn narrow(d: u8) -> u8 = d + 250\npub fn safe(d: u8[1..=9]) -> u8 = 7 % d\n",
        )
        .unwrap();
        let shown = file.display().to_string();
        let out = dir.join("out");
        let w = independent(std::slice::from_ref(&file), &out).unwrap();
        assert!(w.queries > 0, "{:#?}", w);
        let expect = expected(&shown, &fs::read_to_string(&file).unwrap());
        assert_eq!(
            expect,
            [
                (
                    shown.clone(),
                    "rem".to_string(),
                    "divisor at 4:31".to_string()
                ),
                (
                    shown.clone(),
                    "narrow".to_string(),
                    "range at 5:30 into `u8`".to_string()
                ),
            ]
        );
        // The checks Tier 1 keeps, each asked as if it had erased it: its query holds of the
        // input that traps.
        let bug = |name: &str, place: &str, obligation: &str, trap: &str, d: u32| {
            let path = out.join(format!("{}.smt2", name));
            fs::write(
                &path,
                format!(
                    "; source: {}:{}\n; function: {}\n; obligation: {}\n; trap: {}\n(declare-const p.d Int)\n(assert (= p.d {}))\n(check-sat)\n(get-value (p.d))\n",
                    shown, place, name, obligation, trap, d
                ),
            )
            .unwrap();
            path
        };
        let rem = bug(
            "rem",
            "4:31",
            "divisor at 4:31",
            "division-by-zero at 4:27",
            0,
        );
        let narrow = bug(
            "narrow",
            "5:30",
            "range at 5:30 into `u8`",
            "narrowing at 5:30",
            6,
        );
        let mut queries: Vec<PathBuf> = Vec::new();
        let mut stack = vec![out.clone()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    queries.push(p);
                }
            }
        }
        queries.sort();
        let listed: Vec<String> = queries.iter().map(|q| q.display().to_string()).collect();
        let line = |answer: &str, q: &str| format!("{}\t{}\n", answer, q);
        let mut answers = String::new();
        for (q, path) in queries.iter().zip(&listed) {
            if *q == rem {
                answers.push_str(&line("sat ((p.d 0))", path));
            } else if *q == narrow {
                answers.push_str(&line("sat ((p.d 6))", path));
            } else {
                answers.push_str(&line("unsat (error \"model is not available\")", path));
            }
        }
        let wrong = line("sat ((p.d 1))", &rem.display().to_string());
        let gate = Gate {
            expect: expect.clone(),
            queries: Some(listed.clone()),
        };
        let c = confirm(&answers, &gate);
        assert!(c.passes() && c.known.len() == 2, "{:#?}", c);
        assert!(
            c.unsat + 2 == queries.len() && c.undecided.is_empty(),
            "{:#?}",
            c
        );
        let reported = c.known.join("\n");
        for part in [
            "the obligation `divisor at 4:31` of `rem`, and rem(0) stops: trap[division-by-zero] at 4:27",
            "the obligation `range at 5:30 into `u8`` of `narrow`, and narrow(6) stops: trap[narrowing] at 5:30",
        ] {
            assert!(reported.contains(part), "{} not in:\n{}", part, reported);
        }
        // Not known, each is a soundness bug that fails the job; so is one known by another
        // obligation, or in another file, which is then missing too.
        let c = confirm(&answers, &Gate::default());
        assert_eq!((c.confirmed.len(), c.passes()), (2, false), "{:#?}", c);
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
            (0, 2, 2),
            "{:#?}",
            c
        );
        // A model that does not satisfy its query is a defect, and a known bug not confirmed
        // fails too; a timeout decides nothing, which the comparison with the base counts.
        let c = confirm(&format!("{}timeout\tsomewhere.smt2\n", wrong), &gate);
        assert_eq!(
            (c.unconfirmed.len(), c.missing.len(), c.undecided.len()),
            (1, 2, 1),
            "{:#?}",
            c
        );
        assert!(
            c.unconfirmed[0].contains("rem(1)") && c.unconfirmed[0].contains("does not satisfy"),
            "{:#?}",
            c
        );
        assert!(!c.passes());
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
        let _ = fs::remove_dir_all(&dir);
    }

    /// A trap tells its obligation by its tag where it carries enough to: a narrowing by the type it
    /// narrows into, a clause's trap by the clause. A model whose run traps at the kind and place
    /// its query names, but into another type than its obligation's, confirms nothing. Any other
    /// trap agrees with every obligation of its kind and place, which is the known limit.
    #[test]
    fn test_a_trap_tells_its_obligation_where_it_carries_enough() {
        use mente_interp::TrapKind as K;
        let into = |t: &'static str| K::Narrowing {
            value: None,
            into: t,
        };
        assert!(tag_agrees("range at 4:19 into `i32`", &into("i32"), None));
        assert!(!tag_agrees("range at 4:19 into `i32`", &into("i8"), None));
        assert!(!tag_agrees(
            "range at 4:19 into `i32`",
            &K::DivisionByZero,
            None
        ));
        let clause = "requires at 2:3 of the clause at 1:9";
        let requires = K::Requires("(> x 0)".into());
        assert!(tag_agrees(clause, &requires, Some("(> x 0)")));
        assert!(!tag_agrees(clause, &requires, Some("(> y 0)")));
        assert!(!tag_agrees(clause, &requires, None));
        let index = K::IndexOutOfBounds {
            index: Int::ZERO,
            len: 1,
        };
        assert!(!tag_agrees(clause, &index, Some("(> x 0)")));
        assert!(tag_agrees("divisor at 3:9", &K::DivisionByZero, None));
        // A clause by its place, written as the interpreter writes it in a trap.
        let src = "module m\npub fn f(x: u8) -> u8\n  requires x > 0\n{\n  var i: u8 = 0\n  while i < x invariant i <= x {\n    i += 1\n  }\n  i\n}\n";
        let checked = mente_sema::check_program_without_proofs(src).unwrap();
        assert_eq!(clause_at(&checked, 3, 12).as_deref(), Some("(> x 0)"));
        assert_eq!(clause_at(&checked, 6, 25).as_deref(), Some("(<= i x)"));
        assert_eq!(clause_at(&checked, 6, 9), None);
        // Two obligations share one trap at the literal: its own narrowing of `x` into `i16`,
        // which never fails, and the binding's narrowing of it into `u8`. A run of x = 300 traps
        // into `u8`, which confirms the second and not the first, whatever its query holds.
        let dir = scratch("tag");
        let file = dir.join("t.mn");
        fs::write(
            &file,
            "module m\npub fn f(x: i16) -> u8 {\n  val a: [2]u8 = [x, 1]\n  a[0]\n}\n",
        )
        .unwrap();
        let query = |obligation: &str| {
            format!(
                "; source: {}:3:19\n; function: f\n; obligation: {}\n; trap: narrowing at 3:18\n(declare-const p.x Int)\n(assert (= p.x 300))\n(check-sat)\n(get-value (p.x))\n",
                file.display(),
                obligation
            )
        };
        let path = dir.join("q.smt2");
        fs::write(&path, query("range at 3:19 into `u8`")).unwrap();
        let found = confirm_one(&path, "((p.x 300))");
        assert!(
            matches!(found, Ok(Verdict::Confirmed { .. })),
            "{:?}",
            found
        );
        fs::write(&path, query("range at 3:19 into `i16`")).unwrap();
        let found = confirm_one(&path, "((p.x 300))");
        assert!(matches!(found, Ok(Verdict::Unconfirmed(_))), "{:?}", found);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A check of the comparison's tests: its file, the hash of the file, its function, its
    /// obligation and its classes.
    fn check(file: &str, function: &str, obligation: &str, classes: &[Class]) -> Erased {
        Erased {
            file: file.to_string(),
            hash: format!("h-{}", file),
            function: function.to_string(),
            obligation: obligation.to_string(),
            classes: classes.to_vec(),
        }
    }

    fn q(path: &str) -> Class {
        Class::Query(path.to_string())
    }

    /// The files of an export of the comparison's tests: each path, its text's hash, and whether
    /// its module checks.
    fn programs(files: &[(&str, &str, bool)]) -> BTreeMap<String, Program> {
        files
            .iter()
            .map(|(f, h, checks)| {
                (
                    f.to_string(),
                    Program {
                        hash: h.to_string(),
                        checks: *checks,
                    },
                )
            })
            .collect()
    }

    fn answers(lines: &[(&str, &str)]) -> HashMap<String, String> {
        answers_of(
            &lines
                .iter()
                .map(|(a, p)| format!("{}\t{}\n", a, p))
                .collect::<String>(),
        )
    }

    /// A base, and a run of it that lists one of each thing the comparison lists, and holds a
    /// program the base checks and the run does not.
    fn base_and_run() -> (Export, Export) {
        let base = Export {
            files: programs(&[
                ("a.mn", "h-a.mn", true),
                ("b.mn", "h-b.mn", true),
                ("c.mn", "h-c.mn", true),
                ("d.mn", "h-d.mn", true),
                ("r.mn", "h-r.mn", true),
                ("u.mn", "h-u.mn", true),
            ]),
            checks: vec![
                check("a.mn", "f", "divisor at 3:9", &[q("base/f-3.smt2")]),
                check("a.mn", "f", "range at 4:1 into `u8`", &[Class::BeyondBound]),
                check("a.mn", "g", "index at 5:2", &[Class::Unreachable]),
                check("a.mn", "g", "shift at 6:3", &[q("base/g-6.smt2")]),
                check(
                    "a.mn",
                    "k",
                    "range at 8:2 into `i8`",
                    &[q("base/k-8.smt2"), Class::BeyondBound],
                ),
                check("b.mn", "f", "divisor at 1:1", &[q("base/b.smt2")]),
                check("c.mn", "f", "divisor at 1:1", &[Class::BeyondLacked]),
                check("d.mn", "f", "divisor at 1:1", &[q("base/d.smt2")]),
                check("r.mn", "f", "divisor at 2:2", &[q("base/r.smt2")]),
                check("u.mn", "f", "divisor at 1:1", &[q("base/u.smt2")]),
            ],
        };
        let mut run = Export {
            files: programs(&[
                ("a.mn", "h-a.mn", true),
                ("b.mn", "h-b2", true),
                ("e.mn", "h-e.mn", true),
                ("s.mn", "h-r.mn", true),
                ("u.mn", "h-u.mn", false),
            ]),
            checks: vec![
                check("a.mn", "f", "divisor at 3:9", &[q("head/f-3.smt2")]),
                check("a.mn", "f", "range at 4:1 into `u8`", &[q("head/f-4.smt2")]),
                check("a.mn", "g", "index at 5:2", &[Class::Unreachable]),
                check("a.mn", "h", "range at 7:4 into `i8`", &[q("head/h-7.smt2")]),
                check(
                    "a.mn",
                    "k",
                    "range at 8:2 into `i8`",
                    &[q("head/k-8.smt2"), Class::BeyondBound],
                ),
                check("b.mn", "f", "divisor at 1:1", &[Class::BeyondBound]),
                check("e.mn", "f", "divisor at 1:1", &[q("head/e.smt2")]),
                check("s.mn", "f", "divisor at 2:2", &[q("head/s.smt2")]),
            ],
        };
        for e in &mut run.checks {
            if e.file == "s.mn" {
                e.hash = "h-r.mn".to_string();
            }
            if e.file == "b.mn" {
                e.hash = "h-b2".to_string();
            }
        }
        (base, run)
    }

    /// The run of [`base_and_run`] with nothing left to fail it: the module the base checks
    /// checked again.
    fn passing_run() -> Export {
        let (_, mut run) = base_and_run();
        run.files.get_mut("u.mn").unwrap().checks = true;
        run.checks
            .push(check("u.mn", "f", "divisor at 1:1", &[q("head/u.smt2")]));
        run
    }

    /// A run is held to its base check by check over the programs both trees hold unchanged,
    /// matched by their text whatever their paths, and each check by its function and obligation:
    /// such a program either export did not read fails the run, and so does one the run no longer
    /// checks; a check verified more, a check Tier 1 erases in one run alone that is still
    /// verified, and programs added, changed, renamed or deleted are listed. Undecided answers
    /// count over the queries both runs ask, and over all of the run's. The keys are written and
    /// read back strictly.
    #[test]
    fn test_a_run_is_held_to_its_base_on_the_programs_both_hold_unchanged() {
        let (base, run) = base_and_run();
        let base_answers = answers(&[("timeout", "base/f-3.smt2"), ("unsat", "base/k-8.smt2")]);
        let run_answers = answers(&[
            ("timeout", "head/f-3.smt2"),
            ("unknown", "head/f-4.smt2"),
            ("unsat", "head/k-8.smt2"),
            ("unknown", "head/e.smt2"),
        ]);
        let d_here = |file: &str, _: &str| file == "d.mn";
        let nowhere = |_: &str, _: &str| false;
        let c = compare(
            &base,
            &base_answers,
            &run,
            &run_answers,
            &d_here,
            &nowhere,
            &[],
        );
        assert_eq!(c.dropped, Vec::<String>::new());
        assert_eq!(
            c.lost,
            ["d.mn: both trees hold it unchanged, but this export did not read it; the base's gave 1 checks: 1 with a query, 0 unreachable, 0 cut off by the bound, 0 by a construct the encoding lacks, 0 without a site"]
        );
        assert_eq!(
            c.unchecked,
            ["u.mn: the base's checker accepts its module and this run's does not"]
        );
        assert!(!c.passes());
        // Only a check both runs query counts toward the undecided answers over both, each side's
        // own: not `f`'s range, which the base cut off and this run queries undecided. Every
        // query of the run counts toward its share.
        assert_eq!((c.undecided, c.total), ((1, 1), (3, 6)));
        let moved = c.moved.join("\n");
        for part in [
            "a.mn: `f`'s obligation `range at 4:1 into `u8`` is cut off by the bound on the base and a query here",
            "a.mn: `g`'s obligation `shift at 6:3` is erased by Tier 1 on the base alone",
            "a.mn: `h`'s obligation `range at 7:4 into `i8`` is erased by Tier 1 here alone, and a query",
        ] {
            assert!(moved.contains(part), "{} not in:\n{}", part, moved);
        }
        assert_eq!(c.moved.len(), 3, "{:#?}", c);
        let listed = c.programs.join("\n");
        for part in [
            "renamed: r.mn to s.mn",
            "changed: b.mn: 1 checks: 1 with a query, 0 unreachable, 0 cut off by the bound, 0 by a construct the encoding lacks, 0 without a site on the base; 1 checks: 0 with a query, 0 unreachable, 1 cut off by the bound",
            "deleted: c.mn: 1 checks: 0 with a query, 0 unreachable, 0 cut off by the bound, 1 by a construct",
            "added: e.mn: 1 checks: 1 with a query",
        ] {
            assert!(listed.contains(part), "{} not in:\n{}", part, listed);
        }
        assert_eq!(c.programs.len(), 4, "{:#?}", c);
        // With every program read and checked, and no undecided answer, the run passes.
        let quiet = answers(&[]);
        let run = passing_run();
        let all = compare(&base, &quiet, &run, &quiet, &nowhere, &nowhere, &[]);
        assert!(all.passes(), "{:#?}", all);
        // A program the base's tree holds unchanged that its export did not read fails too.
        let e_there = |file: &str, _: &str| file == "e.mn";
        let c = compare(&base, &quiet, &run, &quiet, &nowhere, &e_there, &[]);
        assert!(!c.passes() && c.lost.len() == 1, "{:#?}", c);
        assert!(c.lost[0].starts_with(
            "e.mn: both trees hold it unchanged, but the base's export did not read it; this one gave 1 checks"
        ));
        // Undecided answers may rise by the slack over the queries both ask, and be at most one
        // in UNDECIDED_SHARE of the run's.
        for (undecided, total, holds) in [
            ((10, 15), (15, 3000), true),
            ((10, 16), (16, 3200), false),
            ((0, 5), (5, 1000), true),
            ((3, 0), (0, 0), true),
            ((0, 0), (1, 200), true),
            ((0, 0), (2, 399), false),
            ((0, 0), (2, 400), true),
            ((0, 0), (1, 0), false),
        ] {
            let c = Comparison {
                undecided,
                total,
                ..Comparison::default()
            };
            assert_eq!(c.passes(), holds, "{:?} {:?}", undecided, total);
        }
        // The keys are written and read back, and read strictly.
        assert_eq!(Export::read(&base.text()), Ok(base.clone()));
        assert_eq!(Export::read(&run.text()), Ok(run.clone()));
        for (text, says) in [
            (
                "check\ta.mn\th\tf",
                "`a.mn h f` is no check: a file, a hash, a function, an obligation and its classes",
            ),
            (
                "check\ta.mn\th\tf\to",
                "`a.mn h f o` is a check with no class",
            ),
            (
                "check\ta.mn\th\tf\to\tunreachable\tqueried",
                "`queried` is no class",
            ),
            (
                "file\ta.mn\th",
                "`file\ta.mn\th` is neither a file nor a check",
            ),
            (
                "file\ta.mn\th\tmaybe",
                "`file\ta.mn\th\tmaybe` is neither a file nor a check",
            ),
        ] {
            assert_eq!(Export::read(text), Err(says.to_string()), "{}", text);
        }
    }

    /// A check the run verifies less than its base fails it: unless each of its traps, best
    /// first, is verified at least as well as the base's, with no fewer of them; and so does a
    /// check Tier 1 erases in the run alone that the encoding cuts off or has no site for. Two runs tell a check by its
    /// function and obligation, so a query that becomes unreachable is one check verified less,
    /// not two that moved.
    #[test]
    fn test_a_check_verified_less_than_on_the_base_fails_the_run() {
        let (base, _) = base_and_run();
        let quiet = answers(&[]);
        let nowhere = |_: &str, _: &str| false;
        let at = |run: &Export, function: &str, obligation: &str| {
            run.checks
                .iter()
                .position(|e| {
                    e.file == "a.mn" && e.function == function && e.obligation == obligation
                })
                .unwrap()
        };
        for (function, obligation, classes, says) in [
            (
                "f",
                "divisor at 3:9",
                vec![Class::BeyondBound],
                "`f`'s obligation `divisor at 3:9` is a query on the base and cut off by the bound here",
            ),
            (
                "f",
                "divisor at 3:9",
                vec![Class::Unreachable],
                "`f`'s obligation `divisor at 3:9` is a query on the base and unreachable here",
            ),
            (
                "g",
                "index at 5:2",
                vec![Class::NoSite],
                "`g`'s obligation `index at 5:2` is unreachable on the base and without a site in the encoding here",
            ),
            (
                "h",
                "range at 7:4 into `i8`",
                vec![Class::BeyondBound],
                "`h`'s obligation `range at 7:4 into `i8`` is erased by Tier 1 here alone, and cut off by the bound",
            ),
            (
                "h",
                "range at 7:4 into `i8`",
                vec![Class::NoSite],
                "`h`'s obligation `range at 7:4 into `i8`` is erased by Tier 1 here alone, and without a site in the encoding",
            ),
            (
                "k",
                "range at 8:2 into `i8`",
                vec![Class::BeyondBound],
                "`k`'s obligation `range at 8:2 into `i8`` is a query, cut off by the bound on the base and cut off by the bound here",
            ),
            (
                "k",
                "range at 8:2 into `i8`",
                vec![Class::BeyondBound, Class::BeyondBound],
                "is a query, cut off by the bound on the base and cut off by the bound, cut off by the bound here",
            ),
        ] {
            let mut run = passing_run();
            let k = at(&run, function, obligation);
            run.checks[k].classes = classes;
            let c = compare(&base, &quiet, &run, &quiet, &nowhere, &nowhere, &[]);
            assert!(
                !c.passes() && c.dropped.len() == 1 && c.dropped[0].contains(says),
                "{}: {:#?}",
                says,
                c
            );
            assert!(c.moved.iter().all(|m| !m.contains(obligation)), "{:#?}", c);
        }
        // Trap for trap: a trap lost is verified less, however well the others are. The review's
        // three cases, then a query kept where a cut one went.
        let k = "range at 8:2 into `i8`";
        let with = |export: &Export, classes: Vec<Class>| {
            let mut export = export.clone();
            let at = at(&export, "k", k);
            export.checks[at].classes = classes;
            export
        };
        let u = || Class::Unreachable;
        for (was, now) in [
            (vec![q("b1"), q("b2")], vec![q("h1")]),
            (vec![q("b1"), u()], vec![q("h1")]),
            (vec![q("b1"), u(), u()], vec![q("h1"), u()]),
            (vec![q("b1"), Class::BeyondBound], vec![q("h1")]),
        ] {
            let c = compare(
                &with(&base, was.clone()),
                &quiet,
                &with(&passing_run(), now.clone()),
                &quiet,
                &nowhere,
                &nowhere,
                &[],
            );
            assert!(
                !c.passes() && c.dropped.len() == 1 && c.dropped[0].contains("`k`'s obligation"),
                "{:?} to {:?}: {:#?}",
                was,
                now,
                c
            );
        }
        // Each trap verified at least as well, in any order, with no fewer of them, passes.
        for (was, now) in [
            (
                vec![q("b1"), Class::BeyondBound],
                vec![Class::BeyondLacked, q("h1")],
            ),
            (vec![q("b1"), Class::BeyondBound], vec![q("h1"), q("h2")]),
            (vec![q("b1")], vec![u(), q("h1")]),
        ] {
            let c = compare(
                &with(&base, was.clone()),
                &quiet,
                &with(&passing_run(), now.clone()),
                &quiet,
                &nowhere,
                &nowhere,
                &[],
            );
            assert!(c.passes(), "{:?} to {:?}: {:#?}", was, now, c);
        }
        // Cut off at one place or at another, or with no site, verifies as little.
        let mut cut = base.clone();
        cut.checks[1].classes = vec![Class::BeyondLacked];
        let mut run = passing_run();
        run.checks[1].classes = vec![Class::NoSite];
        let c = compare(&cut, &quiet, &run, &quiet, &nowhere, &nowhere, &[]);
        assert!(c.passes(), "{:#?}", c);
        assert!(c.moved.iter().any(|m| m.contains(
            "cut off by a construct the encoding lacks on the base and without a site in the encoding here"
        )));
    }

    /// A trailer `Smt-Drop: <file> <function> <obligation>: <reason>` lets exactly the check it
    /// names be verified less, and the summary says why; a waiver that names no such check, or a
    /// trailer that is no waiver, fails the run.
    #[test]
    fn test_a_waiver_lets_exactly_the_check_it_names_through() {
        let (base, _) = base_and_run();
        let quiet = answers(&[]);
        let nowhere = |_: &str, _: &str| false;
        let mut run = passing_run();
        run.checks[0].classes = vec![Class::BeyondBound];
        let (waivers, bad) = waivers_of(
            "a.mn f divisor at 3:9: a deeper loop now\n\nb.mn f divisor at 1:1: changed anyway\n",
        );
        assert!(bad.is_empty(), "{:?}", bad);
        let c = compare(
            &base,
            &quiet,
            &run,
            &quiet,
            &nowhere,
            &nowhere,
            &waivers[..1],
        );
        assert!(c.passes(), "{:#?}", c);
        assert_eq!(
            c.waived,
            ["a.mn: `f`'s obligation `divisor at 3:9` is a query on the base and cut off by the bound here; a deeper loop now"]
        );
        // A waiver of another file, function or obligation lets nothing through, and is unused.
        for (other, waiver) in [
            ("file", "b.mn f divisor at 3:9: elsewhere"),
            ("function", "a.mn g divisor at 3:9: elsewhere"),
            ("obligation", "a.mn f divisor at 3:8: elsewhere"),
        ] {
            let (waivers, _) = waivers_of(waiver);
            let c = compare(&base, &quiet, &run, &quiet, &nowhere, &nowhere, &waivers);
            assert!(
                !c.passes() && c.dropped.len() == 1 && c.unused.len() == 1,
                "{}: {:#?}",
                other,
                c
            );
            assert!(
                c.unused[0].starts_with(&format!("Smt-Drop: {}", waiver)),
                "{:#?}",
                c
            );
        }
        // A waiver nothing needs fails the run.
        let c = compare(
            &base,
            &quiet,
            &passing_run(),
            &quiet,
            &nowhere,
            &nowhere,
            &waivers,
        );
        assert!(!c.passes() && c.unused.len() == 2, "{:#?}", c);
        // The obligation runs to the first `: `, and each part must be there.
        assert_eq!(
            Waiver::read("a.mn f range at 4:19 into `u8`: why not"),
            Ok(Waiver {
                file: "a.mn".to_string(),
                function: "f".to_string(),
                obligation: "range at 4:19 into `u8`".to_string(),
                reason: "why not".to_string(),
            })
        );
        // Each part missing alone: the function, after a doubled space; the obligation, a `: `
        // right after the function; the `: ` and the reason.
        let (none, bad) = waivers_of(
            "a.mn f\na.mn  divisor at 3:9: why\na.mn f : why\na.mn f divisor at 3:9\na.mn f divisor at 3:9:  \n",
        );
        assert!(none.is_empty(), "{:?}", none);
        assert_eq!(bad.len(), 5, "{:?}", bad);
        assert!(bad[0]
            .contains("is no waiver: write `Smt-Drop: <file> <function> <obligation>: <reason>`"));
        // The job reads the trailers of the commits since the base by this key.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let workflow = fs::read_to_string(root.join(".github/workflows/smt.yml")).unwrap();
        assert!(workflow.contains(&format!(
            "git log --format='%(trailers:key={},valueonly,unfold)' \"$base..HEAD\" > \"$RUNNER_TEMP/waivers.txt\"",
            WAIVER_TRAILER
        )));
    }

    /// The review's reproduction: an encoder that loses the sites of every function with an
    /// array parameter, which before this was a check gone from the export and passed as Tier 1's
    /// change. Every check Tier 1 erases is still exported, with no site, and each one the base
    /// verified fails the comparison.
    #[test]
    fn test_a_check_that_loses_its_site_is_verified_less_not_moved() {
        let dir = scratch("lost-site");
        let file = dir.join("m.mn");
        fs::write(
            &file,
            "module m\npub fn first(a: [4]u8, i: u8[0..=3]) -> u8 = a[i]\npub fn half(x: u8) -> u8 = x / 2\n",
        )
        .unwrap();
        let files = std::slice::from_ref(&file);
        let base = independent(files, &dir.join("base")).unwrap();
        let lose = |m: &Module, i: usize| {
            let mut enc = mente_smt::encode(m, i);
            if let Some(Some(f)) = m.functions.get(i) {
                if f.syntax.name == "first" {
                    enc.sites.clear();
                    enc.unencoded = vec![(f.syntax.name_span, WHOLE[1])];
                }
            }
            enc
        };
        let run = independent_with(files, &dir.join("run"), &lose).unwrap();
        // The export does not fail: a function the encoding lacks a parameter of has no site by
        // design. Its checks are still there, with no site.
        assert_eq!(run.unmatched, 0);
        let first: Vec<&Erased> = base
            .export
            .checks
            .iter()
            .filter(|e| e.function == "first")
            .collect();
        assert!(
            first.iter().any(|e| !e.queries().is_empty()),
            "{:#?}",
            base.export
        );
        assert_eq!(run.export.checks.len(), base.export.checks.len());
        assert!(run
            .export
            .checks
            .iter()
            .filter(|e| e.function == "first")
            .all(|e| e.classes == [Class::NoSite]));
        let quiet = answers(&[]);
        let nowhere = |_: &str, _: &str| false;
        let c = compare(
            &base.export,
            &quiet,
            &run.export,
            &quiet,
            &nowhere,
            &nowhere,
            &[],
        );
        assert!(!c.passes(), "{:#?}", c);
        assert_eq!(c.dropped.len(), first.len(), "{:#?}", c);
        assert!(c.dropped.iter().all(|d| d.contains("`first`'s obligation")
            && d.ends_with("without a site in the encoding here")));
        assert!(c.moved.is_empty(), "{:#?}", c);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A tree has the encoding when its workspace's manifest lists a member whose package is
    /// mente-smt, wherever it lies: moved, it is still found, and a crate of that path the
    /// workspace does not list is not.
    #[test]
    fn test_the_base_has_the_encoding_when_its_workspace_holds_the_package() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_eq!(encodes(&root), Ok(true));
        let dir = scratch("encodes");
        let package = |at: &Path, name: &str| {
            fs::create_dir_all(at).unwrap();
            fs::write(
                at.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{}\"\nversion = \"0.1.0\"\n\n[dependencies]\n",
                    name
                ),
            )
            .unwrap();
        };
        let workspace = |members: &str| {
            fs::write(
                dir.join("Cargo.toml"),
                format!("[workspace]\nresolver = \"2\"\nmembers = [\n{}]\n", members),
            )
            .unwrap();
        };
        package(&dir.join("tools/smt"), "mente-smt");
        package(&dir.join("compiler/mente-smt"), "mente-other");
        package(&dir.join("compiler/mente-syntax"), "mente-syntax");
        workspace("    \"compiler/mente-syntax\",\n    \"tools/smt\",\n");
        assert_eq!(encodes(&dir), Ok(true));
        workspace("    \"compiler/mente-syntax\",\n    \"compiler/mente-smt\",\n");
        assert_eq!(encodes(&dir), Ok(false));
        fs::write(dir.join("Cargo.toml"), "[workspace]\n").unwrap();
        assert!(encodes(&dir).unwrap_err().ends_with("lists no members"));
        let _ = fs::remove_dir_all(&dir);
        assert!(encodes(&dir).unwrap_err().starts_with("cannot read `"));
    }

    /// The checks `tests/smt-known` names as lying past the bounds are erased by Tier 1 and have
    /// no query, the encoding naming them cut off by the bound, never unreachable: the known limit
    /// of K, which the comparison with the base holds to the base's. No input fails such a check
    /// where Tier 1 is sound, and none of the paths the encoding follows reaches it.
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
            assert!(group.iter().all(|&k| enc.sites[k].beyond_bound));
            assert_eq!(unqueried(&enc, &group), Unqueried::BeyondBound);
            assert_eq!(enc.query(&group), None);
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

    /// A check none of whose sites fails is unreachable only where no path of its function is
    /// cut off: a loop cut past the bound after the check's last round may fail it in a later
    /// one, so the check is cut off by the bound, though no cut came before any of its sites.
    #[test]
    fn test_a_check_whose_path_is_cut_after_it_is_cut_off_not_unreachable() {
        let src =
            "module m\npub fn count(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n {\n    i += 1\n  }\n  i\n}\n";
        let checked = mente_sema::check_program_without_proofs(src).unwrap();
        let module = module_of(&checked);
        let mut enc = mente_smt::encode(&module, 0);
        assert!(enc.cut_past_bound && !enc.cut_at_lacked);
        let silent: Vec<Vec<usize>> = groups(&enc, |_| true)
            .into_iter()
            .filter(|g| enc.query(g).is_none())
            .collect();
        assert!(!silent.is_empty());
        for g in &silent {
            assert!(g.iter().all(|&k| !enc.sites[k].beyond_bound));
            assert_eq!(unqueried(&enc, g), Unqueried::BeyondBound);
        }
        // With no path cut anywhere, a check no input fails stays unreachable; with a path cut
        // at a construct the encoding lacks, it is cut off there.
        enc.cut_past_bound = false;
        assert_eq!(unqueried(&enc, &silent[0]), Unqueried::Unreachable);
        enc.cut_at_lacked = true;
        assert_eq!(unqueried(&enc, &silent[0]), Unqueried::BeyondLacked);
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

    /// A key is erased only where every record of it is proven; the keys keep the order of
    /// their first records.
    #[test]
    fn test_a_key_is_erased_only_where_every_record_of_it_is_proven() {
        let at = |line: u32, col: u32| Span {
            start: (line * 100 + col) as usize,
            end: (line * 100 + col + 1) as usize,
            line,
            col,
        };
        let o = |span: Span, proven: bool| Obligation {
            kind: ObligationKind::Divisor,
            span,
            must: true,
            proven,
            detail: String::new(),
            tag: Tier1Tag::None,
        };
        let (a, b, c) = (at(3, 9), at(4, 9), at(5, 9));
        let records = [o(a, true), o(b, true), o(a, false), o(c, false), o(b, true)];
        assert_eq!(erased(&records), [(Kind::Divisor, b, Tag::None)]);
        assert_eq!(
            erased(&[o(c, true), o(a, true)]),
            [(Kind::Divisor, c, Tag::None), (Kind::Divisor, a, Tag::None)]
        );
    }

    /// The sites of one obligation and one trap are one group, in the order they first appear;
    /// another trap of the same obligation, or another obligation at the same trap, is a group
    /// of its own. A function with no site and no construct that leaves it whole unencoded has
    /// each of its obligations unmatched.
    #[test]
    fn test_sites_are_grouped_by_obligation_and_trap() {
        let src = "module m\npub fn safe(d: u8[1..=9]) -> u8 = 7 % d\n";
        let checked = mente_sema::check_program(src).unwrap();
        let module = module_of(&checked);
        let mut enc = mente_smt::encode(&module, 0);
        let first = enc.sites[0].clone();
        let n = enc.sites.len();
        let mut other_trap = first.clone();
        other_trap.trap.span.col += 1;
        let mut other_key = first.clone();
        other_key.span.col += 1;
        enc.sites.extend([first.clone(), other_trap, other_key]);
        let found = groups(&enc, |_| true);
        let of = |k: usize| found.iter().position(|g| g.contains(&k)).unwrap();
        assert_eq!(found[of(0)], [0, n]);
        assert_eq!(found[of(n + 1)], [n + 1]);
        assert_eq!(found[of(n + 2)], [n + 2]);
        assert_eq!(found.iter().map(Vec::len).sum::<usize>(), enc.sites.len());
        assert!(groups(&enc, |_| false).is_empty());
        let cf = checked.functions[0].as_ref().unwrap();
        assert!(unmatched(&cf.obligations, &enc).is_empty());
        enc.sites.clear();
        assert!(enc.unencoded.is_empty());
        assert_eq!(unmatched(&cf.obligations, &enc).len(), cf.obligations.len());
    }

    /// The export counts each check by what the encoding made of it, each count the number of
    /// its class in the keys, over programs that give every class: a loop past the bound, a
    /// construct the encoding lacks before a check and around one, a module that does not check,
    /// and a function whose sites an encoder lost, whose obligations are then unmatched and make
    /// the export fail.
    #[test]
    fn test_the_export_counts_each_check_by_what_the_encoding_made_of_it() {
        let dir = scratch("counts");
        let lacked = dir.join("lacked.mn");
        fs::write(
            &lacked,
            "module m\npub struct P {\n  pub a: u8\n}\npub fn g(d: u8[1..=9]) -> u8 {\n  val p: P = P(a: 7 % d)\n  p.a % d\n}\n",
        )
        .unwrap();
        let unchecked = dir.join("unchecked.mn");
        fs::write(&unchecked, "module m\npub fn f() -> u8 = true\n").unwrap();
        let lost = dir.join("lost.mn");
        fs::write(&lost, "module m\npub fn lost(d: u8[1..=9]) -> u8 = 7 % d\n").unwrap();
        let bound =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/smt-known/loop-past-the-bound.mn");
        let safe = dir.join("safe.mn");
        fs::write(&safe, "module m\npub fn safe(d: u8[1..=9]) -> u8 = 7 % d\n").unwrap();
        let files = [
            bound.clone(),
            lacked.clone(),
            unchecked,
            lost.clone(),
            safe.clone(),
        ];
        let lose = |m: &Module, i: usize| {
            let mut enc = mente_smt::encode(m, i);
            if m.functions[i]
                .as_ref()
                .is_some_and(|f| f.syntax.name == "lost")
            {
                enc.sites.clear();
            }
            enc
        };
        let w = independent_with(&files, &dir.join("out"), &lose).unwrap();
        let n = |c: &dyn Fn(&Class) -> bool| {
            w.export
                .checks
                .iter()
                .flat_map(|e| &e.classes)
                .filter(|k| c(k))
                .count()
        };
        let classes = (
            n(&|c| matches!(c, Class::Query(_))),
            n(&|c| *c == Class::Unreachable),
            n(&|c| *c == Class::BeyondBound),
            n(&|c| *c == Class::BeyondLacked),
            n(&|c| *c == Class::NoSite),
        );
        assert_eq!(
            classes,
            (
                w.queries,
                w.unreachable,
                w.beyond_bound,
                w.beyond_lacked,
                w.no_site
            )
        );
        assert!(
            w.queries > 0 && w.beyond_bound > 0 && w.beyond_lacked > 0 && w.no_site > 0,
            "{:#?}",
            w
        );
        assert_eq!((w.files, w.unchecked), (5, 1));
        // Every check Tier 1 erases in a file that checks is exported once, in Tier 1's order,
        // with a class for each of its traps, each query naming its check; and a file that does
        // not check is exported as one.
        for file in [&bound, &lacked, &lost, &safe] {
            let shown = file.display().to_string();
            let source = fs::read_to_string(file).unwrap();
            assert_eq!(
                w.export.files.get(&shown),
                Some(&Program {
                    hash: text_hash(&source),
                    checks: true
                })
            );
            let checked = mente_sema::check_program(&source).unwrap();
            let erased_keys: Vec<(String, String)> = checked
                .functions
                .iter()
                .flatten()
                .flat_map(|cf| {
                    erased(&cf.obligations)
                        .into_iter()
                        .map(|k| (cf.syntax.name.clone(), mente_smt::obligation_text(k)))
                })
                .collect();
            let exported: Vec<(String, String)> = w
                .export
                .of(&shown)
                .iter()
                .map(|e| (e.function.clone(), e.obligation.clone()))
                .collect();
            assert_eq!(exported, erased_keys, "{}", shown);
        }
        let unchecked = dir.join("unchecked.mn").display().to_string();
        assert_eq!(
            w.export.files.get(&unchecked).map(|p| p.checks),
            Some(false)
        );
        for e in &w.export.checks {
            assert!(!e.classes.is_empty());
            assert_eq!(e.hash, w.export.files[&e.file].hash);
            for path in e.queries() {
                let h = mente_smt::header(&fs::read_to_string(path).unwrap()).unwrap();
                assert_eq!((&h.function, &h.obligation), (&e.function, &e.obligation));
            }
        }
        assert_eq!(Export::read(&w.export.text()), Ok(w.export.clone()));
        // Each place of a construct the encoding lacks is counted, and each obligation of the
        // function whose sites were lost.
        let mut unencoded = 0;
        let mut obligations = 0;
        for file in [&bound, &lacked, &lost, &safe] {
            let checked = mente_sema::check_program(&fs::read_to_string(file).unwrap()).unwrap();
            let module = module_of(&checked);
            for (i, cf) in checked.functions.iter().enumerate() {
                let Some(cf) = cf else {
                    continue;
                };
                let enc = lose(&module, i);
                unencoded += enc.unencoded.len();
                obligations += unmatched(&cf.obligations, &enc).len();
            }
        }
        assert!(unencoded > 0 && obligations > 0);
        assert_eq!((w.unencoded, w.unmatched), (unencoded, obligations));
        assert_eq!(independent_status(&w), 1);
        assert_eq!(
            independent_status(&Written {
                unmatched: 0,
                ..w.clone()
            }),
            0
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Each class reads back from its text, and the hash of a text is 64-bit FNV-1a, as its
    /// published values give it.
    #[test]
    fn test_the_keys_read_each_class_back_and_hash_each_text_as_fnv_1a() {
        for class in [
            Class::Query("a/f-3-divisor.smt2".to_string()),
            Class::Unreachable,
            Class::BeyondBound,
            Class::BeyondLacked,
            Class::NoSite,
        ] {
            assert_eq!(Class::read(&class.text()), Some(class.clone()));
        }
        assert_eq!(Class::read("query"), None);
        assert_eq!(text_hash(""), "cbf29ce484222325");
        assert_eq!(text_hash("a"), "af63dc4c8601ec8c");
        assert_eq!(text_hash("foobar"), "85944171f73967e8");
    }

    /// A model gives a boolean input its value: a run of `true` takes the branch that traps,
    /// which a run of `false` would not.
    #[test]
    fn test_a_model_gives_a_boolean_input_its_value() {
        let dir = scratch("boolean");
        let file = dir.join("b.mn");
        fs::write(
            &file,
            "module m\npub fn f(c: bool, d: u8) -> u8 {\n  if c {\n    7 % d\n  } else {\n    0\n  }\n}\n",
        )
        .unwrap();
        let path = dir.join("q.smt2");
        fs::write(
            &path,
            format!(
                "; source: {}:4:9\n; function: f\n; obligation: divisor at 4:9\n; trap: division-by-zero at 4:5\n(declare-const p.c Bool)\n(declare-const p.d Int)\n(assert (and p.c (= p.d 0)))\n(check-sat)\n(get-value (p.c p.d))\n",
                file.display()
            ),
        )
        .unwrap();
        let found = confirm_one(&path, "((p.c true) (p.d 0))");
        assert!(
            matches!(found, Ok(Verdict::Confirmed { .. })),
            "{:?}",
            found
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The encoding of function `function` of `src`, and the obligations Tier 1 records in it.
    fn encoded(src: &str, function: &str) -> (Encoding, Vec<Obligation>) {
        let checked = mente_sema::check_program(src).unwrap();
        let module = module_of(&checked);
        let index = checked.index(function).unwrap();
        let cf = checked.functions[index].as_ref().unwrap();
        (mente_smt::encode(&module, index), cf.obligations.clone())
    }

    /// The narrowing of a block's result is keyed at the block's last expression, where Tier 1
    /// records it, an array literal there element by element; and a call's checks are the
    /// callee's, never sites of the caller.
    #[test]
    fn test_a_result_is_keyed_at_its_expression_and_a_callee_keeps_its_checks() {
        let src = "module m\npub fn f(a: u8) -> u8 {\n  val b: u8 = a\n  b + 1\n}\n";
        let (enc, obligations) = encoded(src, "f");
        assert!(unmatched(&obligations, &enc).is_empty(), "{:?}", enc.sites);
        assert!(enc
            .sites
            .iter()
            .any(|s| s.kind == Kind::Range && (s.span.line, s.span.col) == (4, 3)));
        let src = "module m\npub fn pair(x: i16) -> [2]u8 {\n  val y: i16 = x\n  [y, 1]\n}\n";
        let (enc, obligations) = encoded(src, "pair");
        assert!(
            obligations
                .iter()
                .any(|o| (o.span.line, o.span.col) == (4, 4)),
            "{:?}",
            obligations
                .iter()
                .map(|o| (o.span.line, o.span.col, o.kind.name(), o.tag))
                .collect::<Vec<_>>()
        );
        assert!(unmatched(&obligations, &enc).is_empty(), "{:?}", enc.sites);
        let src = "module m\npub fn half(x: u8, d: u8[1..=9]) -> u8 = x / d\npub fn g(x: u8) -> u8 = half(x, 3)\n";
        let (enc, _) = encoded(src, "g");
        assert!(!enc.sites.is_empty());
        assert!(
            enc.sites.iter().all(|s| s.span.line == 3),
            "{:?}",
            enc.sites
        );
    }

    /// A check whose path was cut off at a construct the encoding lacks before it is cut off
    /// there, though the function is also cut past the bound, after it.
    #[test]
    fn test_a_cut_before_a_check_names_it_over_a_cut_after() {
        let src = "module m\npub struct P {\n  pub a: u8\n}\npub fn g(c: bool, n: u8) -> u8 {\n  var i: u8 = 0\n  if c {\n    val p: P = P(a: 1)\n    i = p.a\n  }\n  val q: u8 = 9 % 3\n  while i < n {\n    i += 1\n  }\n  q\n}\n";
        let (mut enc, _) = encoded(src, "g");
        assert!(enc.cut_past_bound && enc.cut_at_lacked);
        let divisor = enc
            .sites
            .iter()
            .position(|s| s.kind == Kind::Divisor)
            .unwrap();
        assert!(enc.sites[divisor].beyond_lacked && !enc.sites[divisor].beyond_bound);
        assert_eq!(enc.query(&[divisor]), None);
        assert_eq!(unqueried(&enc, &[divisor]), Unqueried::BeyondLacked);
    }

    /// A value of an array of more elements than an encoding holds is narrowed into its own type
    /// without a cut: its elements already lie in it; and a type with no refinement checks none.
    #[test]
    fn test_a_large_array_of_its_own_type_is_narrowed_without_a_cut() {
        let src = "module m\npub fn big(x: u8) -> u8 {\n  var a: [300]u8 = [0; 300]\n  a[1] = x\n  val b: [300]u8 = a\n  b[1]\n}\n";
        let (enc, _) = encoded(src, "big");
        assert!(enc.unencoded.is_empty(), "{:?}", enc.unencoded);
        assert!(!enc.cut_past_bound && !enc.cut_at_lacked);
    }

    /// An expression begins within the budget while the terms already made do not pass it: a
    /// function whose body is a literal begins it with the two constants every encoding starts
    /// with, so a budget of two holds it, and one does not.
    #[test]
    fn test_the_budget_counts_the_terms_made_before_an_expression_begins() {
        let checked =
            mente_sema::check_program_without_proofs("module m\npub fn one() -> u8 = 1\n").unwrap();
        let module = module_of(&checked);
        let exhausted = |terms: usize| {
            mente_smt::encode_within(&module, 0, terms)
                .unencoded
                .iter()
                .any(|(_, what)| *what == "a function too large to encode")
        };
        assert!(!exhausted(2));
        assert!(exhausted(1));
    }

    /// The length of an array is its type's, read without a cut; and a negation is checked at
    /// the operator on a 128-bit operand alone, where no narrowing point follows it.
    #[test]
    fn test_a_length_is_read_and_a_negation_checked_on_128_bits_alone() {
        for src in [
            "module m\npub fn n(xs: [4]u8) -> usize = xs.len\n",
            "module m\npub fn neg(x: i128) -> i128 = (-x) / 2\n",
            "module m\npub fn neg8(x: i8) -> i8 = (-x) / 2\n",
        ] {
            let runs = agreement(src, &[]);
            assert!(
                runs.wrong.is_empty() && runs.compared > 0 && runs.cut == 0,
                "{}: {} compared, {} cut: {:?}",
                src,
                runs.compared,
                runs.cut,
                runs.wrong
            );
        }
    }
}
