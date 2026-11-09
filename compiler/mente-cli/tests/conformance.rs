//! The conformance runner (`conformance/README.md`).
//!
//! Every program under `conformance/<spec>/` states in its first line what `mn check --json` must
//! report for it: `// expect: pass`, or the codes of every diagnostic in order, such as
//! `// expect: E0413` or `// expect: E0402 E0109`. The runner reads nothing from the compiler but that command's output, so it runs
//! unchanged against another implementation: set `MENTE_COMPILER` to its path.

// A failed setup step here is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compiler() -> PathBuf {
    std::env::var_os("MENTE_COMPILER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mn")))
}

/// Every conformance program, as (spec directory, file).
fn programs() -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for dir in fs::read_dir(repo_root().join("conformance"))
        .unwrap()
        .flatten()
    {
        if !dir.path().is_dir() {
            continue;
        }
        let spec = dir.file_name().to_string_lossy().into_owned();
        for file in fs::read_dir(dir.path()).unwrap().flatten() {
            if file.path().extension().is_some_and(|e| e == "mn") {
                found.push((spec.clone(), file.path()));
            }
        }
    }
    found.sort();
    found
}

/// The codes, in order, of a JSON document `mn check --json` printed.
fn reported_codes(json: &str) -> Vec<String> {
    json.match_indices("\"code\":\"")
        .map(|(at, key)| {
            let rest = &json[at + key.len()..];
            rest[..rest.find('"').unwrap_or(0)].to_string()
        })
        .collect()
}

#[test]
fn test_conformance_suite() {
    let programs = programs();
    assert!(!programs.is_empty(), "no conformance programs found");
    let mut failures = Vec::new();
    for (spec, file) in &programs {
        let name = format!("{}/{}", spec, file.file_name().unwrap().to_string_lossy());
        if !repo_root()
            .join("spec")
            .join(format!("{}.md", spec))
            .is_file()
        {
            failures.push(format!(
                "{}: no spec/{}.md governs this directory",
                name, spec
            ));
            continue;
        }
        let src = fs::read_to_string(file).unwrap();
        let Some(expect) = src
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("// expect: "))
        else {
            failures.push(format!("{}: the first line is not `// expect: ...`", name));
            continue;
        };
        let expected: Vec<String> = match expect.trim() {
            "pass" => Vec::new(),
            codes => codes.split_whitespace().map(String::from).collect(),
        };
        let out = Command::new(compiler())
            .args(["check", "--json"])
            .arg(file)
            .output()
            .expect("run the compiler");
        let json = String::from_utf8_lossy(&out.stdout);
        let reported = reported_codes(&json);
        let status_fits = out.status.success() == expected.is_empty();
        if reported != expected || !status_fits || !json.starts_with("{\"version\":1,") {
            failures.push(format!(
                "{}: expected {:?}, reported {:?} (exit {:?})",
                name,
                expected,
                reported,
                out.status.code()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} conformance programs failed:\n{}",
        failures.len(),
        programs.len(),
        failures.join("\n")
    );
}

#[test]
fn test_reported_codes_reads_the_json_form() {
    assert_eq!(
        reported_codes("{\"version\":1,\"diagnostics\":[]}"),
        Vec::<String>::new()
    );
    assert_eq!(
        reported_codes("{\"version\":1,\"diagnostics\":[{\"code\":\"E0413\",\"message\":\"a \\\"code\\\":\"},{\"code\":\"E0501\"}]}"),
        vec!["E0413", "E0501"]
    );
}
