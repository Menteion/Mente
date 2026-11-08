//! End-to-end tests of the `mn` commands that `spec/comptime-and-ffi.md` lists as implemented.
//! Each runs the built binary, as a user would.

// These helpers are test code: a failed setup step is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn mn(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mn"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run mn")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A fresh scratch directory for one test, removed by the caller.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mente-cli-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_mn_check_passes_examples_and_rejects_a_broken_layout() {
    let ok = mn(&repo_root(), &["check", "examples"]);
    let out = text(&ok.stdout);
    assert!(ok.status.success(), "{}{}", out, text(&ok.stderr));
    assert_eq!(out.matches("[PASS]").count(), 4, "{}", out);
    assert!(
        out.contains("Proven @layout") && out.contains("spec_hash:"),
        "{}",
        out
    );

    let dir = scratch("check");
    fs::write(
        dir.join("broken.mn"),
        "module test.bad\n@layout(size: 16, align: 8)\npub struct Hole {\n  @bytes(0..8) pub id: u64\n  @bytes(12..16) pub rest: u32\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("total.mn"),
        "module test.total\n@total\npub fn spin(n: u32) -> u32 {\n  while n > 0 {\n  }\n  n\n}\n",
    )
    .unwrap();
    let bad = mn(&dir, &["check", "."]);
    fs::remove_dir_all(&dir).unwrap();
    let err = text(&bad.stderr);
    assert_eq!(bad.status.code(), Some(1));
    assert!(
        err.contains("E0413") && err.contains("broken.mn"),
        "{}",
        err
    );
    assert!(err.contains("E0501") && err.contains("total.mn"), "{}", err);
}

#[test]
fn test_mn_view_spec_prints_the_blueprint() {
    let dir = scratch("view");
    fs::write(
        dir.join("demo.mn"),
        "module demo.view\npub fn score(x: i64) -> i64\n  requires x >= 0\n{\n  val hidden_local = x * 2\n  hidden_local\n}\n",
    )
    .unwrap();
    let run = mn(&dir, &["view", "--spec", "demo.mn"]);
    fs::remove_dir_all(&dir).unwrap();
    let out = text(&run.stdout);
    assert!(run.status.success(), "{}", text(&run.stderr));
    for kept in [
        "// spec_hash: ",
        "// body_hash: ",
        "pub fn score(x: i64) -> i64",
        "  requires x >= 0",
        "= ...",
        "Projection completed in",
    ] {
        assert!(out.contains(kept), "missing `{}` in:\n{}", kept, out);
    }
    assert!(!out.contains("hidden_local"), "{}", out);
}

#[test]
fn test_mn_test_mutate_runs_the_demo_gate() {
    let plain = mn(&repo_root(), &["test", "examples"]);
    let out = text(&plain.stdout);
    assert!(plain.status.success(), "{}", text(&plain.stderr));
    assert!(
        out.contains("[PASS]") && out.contains("2 test vectors passed"),
        "{}",
        out
    );
    assert!(!out.contains("Mutation gate"), "{}", out);

    let mutate = mn(&repo_root(), &["test", "--mutate", "examples"]);
    let out = text(&mutate.stdout);
    assert!(mutate.status.success(), "{}", text(&mutate.stderr));
    assert!(out.contains("4/4 mutants killed (100.0%)"), "{}", out);
}

#[test]
fn test_mn_spec_checks_front_matter_and_directives() {
    let dir = scratch("spec");
    fs::create_dir_all(dir.join("spec")).unwrap();
    fs::create_dir_all(dir.join("examples")).unwrap();
    fs::write(dir.join("lib.rs"), "fn test_present() {}\n").unwrap();
    fs::write(dir.join("examples/demo.mn"), "module demo\nval x = 1\n").unwrap();
    fs::write(
        dir.join("spec/good.md"),
        concat!(
            "---\ntitle: good\n---\n\n",
            "<!-- @assert-count target=\"lib.rs\" symbol=\"fn test_present\" min=\"1\" -->\n",
            "<!-- @assert-present file=\"lib.rs,examples/demo.mn\" -->\n",
            "<!-- @assert-absence target=\"examples\" symbol=\"let mut\" -->\n",
        ),
    )
    .unwrap();
    let ok = mn(&dir, &["spec"]);
    let out = text(&ok.stdout);
    assert!(ok.status.success(), "{}{}", out, text(&ok.stderr));
    assert!(
        out.contains("[PASS] mn spec: 1 spec files, 3 @assert-* directives hold"),
        "{}",
        out
    );

    fs::write(
        dir.join("spec/bare.md"),
        "# no front matter and no directive\n",
    )
    .unwrap();
    fs::write(
        dir.join("spec/missing.md"),
        "---\ntitle: missing\n---\n\n<!-- @assert-count target=\"lib.rs\" symbol=\"fn test_absent\" min=\"1\" -->\n",
    )
    .unwrap();
    let bad = mn(&dir, &["spec"]);
    fs::remove_dir_all(&dir).unwrap();
    let err = text(&bad.stderr);
    assert_eq!(bad.status.code(), Some(1));
    assert!(
        err.contains("spec/bare.md is missing YAML front-matter"),
        "{}",
        err
    );
    assert!(
        err.contains("spec/bare.md carries no @assert-* directive"),
        "{}",
        err
    );
    // Each failure names its spec file and line.
    assert!(
        err.contains("spec/missing.md:5 @assert-count failed") && err.contains("matched 0 times"),
        "{}",
        err
    );
}
