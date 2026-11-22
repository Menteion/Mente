//! The three execution paths compared on generated programs and on the conformance suite, and
//! soundness by execution (`docs/design-brief.md`, "The quality system", and exit criteria 4 and
//! 5 of "Every milestone ends when").
//!
//! Every call runs four ways: on the reference interpreter, which checks every obligation at run
//! time; on MIR lowered with every check kept, from the module checked without Tier 1; on MIR
//! from which the lowering leaves out each check Tier 1 proves; and as native code built from
//! that MIR. All four must print the same, traps included. The reference interpreter against
//! the MIR with every check finds a lowering bug, and MIR against native code a code-generation
//! bug. Where the reference interpreter and the MIR with every check trap alike but the MIR
//! without the proven checks does not, a check Tier 1 proved was reached and failed: a wrong
//! proof, which the report names with the obligations Tier 1 proved at that place.
//!
//! The unit tests run a few programs; CI runs many more in release, from a range of seeds the
//! environment gives, which a failure's report names so that it can be replayed:
//!
//! ```text
//! MENTE_GEN_FIRST_SEED=1234 MENTE_GEN_PROGRAMS=1 cargo test --release -p mente-cli -- --ignored --nocapture generated_programs_run_alike
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use mente_threads::parallel;

use mente_codegen::lower;
use mente_interp::{Limits, Stop};
use mente_sema::tier1::ObligationKind;
use mente_sema::CheckedModule;

use super::containment;
use super::generate::{self, CallPlan, Mode};
use super::{native_within, reference_within, run_lowered, CallArgs, Printed};

/// The reference interpreter's limits in a comparison: a generated call does a few thousand
/// steps at most, far below MIR's own limit. The stack is no limit of these: item 25's budget
/// bounds every run alike.
const STEPS: Limits = Limits {
    max_steps: 1_000_000,
    max_array: 1 << 24,
};

/// How long native code may take on a call the reference interpreter finished within [`STEPS`],
/// which takes milliseconds: a call that runs longer is a code-generation bug.
const NATIVE_TIME: std::time::Duration = std::time::Duration::from_secs(20);

/// What a difference between the paths means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// The reference interpreter reached a state the checks rule out (`Stop::Broken`): the
    /// interpreter or the checks are wrong, not the paths it is compared with.
    Reference,
    /// The reference interpreter trapped at a check Tier 1 proved, which the lowering left out.
    WrongProof,
    /// Leaving out the checks Tier 1 proved changed a result without the reference
    /// interpreter trapping.
    Erasure,
    /// The reference interpreter reached a value outside Tier 1's state at that point, or a
    /// point Tier 1 holds empty or unreachable, whether or not a check it proved failed there
    /// (`containment`).
    Containment,
    /// The MIR with every check differs from the reference interpreter, or did not lower.
    Lowering,
    /// Native code differs from the MIR it was built from, or did not build.
    Codegen,
    /// A phase panicked, or a module checked with Tier 1 and not without it.
    Crash,
}

#[derive(Clone, Debug)]
struct Finding {
    kind: Kind,
    call: String,
    detail: String,
}

/// What comparing one program gave.
#[derive(Clone, Debug, Default)]
struct Outcome {
    calls: u64,
    traps: u64,
    /// Calls left out because they did not end within the reference interpreter's steps.
    unfinished: u64,
    proven: u64,
    unproven: u64,
    /// The points and the values of expressions the containment oracle held against Tier 1's
    /// claims.
    contained: (u64, u64),
    /// How long MIR and native code took, and the reference interpreter's traced runs with the
    /// containment oracle and Tier 1's claims it reads.
    timing: (std::time::Duration, std::time::Duration),
    findings: Vec<Finding>,
}

/// A call as `mn run` reads it.
fn show_call(f: &str, args: &[(bool, mente_interp::Value)]) -> String {
    let args: Vec<String> = args
        .iter()
        .map(|(m, v)| format!("{}{}", if *m { "mut " } else { "" }, v))
        .collect();
    format!("{}({})", f, args.join(", "))
}

fn show(p: &Printed) -> String {
    match p {
        Ok(lines) => lines.join("; "),
        Err(e) => format!("stops: {}", e),
    }
}

/// The kind, line and column of the trap a path printed, from `trap[kind] at line:col: ...`.
fn trap_at(p: &Printed) -> Option<(String, u32, u32)> {
    let rest = p.as_ref().err()?.strip_prefix("trap[")?;
    let (kind, rest) = rest.split_once("] at ")?;
    let (place, _) = rest.split_once(": ")?;
    let (line, col) = place.split_once(':')?;
    Some((kind.to_string(), line.parse().ok()?, col.parse().ok()?))
}

/// The obligation Tier 1 records for a run-time check of a trap's kind.
fn obligation_kind(trap: &str) -> Option<ObligationKind> {
    Some(match trap {
        "narrowing" => ObligationKind::Range,
        "refinement" => ObligationKind::Refinement,
        "division-by-zero" => ObligationKind::Divisor,
        "index-out-of-bounds" => ObligationKind::Index,
        "shift-amount" => ObligationKind::Shift,
        "requires" => ObligationKind::Requires,
        "ensures" => ObligationKind::Ensures,
        "invariant" => ObligationKind::Invariant,
        "decreases" => ObligationKind::Decreases,
        _ => return None,
    })
}

/// The obligations of a trap's kind Tier 1 proved on the trap's line: the proofs that left the
/// check out.
fn proofs_behind(module: &CheckedModule, trap: &str, line: u32) -> String {
    let Some(kind) = obligation_kind(trap) else {
        return format!("no obligation records a `{}` trap", trap);
    };
    let found: Vec<String> = module
        .functions
        .iter()
        .flatten()
        .flat_map(|f| &f.obligations)
        .filter(|o| o.kind == kind && o.span.line == line && o.proven)
        .map(|o| {
            format!(
                "{:?} at {}:{}{}",
                o.kind,
                o.span.line,
                o.span.col,
                if o.detail.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", o.detail)
                }
            )
        })
        .collect();
    if found.is_empty() {
        format!("Tier 1 proved no {:?} obligation on line {}", kind, line)
    } else {
        format!("Tier 1 proved {}", found.join(", "))
    }
}

