//! `mente-cli` (`mn` / `mente`) — Unified Mente Compiler & Verification CLI.
//!
//! Commands:
//! - `mn view --spec <file.mn>` : Instant `<0.1ms` Holographic Spec Blueprint projection & dual hash report.
//! - `mn check <path>`          : Full syntax, 64B Cache-Line `@layout`, `bitstruct`, `@total` & `@branchless` verification.
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
use mente_syntax::parse_module;

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
    println!("  mn check [file.mn | dir]     Verify .mn syntax, 64B Cache-Line @layout, bitstruct, @total & @branchless contracts");
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
        eprintln!("Parse error in `{}`: {}", path_str, e);
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
    let target_str = args.first().map(|s| s.as_str()).unwrap_or("examples");
    let files = collect_mn_files(Path::new(target_str));
    if files.is_empty() {
        eprintln!("No `.mn` files found at `{}`", target_str);
        process::exit(1);
    }

    let t0 = Instant::now();
    let mut all_ok = true;
    for file in &files {
        let src = match fs::read_to_string(file) {
            Ok(src) => src,
            Err(e) => {
                eprintln!("[FAIL] [{}] cannot be read: {}", file.display(), e);
                all_ok = false;
                continue;
            }
        };
        let module = match parse_module(&src) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[FAIL] [{}] Parse Error: {}", file.display(), e);
                all_ok = false;
                continue;
            }
        };
        let proj = project_spec_blueprint(&module);
        match verify_module(&module) {
            Ok(rep) => {
                println!(
                    "[PASS] [{}] module `{}` | spec_hash: {}.. | layouts: {} | bitstructs: {} | @branchless: {} | @total: {} | @kernel: {}",
                    file.display(),
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
            Err(errs) => {
                all_ok = false;
                for e in errs {
                    eprintln!("[FAIL] [{}] {}", file.display(), e);
                }
            }
        }
    }
    println!(
        "\nChecked {} `.mn` file(s) in {:.2} ms.",
        files.len(),
        t0.elapsed().as_secs_f64() * 1000.0
    );
    if !all_ok {
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

    if let Err(e) = r.verify_branchless() {
        eprintln!("[FAIL] {}", e);
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

    /// The `mn view --spec` latency protocol in ROADMAP.md. Run it with
    /// `cargo test --release -p mente-cli -- --ignored --nocapture view_projection_protocol`.
    #[test]
    #[ignore = "benchmark; run on demand"]
    fn view_projection_protocol() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        for file in collect_mn_files(&root) {
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
            println!(
                "{}: median {:.1} µs, p99 {:.1} µs",
                file.file_name().unwrap().to_string_lossy(),
                samples[500] as f64 / 1000.0,
                samples[990] as f64 / 1000.0
            );
        }
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
