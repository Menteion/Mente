//! The frame the checker computes for each function (`core-calculus.md`, item 25), against the
//! figures the conformance programs state in `// frame: <function> <bytes>` lines and the
//! figures of the review programs of `conformance/core-calculus/stack-budget-programs.txt`,
//! which stand as the model of the item: a disagreement is settled in the spec, the list and
//! the code together.

// A failed setup step here is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Each function's frame in bytes, by name, for a module that checks.
fn frames(src: &str, what: &str) -> BTreeMap<String, u128> {
    let module = mente_sema::check_program_without_proofs(src)
        .unwrap_or_else(|d| panic!("{} does not check: {:?}", what, d));
    module
        .names
        .iter()
        .zip(&module.frames)
        .filter_map(|(n, f)| f.map(|f| (n.clone(), f.bytes)))
        .collect()
}

/// The `name bytes` pairs a list of lines states, after `prefix`.
fn stated(lines: &[&str], prefix: &str) -> Vec<(String, u128)> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix(prefix))
        .flat_map(|rest| {
            let words: Vec<&str> = rest.split_whitespace().collect();
            words
                .chunks(2)
                .map(|p| {
                    let bytes = p[1].replace(',', "").parse().expect("a frame in bytes");
                    (p[0].to_string(), bytes)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn check(src: &str, expected: &[(String, u128)], what: &str, failures: &mut Vec<String>) {
    let got = frames(src, what);
    for (name, bytes) in expected {
        match got.get(name) {
            Some(b) if b == bytes => {}
            Some(b) => failures.push(format!(
                "{}: {} has a frame of {}, stated {}",
                what, name, b, bytes
            )),
            None => failures.push(format!("{}: no function {} with a body", what, name)),
        }
    }
}

#[test]
fn test_conformance_programs_give_the_frames_they_state() {
    let dir = repo().join("conformance/core-calculus");
    let mut failures = Vec::new();
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "mn") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = src.lines().collect();
        let expected = stated(&lines, "// frame: ");
        if expected.is_empty() {
            continue;
        }
        checked += expected.len();
        check(&src, &expected, &path.display().to_string(), &mut failures);
    }
    assert!(checked > 0, "no conformance program states a frame");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `mn view --frames` prints each function with a body, its frame and the parts of it, in the
/// module's order; a function declared without a body has no frame and no line.
#[test]
fn test_the_report_gives_each_function_its_frame_and_its_parts() {
    let src = "module conformance\n\
               fn declared(n: u64) -> u64\n\n\
               fn measured(n: u32) -> u32\n  decreases n\n{\n  if n == 0 {\n    return 0\n  }\n  measured(n - 1)\n}\n\n\
               pub fn mix(a: u8, b: u32) -> u64 = a + b\n";
    let module = mente_sema::check_program_without_proofs(src).unwrap();
    assert_eq!(
        mente_sema::frames::report(&module),
        "fn measured: 100 bytes = 64 for the call + 4 for the parameters + 16 for decreases + 16 held at most\n\
         fn mix: 98 bytes = 64 for the call + 5 for the parameters + 0 for decreases + 29 held at most\n"
    );
}

/// One program of the list: its key, what it waits on (`none` where it runs today), the frames
/// it states, and its source.
struct Program {
    key: String,
    waits: String,
    frames: Vec<(String, u128)>,
    source: String,
}

fn programs() -> Vec<Program> {
    let text =
        fs::read_to_string(repo().join("conformance/core-calculus/stack-budget-programs.txt"))
            .unwrap();
    let mut out = Vec::new();
    for entry in text.split("\n== ").skip(1) {
        let (head, source) = entry.split_once("\n---\n").unwrap_or((entry, ""));
        let lines: Vec<&str> = head.lines().collect();
        let key = lines[0].trim().to_string();
        let field = |name: &str| {
            lines
                .iter()
                .find_map(|l| l.strip_prefix(name))
                .map(|v| v.trim().to_string())
                .unwrap_or_else(|| panic!("{} has no `{}` line", key, name.trim_end()))
        };
        out.push(Program {
            waits: field("waits: "),
            frames: stated(&lines, "frames: "),
            source: source.trim_end_matches("\n==").to_string(),
            key,
        });
    }
    out
}

#[test]
fn test_review_programs_give_the_frames_they_state() {
    let programs = programs();
    assert!(!programs.is_empty(), "the list holds no program");
    let mut failures = Vec::new();
    let mut ran = 0;
    for p in &programs {
        if p.waits != "none" {
            // The pull request that brings what it waits on gives it a source and its frames.
            assert!(
                p.frames.is_empty() && p.source.is_empty(),
                "{} waits on {} and states a frame or a source",
                p.key,
                p.waits
            );
            continue;
        }
        assert!(
            !p.source.is_empty() && !p.frames.is_empty(),
            "{} runs today and states no source or no frame",
            p.key
        );
        ran += 1;
        check(&p.source, &p.frames, &p.key, &mut failures);
    }
    assert!(ran > 0, "no program of the list runs today");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