/// Compares every call on the four paths and reports each difference. `proved` is the module
/// as Tier 1 checked it, whose proofs the lowering uses; `kept`, the same module checked without
/// Tier 1, whose lowering keeps every check. `expected` holds each call's outcome on the
/// reference interpreter, from the run the containment oracle traces (`containment::check`),
/// so that each call runs there once. Native code is built in `scratch` when one is given.
fn compare_module(
    proved: &CheckedModule,
    kept: &CheckedModule,
    calls: &[CallArgs],
    expected: Vec<Printed>,
    scratch: Option<&Path>,
) -> Outcome {
    let mut out = Outcome::default();
    for o in proved
        .functions
        .iter()
        .flatten()
        .flat_map(|f| &f.obligations)
    {
        if o.proven {
            out.proven += 1;
        } else {
            out.unproven += 1;
        }
    }
    let mut finding = |kind: Kind, call: &str, detail: String| {
        out.findings.push(Finding {
            kind,
            call: call.to_string(),
            detail,
        })
    };
    let lowered = match lower::lower(proved) {
        Ok(l) => l,
        Err(e) => {
            finding(Kind::Lowering, "", format!("not lowered: {}", e));
            return out;
        }
    };
    let every = match lower::lower(kept) {
        Ok(l) => l,
        Err(e) => {
            finding(
                Kind::Lowering,
                "",
                format!("not lowered without Tier 1: {}", e),
            );
            return out;
        }
    };
    // A call that does not end within the reference interpreter's steps, such as a loop that
    // never ends on its arguments, is left out: the paths count steps differently and native
    // code has no limit.
    let unfinished = Stop::OutOfSteps.to_string();
    let mut finished: Vec<(&CallArgs, Printed)> = Vec::new();
    for (c, expected) in calls.iter().zip(expected) {
        if expected.as_ref().err() == Some(&unfinished) {
            out.unfinished += 1;
        } else {
            finished.push((c, expected));
        }
    }
    // A call the reference interpreter ends at the stack budget is compared on MIR, and not run
    // natively: native code counts no frame yet, and its stack ends at the system's guard page
    // (`core-calculus.md`, item 25, what the paths do not do yet).
    let native: Vec<usize> = (0..finished.len())
        .filter(|&k| trap_at(&finished[k].1).is_none_or(|(kind, _, _)| kind != "stack-overflow"))
        .collect();
    let natively = match scratch {
        Some(dir) if !native.is_empty() => {
            let list: Vec<CallArgs> = native.iter().map(|&k| finished[k].0.clone()).collect();
            match native_within(&lowered, &list, dir, "program", Some(NATIVE_TIME)) {
                Ok(all) => Some(all),
                Err(e) => {
                    finding(Kind::Codegen, "", format!("not built: {}", e));
                    None
                }
            }
        }
        _ => None,
    };
    let mut calls_run = 0;
    let mut traps = 0;
    for (k, ((f, args), expected)) in finished.into_iter().enumerate() {
        let call = show_call(f, args);
        // A broken reference run is the interpreter's fault, or the checks', whatever MIR prints.
        if let Err(stop) = &expected {
            if stop.starts_with(mente_interp::BROKEN) {
                calls_run += 1;
                finding(
                    Kind::Reference,
                    &call,
                    format!("the reference interpreter {}", stop),
                );
                continue;
            }
        }
        let checked = run_lowered(&every, f, args.clone());
        let got = run_lowered(&lowered, f, args.clone());
        calls_run += 1;
        if trap_at(&expected).is_some() {
            traps += 1;
        }
        if checked != expected {
            finding(
                Kind::Lowering,
                &call,
                format!(
                    "the reference interpreter prints {}, MIR with every check {}",
                    show(&expected),
                    show(&checked)
                ),
            );
        } else if got != expected {
            match trap_at(&expected) {
                Some((trap, line, _)) => finding(
                    Kind::WrongProof,
                    &call,
                    format!(
                        "the reference interpreter and MIR with every check stop: {}; MIR \
                         without the checks Tier 1 proved prints {}; {}",
                        show(&expected),
                        show(&got),
                        proofs_behind(proved, &trap, line)
                    ),
                ),
                None => finding(
                    Kind::Erasure,
                    &call,
                    format!(
                        "the reference interpreter prints {}, MIR without the checks Tier 1 \
                         proved {}",
                        show(&expected),
                        show(&got)
                    ),
                ),
            }
        }
        let natively = natively
            .as_ref()
            .zip(native.iter().position(|&n| n == k))
            .map(|(all, j)| &all[j]);
        if let Some(printed) = natively {
            if *printed != got {
                finding(
                    Kind::Codegen,
                    &call,
                    format!("MIR prints {}, native code {}", show(&got), show(printed)),
                );
            }
        }
    }
    out.calls = calls_run;
    out.traps = traps;
    out
}

/// Runs `f` and turns a panic into a finding.
fn guarded(f: impl FnOnce() -> Outcome) -> Outcome {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(o) => o,
        Err(e) => {
            let message = generate::panic_message(&*e);
            Outcome {
                findings: vec![Finding {
                    kind: Kind::Crash,
                    call: String::new(),
                    detail: format!("panicked: {}", message),
                }],
                ..Outcome::default()
            }
        }
    }
}

/// Checks `src` with Tier 1 and without, makes the calls `plan` gives with `seed`, and compares
/// them; `Err` with the first code of each diagnostic when the program does not check.
fn compare_source(
    src: &str,
    seed: u64,
    plan: CallPlan,
    scratch: Option<&Path>,
) -> Result<Outcome, Vec<String>> {
    let checked = panic::catch_unwind(|| mente_sema::check_program(src));
    let proved = match checked {
        Ok(Ok(m)) => m,
        Ok(Err(diagnostics)) => {
            return Err(diagnostics.iter().map(|d| d.code.to_string()).collect());
        }
        Err(_) => {
            return Ok(Outcome {
                findings: vec![Finding {
                    kind: Kind::Crash,
                    call: String::new(),
                    detail: "the checker panicked".into(),
                }],
                ..Outcome::default()
            })
        }
    };
    Ok(compare_proved(src, &proved, seed, plan, scratch))
}

/// [`compare_source`] for a program that passes every check, whose checked module is `proved`.
fn compare_proved(
    src: &str,
    proved: &CheckedModule,
    seed: u64,
    plan: CallPlan,
    scratch: Option<&Path>,
) -> Outcome {
    guarded(|| {
        let kept = match mente_sema::check_program_without_proofs(src) {
            Ok(m) => m,
            Err(d) => {
                return Outcome {
                    findings: vec![Finding {
                        kind: Kind::Crash,
                        call: String::new(),
                        detail: format!("checks with Tier 1 but not without it: {:?}", d),
                    }],
                    ..Outcome::default()
                }
            }
        };
        let calls = generate::calls(src, proved, seed, plan);
        // Each call runs once on the reference interpreter, traced: the oracle holds its points
        // against Tier 1's states, and its outcome is the one the other paths are compared with.
        let started = std::time::Instant::now();
        let held = containment::check(src, proved, &calls, STEPS);
        let referenced = started.elapsed();
        let mut out = compare_module(proved, &kept, &calls, held.printed, scratch);
        out.contained = (held.points, held.values);
        out.timing = (started.elapsed() - referenced, referenced);
        for (k, detail) in held.problems {
            let call = calls.get(k).map_or(String::new(), |(f, a)| show_call(f, a));
            out.findings.push(Finding {
                kind: Kind::Containment,
                call,
                detail,
            });
        }
        out
    })
}

