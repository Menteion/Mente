//! `mente-cli` (`mn` / `mente`) — Unified Mente Compiler & Verification CLI.
//!
//! Commands:
//! - `mn view --spec <file.mn>` : Instant `<0.1ms` Holographic Spec Blueprint projection & dual hash report.
//! - `mn view --checks <file.mn>`: Each run-time check Tier 1 leaves, function by function.
//! - `mn run <file.mn> [--call "f(args)"] [--mir | --native]`: Runs a call on the reference
//!   interpreter, with `--mir` through MIR, or with `--native` as native code, and prints the
//!   result or the trap.
//! - `mn build <file.mn> [--call "f(args)"] [-o <exe>]`: Builds the call as an executable, with
//!   debug information, that prints what `mn run` prints; with `--object`, writes only its
//!   object, for the host or the `--target` named.
//! - `mn card`: Prints the language card of the language version this compiler implements.
//! - `mn check [--json] <path>` : Full syntax, 64B Cache-Line `@layout`, `bitstruct`, `@total` & `@branchless` verification;
//!   `--json` prints the diagnostics in the versioned JSON form of `spec/diagnostics.md`.
//! - `mn explain [code]`        : What an error code means, from the catalogue in `catalogue.rs`.
//! - `mn test [--mutate]`       : Runs `mn check`, then a hand-written demo MIR region and its mutation gate;
//!   `@example` bodies are parsed as tokens but not executed.
//! - `mn spec`                   : Checks `spec/*.md` front matter and runs their `@assert-*` directives.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::Instant;

use mente_ast::project_spec_blueprint;
use mente_mir::{run_in_memory_mutation_gate, MirOp, MirRegionSoA, MirType, NodeId, ValueLattice};
use mente_sema::check_module;
use mente_syntax::diagnostics::{line_col, render_json, render_text, Diagnostic, FileDiagnostics};
use mente_syntax::parse_module;

mod catalogue;
mod fuzz;

/// The language card of the language version this compiler implements, which ships with it
/// (`docs/design-review.md`, `language-evolution`).
const CARD: &str = include_str!("../../../docs/language-card.md");

/// The most words the language card may hold, its front matter aside, so that it stays a card
/// and never grows into a second specification (`docs/design-review.md`, `language-card`).
#[cfg(test)]
const CARD_WORD_BUDGET: usize = 2_000;
#[cfg(test)]
mod gates;
#[cfg(test)]
mod latency;
mod mir_mutants;
mod mir_run;

/// The stack `mn`'s work runs on: every phase recurses over a function's tree, as deep as the
/// nesting limit allows (`E0127`), and a debug build's frames need more than a main thread's
/// stack holds for that.
const STACK: usize = 256 << 20;

fn main() {
    match std::thread::Builder::new().stack_size(STACK).spawn(run) {
        Ok(work) => {
            if work.join().is_err() {
                process::exit(101);
            }
        }
        Err(e) => {
            eprintln!("error: mn could not start its work: {}", e);
            process::exit(1);
        }
    }
}

fn run() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_help();
        return;
    }

    match args[0].as_str() {
        "view" => cmd_view(&args[1..]),
        "check" => cmd_check(&args[1..]),
        "test" => cmd_test(&args[1..]),
        "spec" => cmd_spec(&args[1..]),
        "explain" => cmd_explain(&args[1..]),
        "run" => cmd_run(&args[1..]),
        "build" => cmd_build(&args[1..]),
        "card" => print!("{}", CARD),
        "dump" => cmd_dump(&args[1..]),
        "smt" => cmd_smt(&args[1..]),
        "fuzz" => cmd_fuzz(&args[1..]),
        other => {
            eprintln!("Unknown command `{}`. Run `mn --help` for usage.", other);
            process::exit(1);
        }
    }
}

fn print_help() {
    println!("Mente Programming Language Toolchain (mn v0.1.0)");
    println!("Usage:");
    println!("  mn view --spec <file.mn>     Project Holographic Spec Blueprint (strips bodies, computes spec_hash/body_hash)");
    println!("  mn view --checks <file.mn>   List each run-time check Tier 1 leaves, and count those it erased");
    println!("  mn check [--json] [file.mn | dir]");
    println!("  mn check --watch <file.mn> [--rounds N]");
    println!(
        "                               Check again, item by item, each time the file changes"
    );
    println!("                               Verify .mn syntax, 64B Cache-Line @layout, bitstruct, @total & @branchless contracts");
    println!("  mn explain [code]            Explain an error code, or list them all");
    println!("  mn dump --tokens|--tree|--typed|--mir <file.mn>");
    println!("                               Print a phase's output in its canonical form");
    println!("  mn run <file.mn> [--call \"f(1, [2, 3])\"] [--mir | --native]");
    println!("                               Run main, or one call, on the reference interpreter");
    println!("  mn build <file.mn> [--call \"f(1, [2, 3])\"] [-o <exe>]");
    println!("                               Build main, or one call, as a native executable");
    println!("           [--object [--target <triple>] [--compilation-dir <dir>]]");
    println!("                               ...or write only its object, for the host or another target");
    println!("  mn card                      Print the language card of this compiler's language version");
    println!("  mn test [--mutate] [path]    Run mn check, then the hand-written demo MIR region (and its mutation gate)");
    println!("  mn spec                      Check spec/*.md front matter and run their @assert-* directives");
    println!("  mn fuzz [--seed N] [--seconds N | --runs N] [--jobs N] [--corpus <path>]... [--out <dir>] | --replay <path>...");
    println!("                               Fuzz every phase of the compiler, and minimize each crash found");
}

/// Fuzzes the compiler, or replays files through every phase (`fuzz.rs`).
fn cmd_fuzz(args: &[String]) {
    process::exit(fuzz::main(args));
}

