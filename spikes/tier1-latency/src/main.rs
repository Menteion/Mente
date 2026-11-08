//! `spike-tier1`: runs the Tier-1 latency protocol of `ROADMAP.md` on the generated module.
//!
//!     spike-tier1 measure [--functions N] [--warmup N] [--edits N] [--seed N]
//!     spike-tier1 generate [--functions N] [--seed N]
//!
//! `measure` is a benchmark: it runs on GitHub Actions (`.github/workflows/spike-tier1.yml`),
//! not on a developer's machine.

use std::process::ExitCode;
use std::time::Instant;

use spike_tier1_latency::generate::{module, random_edit, Rng};
use spike_tier1_latency::session::{Module, Outcome};

struct Options {
    functions: usize,
    warmup: usize,
    edits: usize,
    seed: u64,
}

fn options(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        functions: 200,
        warmup: 100,
        edits: 1000,
        seed: 0x5eed,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let v = it.next().ok_or(format!("{a} needs a value"))?;
        let n: u64 = v
            .parse()
            .map_err(|_| format!("{a} takes a number, not {v}"))?;
        match a.as_str() {
            "--functions" => o.functions = n as usize,
            "--warmup" => o.warmup = n as usize,
            "--edits" => o.edits = n as usize,
            "--seed" => o.seed = n,
            _ => return Err(format!("unknown option {a}")),
        }
    }
    Ok(o)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let k = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[k.min(sorted.len() - 1)]
}

#[derive(Default)]
struct Tally {
    micros: Vec<f64>,
    must_failures: usize,
    parse_errors: usize,
    other_diags: usize,
}

impl Tally {
    fn add(&mut self, us: f64, o: &Outcome) {
        self.micros.push(us);
        if o.parse_error {
            self.parse_errors += 1;
        } else if o.must_failures > 0 {
            self.must_failures += 1;
        } else if o.diags > 0 {
            self.other_diags += 1;
        }
    }
    fn line(&mut self, name: &str) -> String {
        self.micros.sort_by(f64::total_cmp);
        format!(
            "{name}: median {:.1} us, 99th percentile {:.1} us, max {:.1} us over {} edits; edits with a must-prove failure {}, with a parse error {}, with another diagnostic {}",
            percentile(&self.micros, 0.5),
            percentile(&self.micros, 0.99),
            self.micros.last().copied().unwrap_or(0.0),
            self.micros.len(),
            self.must_failures,
            self.parse_errors,
            self.other_diags
        )
    }
}

/// Times `apply` on `edits` random edits after `warmup` untimed ones; each edit is undone,
/// untimed, before the next.
fn run(
    m: &mut Module,
    o: &Options,
    seed: u64,
    apply: fn(&mut Module, usize, usize, &str) -> Outcome,
) -> Result<Tally, String> {
    let mut r = Rng(seed | 1);
    let mut tally = Tally::default();
    for n in 0..(o.warmup + o.edits) {
        let f = &m.funcs[r.below(m.funcs.len())];
        let e = random_edit(&m.src, f.body_start, f.span.end, &mut r).ok_or("no editable token")?;
        let old = m.src[e.start..e.end].to_string();
        let t = Instant::now();
        let outcome = apply(m, e.start, e.end, &e.text);
        let us = t.elapsed().as_secs_f64() * 1e6;
        apply(m, e.start, e.start + e.text.len(), &old);
        if n >= o.warmup {
            tally.add(us, &outcome);
        }
    }
    Ok(tally)
}

fn measure(o: &Options) -> Result<(), String> {
    let src = module(o.functions, o.seed);
    let t = Instant::now();
    let mut m = Module::load(&src).map_err(|d| d.message)?;
    let cold = t.elapsed().as_secs_f64() * 1e6;
    let obligations: usize = m.reports.iter().map(|r| r.obligations.len()).sum();
    let must: usize = m
        .reports
        .iter()
        .flat_map(|r| &r.obligations)
        .filter(|x| x.must)
        .count();
    let runtime: usize = m
        .reports
        .iter()
        .flat_map(|r| &r.obligations)
        .filter(|x| !x.proven && !x.must)
        .count();
    let diags = m.diagnostics().len();
    println!(
        "generated module: {} functions, {} bytes, {obligations} obligations ({must} must be proven; {runtime} left as run-time checks), {diags} diagnostics",
        m.funcs.len(),
        src.len()
    );
    if diags != 0 {
        return Err("the generated module should have no diagnostic".into());
    }
    let mut cold_runs: Vec<f64> = (0..20)
        .map(|_| {
            let t = Instant::now();
            let fresh = Module::load(&src);
            let us = t.elapsed().as_secs_f64() * 1e6;
            drop(fresh);
            us
        })
        .collect();
    cold_runs.sort_by(f64::total_cmp);
    println!("cold check of the whole module (parse and Tier 1 on every function): first {cold:.1} us, median of 20 {:.1} us", percentile(&cold_runs, 0.5));
    let mut incremental = run(&mut m, o, o.seed ^ 0xa5a5, Module::edit)?;
    println!(
        "{}",
        incremental.line("re-check after an edit (one function re-parsed and re-checked)")
    );
    let p99 = percentile(&incremental.micros, 0.99);
    println!(
        "target, a 99th percentile under 5 ms: {} ({p99:.1} us)",
        if p99 < 5000.0 { "met" } else { "missed" }
    );
    println!(
        "{}",
        run(&mut m, o, o.seed ^ 0xa5a5, Module::edit_reparsing_module)?
            .line("re-check after an edit (whole module re-parsed, changed functions re-checked)")
    );
    if m.src != src {
        return Err("the edits were not all undone".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("measure") => options(&args[1..]).and_then(|o| measure(&o)),
        Some("generate") => options(&args[1..]).map(|o| print!("{}", module(o.functions, o.seed))),
        _ => Err("usage: spike-tier1 measure|generate [--functions N] [--warmup N] [--edits N] [--seed N]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("spike-tier1: {e}");
            ExitCode::FAILURE
        }
    }
}