/// A folder of its own for each comparison, since tests run side by side in one process.
fn scratch(tag: &str) -> PathBuf {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mente-{}-{}-{}", tag, std::process::id(), run));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// What a run over many programs found.
#[derive(Debug, Default)]
struct Report {
    programs: u64,
    accepted: BTreeMap<String, (u64, u64)>,
    rejected: BTreeMap<String, u64>,
    calls: u64,
    traps: u64,
    unfinished: u64,
    /// The programs with calls left out unfinished, with how many.
    unfinished_in: BTreeMap<String, u64>,
    proven: u64,
    unproven: u64,
    /// The points and the values of expressions the containment oracle checked.
    contained: (u64, u64),
    /// How long MIR and native code took together, and the reference interpreter's traced runs
    /// with the containment oracle.
    timing: (std::time::Duration, std::time::Duration),
    /// Each program with findings: its name, its text and what was found.
    failures: Vec<(String, String, Vec<Finding>)>,
    /// How many functions were generated again for a narrowing Tier 1 proved always traps
    /// (`E0508`), and each such claim with its seed and what the reference interpreter saw
    /// of it.
    regenerated: usize,
    claims: Vec<(u64, generate::AlwaysTraps)>,
    /// What no generated program may show (`generate::Checked::problems`).
    problems: Vec<String>,
}

impl Report {
    fn add(&mut self, name: String, group: &str, src: &str, r: Result<Outcome, Vec<String>>) {
        self.programs += 1;
        let entry = self.accepted.entry(group.to_string()).or_default();
        entry.1 += 1;
        match r {
            Err(codes) => {
                let mut codes = codes;
                codes.dedup();
                *self.rejected.entry(codes.join(" ")).or_default() += 1;
            }
            Ok(o) => {
                entry.0 += 1;
                self.calls += o.calls;
                self.traps += o.traps;
                self.unfinished += o.unfinished;
                if o.unfinished > 0 {
                    self.unfinished_in.insert(name.clone(), o.unfinished);
                }
                self.proven += o.proven;
                self.unproven += o.unproven;
                self.contained.0 += o.contained.0;
                self.contained.1 += o.contained.1;
                self.timing.0 += o.timing.0;
                self.timing.1 += o.timing.1;
                if !o.findings.is_empty() {
                    self.failures.push((name, src.to_string(), o.findings));
                }
            }
        }
    }

    fn accepted(&self) -> u64 {
        self.accepted.values().map(|a| a.0).sum()
    }

    fn summary(&self) -> String {
        let mut s = format!(
            "{} programs, {} pass every check, Tier 1 included:",
            self.programs,
            self.accepted()
        );
        for (group, (ok, all)) in &self.accepted {
            s.push_str(&format!(
                " {} {}/{} ({}%)",
                group,
                ok,
                all,
                if *all == 0 { 0 } else { ok * 100 / all }
            ));
        }
        if !self.rejected.is_empty() {
            let rejected: Vec<String> = self
                .rejected
                .iter()
                .map(|(codes, n)| format!("{} x{}", codes, n))
                .collect();
            s.push_str(&format!(
                "\ndiscarded by their diagnostics: {}",
                rejected.join(", ")
            ));
        }
        if self.regenerated > 0 {
            s.push('\n');
            s.push_str(&generate::claims_summary(self.regenerated, &self.claims));
        }
        for p in &self.problems {
            s.push_str(&format!("\nproblem: {}", p));
        }
        s.push_str(&format!(
            "\n{} calls compared on the reference interpreter, MIR with every check, MIR without \
             the checks Tier 1 proved, and native code; {} of them trap; {} more left out, \
             unfinished within the reference interpreter's steps\n{} obligations proven by Tier \
             1, {} left to run time\n{} points and {} values of expressions the reference \
             interpreter reached, held against Tier 1's states there, in {:.1} s of traced \
             reference runs beside MIR's and native code's {:.1} s\n{} programs with \
             differences",
            self.calls,
            self.traps,
            self.unfinished,
            self.proven,
            self.unproven,
            self.contained.0,
            self.contained.1,
            self.timing.1.as_secs_f64(),
            self.timing.0.as_secs_f64(),
            self.failures.len()
        ));
        if !self.unfinished_in.is_empty() {
            let left: Vec<String> = self
                .unfinished_in
                .iter()
                .map(|(name, n)| format!("{} x{}", name, n))
                .collect();
            s.push_str(&format!(
                "
unfinished calls left out of: {}",
                left.join(", ")
            ));
        }
        s
    }

    /// The first programs with differences, `MENTE_GEN_SHOW` (5) of them, each with its text and
    /// its differences.
    fn failures(&self) -> String {
        let mut s = String::new();
        let shown = env_or("MENTE_GEN_SHOW", 5) as usize;
        for (name, src, findings) in self.failures.iter().take(shown) {
            s.push_str(&format!("\n=== {}\n{}\n", name, src));
            for f in findings.iter().take(10) {
                s.push_str(&format!("{:?} {}: {}\n", f.kind, f.call, f.detail));
            }
        }
        s
    }
}

fn group(mode: Mode) -> &'static str {
    match mode {
        Mode::Plain => "plain",
        Mode::Guarded => "guarded",
        Mode::Refined => "refined",
    }
}

/// The programs of the seeds a pull request's comparison checks without running, from 400 to
/// 4399, that the checker refuses for the type it gives a literal, each with the code: root cause
/// 8 of the nightly triage of 2026-11-21, a generator that types a literal by its neighbour alone
/// where the checker types it by every operand of its expression. The checker's refusal stands
/// for these alone, and only for their code; a listed program that checks is a problem too, so
/// that the list does not outlive its cause. The generator's rework (pull request 204) removes
/// it. Generation is deterministic per seed, so the list holds while the generator stays.
pub(crate) const ROOT_CAUSE_8: &[(u64, &str)] = &[
    (1551, "E0212"),
    (1706, "E0212"),
    (1901, "E0212"),
    (2907, "E0212"),
    (2984, "E0212"),
    (3047, "E0212"),
    (3618, "E0222"),
    (3661, "E0222"),
    (3901, "E0212"),
];

/// The problems of the program of `seed` that `list` does not excuse: the refusals of a code it
/// lists for the seed are excused, and none other; a listed program that checks is a problem
/// of its own, so that the list does not outlive its cause.
fn unexcused(seed: u64, checks: bool, problems: Vec<String>, list: &[(u64, &str)]) -> Vec<String> {
    let listed: Vec<&str> = list
        .iter()
        .filter(|(s, _)| *s == seed)
        .map(|(_, code)| *code)
        .collect();
    let mut out = Vec::new();
    if checks && !listed.is_empty() {
        out.push(format!(
            "seed {}: listed in ROOT_CAUSE_8 as refused, but it checks: remove it from the list",
            seed
        ));
    }
    let excused = |p: &String| {
        listed
            .iter()
            .any(|code| p.starts_with(&format!("seed {}: {} at ", seed, code)))
    };
    out.extend(problems.into_iter().filter(|p| !excused(p)));
    out
}

/// The programs of the `count` seeds from `first`, generated and checked, Tier 1 included, but
/// not run: a line of what they show, and what no generated program may show
/// (`generate::Checked::problems`, but the refusals of [`ROOT_CAUSE_8`]), with the bars of
/// every run of `count` programs.
fn check_generated(first: u64, count: u64) -> (String, Vec<String>) {
    let outcomes = parallel((first..first + count).collect(), generate::checked);
    let (mut accepted, mut regenerated) = (0, 0);
    let mut claims = Vec::new();
    let mut problems = Vec::new();
    for c in outcomes {
        let seed = c.program.seed;
        accepted += usize::from(c.module.is_some());
        regenerated += c.regenerated;
        problems.extend(unexcused(
            seed,
            c.module.is_some(),
            c.problems,
            ROOT_CAUSE_8,
        ));
        claims.extend(c.claims.into_iter().map(|a| (seed, a)));
    }
    problems.extend(generate::beyond_the_bars(count, regenerated, &claims));
    let line = format!(
        "{} more programs generated and checked, not run, seeds {} ..= {}: {} pass every check; \
         {} functions generated again for a narrowing that always traps (E0508) or their depth",
        count,
        first,
        (first + count).saturating_sub(1),
        accepted,
        regenerated
    );
    (line, problems)
}