/// Prints one phase's output for a module in its canonical form, which two implementations of
/// the phase must print byte for byte alike.
fn cmd_dump(args: &[String]) {
    let phase = args
        .iter()
        .find(|a| a.starts_with("--"))
        .map(String::as_str);
    let path = args.iter().find(|a| !a.starts_with('-'));
    let (Some(phase), Some(path)) = (phase, path) else {
        eprintln!("Usage: mn dump --tokens|--tree|--typed|--mir <file.mn>");
        process::exit(2);
    };
    let src = match fs::read_to_string(path) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {}", path, e);
            process::exit(2);
        }
    };
    let out = match phase {
        "--tokens" => mente_syntax::dump_tokens(&src),
        "--tree" => mente_syntax::dump_tree(&src),
        "--typed" => mente_sema::dump_typed(&src),
        "--mir" => match mente_sema::check_program(&src) {
            Err(diagnostics) => {
                for d in &diagnostics {
                    eprint!("{}", render_text(d, path, &src));
                }
                process::exit(2);
            }
            Ok(module) => match mente_codegen::lower::lower(&module) {
                Ok(lowered) => format!("mente-dump 2 mir\n{}", lowered.module),
                Err(e) => {
                    eprintln!("error: not lowered: {}", e);
                    process::exit(1);
                }
            },
        },
        other => {
            eprintln!(
                "error: `mn dump` knows --tokens, --tree, --typed and --mir, not `{}`",
                other
            );
            process::exit(2);
        }
    };
    print!("{}", out);
}

/// Runs a module on the reference interpreter: its `main`, or the call `--call` names. Prints
/// the result's canonical form, or the trap and where it happened (exit 1); a module that
/// does not check prints its diagnostics (exit 2).
fn cmd_run(args: &[String]) {
    let mut call = None;
    let mut path = None;
    let mut through_mir = false;
    let mut native = false;
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        if a == "--call" {
            call = rest.next().cloned();
        } else if a == "--mir" {
            through_mir = true;
        } else if a == "--native" {
            native = true;
        } else if !a.starts_with('-') {
            path = Some(a.clone());
        }
    }
    let Some(path) = path else {
        eprintln!("Usage: mn run <file.mn> [--call \"f(args)\"] [--mir | --native]");
        process::exit(2);
    };
    let (module, name, args) = checked_call(&path, call);
    if native {
        let dir = env::temp_dir().join(format!("mn-run-{}", process::id()));
        let printed = fs::create_dir_all(&dir)
            .map_err(|e| e.to_string())
            .and_then(|()| mente_codegen::lower::lower(&module))
            .and_then(|lowered| mir_run::native(&lowered, &[(name.clone(), args)], &dir, "run"))
            .map(|mut p| p.remove(0))
            .unwrap_or_else(Err);
        fs::remove_dir_all(&dir).ok();
        match printed {
            Ok(lines) => lines.iter().for_each(|l| println!("{}", l)),
            Err(stop) => {
                eprintln!("{}: {}", path, stop);
                process::exit(1);
            }
        }
        return;
    }
    if through_mir {
        match mir_run::run(&module, &name, args) {
            Ok(lines) => lines.iter().for_each(|l| println!("{}", l)),
            Err(stop) => {
                eprintln!("{}: {}", path, stop);
                process::exit(1);
            }
        }
        return;
    }
    let marked: Vec<bool> = args.iter().map(|(m, _)| *m).collect();
    let values = args.into_iter().map(|(_, v)| v).collect();
    match mente_interp::call_leaving(&module, &name, values, mente_interp::Limits::default()) {
        Ok((v, finals)) => {
            println!("{}", v);
            // What each `mut` argument is left holding.
            for (_, last) in marked.iter().zip(finals).filter(|(m, _)| **m) {
                println!("leaves {}", last);
            }
        }
        Err(stop) => {
            eprintln!("{}: {}", path, stop);
            process::exit(1);
        }
    }
}

/// Builds a module's call as an executable: its `main`, or the call `--call` names, made when
/// the executable starts and printed as `mn run` prints it, a trap with the file's name. `-o`
/// names the executable, by default the source's path without `.mn`; its object file, and on
/// Windows its PDB, lie beside it. A module that does not check prints its diagnostics
/// (exit 2); a call that cannot be built, why (exit 1).
fn cmd_build(args: &[String]) {
    let mut call = None;
    let mut path = None;
    let mut out = None;
    let mut object = false;
    let mut target = None;
    let mut directory = None;
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        if a == "--call" {
            call = rest.next().cloned();
        } else if a == "-o" {
            out = rest.next().cloned();
        } else if a == "--object" {
            object = true;
        } else if a == "--target" {
            target = Some(rest.next().cloned().unwrap_or_default());
        } else if a == "--compilation-dir" {
            directory = rest.next().cloned();
        } else if !a.starts_with('-') {
            path = Some(a.clone());
        }
    }
    let Some(path) = path else {
        eprintln!("Usage: mn build <file.mn> [--call \"f(args)\"] [-o <exe>] [--object [--target <triple>] [--compilation-dir <dir>]]");
        process::exit(2);
    };
    if !object && (target.is_some() || directory.is_some()) {
        eprintln!("error: `--target` and `--compilation-dir` go with `--object`: only the host's executables are linked");
        process::exit(2);
    }
    let target = match target {
        None => mente_codegen::Target::host(),
        Some(triple) => {
            let named = mente_codegen::Target::named(&triple);
            if named.is_none() {
                eprintln!(
                    "error: `--target` takes one of {}, not `{}`",
                    mente_codegen::Target::ALL
                        .map(mente_codegen::Target::triple)
                        .join(", "),
                    triple
                );
                process::exit(2);
            }
            named
        }
    };
    let (module, name, args) = checked_call(&path, call);
    let source = Path::new(&path);
    if object {
        let Some(target) = target else {
            eprintln!("error: no native code for this host; name a `--target`");
            process::exit(1);
        };
        let out = out
            .map(PathBuf::from)
            .unwrap_or_else(|| mente_codegen::native::object_path(source, target));
        let built = mente_codegen::lower::lower(&module).and_then(|lowered| {
            mir_run::object(
                &lowered,
                &[(name, args)],
                (source, directory.as_deref()),
                &format!("{}: ", path),
                (target, &out),
            )
        });
        match built {
            Ok(()) => println!("{}", out.display()),
            Err(e) => {
                eprintln!("error: {}", e);
                process::exit(1);
            }
        }
        return;
    }
    let exe = out
        .map(PathBuf::from)
        .unwrap_or_else(|| source.with_extension(env::consts::EXE_EXTENSION));
    let built = mente_codegen::lower::lower(&module).and_then(|lowered| {
        mir_run::build(
            &lowered,
            &[(name, args)],
            source,
            &format!("{}: ", path),
            &exe,
        )
    });
    match built {
        Ok(()) => println!("{}", exe.display()),
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(1);
        }
    }
}

