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
    // A `.mn` path that cannot be read as a file fails the run.
    let dir = scratch("unreadable");
    fs::create_dir_all(dir.join("folder.mn")).unwrap();
    let unreadable = mn(&dir, &["test", "."]);
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(unreadable.status.code(), Some(1));
    assert!(text(&unreadable.stderr).contains("cannot read"));

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
    assert_eq!(
        text(&clean.stdout),
        "{\"version\":1,\"diagnostics\":[],\"todos\":[],\"trusted\":[]}\n"
    );

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
            "mn build <file.mn>",
            "mn card",
        ] {
            assert!(
                out.contains(command),
                "`{}` missing from:\n{}",
                command,
                out
            );
        }
    }
    let unknown = mn(&repo_root(), &["deploy"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert!(text(&unknown.stderr).contains("Unknown command `deploy`"));
}

/// `mn run` runs `main`, or the call `--call` names, on the reference interpreter: it prints
/// the result and what each `mut` argument is left holding, or the trap and where (exit 1), or
/// the diagnostics of a module that does not check (exit 2).
#[test]
fn test_mn_run_prints_a_result_or_a_trap() {
    let dir = scratch("run");
    fs::write(
        dir.join("prog.mn"),
        "module t\npub fn bump(mut x: u8) -> bool {\n  x += 1\n  true\n}\npub fn half(x: u8) -> u8 = x / 2\npub fn main() -> u8 = half(9)\n",
    )
    .unwrap();
    fs::write(dir.join("bad.mn"), "module t\npub fn main() -> u8 = zz\n").unwrap();
    let main = mn(&dir, &["run", "prog.mn"]);
    assert_eq!(main.status.code(), Some(0));
    assert_eq!(text(&main.stdout), "4\n");
    let call = mn(&dir, &["run", "prog.mn", "--call", "half(200)"]);
    assert_eq!(text(&call.stdout), "100\n");
    let leaves = mn(&dir, &["run", "prog.mn", "--call", "bump(mut 41)"]);
    assert_eq!(text(&leaves.stdout), "true\nleaves 42\n");
    let trap = mn(&dir, &["run", "prog.mn", "--call", "bump(mut 255)"]);
    assert_eq!(trap.status.code(), Some(1));
    assert_eq!(
        text(&trap.stderr),
        "prog.mn: trap[narrowing] at 3:8: 256 does not fit in `u8`\n"
    );
    let unmarked = mn(&dir, &["run", "prog.mn", "--call", "bump(41)"]);
    assert_eq!(unmarked.status.code(), Some(2));
    assert!(text(&unmarked.stderr)
        .contains("parameter `x` of `bump` is `mut`, so its argument is marked `mut`"));
    let marked = mn(&dir, &["run", "prog.mn", "--call", "half(mut 4)"]);
    assert_eq!(marked.status.code(), Some(2));
    assert!(text(&marked.stderr).contains("is read-only, so its argument is not marked `mut`"));
    let garbled = mn(&dir, &["run", "prog.mn", "--call", "half(4"]);
    assert_eq!(garbled.status.code(), Some(2));
    assert!(text(&garbled.stderr).contains("`--call` takes a call"));
    let bad = mn(&dir, &["run", "bad.mn"]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(text(&bad.stderr).contains("E0201"));
    let missing = mn(&dir, &["run", "missing.mn"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(text(&missing.stderr).contains("cannot read"));
    let usage = mn(&dir, &["run"]);
    assert_eq!(usage.status.code(), Some(2));
    assert!(text(&usage.stderr).contains("Usage: mn run"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_mn_run_mir_prints_what_the_interpreter_prints() {
    let dir = scratch("run-mir");
    fs::write(
        dir.join("prog.mn"),
        "module t\npub fn bump(mut x: u8) -> bool {\n  x += 1\n  true\n}\npub fn half(x: u8) -> u8 = x / 2\npub fn main() -> u8 = half(9)\n",
    )
    .unwrap();
    for args in [
        &["run", "prog.mn"][..],
        &["run", "prog.mn", "--call", "bump(mut 41)"],
        &["run", "prog.mn", "--call", "bump(mut 255)"],
    ] {
        let reference = mn(&dir, args);
        let through_mir = mn(&dir, &[args, &["--mir"]].concat());
        assert_eq!(
            through_mir.status.code(),
            reference.status.code(),
            "{:?}",
            args
        );
        assert_eq!(
            text(&through_mir.stdout),
            text(&reference.stdout),
            "{:?}",
            args
        );
        assert_eq!(
            text(&through_mir.stderr),
            text(&reference.stderr),
            "{:?}",
            args
        );
    }
    let main = mn(&dir, &["run", "prog.mn", "--mir"]);
    assert_eq!(text(&main.stdout), "4\n");
    // Native code prints the same, its trap's file name included.
    for args in [
        &["run", "prog.mn"][..],
        &["run", "prog.mn", "--call", "bump(mut 41)"],
        &["run", "prog.mn", "--call", "bump(mut 255)"],
    ] {
        let reference = mn(&dir, args);
        let native = mn(&dir, &[args, &["--native"]].concat());
        assert_eq!(native.status.code(), reference.status.code(), "{:?}", args);
        assert_eq!(
            text(&native.stdout).replace("\r\n", "\n"),
            text(&reference.stdout),
            "{:?}",
            args
        );
        assert_eq!(text(&native.stderr), text(&reference.stderr), "{:?}", args);
    }
    let missing = mn(&dir, &["run", "prog.mn", "--mir", "--call", "nothing()"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(text(&missing.stderr).contains("no function `nothing`"));
    fs::remove_dir_all(&dir).unwrap();
}

/// `mn card` prints the language card the compiler was built with, word for word.
#[test]
fn test_mn_card_prints_the_language_card_it_ships_with() {
    let run = mn(&repo_root(), &["card"]);
    assert!(run.status.success());
    let card = fs::read_to_string(repo_root().join("docs/language-card.md")).unwrap();
    assert_eq!(text(&run.stdout), card);
}

/// `mn build` writes an executable that makes the call and prints what `mn run` prints, its
/// trap's file name included; by default beside the source, without `.mn`.
#[test]
fn test_mn_build_writes_an_executable_that_prints_what_mn_run_prints() {
    let dir = scratch("build");
    fs::write(
        dir.join("prog.mn"),
        "module t
pub fn bump(mut x: u8) -> bool {
  x += 1
  true
}
pub fn half(x: u8) -> u8 = x / 2
pub fn main() -> u8 = half(9)
",
    )
    .unwrap();
    let suffix = std::env::consts::EXE_SUFFIX;
    for (call, exe) in [
        (None, format!("prog{suffix}")),
        (Some("bump(mut 41)"), format!("bumped{suffix}")),
        (Some("bump(mut 255)"), format!("trapped{suffix}")),
    ] {
        let mut build = vec!["build", "prog.mn"];
        let mut run = vec!["run", "prog.mn"];
        if let Some(call) = call {
            build.extend(["--call", call, "-o", &exe]);
            run.extend(["--call", call]);
        }
        let built = mn(&dir, &build);
        assert_eq!(built.status.code(), Some(0), "{}", text(&built.stderr));
        assert!(text(&built.stdout).trim_end().ends_with(&exe));
        let reference = mn(&dir, &run);
        let native = Command::new(dir.join(&exe))
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(native.status.code(), reference.status.code(), "{:?}", call);
        assert_eq!(
            text(&native.stdout).replace("\r\n", "\n"),
            text(&reference.stdout),
            "{:?}",
            call
        );
        assert_eq!(
            text(&native.stderr).replace("\r\n", "\n"),
            text(&reference.stderr),
            "{:?}",
            call
        );
    }
    // A call that names no function is not built.
    let missing = mn(&dir, &["build", "prog.mn", "--call", "nothing()"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(text(&missing.stderr).contains("no function `nothing`"));
    fs::remove_dir_all(&dir).unwrap();
}

/// `mn check --watch` checks a module item by item and reports what it re-parsed and re-checked;
/// `--rounds` stops it.
#[test]
fn test_mn_check_watch_checks_item_by_item() {
    let dir = scratch("watch");
    fs::write(
        dir.join("w.mn"),
        "module t\npub fn f(x: u8) -> u8 = y\npub fn g() -> u8 = f(1)\n",
    )
    .unwrap();
    let out = mn(&dir, &["check", "--watch", "w.mn", "--rounds", "1"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = text(&out.stdout);
    assert!(
        stdout.starts_with(
            "[watch] w.mn: 1 problem(s); re-parsed 3 item(s), re-checked 2 function(s) in "
        ),
        "{}",
        stdout
    );
    assert!(text(&out.stderr).contains("E0201"));
    fs::write(dir.join("s.mn"), "module t\npub struct P {\n  x: u8\n}\n").unwrap();
    let whole = mn(&dir, &["check", "--watch", "s.mn", "--rounds", "1"]);
    assert!(
        text(&whole.stdout).contains("checked the whole module"),
        "{}",
        text(&whole.stdout)
    );
    let usage = mn(&dir, &["check", "--watch"]);
    assert_eq!(usage.status.code(), Some(2));
    let missing = mn(&dir, &["check", "--watch", "gone.mn", "--rounds", "1"]);
    assert_eq!(missing.status.code(), Some(2));
    fs::remove_dir_all(&dir).unwrap();
}

/// `mn dump` prints a phase's output in its canonical form, the same bytes on every run.
#[test]
fn test_mn_dump_prints_canonical_forms() {
    let dir = scratch("dump");
    fs::write(dir.join("d.mn"), "module t\npub fn f(x: u8) -> u8 = x\n").unwrap();
    let tokens = mn(&dir, &["dump", "--tokens", "d.mn"]);
    assert_eq!(tokens.status.code(), Some(0));
    assert!(text(&tokens.stdout).starts_with("mente-dump 1 tokens\n1:1 Module module\n"));
    let tree = mn(&dir, &["dump", "--tree", "d.mn"]);
    assert_eq!(
        text(&tree.stdout),
        "mente-dump 1 tree\n(fn f ((x u8)) u8 () (= x))\n"
    );
    let typed = mn(&dir, &["dump", "--typed", "d.mn"]);
    assert_eq!(
        text(&typed.stdout),
        "mente-dump 1 typed\n(fn f ((x u8)) u8 () (= x))\n  (binding x u8)\n  (expr 0 2:25 1 u8)\n"
    );
    assert_eq!(mn(&dir, &["dump", "--typed", "d.mn"]).stdout, typed.stdout);
    let unknown = mn(&dir, &["dump", "--mir", "d.mn"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(text(&unknown.stderr).contains("knows --tokens, --tree and --typed"));
    assert_eq!(mn(&dir, &["dump", "d.mn"]).status.code(), Some(2));
    let missing = mn(&dir, &["dump", "--tree", "gone.mn"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(text(&missing.stderr).contains("cannot read"));
    fs::remove_dir_all(&dir).unwrap();
}

/// `mn check` lists every `todo` with its reason and every trusted clause with its place, after
/// a file's diagnostics and in `--json`, without changing its verdict.
#[test]
fn test_mn_check_lists_todos_and_trusted_clauses() {
    let dir = scratch("listings");
    fs::write(
        dir.join("open.mn"),
        "module m\npub fn later(x: u8) -> u8 = todo(\"think about x\")\n@prove\npub fn half(x: u8) -> u8\n  requires x % 2 == 0\n  ensures x > 1 or x == 0\n= x / 2\n@prove\npub fn g(n: u8) -> u8 {\n  var i: u8 = 0\n  while i < n invariant i != 300 {\n    i += 1\n  }\n  i\n}\n",
    )
    .unwrap();
    fs::write(
        dir.join("broken.mn"),
        "module m\npub fn f() -> u8 {\n  val y: u8 = zz\n  todo(\"after\")\n}\n",
    )
    .unwrap();
    let run = mn(&dir, &["check", "open.mn"]);
    let out = text(&run.stdout);
    assert!(run.status.success(), "{}", text(&run.stderr));
    let listed: Vec<&str> = out
        .lines()
        .filter(|l| l.starts_with("todo:") || l.starts_with("trusted:"))
        .collect();
    assert_eq!(
        listed,
        [
            "todo: open.mn:2:29: \"think about x\"",
            "trusted: open.mn:5:12: requires x % 2 == 0 of half, assumed until mn prove runs",
            "trusted: open.mn:6:11: ensures x > 1 or x == 0 of half, assumed until mn prove runs",
            "trusted: open.mn:11:25: invariant i != 300 in g, assumed until mn prove runs",
        ]
    );
    let json = text(&mn(&dir, &["check", "--json", "open.mn"]).stdout);
    assert!(
        json.contains(
            "\"todos\":[{\"reason\":\"think about x\",\"span\":{\"file\":\"open.mn\",\"start\":37,"
        ),
        "{}",
        json
    );
    assert!(
        json.contains("\"trusted\":[{\"clause\":\"requires x % 2 == 0 of half\",\"span\":{\"file\":\"open.mn\","),
        "{}",
        json
    );
    // A module with an error still lists its todo, and fails for its error alone.
    let broken = mn(&dir, &["check", "broken.mn"]);
    assert_eq!(broken.status.code(), Some(1));
    assert!(text(&broken.stdout).contains("todo: broken.mn:4:3: \"after\"\n"));
    assert!(text(&broken.stderr).contains("E0201"));
    fs::remove_dir_all(&dir).unwrap();
}

/// `mn view --checks` lists, for each function whose body checks, each run-time check Tier 1
/// leaves, then how many it erased.
#[test]
fn test_mn_view_checks_lists_the_run_time_checks_tier1_leaves() {
    let dir = scratch("checks");
    fs::write(
        dir.join("c.mn"),
        "module m\npub fn sum(a: [16]i32, n: usize[0..=16]) -> i64 {\n  var i: usize = 0\n  var s: i64 = 0\n  while i < n {\n    s += a[i]\n    i += 1\n  }\n  s\n}\npub fn pick(xs: [4]u8, i: usize, d: u8, k: u8) -> u8 = xs[i] / d << k\npub fn one(x: u8) -> u8 = x\npub fn bad() -> u8 = zz\n",
    )
    .unwrap();
    fs::write(dir.join("broken.mn"), "module m\nlet x = 1\n").unwrap();
    let run = mn(&dir, &["view", "--checks", "c.mn"]);
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(
        text(&run.stdout),
        concat!(
            "fn sum\n",
            "  6:10 narrowing into `i64`\n",
            "  Tier 1 erased 5 run-time checks\n",
            "fn pick\n",
            "  11:56 narrowing into `u8`\n",
            "  11:59 index into an array of 4\n",
            "  11:64 divisor\n",
            "  11:69 shift amount below 8\n",
            "  Tier 1 erased 1 run-time check\n",
            "fn one\n",
            "  Tier 1 erased 1 run-time check\n",
        )
    );
    let broken = mn(&dir, &["view", "--checks", "broken.mn"]);
    assert_eq!(broken.status.code(), Some(1));
    assert!(text(&broken.stderr).contains("E0111"));
    let missing = mn(&dir, &["view", "--checks", "missing.mn"]);
    assert_eq!(missing.status.code(), Some(2));
    let usage = mn(&dir, &["view", "--checks"]);
    assert_eq!(usage.status.code(), Some(2));
    assert!(text(&usage.stderr).contains("Usage: mn view --checks"));
    fs::remove_dir_all(&dir).unwrap();
}