/// Generates the programs of seeds `first .. first + count`, each made to pass every check
/// (`generate::checked`), and compares each, building native code when `natively`.
fn run_generated(first: u64, count: u64, plan: CallPlan, natively: bool) -> Report {
    let dir = natively.then(|| scratch("generated"));
    let outcomes = parallel((first..first + count).collect(), |seed| {
        let c = generate::checked(seed);
        let own = dir.as_ref().map(|d| d.join(format!("g{}", seed)));
        if let Some(d) = &own {
            fs::create_dir_all(d).unwrap();
        }
        let text = &c.program.text;
        let r = match &c.module {
            Some(proved) => Ok(compare_proved(text, proved, seed, plan, own.as_deref())),
            // A program the checker still refuses is counted with its codes.
            None => compare_source(text, seed, plan, own.as_deref()),
        };
        if let Some(d) = &own {
            fs::remove_dir_all(d).ok();
        }
        (c, r)
    });
    if let Some(d) = &dir {
        fs::remove_dir_all(d).ok();
    }
    let mut report = Report::default();
    for (c, r) in outcomes {
        let p = &c.program;
        let name = format!("seed {} ({})", p.seed, group(p.mode));
        report.add(name, group(p.mode), &p.text, r);
        report.regenerated += c.regenerated;
        report.problems.extend(c.problems);
        report
            .claims
            .extend(c.claims.into_iter().map(|a| (p.seed, a)));
    }
    // Whatever its size, a run is held to the bars of its claims.
    let beyond = generate::beyond_the_bars(count, report.regenerated, &report.claims);
    report.problems.extend(beyond);
    report
}

/// Every conformance program marked `// expect: pass`, by its path under `conformance/`.
fn conformance_programs() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let mut out = Vec::new();
    let mut dirs = vec![root.clone()];
    while let Some(d) = dirs.pop() {
        for entry in fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "mn") {
                let src = fs::read_to_string(&path).unwrap();
                if src
                    .lines()
                    .next()
                    .is_some_and(|l| l.trim() == "// expect: pass")
                {
                    let name = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push((name, src));
                }
            }
        }
    }
    out.sort();
    out
}

/// Compares every conformance program that must pass, with generated arguments for its public
/// functions; the seed of each program's arguments is the length of its name and text, so that
/// a run is the same everywhere.
/// The parts the conformance suite's test is split into, so that none holds most of its time
/// limit (`.config/nextest.toml`): each part takes the programs whose index in the sorted list
/// leaves its number when divided by them, which spreads every directory across the parts.
const PARTS: usize = 4;

/// Part `k` of `PARTS` of the conformance programs that must pass.
fn conformance_part(k: usize) -> Vec<(String, String)> {
    conformance_programs()
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % PARTS == k)
        .map(|(_, p)| p)
        .collect()
}

fn run_conformance(plan: CallPlan, natively: bool) -> Report {
    run_programs(conformance_programs(), plan, natively)
}

fn run_programs(programs: Vec<(String, String)>, plan: CallPlan, natively: bool) -> Report {
    let dir = natively.then(|| scratch("conformance"));
    let outcomes = parallel(programs, |(name, src)| {
        let seed = (name.len() * 7919 + src.len()) as u64;
        let own = dir.as_ref().map(|d| {
            let d = d.join(name.replace(['/', '.'], "_"));
            fs::create_dir_all(&d).unwrap();
            d
        });
        let r = compare_source(&src, seed, plan, own.as_deref());
        (name, src, r)
    });
    if let Some(d) = &dir {
        fs::remove_dir_all(d).ok();
    }
    let mut report = Report::default();
    for (name, src, r) in outcomes {
        let group = name.split('/').next().unwrap_or("").to_string();
        report.add(name, &group, &src, r);
    }
    report
}

/// The shortest program, by whole lines left out, that is still `interesting`: chunks of lines
/// are dropped while the rest keeps the property, halving the chunk when none can go.
fn reduce(src: &str, interesting: impl Fn(&str) -> bool) -> String {
    let join = |lines: &[&str]| lines.join("\n") + "\n";
    let mut lines: Vec<&str> = src.lines().collect();
    let mut chunk = (lines.len() / 2).max(1);
    loop {
        let mut i = 0;
        let mut dropped = false;
        while i < lines.len() {
            let end = (i + chunk).min(lines.len());
            let candidate: Vec<&str> = lines[..i].iter().chain(&lines[end..]).copied().collect();
            if interesting(&join(&candidate)) {
                lines = candidate;
                dropped = true;
            } else {
                i += chunk;
            }
        }
        if !dropped {
            if chunk == 1 {
                break;
            }
            chunk /= 2;
        }
    }
    join(&lines)
}