/// The module at `path`, checked, and the call `call` names in it, `main` by default, with
/// each argument marked `mut` exactly where its parameter is. Exits with status 2 on anything
/// that does not check.
fn checked_call(
    path: &str,
    call: Option<String>,
) -> (
    mente_sema::CheckedModule,
    String,
    Vec<(bool, mente_interp::Value)>,
) {
    let src = match fs::read_to_string(path) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {}", path, e);
            process::exit(2);
        }
    };
    let module = match mente_sema::check_program(&src) {
        Ok(m) => m,
        Err(diagnostics) => {
            for d in &diagnostics {
                eprint!("{}", render_text(d, path, &src));
            }
            process::exit(2);
        }
    };
    let (name, args) = match call.as_deref().map(mente_interp::parse_call_modes) {
        None => ("main".to_string(), Vec::new()),
        Some(Some(parsed)) => parsed,
        Some(None) => {
            eprintln!(
                "error: `--call` takes a call such as \"f(1, true, mut [2, 3])\"; got `{}`",
                call.unwrap_or_default()
            );
            process::exit(2);
        }
    };
    // An argument is marked `mut` exactly where its parameter is.
    let params = module
        .index(&name)
        .and_then(|i| module.functions[i].as_ref())
        .map(|f| f.syntax.params.clone())
        .unwrap_or_default();
    for ((is_mut, _), p) in args.iter().zip(&params) {
        if *is_mut != p.is_mut {
            eprintln!(
                "error: parameter `{}` of `{}` is {}, so its argument is {}marked `mut`",
                p.name,
                name,
                if p.is_mut { "`mut`" } else { "read-only" },
                if p.is_mut { "" } else { "not " }
            );
            process::exit(2);
        }
    }
    (module, name, args)
}

/// Checks one module item by item, then again each time its file changes, re-parsing and
/// re-checking only what an edit changed. `--rounds N` stops after N checks.
fn cmd_watch(args: &[String]) {
    let mut path = None;
    let mut rounds = None;
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        if a == "--rounds" {
            rounds = rest.next().and_then(|n| n.parse::<u64>().ok());
        } else if !a.starts_with('-') {
            path = Some(a.clone());
        }
    }
    let Some(path) = path else {
        eprintln!("Usage: mn check --watch <file.mn> [--rounds N]");
        process::exit(2);
    };
    let mut session: Option<mente_sema::incremental::Session> = None;
    let mut done = 0;
    loop {
        let src = match fs::read_to_string(&path) {
            Ok(src) => src,
            Err(e) => {
                eprintln!("error: cannot read `{}`: {}", path, e);
                process::exit(2);
            }
        };
        if needs_check(session.as_ref(), &src) {
            let t0 = Instant::now();
            let s = session.get_or_insert_with(mente_sema::incremental::Session::default);
            s.update(&src);
            let took = t0.elapsed();
            for d in s.diagnostics() {
                eprint!("{}", render_text(d, &path, &src));
            }
            let work = s.work();
            println!(
                "[watch] {}: {} problem(s); {} in {} µs",
                path,
                s.diagnostics().len(),
                if work.whole {
                    "checked the whole module".to_string()
                } else {
                    format!(
                        "re-parsed {} item(s), re-checked {} function(s)",
                        work.parsed, work.checked
                    )
                },
                took.as_micros()
            );
            done += 1;
            if rounds.is_some_and(|n| done >= n) {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// Whether `mn check --watch` checks again: first, and each time the text changed.
fn needs_check(session: Option<&mente_sema::incremental::Session>, src: &str) -> bool {
    session.is_none_or(|s| s.source() != src)
}

fn cmd_view(args: &[String]) {
    if args.iter().any(|a| a == "--checks") {
        cmd_view_checks(args);
        return;
    }
    let path_str = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("examples/01_refinement_and_units.mn");

    let src = fs::read_to_string(path_str).unwrap_or_else(|e| {
        eprintln!("Failed to read `{}`: {}", path_str, e);
        process::exit(1);
    });
    // Timed from source text in memory, as the ROADMAP.md protocol measures it: file I/O excluded.
    let t0 = Instant::now();
    let module = parse_module(&src).unwrap_or_else(|e| {
        eprint!("{}", render_text(&e, path_str, &src));
        process::exit(1);
    });
    let report = project_spec_blueprint(&module);
    let elapsed_us = t0.elapsed().as_micros();

    println!("{}", report.projected_source);
    println!("// ----------------------------------------------------------------------------");
    println!(
        "// Projection completed in {} µs (parse, hash and project; file read excluded) | Tokens: {} -> {} ({:.1}% Context Saved)",
        elapsed_us, report.total_tokens, report.spec_tokens, report.savings_percent
    );
}

/// `mn view --checks <file.mn>`: for each function whose body checks, each run-time check Tier 1
/// leaves, as `line:col kind detail`, then how many it erased (`philosophy.md`, axiom 2).
fn cmd_view_checks(args: &[String]) {
    let Some(path) = args.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("Usage: mn view --checks <file.mn>");
        process::exit(2);
    };
    let src = match fs::read_to_string(path) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {}", path, e);
            process::exit(2);
        }
    };
    if let Err(d) = parse_module(&src) {
        eprint!("{}", render_text(&d, path, &src));
        process::exit(1);
    }
    print!("{}", run_time_checks(&src));
}

/// The run-time checks Tier 1 leaves in a module, in the form `mn view --checks` prints.
fn run_time_checks(src: &str) -> String {
    use mente_sema::tier1::ObligationKind;
    let mut out = String::new();
    for (name, report) in mente_sema::tier1_reports(src) {
        out.push_str(&format!("fn {}\n", name));
        let (erased, mut left): (Vec<_>, Vec<_>) = report
            .obligations
            .iter()
            .filter(|o| !o.must)
            .partition(|o| o.proven);
        left.sort_by_key(|o| o.span.start);
        for o in left {
            let kind = match o.kind {
                ObligationKind::Index => "index",
                ObligationKind::Divisor => "divisor",
                ObligationKind::Shift => "shift amount",
                _ => "narrowing",
            };
            let line = format!("{}:{} {} {}", o.span.line, o.span.col, kind, o.detail);
            out.push_str(&format!("  {}\n", line.trim_end()));
        }
        let plural = if erased.len() == 1 { "" } else { "s" };
        out.push_str(&format!(
            "  Tier 1 erased {} run-time check{}\n",
            erased.len(),
            plural
        ));
    }
    out
}

