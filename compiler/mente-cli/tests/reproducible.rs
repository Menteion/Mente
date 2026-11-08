//! Compiling twice gives identical output, on every platform (`docs/design-brief.md`, "The
//! quality system"; foundation gate condition 8).
//!
//! Each example's projection (`mn view --spec`) and each conformance program's diagnostics
//! (`mn check --json`) are produced twice and must match byte for byte, and then must match the
//! golden files in `tests/golden/`, which the CI matrix checks on x86-64 Linux, x86-64 Windows
//! and AArch64 Linux alike. When an output changes on purpose, run with `MENTE_BLESS=1` to
//! rewrite the golden files, and review their diff.

// A failed setup step here is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

/// Runs `mn` in `dir` and returns its standard output.
fn mn(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_mn"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run mn");
    String::from_utf8(out.stdout).expect("mn prints UTF-8")
}

/// The same output twice, or a failure naming what differed.
fn twice(what: &str, run: impl Fn() -> String) -> String {
    let first = run();
    let second = run();
    assert_eq!(first, second, "{} differs between two runs", what);
    first
}

/// Compares `actual` with a golden file, or rewrites it under `MENTE_BLESS=1`.
fn check_golden(name: &str, actual: &str) {
    let path = golden(name);
    if std::env::var_os("MENTE_BLESS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_default();
    if expected != actual {
        let line = expected
            .lines()
            .zip(actual.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
        panic!(
            "{} differs from tests/golden/{} at line {}:\n  golden: {:?}\n  now:    {:?}\nRun with MENTE_BLESS=1 if the change is intended.",
            name,
            name,
            line + 1,
            expected.lines().nth(line).unwrap_or(""),
            actual.lines().nth(line).unwrap_or("")
        );
    }
}

fn sorted_files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == extension))
        .collect();
    files.sort();
    files
}

#[test]
fn test_projections_are_reproducible() {
    let root = repo_root();
    let mut all = String::new();
    for file in sorted_files(&root.join("examples"), "mn") {
        let rel = format!("examples/{}", file.file_name().unwrap().to_string_lossy());
        let out = twice(&rel, || {
            // The last two lines report the time the projection took; everything else is output.
            let full = mn(&root, &["view", "--spec", &rel]);
            let lines: Vec<&str> = full.lines().collect();
            lines[..lines.len().saturating_sub(2)].join("\n")
        });
        assert!(out.contains("// spec_hash: "), "{}: {}", rel, out);
        all.push_str(&format!("== {} ==\n{}\n", rel, out));
    }
    check_golden("projections.txt", &all);
}

#[test]
fn test_diagnostics_are_reproducible() {
    let root = repo_root().join("conformance");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut all = String::new();
    for dir in dirs {
        for file in sorted_files(&dir, "mn") {
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            let rel = format!("{}/{}", dir.file_name().unwrap().to_string_lossy(), name);
            // Run from the program's own directory, so the file named in the JSON has no
            // platform-specific separators.
            let out = twice(&rel, || mn(&dir, &["check", "--json", &name]));
            all.push_str(&format!("== {} ==\n{}", rel, out));
        }
    }
    check_golden("diagnostics.txt", &all);
}