fn env_or(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use mente_interp::{Int, Value};

    use super::*;

    /// Each call's outcome on the reference interpreter, for a test that compares a module it
    /// builds by hand, without the oracle's traced runs.
    fn references(m: &CheckedModule, calls: &[CallArgs]) -> Vec<Printed> {
        calls
            .iter()
            .map(|(f, args)| reference_within(m, f, args.clone(), STEPS))
            .collect()
    }

    const FEW: CallPlan = CallPlan {
        random: 6,
        exhaustive: 64,
    };

    /// The programs of `seeds`, the first of each mode: each seed gives the same text every
    /// time, a program with a function generated again the same one from its seed and the
    /// attempt each function is at, every one passes every check, Tier 1 included, and together
    /// they are held to the bars of every run. The first ninety seeds are checked in three parts
    /// of thirty, ten of each mode, so that each part stays within half a test's time limit on
    /// the slowest runner: a seed whose function Tier 1 refuses is checked twice, and a few take
    /// a second or more in a debug build. That some seed has a function generated again, and so
    /// a program replayed from its attempts, the coverage test asserts over its own seeds.
    fn well_typed_and_replayable(seeds: std::ops::Range<u64>) {
        let count = seeds.end - seeds.start;
        // Each seed on its own, on as many threads as the machine has: the program that checks,
        // what Tier 1 proves of it, and whether the same text comes of the seed again.
        let programs = parallel(seeds.collect(), |seed| {
            let c = generate::checked(seed);
            let first = generate::program(seed).text;
            assert_eq!(first, generate::program(seed).text, "seed {}", seed);
            if c.regenerated > 0 {
                let text = &c.program.text;
                assert_eq!(
                    *text,
                    generate::generated(seed, &c.attempts).text,
                    "seed {}",
                    seed
                );
                assert_ne!(*text, first, "seed {}", seed);
            }
            assert!(c.problems.is_empty(), "seed {}: {:?}", seed, c.problems);
            let Some(m) = &c.module else {
                panic!("seed {} does not check", seed);
            };
            // Every program keeps within the allowance of the interpreter's host memory.
            assert!(
                super::super::within_host_allowance(mente_interp::host_bound_of(m)),
                "seed {} passes the host allowance: {} bytes",
                seed,
                mente_interp::host_bound_of(m)
            );
            let proven = m
                .functions
                .iter()
                .flatten()
                .flat_map(|f| &f.obligations)
                .filter(|o| o.proven)
                .count();
            (c.program.seed, proven, c.regenerated, c.claims)
        });
        let mut proven = 0;
        let mut functions = 0;
        let mut claims = Vec::new();
        for (seed, proved, count, claimed) in programs {
            proven += proved;
            functions += count;
            claims.extend(claimed.into_iter().map(|c| (seed, c)));
        }
        // These programs are held to the bars of every run.
        assert_eq!(
            generate::beyond_the_bars(count, functions, &claims),
            Vec::<String>::new()
        );
        assert!(proven >= 100, "Tier 1 proves only {} obligations", proven);
    }

    /// The allowance every program a harness generates or mutates keeps its bound on the reference
    /// interpreter's host memory within (`mente_interp::host_bound_of`) fits CI's limits, with a
    /// quarter of what a process may reserve kept as headroom, which is read where
    /// `.github/scripts/limits.sh` sets it for Linux and for Windows. Both processes that run
    /// generated or mutated programs fit: the comparison's, with an interpreter on each of CI's
    /// cores, and fuzz's worker, with one interpreter beside the two stacks of `mn`'s work and
    /// its phase thread. A function of the narrowest frame nests 10 levels within it, one of a
    /// wide frame 22; a bound of the allowance keeps within it, and a byte more does not.
    #[test]
    fn test_the_host_allowance_fits_ci_memory() {
        use super::super::{
            deepest_at, within_host_allowance, CI_ARENAS, CI_HEADROOM, CI_PROCESS_BYTES,
            CI_PROGRAM, CI_THREADS, HOST_ALLOWANCE,
        };
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/scripts/limits.sh");
        let limits = fs::read_to_string(script).unwrap();
        for set in [
            format!("ulimit -v {} ", CI_PROCESS_BYTES >> 10),
            format!("-ProcessBytes {} ", CI_PROCESS_BYTES),
        ] {
            assert!(limits.contains(&set), "limits.sh no longer sets {:?}", set);
        }
        // A quarter of what a process may reserve is headroom, which no bound counts on.
        assert_eq!(CI_HEADROOM, CI_PROCESS_BYTES / 4);
        let usable = CI_PROCESS_BYTES / 4 * 3;
        let fuzz_stacks = 2 * crate::STACK as u128;
        let compared = CI_THREADS * HOST_ALLOWANCE + CI_ARENAS + CI_PROGRAM;
        let fuzzed = HOST_ALLOWANCE + fuzz_stacks + CI_ARENAS + CI_PROGRAM;
        let (narrow, wide) = (deepest_at(64 + 4), deepest_at(64 + 4 + 5 * 16));
        println!(
            "{} bytes an interpreter: {} for the comparison's process, {} for fuzz's worker, of {} \
             usable; at a frame of 68 bytes a function nests at most {} levels, at 148 {}",
            HOST_ALLOWANCE, compared, fuzzed, usable, narrow, wide
        );
        assert!(compared <= usable && fuzzed <= usable);
        assert!(CI_THREADS * (HOST_ALLOWANCE + CI_THREADS) + CI_ARENAS + CI_PROGRAM > usable);
        assert_eq!((narrow, wide), (10, 22));
        assert!(within_host_allowance(HOST_ALLOWANCE));
        assert!(!within_host_allowance(HOST_ALLOWANCE + 1));
    }

    #[test]
    fn test_generated_programs_of_seeds_0_to_29_are_well_typed_and_replayable() {
        well_typed_and_replayable(0..30);
    }

    #[test]
    fn test_generated_programs_of_seeds_30_to_59_are_well_typed_and_replayable() {
        well_typed_and_replayable(30..60);
    }

    #[test]
    fn test_generated_programs_of_seeds_60_to_89_are_well_typed_and_replayable() {
        well_typed_and_replayable(60..90);
    }

    /// Together the first programs of seeds 0 to 89, thirty of each mode, reach what each mode
    /// adds: a plain program its statements and operators, a refined one its contracts and
    /// refinements, which no plain program has. Generating them takes a fraction of a second.
    #[test]
    fn test_generated_programs_of_each_mode_hold_what_it_adds() {
        let mut text = BTreeMap::<&str, String>::new();
        for seed in 0..90 {
            let p = generate::program(seed);
            text.entry(group(p.mode)).or_default().push_str(&p.text);
        }
        for word in [
            "while ", "guard ", "break", "continue", "mut ", "if ", "[", "<<", "%",
        ] {
            assert!(
                text["plain"].contains(word),
                "no plain program has `{}`",
                word
            );
        }
        for word in ["requires ", "ensures ", "decreases ", "invariant ", "..="] {
            assert!(
                text["refined"].contains(word),
                "no refined program has `{}`",
                word
            );
            assert!(
                !text["plain"].contains(word),
                "a plain program has `{}`",
                word
            );
        }
    }

    /// Arguments fit their parameters' types and refinements, small domains are called
    /// exhaustively, larger ones start with their extremes, and only public functions are
    /// called.
    #[test]
    fn test_arguments_fit_their_parameters_and_include_the_extremes() {
        let src = "module m\npub fn small(x: u8, b: bool) -> u8 = x\npub fn refined(n: usize[0..xs.len], xs: [3]i8, k: i16[-5..=5], mut m: u64) -> u8 = 0\nfn hidden(x: u8) -> u8 = x\n";
        let module = mente_sema::check_program(src).unwrap();
        let int = |v: &Value| match v {
            Value::Int(i) => {
                let m = i128::try_from(i.magnitude()).unwrap();
                if i.is_negative() {
                    -m
                } else {
                    m
                }
            }
            _ => panic!("not an integer: {}", v),
        };
        let calls = generate::calls(
            src,
            &module,
            7,
            CallPlan {
                random: 12,
                exhaustive: 512,
            },
        );
        let small: Vec<&CallArgs> = calls.iter().filter(|c| c.0 == "small").collect();
        assert_eq!(small.len(), 512, "every u8 with every bool");
        let distinct: std::collections::BTreeSet<String> =
            small.iter().map(|c| show_call(&c.0, &c.1)).collect();
        assert_eq!(distinct.len(), 512);
        assert!(calls.iter().all(|c| c.0 != "hidden"));
        let refined: Vec<&CallArgs> = calls.iter().filter(|c| c.0 == "refined").collect();
        assert_eq!(refined.len(), 12);
        for (_, args) in &refined {
            assert!((0..3).contains(&int(&args[0].1)));
            assert!((-5..=5).contains(&int(&args[2].1)));
            assert_eq!(
                args.iter().map(|a| a.0).collect::<Vec<_>>(),
                [false, false, false, true]
            );
            let Value::Array(xs) = &args[1].1 else {
                panic!("not an array")
            };
            assert!(xs.iter().all(|x| (-128..=127).contains(&int(x))));
        }
        assert_eq!(int(&refined[0].1[3].1), 0);
        assert_eq!(int(&refined[1].1[3].1), i128::from(u64::MAX));
        assert_eq!(int(&refined[0].1[2].1), -5);
        assert_eq!(int(&refined[1].1[0].1), 2);
        let fewer = generate::calls(src, &module, 7, FEW);
        assert_eq!(fewer.iter().filter(|c| c.0 == "small").count(), 6);
    }

    /// A function whose arguments the reference interpreter could not hold, more values than
    /// a literal of its may have, is not called: here 2^32 empty arrays, and an array just past
    /// the limit inside a struct; one at the limit is.
    #[test]
    fn test_calls_leave_out_arguments_too_large_to_hold() {
        let called = |src: &str, most: Option<u128>| {
            let module = mente_sema::check_program(src).unwrap();
            let calls = match most {
                Some(m) => generate::calls_within(src, &module, 7, FEW, m),
                None => generate::calls(src, &module, 7, FEW),
            };
            calls
                .into_iter()
                .map(|c| c.0)
                .collect::<std::collections::BTreeSet<String>>()
        };
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect();
        // Within a limit of 64 values: an array of 63 and the array itself are 64, a struct
        // holding an array of 64 more.
        let src = "module m\npub struct S {\n  xs: [64]u8\n}\npub fn held(s: S) -> u8 = 0\npub fn at(xs: [63]u8) -> u8 = 0\npub fn small(x: u8) -> u8 = x\n";
        assert_eq!(called(src, Some(64)), names(&["at", "small"]));
        // The reference interpreter's own limit leaves out 2^32 empty arrays.
        let src = "module m\npub fn empty(xs: [4294967295][0]u8) -> u8 = 0\npub fn small(x: u8) -> u8 = x\n";
        assert_eq!(called(src, None), names(&["small"]));
    }

    /// Generated programs of every mode, each call on the four paths: the reference
    /// interpreter, MIR with every check, MIR without the checks Tier 1 proved, and native code
    /// print the same.
    #[test]
    fn test_three_paths_agree_on_generated_programs() {
        let report = run_generated(0, 9, FEW, true);
        println!("{}", report.summary());
        assert!(report.failures.is_empty(), "{}", report.failures());
        assert!(report.problems.is_empty(), "{}", report.summary());
        assert_eq!(report.accepted(), report.programs, "{}", report.summary());
        assert!(report.calls >= 60, "{}", report.summary());
        assert!(report.contained.0 > 0, "{}", report.summary());
    }

    /// Among the seeds the comparison above runs, the generator ends some programs in a recursion
    /// the stack budget ends, through nested `if`s, `match`es and operators
    /// (`generate::deep_recursion`): on each, the reference interpreter and MIR grow their host
    /// stacks through those shapes and trap at the same call, item 25's (`core-calculus.md`). A
    /// thread sized per call ran out of host stack on such a recursion before the budget did.
    #[test]
    fn test_generated_recursions_run_to_the_budget_on_both_interpreters() {
        let mut deep = 0;
        for seed in 0..9 {
            let c = generate::checked(seed);
            let Some(m) = &c.module else {
                continue;
            };
            let Some((name, at)) = c.program.functions.last() else {
                continue;
            };
            if !c.program.text[at.clone()].contains("(4294967295)") {
                continue;
            }
            let reference = reference_within(m, name, Vec::new(), STEPS);
            let mir = match lower::lower(m) {
                Ok(l) => run_lowered(&l, name, Vec::new()),
                Err(e) => panic!("seed {}: not lowered: {}", seed, e),
            };
            assert!(
                trap_at(&reference).is_some_and(|(kind, _, _)| kind == "stack-overflow"),
                "seed {}: {:?}",
                seed,
                reference
            );
            assert_eq!(reference, mir, "seed {}", seed);
            deep += 1;
        }
        assert!(deep > 0, "no program of seeds 0 to 8 ends in a recursion");
    }

    /// Every conformance program that must pass of part `k`, with generated arguments for its
    /// public functions, prints the same on the four paths, and the reference interpreter's
    /// runs lie within Tier 1's states (`containment`).
    fn conformance_part_agrees(k: usize) {
        let report = run_programs(conformance_part(k), FEW, true);
        println!("{}", report.summary());
        assert!(report.failures.is_empty(), "{}", report.failures());
        assert!(report.rejected.is_empty(), "{}", report.summary());
        assert!(report.calls >= 25, "{}", report.summary());
        assert!(report.contained.0 > 0, "{}", report.summary());
    }

    #[test]
    fn test_three_paths_agree_on_the_conformance_suite_part_1_of_4() {
        conformance_part_agrees(0);
    }

    #[test]
    fn test_three_paths_agree_on_the_conformance_suite_part_2_of_4() {
        conformance_part_agrees(1);
    }

    #[test]
    fn test_three_paths_agree_on_the_conformance_suite_part_3_of_4() {
        conformance_part_agrees(2);
    }

    #[test]
    fn test_three_paths_agree_on_the_conformance_suite_part_4_of_4() {
        conformance_part_agrees(3);
    }

    /// The parts hold every conformance program that must pass, each once: a program in no part
    /// would run on no path.
    #[test]
    fn test_the_conformance_suite_parts_hold_every_program_once() {
        let mut parts: Vec<String> = (0..PARTS)
            .flat_map(|k| conformance_part(k).into_iter().map(|(name, _)| name))
            .collect();
        parts.sort();
        let all: Vec<String> = conformance_programs()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert!(!all.is_empty());
        assert_eq!(parts, all);
        assert_eq!(PARTS, 4, "the tests above name four parts");
    }

    /// The tracer only observes: the reference interpreter's traced run, whose outcome the other
    /// paths are compared with, prints what a run nobody traces prints, value, trap or what a
    /// `mut` argument leaves, on every fourth conformance program and a few generated ones. A
    /// tracer that changed what a run does would otherwise move the reference itself, and no
    /// comparison would see it.
    #[test]
    fn test_a_traced_run_prints_what_an_untraced_one_does() {
        let mut programs: Vec<(String, CheckedModule, u64)> = conformance_programs()
            .into_iter()
            .step_by(4)
            .filter_map(|(name, src)| {
                let m = mente_sema::check_program(&src).ok()?;
                Some((src, m, name.len() as u64))
            })
            .collect();
        for seed in 0..6 {
            let c = generate::checked(seed);
            if let Some(m) = c.module {
                programs.push((c.program.text, m, seed));
            }
        }
        assert!(programs.len() > 25, "{}", programs.len());
        let mut calls_seen = 0;
        let mut traps = 0;
        for (src, m, seed) in &programs {
            // A few calls a function: the run's outcome, not its coverage, is what is compared.
            let plan = CallPlan {
                random: 3,
                exhaustive: 8,
            };
            let calls = generate::calls(src, m, *seed, plan);
            let traced = containment::traced_outcomes(m, &calls, STEPS);
            let untraced = references(m, &calls);
            assert_eq!(traced, untraced, "{}", src);
            calls_seen += calls.len();
            traps += untraced.iter().filter(|p| trap_at(p).is_some()).count();
        }
        // The runs compared hold traps as well as values.
        assert!(
            calls_seen > 100 && traps > 0,
            "{} calls, {} traps",
            calls_seen,
            traps
        );
    }

    /// A check Tier 1 did not prove, marked proven by hand, is left out of MIR; the call that
    /// reaches it traps on the reference interpreter and not on that MIR, and the comparison
    /// reports a wrong proof with the obligation behind it.
    #[test]
    fn test_a_wrong_proof_is_reported_as_a_soundness_failure() {
        let src =
            "module m\npub fn get(xs: [4]u8, i: usize) -> u8 {\n  val k: u8 = xs[i] + 1\n  k\n}\n";
        let kept = mente_sema::check_program_without_proofs(src).unwrap();
        let honest = mente_sema::check_program(src).unwrap();
        let call = |i: u128| {
            let xs = Value::Array((1..=4).map(|v| Value::Int(Int::from_u128(v))).collect());
            (
                "get".to_string(),
                vec![(false, xs), (false, Value::Int(Int::from_u128(i)))],
            )
        };
        let calls = vec![call(2), call(9)];
        let fine = compare_module(&honest, &kept, &calls, references(&honest, &calls), None);
        assert!(fine.findings.is_empty(), "{:?}", fine.findings);
        assert_eq!((fine.calls, fine.traps), (2, 1));
        let mut wrong = honest.clone();
        for f in wrong.functions.iter_mut().flatten() {
            for o in &mut f.obligations {
                if o.kind == ObligationKind::Index {
                    assert!(!o.proven, "Tier 1 cannot prove i < 4 here");
                    o.proven = true;
                }
            }
        }
        let found = compare_module(&wrong, &kept, &calls, references(&wrong, &calls), None);
        assert_eq!(found.findings.len(), 1, "{:?}", found.findings);
        let f = &found.findings[0];
        assert_eq!(f.kind, Kind::WrongProof);
        assert_eq!(f.call, "get([1, 2, 3, 4], 9)");
        assert!(f.detail.contains("index-out-of-bounds"), "{}", f.detail);
        assert!(
            f.detail.contains("Tier 1 proved Index at 3:"),
            "{}",
            f.detail
        );
    }

    /// A reference interpreter that reaches a state the checks rule out is the finding, not
    /// the lowering that MIR then disagrees with: here a binding's declared refined type is made
    /// a `bool` by hand, which the lowering reads as no refinement to check, and the
    /// interpreter as a value of another shape.
    #[test]
    fn test_a_broken_reference_run_blames_the_interpreter() {
        let src = "module m\npub fn f(x: u8[0..5]) -> u8 {\n  val y: u8[0..9] = x\n  y\n}\n";
        let module = mente_sema::check_program(src).unwrap();
        let calls = vec![(
            "f".to_string(),
            vec![(false, Value::Int(Int::from_u128(3)))],
        )];
        let fine = compare_module(&module, &module, &calls, references(&module, &calls), None);
        assert!(fine.findings.is_empty(), "{:?}", fine.findings);
        let mut broken = module.clone();
        for f in broken.functions.iter_mut().flatten() {
            // The binding's declared type, the last refined one; the parameter's stays.
            let y = f.syntax.tree.types.iter().rposition(|t| {
                matches!(
                    t.kind,
                    mente_syntax::tree::TypeKind::Int {
                        refinement: Some(_),
                        ..
                    }
                )
            });
            if let Some(y) = y {
                f.syntax.tree.types[y].kind = mente_syntax::tree::TypeKind::Bool;
            }
        }
        let found = compare_module(&broken, &broken, &calls, references(&broken, &calls), None);
        assert_eq!(found.findings.len(), 1, "{:?}", found.findings);
        assert_eq!(
            found.findings[0].kind,
            Kind::Reference,
            "{:?}",
            found.findings
        );
        assert!(
            found.findings[0]
                .detail
                .contains("another shape than its declared type"),
            "{}",
            found.findings[0].detail
        );
    }

    /// A call of native code that runs past its limit is stopped and reported, and the calls
    /// after it still run, each its own.
    #[test]
    fn test_native_calls_stop_at_their_time_limit() {
        let src = "module m\npub fn spin(n: u32) -> u32 {\n  while n > 0 {\n  }\n  n\n}\n";
        let module = mente_sema::check_program(src).unwrap();
        let lowered = lower::lower(&module).unwrap();
        let call = |n: u128| {
            (
                "spin".to_string(),
                vec![(false, Value::Int(Int::from_u128(n)))],
            )
        };
        let dir = scratch("limit");
        let limit = std::time::Duration::from_millis(300);
        let got = native_within(&lowered, &[call(1), call(0)], &dir, "spin", Some(limit));
        fs::remove_dir_all(&dir).ok();
        assert_eq!(
            got,
            Ok(vec![
                Err("the program ran for more than 300 ms".to_string()),
                Ok(vec!["0".to_string()]),
            ])
        );
    }

    /// The list of expected refusals excuses exactly what it names (`unexcused`): a refusal of
    /// a listed code at a listed seed, and nothing else; and a listed program that checks is a
    /// problem.
    #[test]
    fn test_the_expected_refusals_excuse_only_what_they_name() {
        let list = [(7, "E0212"), (9, "E0222")];
        let refused = |seed: u64, code: &str| format!("seed {}: {} at 3:4: no type", seed, code);
        assert_eq!(
            unexcused(
                7,
                false,
                vec![refused(7, "E0212"), refused(7, "E0212")],
                &list
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            unexcused(7, false, vec![refused(7, "E0222")], &list),
            [refused(7, "E0222")]
        );
        assert_eq!(
            unexcused(8, false, vec![refused(8, "E0212")], &list),
            [refused(8, "E0212")]
        );
        assert_eq!(
            unexcused(9, true, Vec::new(), &list),
            ["seed 9: listed in ROOT_CAUSE_8 as refused, but it checks: remove it from the list"]
        );
        assert_eq!(unexcused(8, true, Vec::new(), &list), Vec::<String>::new());
        // A refusal for another reason at a listed seed stands, a panic among them.
        let panicked = "seed 7: the checker panicked: internal error".to_string();
        assert_eq!(
            unexcused(7, false, vec![panicked.clone()], &list),
            [panicked]
        );
    }

    /// Many generated programs, from the seeds `MENTE_GEN_FIRST_SEED` (0) to
    /// `MENTE_GEN_PROGRAMS` (300) later, each public function called `MENTE_GEN_CALLS` (12)
    /// times, or exhaustively where at most `MENTE_GEN_EXHAUSTIVE` (256) calls cover it; native
    /// code unless `MENTE_GEN_NATIVE` is 0. Then the `MENTE_GEN_CHECKED` seeds after them, ten
    /// times as many and at most 4000 unless it says otherwise, generated and checked but not run;
    /// 0 checks none, as a replay of one seed wants. A nightly run, whose ranges follow each other,
    /// so checks seeds the next night's run runs again.
    #[test]
    #[ignore = "generates and runs many programs; CI runs it in release (generated.yml)"]
    fn generated_programs_run_alike_on_the_three_paths() {
        // The figures the nesting the generator keeps to rests on hold in this build too, as a
        // test holds them in a build without optimisation (`mente_interp::host_bound`).
        if let Err(over) = super::super::host_shapes::figures_hold() {
            panic!("{}", over);
        }
        let first = env_or("MENTE_GEN_FIRST_SEED", 0);
        let count = env_or("MENTE_GEN_PROGRAMS", 300);
        let plan = CallPlan {
            random: env_or("MENTE_GEN_CALLS", 12) as usize,
            exhaustive: u128::from(env_or("MENTE_GEN_EXHAUSTIVE", 256)),
        };
        let natively = env_or("MENTE_GEN_NATIVE", 1) != 0;
        println!(
            "seeds {} ..= {}, {} random calls a function, exhaustive up to {}, native code {}",
            first,
            first + count - 1,
            plan.random,
            plan.exhaustive,
            if natively { "on" } else { "off" }
        );
        let start = std::time::Instant::now();
        let report = run_generated(first, count, plan, natively);
        println!("{}\nin {:.1?}", report.summary(), start.elapsed());
        // Then the seeds after them, ten times as many and at most 4000, generated and checked
        // but not run: the generator's and Tier 1's self-test at a volume the comparison cannot
        // afford. Pull request 197 merged checking 400 programs, and Tier 1 stopped the compile
        // on one generated program in about 220 (the compound remainder by a site value); from
        // 2026-11-16 the nightly met a program the checker refused in one of 250 to 500.
        let wide = env_or("MENTE_GEN_CHECKED", (10 * count).min(4000));
        let (checked, wrong) = if wide == 0 {
            ("no programs checked beyond them".to_string(), Vec::new())
        } else {
            let start = std::time::Instant::now();
            let (line, wrong) = check_generated(first + count, wide);
            (format!("{}\nin {:.1?}", line, start.elapsed()), wrong)
        };
        println!("{}", checked);
        // Each part is shown whichever fails: a run that differs and a program that does not
        // check are each one's own finding.
        assert!(
            report.failures.is_empty() && wrong.is_empty(),
            "replay one with MENTE_GEN_FIRST_SEED=<seed> MENTE_GEN_PROGRAMS=1 MENTE_GEN_CHECKED=0\n{}{}",
            report.failures(),
            if wrong.is_empty() {
                String::new()
            } else {
                format!("\na generated program does not check:\n{}", wrong.join("\n"))
            }
        );
        // No program is left out: each passes every check, a function Tier 1 refused for a
        // narrowing that always traps having been generated again, and nothing else refuses
        // one.
        assert!(
            report.problems.is_empty() && report.accepted() == report.programs,
            "a generated program does not check, or a run contradicts a claim of Tier 1:\n{}",
            report.summary()
        );
        // The containment oracle ran: the reference runs reached points it held against Tier 1.
        assert!(report.contained.0 > 0, "{}", report.summary());
    }

    /// One program, the file `MENTE_GEN_FILE` names, compared like a generated one, with the
    /// arguments `MENTE_GEN_FIRST_SEED` (0) gives: for reproducing and reducing a difference.
    #[test]
    #[ignore = "reads the program MENTE_GEN_FILE names"]
    fn a_file_runs_alike_on_the_three_paths() {
        let Ok(path) = std::env::var("MENTE_GEN_FILE") else {
            return;
        };
        let src = fs::read_to_string(&path).unwrap();
        let plan = CallPlan {
            random: env_or("MENTE_GEN_CALLS", 12) as usize,
            exhaustive: u128::from(env_or("MENTE_GEN_EXHAUSTIVE", 256)),
        };
        let dir = (env_or("MENTE_GEN_NATIVE", 1) != 0).then(|| scratch("file"));
        let seed = env_or("MENTE_GEN_FIRST_SEED", 0);
        let mut report = Report::default();
        report.add(
            path,
            "file",
            &src,
            compare_source(&src, seed, plan, dir.as_deref()),
        );
        if let Some(d) = &dir {
            fs::remove_dir_all(d).ok();
        }
        println!("{}", report.summary());
        assert!(report.failures.is_empty(), "{}", report.failures());
        assert!(report.rejected.is_empty(), "{}", report.summary());
    }

    /// The programs of seeds `MENTE_GEN_FIRST_SEED` (0) to `MENTE_GEN_PROGRAMS` (1) later, each
    /// with what checking it reports.
    #[test]
    #[ignore = "prints the programs of the seeds the environment names"]
    fn print_generated_programs() {
        let first = env_or("MENTE_GEN_FIRST_SEED", 0);
        for seed in first..first + env_or("MENTE_GEN_PROGRAMS", 1) {
            let p = generate::program(seed);
            println!("=== seed {} ({})\n{}", seed, group(p.mode), p.text);
            match panic::catch_unwind(|| mente_sema::check_program(&p.text)) {
                Ok(Ok(_)) => {}
                Ok(Err(diagnostics)) => {
                    for d in diagnostics {
                        let at = d.primary;
                        println!("{} at {}:{}: {}", d.code, at.line, at.col, d.message);
                    }
                }
                Err(_) => println!("the checker panicked"),
            }
        }
    }

    /// The program `MENTE_GEN_FILE` names, or that seed `MENTE_GEN_FIRST_SEED` generates,
    /// reduced line by line while it still shows a difference of the kind it first shows, on
    /// MIR alone unless that kind needs native code.
    #[test]
    #[ignore = "reduces the program MENTE_GEN_FILE or MENTE_GEN_FIRST_SEED names"]
    fn reduce_a_difference() {
        let seed = env_or("MENTE_GEN_FIRST_SEED", 0);
        let src = match std::env::var("MENTE_GEN_FILE") {
            Ok(path) => fs::read_to_string(path).unwrap(),
            Err(_) => generate::checked(seed).program.text,
        };
        let plan = CallPlan {
            random: env_or("MENTE_GEN_CALLS", 12) as usize,
            exhaustive: u128::from(env_or("MENTE_GEN_EXHAUSTIVE", 256)),
        };
        let dir = scratch("reduce");
        let kinds = |src: &str, natively: bool| -> Vec<Kind> {
            match compare_source(src, seed, plan, natively.then_some(dir.as_path())) {
                Ok(o) => o.findings.iter().map(|f| f.kind).collect(),
                Err(_) => Vec::new(),
            }
        };
        let hook = panic::take_hook();
        panic::set_hook(Box::new(|_| {}));
        let first = kinds(&src, true);
        let reduced = first.first().map(|&kind| {
            let natively = kind == Kind::Codegen;
            reduce(&src, |s| kinds(s, natively).contains(&kind))
        });
        panic::set_hook(hook);
        fs::remove_dir_all(&dir).ok();
        match reduced {
            Some(r) => println!("{:?}:\n{}", first[0], r),
            None => println!("no difference to reduce"),
        }
    }

    /// The conformance suite with more calls a function: `MENTE_GEN_CALLS` (64), or exhaustive
    /// up to `MENTE_GEN_EXHAUSTIVE` (65536).
    #[test]
    #[ignore = "many calls a program; CI runs it in release (generated.yml)"]
    fn conformance_programs_run_alike_on_the_three_paths() {
        let plan = CallPlan {
            random: env_or("MENTE_GEN_CALLS", 64) as usize,
            exhaustive: u128::from(env_or("MENTE_GEN_EXHAUSTIVE", 65536)),
        };
        let report = run_conformance(plan, env_or("MENTE_GEN_NATIVE", 1) != 0);
        println!("{}", report.summary());
        assert!(report.failures.is_empty(), "{}", report.failures());
        assert!(report.rejected.is_empty(), "{}", report.summary());
        assert!(report.contained.0 > 0, "{}", report.summary());
    }
}