/// `mn smt <file.mn | dir>... --out <dir>`: writes each obligation Tier 1 discharges as an
/// SMT-LIB 2 query, for an SMT solver to answer `unsat` (`mente_sema::tier1::smt`). Each goes in
/// a file of its own, `<dir>/<the file's path without .mn>/<function>-<line>-<kind>.smt2`, whose
/// first line is `; source: <file>:<line>:<col>`. An obligation no path reaches, which no query
/// can check, is listed instead. Exits 2 when a file cannot be read or written.
fn cmd_smt(args: &[String]) {
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
        eprintln!("Usage: mn smt <file.mn | dir>... --out <dir>");
        process::exit(2);
    };
    let fail = |what: String| -> ! {
        eprintln!("error: {}", what);
        process::exit(2);
    };
    let files: Vec<PathBuf> = inputs.iter().flat_map(|p| collect_mn_files(p)).collect();
    let (mut written, mut unreached, mut functions) = (0, 0, 0);
    for file in &files {
        let shown = file.display().to_string();
        let src = fs::read_to_string(file)
            .unwrap_or_else(|e| fail(format!("cannot read `{}`: {}", shown, e)));
        let relative: PathBuf = file
            .with_extension("")
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(part) => Some(part),
                _ => None,
            })
            .collect();
        let dir = out.join(relative);
        let mut seen: std::collections::HashMap<String, usize> = Default::default();
        for (name, report, queries) in mente_sema::tier1_queries(&src) {
            functions += 1;
            for q in queries {
                let o = &report.obligations[q.obligation];
                let place = format!("{}:{}:{}", shown, o.span.line, o.span.col);
                let Some(text) = q.text else {
                    println!(
                        "unreached: {}: {} of `{}`, discharged where no path reaches it, which no query can check",
                        place,
                        o.kind.name(),
                        name
                    );
                    unreached += 1;
                    continue;
                };
                let base = format!("{}-{}-{}", name, o.span.line, o.kind.name());
                let n = seen.entry(base.clone()).or_insert(0);
                *n += 1;
                let path = match *n {
                    1 => dir.join(format!("{}.smt2", base)),
                    n => dir.join(format!("{}-{}.smt2", base, n)),
                };
                fs::create_dir_all(&dir)
                    .unwrap_or_else(|e| fail(format!("cannot create `{}`: {}", dir.display(), e)));
                fs::write(&path, format!("; source: {}\n{}", place, text))
                    .unwrap_or_else(|e| fail(format!("cannot write `{}`: {}", path.display(), e)));
                written += 1;
            }
        }
    }
    println!(
        "Wrote {} queries for {} functions of {} files to `{}`; {} discharged obligations are reached by no path.",
        written,
        functions,
        files.len(),
        out.display(),
        unreached
    );
}

/// What `mn check` prints after a file's diagnostics: one line for each `todo` and for each
/// trusted clause.
fn listing_lines(file: &str, src: &str, listings: &mente_sema::Listings) -> Vec<String> {
    let place = |at: usize| {
        let (line, col) = line_col(src, at);
        format!("{}:{}:{}", file, line, col)
    };
    let todos = listings
        .todos
        .iter()
        .map(|(span, reason)| format!("todo: {}: {:?}", place(span.start), reason));
    let axioms = listings.axioms.iter().map(|(span, clause)| {
        format!(
            "trusted: {}: {}, assumed until mn prove runs",
            place(span.start),
            clause
        )
    });
    todos.chain(axioms).collect()
}

fn collect_mn_files(target: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if target.is_file() {
        files.push(target.to_path_buf());
    } else if target.is_dir() {
        if let Ok(entries) = fs::read_dir(target) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("mn") {
                    files.push(p);
                }
            }
        }
        files.sort();
    }
    files
}

fn cmd_check(args: &[String]) {
    if args.iter().any(|a| a == "--watch") {
        cmd_watch(args);
        return;
    }
    let json = args.iter().any(|a| a == "--json");
    let target_str = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("examples");
    let files = collect_mn_files(Path::new(target_str));
    if files.is_empty() {
        eprintln!("No `.mn` files found at `{}`", target_str);
        process::exit(1);
    }

    let t0 = Instant::now();
    let mut all_ok = true;
    // Each file with its source and its diagnostics, for the JSON form.
    let mut checked: Vec<(String, String, Vec<Diagnostic>, mente_sema::Listings)> = Vec::new();
    for file in &files {
        let shown = file.display().to_string();
        let src = match fs::read_to_string(file) {
            Ok(src) => src,
            Err(e) => {
                eprintln!("error: cannot read `{}`: {}", shown, e);
                all_ok = false;
                continue;
            }
        };
        let diagnostics = match parse_module(&src) {
            Err(d) => vec![d],
            Ok(module) => match check_module(&src, &module) {
                Ok(rep) => {
                    if !json {
                        let proj = project_spec_blueprint(&module);
                        println!(
                            "[PASS] [{}] module `{}` | spec_hash: {}.. | layouts: {} | bitstructs: {} | @branchless: {} | @total: {} | @kernel: {}",
                            shown,
                            rep.module_path,
                            &proj.hashes.spec_hash[..12],
                            rep.layout_proofs.len(),
                            rep.bitstruct_proofs.len(),
                            rep.branchless_functions_verified,
                            rep.total_functions_verified,
                            rep.kernel_functions_verified,
                        );
                        for lp in &rep.layout_proofs {
                            println!(
                                "       Proven @layout `{}` ({} bytes, align {} bytes — 0 padding holes)",
                                lp.struct_name, lp.total_bytes, lp.align_bytes
                            );
                        }
                    }
                    Vec::new()
                }
                Err(diagnostics) => diagnostics,
            },
        };
        if !diagnostics.is_empty() {
            all_ok = false;
            if !json {
                for d in &diagnostics {
                    eprint!("{}", render_text(d, &shown, &src));
                }
            }
        }
        // Every todo and every trusted clause, which change nothing in the verdict.
        let listings = mente_sema::listings(&src);
        if !json {
            for line in listing_lines(&shown, &src, &listings) {
                println!("{}", line);
            }
        }
        checked.push((shown, src, diagnostics, listings));
    }
    if json {
        let files: Vec<FileDiagnostics> = checked
            .iter()
            .map(|(file, src, diagnostics, listings)| FileDiagnostics {
                file,
                src,
                diagnostics,
                todos: &listings.todos,
                axioms: &listings.axioms,
            })
            .collect();
        println!("{}", render_json(&files));
    } else {
        println!(
            "
Checked {} `.mn` file(s) in {:.2} ms.",
            files.len(),
            t0.elapsed().as_secs_f64() * 1000.0
        );
    }
    if !all_ok {
        process::exit(1);
    }
}

