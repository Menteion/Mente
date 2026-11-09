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

/// A module of M1 code and a proven layout, which `mn check` passes.
const CLEAN: &str = "module test.clean\n@layout(size: 8, align: 4)\npub struct Pair {\n  @bytes(0..4) pub a: u32\n  @bytes(4..8) pub b: u32\n}\npub fn double(x: i32) -> i32 {\n  val y = x * 2\n  y\n}\n";

#[test]
fn test_mn_check_proves_a_layout_and_rejects_broken_modules() {
    let dir = scratch("clean");
    fs::write(dir.join("clean.mn"), CLEAN).unwrap();
    let ok = mn(&dir, &["check", "."]);
    fs::remove_dir_all(&dir).unwrap();
    let out = text(&ok.stdout);
    assert!(ok.status.success(), "{}{}", out, text(&ok.stderr));
    assert_eq!(out.matches("[PASS]").count(), 1, "{}", out);
    assert!(
        out.contains("Proven @layout `Pair`") && out.contains("spec_hash:"),
        "{}",
        out
    );

    // The examples use constructs of later milestones, which M1 reports.
    let examples = mn(&repo_root(), &["check", "examples"]);
    assert_eq!(examples.status.code(), Some(1));
    assert!(text(&examples.stderr).contains("error[E0109]"));

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
fn test_mn_test_compares_each_module_with_its_expected_file() {
    let dir = scratch("expected");
    fs::write(
        dir.join("later.mn"),
        "module t\npub fn f() -> i32 {\n  val s = \"x\"\n  0\n}\n",
    )
    .unwrap();
    // Without an `.expected` file, a module must report nothing.
    let bare = mn(&dir, &["test", "."]);
    let err = text(&bare.stderr);
    assert_eq!(bare.status.code(), Some(1));
    assert!(
        err.contains("[FAIL]") && err.contains("reported, not expected: 3:11 E0109"),
        "{}",
        err
    );
    assert!(!err.contains("in another order"), "{}", err);
    // With one, it must report exactly what the file lists.
    fs::write(dir.join("later.expected"), "3:11 E0109\n").unwrap();
    let listed = mn(&dir, &["test", "."]);
    let out = text(&listed.stdout);
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    assert!(
        out.contains("reports its 1 expected diagnostics"),
        "{}",
        out
    );
    fs::write(dir.join("later.expected"), "3:12 E0109\n").unwrap();
    let moved = mn(&dir, &["test", "."]);
    let err = text(&moved.stderr);
    assert_eq!(moved.status.code(), Some(1));
    assert!(
        err.contains("reported, not expected: 3:11 E0109")
            && err.contains("expected, not reported: 3:12 E0109")
            && !err.contains("in another order"),
        "{}",
        err
    );
    // The same diagnostics in another order are not what the file lists.
    fs::write(
        dir.join("later.mn"),
        "module t\npub fn f() -> i32 {\n  val s = \"x\"\n  val t = 1.5\n  0\n}\n",
    )
    .unwrap();
    fs::write(dir.join("later.expected"), "4:11 E0109\n3:11 E0109\n").unwrap();
    let swapped = mn(&dir, &["test", "."]);
    fs::remove_dir_all(&dir).unwrap();
    let err = text(&swapped.stderr);
    assert_eq!(swapped.status.code(), Some(1));
    assert!(
        err.contains("the same diagnostics, in another order"),
        "{}",
        err
    );
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

#[test]
fn test_mn_check_json_reports_the_versioned_form() {
    let dir = scratch("json-clean");
    fs::write(dir.join("clean.mn"), CLEAN).unwrap();
    let clean = mn(&dir, &["check", "--json", "clean.mn"]);
    fs::remove_dir_all(&dir).unwrap();
    assert!(clean.status.success(), "{}", text(&clean.stderr));
    assert_eq!(text(&clean.stdout), "{\"version\":1,\"diagnostics\":[]}\n");

    let dir = scratch("json");
    fs::write(
        dir.join("broken.mn"),
        "module test.bad\n@layout(size: 16, align: 8)\npub struct Hole {\n  @bytes(0..8) pub id: u64\n  @bytes(12..16) pub rest: u32\n}\n",
    )
    .unwrap();
    let bad = mn(&dir, &["check", "--json", "broken.mn"]);
    fs::remove_dir_all(&dir).unwrap();
    let out = text(&bad.stdout);
    assert_eq!(bad.status.code(), Some(1));
    // Only the JSON document goes to standard output.
    assert!(
        out.starts_with(
            "{\"version\":1,\"diagnostics\":[{\"code\":\"E0413\",\"severity\":\"error\","
        ),
        "{}",
        out
    );
    assert!(
        out.contains("\"primary\":{\"file\":\"broken.mn\",\"start\":")
            && out.contains("\"line\":5,\"column\":3}"),
        "{}",
        out
    );
    assert!(
        out.contains("\"applicability\":\"suggestion\"")
            && out.contains("\"replacement\":\"@bytes(8..12)\""),
        "{}",
        out
    );
    assert_eq!(out.lines().count(), 1, "{}", out);
}

#[test]
fn test_mn_check_text_shows_code_place_and_fix() {
    let dir = scratch("text");
    fs::write(
        dir.join("open.mn"),
        "module m\n@layout(size: 8, align: 4)\npub struct S {\n  @bytes(0..4) pub a: u32\n  pub b: u32\n}\n",
    )
    .unwrap();
    let run = mn(&dir, &["check", "open.mn"]);
    fs::remove_dir_all(&dir).unwrap();
    let err = text(&run.stderr);
    assert_eq!(run.status.code(), Some(1));
    for part in [
        "error[E0412]: field `S.b` has no `@bytes(start..end)`",
        "  --> open.mn:5:7\n",
        "5 |   pub b: u32\n",
        "  |       ^\n",
        "  fix (machine-applicable): give `b` the bytes 4..8\n",
    ] {
        assert!(err.contains(part), "missing `{}` in:\n{}", part, err);
    }
}

#[test]
fn test_mn_explain_prints_the_catalogue() {
    let one = mn(&repo_root(), &["explain", "E0413"]);
    let out = text(&one.stdout);
    assert!(one.status.success());
    assert!(
        out.starts_with("E0413: A @layout field leaves a gap or overlaps\n"),
        "{}",
        out
    );
    assert!(
        out.contains("Governed by spec/morphic-layouts.md.")
            && out.contains("This fails with E0413:"),
        "{}",
        out
    );

    let reserved = mn(&repo_root(), &["explain", "e0601"]);
    assert!(text(&reserved.stdout).contains("(reserved: specified, not reported yet)"));

    let all = mn(&repo_root(), &["explain"]);
    let list = text(&all.stdout);
    assert!(
        list.contains("E0101  A name was expected\n")
            && list.contains("E0701  A comptime evaluation exceeds its step budget (reserved)\n"),
        "{}",
        list
    );

    let unknown = mn(&repo_root(), &["explain", "E9999"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(text(&unknown.stderr).contains("`E9999` is not a Mente error code"));
}

#[test]
fn test_mn_help_lists_every_command() {
    for args in [&["--help"][..], &[][..]] {
        let run = mn(&repo_root(), args);
        let out = text(&run.stdout);
        assert!(run.status.success());
        for command in [
            "mn view --spec",
            "mn check [--json]",
            "mn test [--mutate]",
            "mn spec",
            "mn explain [code]",
        ] {
            assert!(
                out.contains(command),
                "`{}` missing from:\n{}",
                command,
                out
            );
        }
    }
    let unknown = mn(&repo_root(), &["build"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(text(&unknown.stderr).contains("Unknown command `build`"));
}
