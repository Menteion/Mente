//! `mente-cli` (`mn` / `mente`) — Unified Mente Compiler & Verification CLI.
//!
//! Commands:
//! - `mn view --spec <file.mn>` : Instant `<0.1ms` Holographic Spec Blueprint projection & dual hash report.
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
use mente_sema::verify_module;
use mente_syntax::diagnostics::{render_json, render_text, Diagnostic, FileDiagnostics};
use mente_syntax::parse_module;

mod catalogue;

fn main() {
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
    println!("  mn check [--json] [file.mn | dir]");
    println!("                               Verify .mn syntax, 64B Cache-Line @layout, bitstruct, @total & @branchless contracts");
    println!("  mn explain [code]            Explain an error code, or list them all");
    println!("  mn test [--mutate] [path]    Run mn check, then the hand-written demo MIR region (and its mutation gate)");
    println!("  mn spec                      Check spec/*.md front matter and run their @assert-* directives");
}

fn cmd_view(args: &[String]) {
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
    let mut checked: Vec<(String, String, Vec<Diagnostic>)> = Vec::new();
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
            Ok(module) => match verify_module(&module) {
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
        checked.push((shown, src, diagnostics));
    }
    if json {
        let files: Vec<FileDiagnostics> = checked
            .iter()
            .map(|(file, src, diagnostics)| FileDiagnostics {
                file,
                src,
                diagnostics,
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

fn cmd_test(args: &[String]) {
    let run_mutate = args.iter().any(|a| a == "--mutate");
    let target_str = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .unwrap_or("examples");

    cmd_check(&[target_str.to_string()]);

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

    #[test]
    fn test_all_four_example_mn_files_pass_verification() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let files = collect_mn_files(&root);
        assert_eq!(files.len(), 4, "Expected 4 .mn showcase files in examples/");

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

    /// The dependency policy of CLAUDE.md, applied to one manifest: a compiler crate depends
    /// only on the workspace's own crates, by path; `mente-mir` depends on nothing; only
    /// `mente-codegen` may add Cranelift. Returns each dependency line outside the policy.
    fn dependency_policy_violations(krate: &str, manifest: &str) -> Vec<String> {
        let mut section = String::new();
        let mut found = Vec::new();
        for line in manifest.lines().map(str::trim) {
            if line.starts_with('[') {
                section = line.trim_matches(|c| c == '[' || c == ']').to_string();
                continue;
            }
            if !section.ends_with("dependencies") || line.is_empty() || line.starts_with('#') {
                continue;
            }
            let name = line.split(['=', '.']).next().unwrap_or("").trim();
            let allowed = if name.starts_with("mente-") {
                krate != "mente-mir" && line.contains("path =")
            } else {
                name.starts_with("cranelift") && krate == "mente-codegen"
            };
            if !allowed {
                found.push(format!("{}: [{}] {}", krate, section, line));
            }
        }
        found
    }

    /// A new external dependency needs Norman's approval and a change to this policy.
    #[test]
    fn test_crates_depend_only_on_allowed_crates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut found = dependency_policy_violations(
            "workspace",
            &fs::read_to_string(root.join("Cargo.toml")).unwrap(),
        );
        let mut crates = 0;
        for entry in fs::read_dir(root.join("compiler")).unwrap().flatten() {
            if let Ok(manifest) = fs::read_to_string(entry.path().join("Cargo.toml")) {
                crates += 1;
                found.extend(dependency_policy_violations(
                    &entry.file_name().to_string_lossy(),
                    &manifest,
                ));
            }
        }
        assert!(
            crates >= 6,
            "expected the six compiler crates, read {}",
            crates
        );
        assert!(
            found.is_empty(),
            "dependencies outside the policy:\n{}",
            found.join("\n")
        );

        // The policy itself: what it lets through and what it stops.
        let manifest = "[package]\nname = \"x\"\n\n[dependencies]\nmente-ast = { path = \"../mente-ast\" }\ncranelift-codegen = \"0.135\"\n\n[dev-dependencies]\nproptest = \"1\"\n";
        assert_eq!(
            dependency_policy_violations("mente-sema", manifest),
            vec![
                "mente-sema: [dependencies] cranelift-codegen = \"0.135\"".to_string(),
                "mente-sema: [dev-dependencies] proptest = \"1\"".to_string(),
            ]
        );
        assert_eq!(
            dependency_policy_violations("mente-codegen", manifest).len(),
            1
        );
        assert_eq!(dependency_policy_violations("mente-mir", manifest).len(), 3);
        assert_eq!(
            dependency_policy_violations("mente-cli", "[dependencies]\nmente-ast = \"0.1\"\n")
                .len(),
            1
        );
    }

    /// Every text file in the repository is English: no CJK characters in code, comments, docs or
    /// examples (CLAUDE.md). Build output and dependencies are skipped.
    #[test]
    fn test_repository_text_is_english() {
        fn is_cjk(c: char) -> bool {
            matches!(c as u32,
                0x3000..=0x30FF     // CJK punctuation, Hiragana, Katakana
                | 0x3400..=0x9FFF   // CJK ideographs
                | 0xAC00..=0xD7AF   // Hangul
                | 0xF900..=0xFAFF   // CJK compatibility ideographs
                | 0xFF00..=0xFFEF) // full-width forms
        }
        fn walk(dir: &Path, found: &mut Vec<String>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_dir() {
                    if !matches!(name.as_str(), ".git" | "target" | "node_modules") {
                        walk(&path, found);
                    }
                } else if let Ok(text) = fs::read_to_string(&path) {
                    for (i, line) in text.lines().enumerate() {
                        if line.chars().any(is_cjk) {
                            found.push(format!("{}:{}", path.display(), i + 1));
                        }
                    }
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut found = Vec::new();
        walk(&root, &mut found);
        assert!(
            found.is_empty(),
            "non-English text at:\n{}",
            found.join("\n")
        );
    }

    /// How far a median may rise above its baseline before the benchmark fails. Shared CI
    /// runners vary from run to run, so the tolerance is wide; a real regression is larger.
    const PROJECTION_TOLERANCE: f64 = 1.5;

    /// The per-module median baselines in `tests/projection-baseline.txt`: `<module> <µs>` lines.
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

    /// The `mn view --spec` latency protocol in ROADMAP.md. CI runs it in release on its
    /// x86-64 Linux runner with `MENTE_BENCH_ENFORCE=1`, which fails it when a median rises above
    /// its baseline by more than `PROJECTION_TOLERANCE`:
    /// `cargo test --release -p mente-cli -- --ignored --nocapture view_projection_protocol`.
    #[test]
    #[ignore = "benchmark; run on demand"]
    fn view_projection_protocol() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let baselines = projection_baselines(
            &fs::read_to_string(manifest.join("tests/projection-baseline.txt")).unwrap_or_default(),
        );
        let enforce = env::var_os("MENTE_BENCH_ENFORCE").is_some();
        let mut table = String::from("| Module | Median (µs) | 99th percentile (µs) | Baseline (µs) |\n| :--- | ---: | ---: | ---: |\n");
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
                baseline.map_or_else(|| String::from("none yet"), |b| format!("{:.1}", b))
            ));
            if let Some(b) = baseline {
                if median > b * PROJECTION_TOLERANCE {
                    regressions.push(format!(
                        "{}: median {:.1} µs, above {:.1} µs (baseline {:.1} µs × {})",
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
        let text = "# machine: GitHub ubuntu-24.04\n\n01_a.mn 41.5\n02_b.mn  62\nbroken line\n03_c.mn notanumber\n";
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