/// `mn explain E0413` prints what a code means; `mn explain` lists every code.
fn cmd_explain(args: &[String]) {
    let Some(code) = args.first() else {
        for e in catalogue::CATALOGUE {
            let status = match e.status {
                catalogue::CodeStatus::Emitted => "",
                catalogue::CodeStatus::Reserved => " (reserved)",
                catalogue::CodeStatus::Retired => " (retired)",
            };
            println!("{}  {}{}", e.code, e.title, status);
        }
        return;
    };
    match catalogue::lookup(&code.to_uppercase()) {
        Some(e) => print!("{}", catalogue::explain(e)),
        None => {
            eprintln!(
                "`{}` is not a Mente error code; `mn explain` lists them all",
                code
            );
            process::exit(1);
        }
    }
}

/// What `mn check` reports for one module's source, one diagnostic per line as `line:col code`:
/// the form of a module's `.expected` file.
fn diagnostic_lines(src: &str) -> Vec<String> {
    let diagnostics = match parse_module(src) {
        Err(d) => vec![d],
        Ok(module) => check_module(src, &module).err().unwrap_or_default(),
    };
    diagnostics
        .iter()
        .map(|d| format!("{}:{} {}", d.primary.line, d.primary.col, d.code))
        .collect()
}

/// The diagnostics a module is expected to report: the lines of the `.expected` file beside
/// it, or none when it has no such file. An example uses constructs of later milestones, and
/// its file lists what M1 reports for them until the milestone that runs it in full
/// (`docs/design-review.md`, `example-bodies`).
fn expected_lines(file: &Path) -> Vec<String> {
    fs::read_to_string(file.with_extension("expected"))
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// `mn test`'s first step: every module reports exactly the diagnostics it is expected to.
fn test_modules(target: &Path) {
    let files = collect_mn_files(target);
    if files.is_empty() {
        eprintln!("No `.mn` files found at `{}`", target.display());
        process::exit(1);
    }
    let mut failed = 0;
    for file in &files {
        let shown = file.display().to_string();
        let Ok(src) = fs::read_to_string(file) else {
            eprintln!("error: cannot read `{}`", shown);
            failed += 1;
            continue;
        };
        let actual = diagnostic_lines(&src);
        let expected = expected_lines(file);
        if actual == expected {
            match expected.len() {
                0 => println!("[PASS] [{}] reports no diagnostic", shown),
                n => println!("[PASS] [{}] reports its {} expected diagnostics", shown, n),
            }
            continue;
        }
        failed += 1;
        eprintln!(
            "[FAIL] [{}] does not report what {} lists:",
            shown,
            if expected.is_empty() {
                String::from("an empty `.expected` file")
            } else {
                format!("`{}`", file.with_extension("expected").display())
            }
        );
        for line in actual.iter().filter(|l| !expected.contains(l)) {
            eprintln!("  reported, not expected: {}", line);
        }
        for line in expected.iter().filter(|l| !actual.contains(l)) {
            eprintln!("  expected, not reported: {}", line);
        }
        if actual.iter().all(|l| expected.contains(l))
            && expected.iter().all(|l| actual.contains(l))
        {
            eprintln!("  the same diagnostics, in another order or number");
        }
    }
    if failed > 0 {
        process::exit(1);
    }
}

fn cmd_test(args: &[String]) {
    let run_mutate = args.iter().any(|a| a == "--mutate");
    let target_str = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("examples");

    test_modules(Path::new(target_str));

    let t0 = Instant::now();
    let mut r = MirRegionSoA::new("integrate_cell", true);
    let z = NodeId(0);
    let potential = r.push_node(
        MirOp::Param(0),
        MirType::Q16_16,
        [z, z, z],
        ValueLattice::bounded(0, 655360),
        1,
    );
    let drive = r.push_node(
        MirOp::Param(1),
        MirType::Q16_16,
        [z, z, z],
        ValueLattice::bounded(0, 655360),
        1,
    );
    let leak_shift = r.push_node(
        MirOp::Param(2),
        MirType::I64,
        [z, z, z],
        ValueLattice::bounded(1, 8),
        1,
    );
    let threshold = r.push_node(
        MirOp::Param(3),
        MirType::Q16_16,
        [z, z, z],
        ValueLattice::exact(65536),
        1,
    );
    let resting = r.push_node(
        MirOp::Param(4),
        MirType::Q16_16,
        [z, z, z],
        ValueLattice::exact(0),
        1,
    );

    let shifted = r.push_node(
        MirOp::Shr,
        MirType::Q16_16,
        [potential, leak_shift, z],
        ValueLattice::bounded(0, 327680),
        2,
    );
    let leaked = r.push_node(
        MirOp::SubSatQ16,
        MirType::Q16_16,
        [potential, shifted, z],
        ValueLattice::bounded(0, 655360),
        2,
    );
    let next_v = r.push_node(
        MirOp::AddSatQ16,
        MirType::Q16_16,
        [leaked, drive, z],
        ValueLattice::bounded(0, 655360),
        3,
    );
    let fired = r.push_node(
        MirOp::CmpGe,
        MirType::Bool,
        [next_v, threshold, z],
        ValueLattice::bounded(0, 1),
        4,
    );
    let out_v = r.push_node(
        MirOp::SelectBranchless,
        MirType::Q16_16,
        [fired, resting, next_v],
        ValueLattice::bounded(0, 65536),
        5,
    );
    r.ret_node = Some(out_v);

    if let Err(v) = r.verify_branchless() {
        eprintln!(
            "error[{}]: {} (MIR node #{}, line {})",
            v.code, v.message, v.node.0, v.line
        );
        process::exit(1);
    }

    let vectors: &[(&[i64], i64)] = &[
        (&[32768, 49152, 2, 65536, 0], 0),
        (&[32768, 16384, 2, 65536, 0], 40960),
    ];

    for (inputs, expected) in vectors {
        let actual = r.eval_in_memory(inputs);
        assert_eq!(actual, *expected, "MIR in-memory execution mismatch");
    }

    println!(
        "Demo MIR region `{}` (hand-written, not lowered from examples/): {} test vectors passed.",
        r.name,
        vectors.len()
    );
    if run_mutate {
        let mut_rep = run_in_memory_mutation_gate(&r, vectors);
        println!(
            "Mutation gate on the demo region: {}/{} mutants killed ({:.1}%) in {} µs",
            mut_rep.killed_mutants,
            mut_rep.total_mutants,
            mut_rep.kill_score_pct,
            t0.elapsed().as_micros()
        );
        // The gate on lowered code, run from the repository's root; `--reported` adds the
        // programs whose calls are fixed, whose survivors are reported.
        let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let reported = args.iter().any(|a| a == "--reported");
        if !mir_mutants::command(&root, reported) {
            process::exit(1);
        }
    }
    println!("Note: @example bodies are not executed yet.");
}

fn extract_attr(directive: &str, key: &str) -> Option<String> {
    let pattern = format!("{}=\"", key);
    let start = directive.find(&pattern)? + pattern.len();
    let rest = &directive[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Checks every `spec/*.md`: YAML front matter, at least one `@assert-*` directive, and each
/// directive both holds and could fail.
///
/// Returns `(spec_files, directives)`. This is a substring check, not a proof: seeing each rule
/// fail against a mutated tree is `spec-guard prove`'s job.
pub fn verify_spec_directory(workspace_root: &Path) -> Result<(usize, usize), Vec<String>> {
    let spec_dir = workspace_root.join("spec");
    let mut errors = Vec::new();
    let mut spec_files = Vec::new();

    if let Ok(entries) = fs::read_dir(&spec_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("md") {
                spec_files.push(p);
            }
        }
    }
    spec_files.sort();

    if spec_files.is_empty() {
        errors.push(format!(
            "No spec/*.md files found in {}",
            spec_dir.display()
        ));
        return Err(errors);
    }

    let mut directives_passed = 0usize;

    for file in &spec_files {
        let content = fs::read_to_string(file).unwrap_or_default();
        let fname = file.file_name().unwrap_or_default().to_string_lossy();
        let mut directives_in_file = 0usize;

        // 1. Check YAML front-matter
        if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
            errors.push(format!(
                "[graph] spec/{} is missing YAML front-matter",
                fname
            ));
        }

        // 2. Evaluate <!-- @assert-* --> directives
        for (line_idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if !trimmed.starts_with("<!-- @assert-") {
                continue;
            }
            directives_in_file += 1;
            let line_no = line_idx + 1;
            let at = format!("spec/{}:{}", fname, line_no);
            if trimmed.starts_with("<!-- @assert-present ") {
                let Some(files_csv) = extract_attr(trimmed, "file") else {
                    errors.push(format!(
                        "[guard] {} @assert-present has no `file` attribute",
                        at
                    ));
                    continue;
                };
                let missing: Vec<&str> = files_csv
                    .split(',')
                    .map(str::trim)
                    .filter(|rel| !workspace_root.join(rel).exists())
                    .collect();
                if missing.is_empty() {
                    directives_passed += 1;
                } else {
                    errors.push(format!(
                        "[guard] {} @assert-present failed: `{}` does not exist",
                        at,
                        missing.join("`, `")
                    ));
                }
            } else if trimmed.starts_with("<!-- @assert-count ") {
                let target_rel = extract_attr(trimmed, "target").unwrap_or_default();
                let symbol = extract_attr(trimmed, "symbol").unwrap_or_default();
                let min_count: usize = extract_attr(trimmed, "min")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                if symbol.is_empty() || min_count == 0 {
                    errors.push(format!(
                        "[guard] {} @assert-count cannot fail: it needs a non-empty `symbol` and `min` of at least 1",
                        at
                    ));
                    continue;
                }
                match fs::read_to_string(workspace_root.join(&target_rel)) {
                    Ok(target_src) => {
                        let target_src = if target_rel.ends_with(".mn") {
                            strip_mn_comments(&target_src)
                        } else {
                            target_src
                        };
                        let count = target_src.matches(&symbol).count();
                        if count < min_count {
                            errors.push(format!(
                                "[guard] {} @assert-count failed: `{}` in `{}` matched {} times (expected >= {})",
                                at, symbol, target_rel, count, min_count
                            ));
                        } else {
                            directives_passed += 1;
                        }
                    }
                    Err(_) => errors.push(format!(
                        "[guard] {} @assert-count target `{}` not readable",
                        at, target_rel
                    )),
                }
            } else if trimmed.starts_with("<!-- @assert-absence ") {
                let target_rel = extract_attr(trimmed, "target").unwrap_or_default();
                let symbol = extract_attr(trimmed, "symbol").unwrap_or_default();
                if symbol.is_empty() {
                    errors.push(format!(
                        "[guard] {} @assert-absence has an empty `symbol`",
                        at
                    ));
                    continue;
                }
                // Only `.mn` files are scanned, whatever `glob` says; an empty target would pass vacuously.
                let mn_files = collect_mn_files(&workspace_root.join(&target_rel));
                if mn_files.is_empty() {
                    errors.push(format!(
                        "[guard] {} @assert-absence target `{}` holds no `.mn` file to scan",
                        at, target_rel
                    ));
                    continue;
                }
                let found_in = mn_files.iter().find(|mn| {
                    fs::read_to_string(mn)
                        .map(|src| strip_mn_comments(&src).contains(&symbol))
                        .unwrap_or(false)
                });
                match found_in {
                    Some(bad_file) => errors.push(format!(
                        "[guard] {} @assert-absence failed: forbidden symbol `{}` found in `{}`",
                        at,
                        symbol,
                        bad_file.display()
                    )),
                    None => directives_passed += 1,
                }
            } else {
                errors.push(format!("[guard] {} unknown directive `{}`", at, trimmed));
            }
        }

        if directives_in_file == 0 {
            errors.push(format!(
                "[guard] spec/{} carries no @assert-* directive",
                fname
            ));
        }
    }

    if errors.is_empty() {
        Ok((spec_files.len(), directives_passed))
    } else {
        Err(errors)
    }
}

/// Removes `//` comments (`///` doc comments included) from `.mn` source, so a directive
/// about a `.mn` file is met by code, never by a comment that mentions the code.
fn strip_mn_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.split_inclusive('\n') {
        let bytes = line.as_bytes();
        let mut in_string = false;
        let mut cut = line.len();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' if in_string => i += 1,
                b'"' => in_string = !in_string,
                b'/' if !in_string && bytes.get(i + 1) == Some(&b'/') => {
                    cut = i;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        out.push_str(&line[..cut]);
        if cut < line.len() && line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn cmd_spec(_args: &[String]) {
    let t0 = Instant::now();
    let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match verify_spec_directory(&root) {
        Ok((files_count, directives)) => {
            println!(
                "[PASS] mn spec: {} spec files, {} @assert-* directives hold ({:.2} ms). Counterfactual proof is spec-guard prove's job.",
                files_count,
                directives,
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
        Err(errs) => {
            for e in errs {
                eprintln!("[FAIL] {}", e);
            }
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mente_sema::verify_module;

    /// Every `mente` block of the language card passes `mn check`, so the card cannot drift
    /// from the compiler (`docs/design-review.md`, `language-card`).
    #[test]
    fn test_language_card_snippets_pass_mn_check() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let card = fs::read_to_string(root.join("docs/language-card.md")).unwrap();
        let mut snippets = 0;
        let mut rest = card.as_str();
        while let Some(start) = rest.find("```mente\n") {
            let body = &rest[start + "```mente\n".len()..];
            let end = body.find("```").unwrap_or(body.len());
            let snippet = &body[..end];
            assert!(
                mente_sema::check_source(snippet).is_ok(),
                "a snippet of the language card fails mn check:\n{}\n{:?}",
                snippet,
                mente_sema::check_source(snippet).err()
            );
            snippets += 1;
            rest = &body[end..];
        }
        assert!(snippets > 0, "the language card has no mente block");
    }

    /// The card stays within its word budget, front matter aside, and names the language
    /// version it describes, which is this compiler's.
    #[test]
    fn test_the_language_card_keeps_within_its_word_budget_and_names_its_version() {
        let (front, body) = CARD
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .expect("the card starts with front matter");
        let words = body.split_whitespace().count();
        assert!(
            words <= CARD_WORD_BUDGET,
            "the language card has {} words, over its budget of {}",
            words,
            CARD_WORD_BUDGET
        );
        let version = front
            .lines()
            .find_map(|l| l.strip_prefix("version: "))
            .expect("the card names its language version");
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        assert!(body.contains(&format!("Mente {}", version)));
    }

    /// The control language's document, which the benchmark gives the agent that writes Odin,
    /// is the size of the card within 15%, front matters aside (`docs/design-review.md`,
    /// `control-language`).
    #[test]
    fn test_the_control_document_is_the_size_of_the_language_card() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let words = |text: &str| {
            text.split_once("\n---\n")
                .map_or(text, |(_, body)| body)
                .split_whitespace()
                .count()
        };
        let card = words(CARD);
        let odin = words(&fs::read_to_string(root.join("benchmark/control/odin.md")).unwrap());
        assert!(
            odin * 100 >= card * 85 && odin * 100 <= card * 115,
            "the Odin document has {} words and the card {}: more than 15% apart",
            odin,
            card
        );
    }

    #[test]
    fn test_examples_pass_their_declaration_checks_and_report_what_they_expect() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let files = collect_mn_files(&root);
        assert_eq!(files.len(), 5, "Expected 5 .mn showcase files in examples/");

        for file in files {
            let src = fs::read_to_string(&file).expect("read .mn file");
            let module =
                parse_module(&src).unwrap_or_else(|e| panic!("Failed to parse {:?}: {}", file, e));
            let spec = project_spec_blueprint(&module);
            assert!(!spec.hashes.spec_hash.is_empty());
            assert!(spec.spec_tokens < spec.total_tokens);

            let rep = verify_module(&module).unwrap_or_else(|errs| {
                panic!("Verification failed for {:?}: {:?}", file, errs);
            });
            assert!(rep.examples_count >= 1);

            // Their bodies use constructs of later milestones: each reports exactly what its
            // `.expected` file lists, and only constructs M1 lacks.
            let actual = diagnostic_lines(&src);
            if std::env::var_os("MENTE_BLESS").is_some() {
                let mut text = actual.join("\n");
                text.push('\n');
                fs::write(file.with_extension("expected"), text).unwrap();
            }
            assert_eq!(
                actual,
                expected_lines(&file),
                "{:?} reports other diagnostics; run with MENTE_BLESS=1 if the change is intended",
                file
            );
            assert!(
                actual
                    .iter()
                    .all(|l| l.ends_with(" E0109") || l.ends_with(" E0116")),
                "{:?}: {:?}",
                file,
                actual
            );
        }
    }

    #[test]
    fn test_all_spec_assertions_and_graph_pass() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (files_count, directives) = verify_spec_directory(&root).unwrap_or_else(|errs| {
            panic!("Spec verification failed:\n{}", errs.join("\n"));
        });
        // Every spec file carries at least one directive, or verify_spec_directory fails.
        assert!(directives >= files_count);
    }

    #[test]
    fn test_watch_checks_first_and_then_only_on_a_change() {
        let session = mente_sema::incremental::Session::new(
            "module m
",
        );
        assert!(needs_check(
            None,
            "module m
"
        ));
        assert!(!needs_check(
            Some(&session),
            "module m
"
        ));
        assert!(needs_check(
            Some(&session),
            "module n
"
        ));
    }

    /// How far a median may rise above the base branch's, measured on the same runner in the same
    /// job, before the benchmark fails. Shared runners differ by up to a factor of two from one
    /// run to the next, so only a comparison on one machine means anything.
    const PROJECTION_TOLERANCE: f64 = 1.3;

    /// Per-module medians, as `<module> <µs>` lines: the base branch's, which CI measures first.
    fn projection_baselines(text: &str) -> Vec<(String, f64)> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| {
                let mut parts = l.split_whitespace();
                let name = parts.next()?.to_string();
                let micros = parts.next()?.parse().ok()?;
                Some((name, micros))
            })
            .collect()
    }

    /// The `mn view --spec` latency protocol in ROADMAP.md. CI measures the base branch and then
    /// this change on one runner; with `MENTE_BENCH_BASELINE` naming the base's medians and
    /// `MENTE_BENCH_ENFORCE=1`, a median above the base's by more than `PROJECTION_TOLERANCE`
    /// fails it:
    /// `cargo test --release -p mente-cli -- --ignored --nocapture view_projection_protocol`.
    #[test]
    #[ignore = "benchmark; run on demand"]
    fn view_projection_protocol() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let baselines = projection_baselines(
            &env::var_os("MENTE_BENCH_BASELINE")
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default(),
        );
        let enforce = env::var_os("MENTE_BENCH_ENFORCE").is_some();
        let mut table = String::from("| Module | Median (µs) | 99th percentile (µs) | Base branch (µs) |\n| :--- | ---: | ---: | ---: |\n");
        let mut regressions = Vec::new();
        for file in collect_mn_files(&manifest.join("../../examples")) {
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            let src = fs::read_to_string(&file).unwrap();
            let run = || {
                let t0 = Instant::now();
                let module = parse_module(&src).unwrap();
                std::hint::black_box(project_spec_blueprint(&module));
                t0.elapsed().as_nanos()
            };
            for _ in 0..100 {
                run();
            }
            let mut samples: Vec<u128> = (0..1000).map(|_| run()).collect();
            samples.sort_unstable();
            let median = samples[500] as f64 / 1000.0;
            let p99 = samples[990] as f64 / 1000.0;
            let baseline = baselines.iter().find(|(n, _)| *n == name).map(|(_, b)| *b);
            println!("{}: median {:.1} µs, p99 {:.1} µs", name, median, p99);
            table.push_str(&format!(
                "| {} | {:.1} | {:.1} | {} |\n",
                name,
                median,
                p99,
                baseline.map_or_else(|| String::from("not measured"), |b| format!("{:.1}", b))
            ));
            if let Some(b) = baseline {
                if median > b * PROJECTION_TOLERANCE {
                    regressions.push(format!(
                        "{}: median {:.1} µs, above {:.1} µs (the base branch's {:.1} µs × {})",
                        name,
                        median,
                        b * PROJECTION_TOLERANCE,
                        b,
                        PROJECTION_TOLERANCE
                    ));
                }
            }
        }
        if let Some(summary) = env::var_os("GITHUB_STEP_SUMMARY") {
            let mut text = fs::read_to_string(&summary).unwrap_or_default();
            text.push_str(&format!("## Projection benchmark\n\n{}\n", table));
            fs::write(&summary, text).unwrap();
        }
        assert!(
            !enforce || regressions.is_empty(),
            "projection regressed:\n{}",
            regressions.join("\n")
        );
    }

    #[test]
    fn test_projection_baselines_are_read_line_by_line() {
        let text = "# measured on the base branch\n\n01_a.mn 41.5\n02_b.mn  62\nbroken line\n03_c.mn notanumber\n";
        assert_eq!(
            projection_baselines(text),
            vec![
                (String::from("01_a.mn"), 41.5),
                (String::from("02_b.mn"), 62.0)
            ]
        );
    }

    #[test]
    fn test_spec_directives_on_mn_files_ignore_comments() {
        let root = env::temp_dir().join(format!("mente-spec-comments-{}", process::id()));
        fs::create_dir_all(root.join("spec")).unwrap();
        fs::create_dir_all(root.join("examples")).unwrap();
        fs::write(
            root.join("examples/demo.mn"),
            "module demo\n// pub fn only_in_a_comment() uses let mut\npub fn real(url: str) -> str = \"http://x\"\n",
        )
        .unwrap();
        fs::write(
            root.join("spec/demo.md"),
            "---\ntitle: t\n---\n\n<!-- @assert-count target=\"examples/demo.mn\" symbol=\"only_in_a_comment\" -->\n<!-- @assert-count target=\"examples/demo.mn\" symbol=\"http://x\" -->\n<!-- @assert-absence target=\"examples\" symbol=\"let mut\" -->\n",
        )
        .unwrap();
        let errs = verify_spec_directory(&root).unwrap_err();
        fs::remove_dir_all(&root).unwrap();

        // The name in the comment does not count; the string literal's "//" is not a comment;
        // and the forbidden "let mut" in the comment is not a violation.
        assert_eq!(errs.len(), 1, "{:?}", errs);
        assert!(errs[0].contains("only_in_a_comment") && errs[0].contains("matched 0 times"));
    }

    #[test]
    fn test_strip_mn_comments_keeps_code_strings_and_lines() {
        let src = concat!(
            "val a = 1 // note\n",
            "val s = \"a\\\"//b\" // tail\n",
            "/// doc\n",
            "val c = 3\n",
            "x \\// outside a string, a backslash escapes nothing\n",
            "last // no newline",
        );
        assert_eq!(
            strip_mn_comments(src),
            concat!(
                "val a = 1 \n",
                "val s = \"a\\\"//b\" \n",
                "\n",
                "val c = 3\n",
                "x \\\n",
                "last "
            )
        );
    }

    #[test]
    fn test_spec_directives_that_cannot_fail_are_rejected() {
        let root = env::temp_dir().join(format!("mente-spec-vacuous-{}", process::id()));
        fs::create_dir_all(root.join("spec")).unwrap();
        fs::write(
            root.join("spec/vacuous.md"),
            "---\ntitle: t\n---\n\n<!-- @assert-count target=\"spec/vacuous.md\" symbol=\"title\" min=\"0\" -->\n<!-- @assert-absence target=\"nowhere\" symbol=\"let mut\" -->\n",
        )
        .unwrap();
        let errs = verify_spec_directory(&root).unwrap_err();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(errs.len(), 2, "{:?}", errs);
        assert!(errs[0].contains("cannot fail"));
        assert!(errs[1].contains("holds no `.mn` file"));
    }
}

#[cfg(test)]
mod overflow_traps;
