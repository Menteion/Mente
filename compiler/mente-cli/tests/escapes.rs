//! The record of escaped bugs, one file an escape in `docs/escapes/` (Norman, 2026-11-16 and
//! 2026-11-17), beside the register's tests: every file is checked against the tree, through the
//! one reader of the record, which `mn escapes` uses too, so that an escape and its guard cannot
//! part unseen.

use mente_cli::escapes;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// An escape's file, with day `day`, area `area`, what missed it `missed` and guard `guard`.
fn file(day: &str, area: &str, missed: &str, guard: &str) -> String {
    format!(
        "---\ndate: {}\nfixed-by: https://github.com/Menteion/Mente/pull/129\narea: {}\nmissed-by: {}\nguard: {}\n---\n",
        day, area, missed, guard
    )
}

const LINT: &str = "fn test_exhaustive_crates_deny_wildcard_arms";

/// A fresh directory of this test binary's own, named `name`, holding `files`.
fn planted(name: &str, files: &[(&str, &str)]) -> io::Result<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("escapes")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    for (path, text) in files {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
    }
    Ok(dir)
}

/// Every file of the record is named by a key, has the record's front matter, a day of the
/// calendar, a pull request's address, one of the record's areas, what missed it, and a guard
/// declared where its kind is: a Rust test, a check of the workflow policy, a Node test or a quick
/// check. The directory holds nothing else. Deleting a guard fails this test.
#[test]
fn test_every_escape_has_a_key_an_area_and_a_guard_that_stands() {
    let rows =
        escapes::read(&root().join(escapes::RECORD)).unwrap_or_else(|p| panic!("{}", p.join("\n")));
    assert!(!rows.is_empty(), "{} holds no escape", escapes::RECORD);
    let problems = escapes::problems(&root(), &rows);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The directory is refused for each entry that is no escape: a name that is no key, a number
/// among them, a file of another kind, a README, and a subdirectory. Its escapes are read in the
/// order of their keys; a file of CRLF line ends is refused, as a carriage return anywhere is.
#[test]
fn test_the_record_holds_only_files_named_by_a_key() {
    let good = file("2026-11-14", "checker", "No lint.", LINT);
    let dir = planted(
        "good",
        &[
            ("walker-hid-operands.md", &good),
            ("generator-gap.md", &good),
        ],
    )
    .unwrap();
    let rows = escapes::read(&dir).unwrap();
    let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["generator-gap", "walker-hid-operands"]);
    assert!(escapes::problems(&root(), &rows).is_empty());
    for (name, path) in [
        ("numbered", "ESC-001.md"),
        ("uppercase", "Walker-hid.md"),
        ("double-hyphen", "walker--hid.md"),
        ("digit-first", "1-walker.md"),
        ("other-kind", "walker-hid.txt"),
        ("no-extension", "walker-hid"),
        ("readme", "README.md"),
        ("subdirectory", "older/walker-hid.md"),
    ] {
        let dir = planted(name, &[("walker-hid-operands.md", &good), (path, &good)]).unwrap();
        let problems = escapes::read(&dir).expect_err(path);
        assert_eq!(problems.len(), 1, "{}: {:?}", path, problems);
        let entry = path.split('/').next().unwrap();
        assert!(
            problems[0].starts_with(&format!("{}: no escape", entry)),
            "{}: {:?}",
            path,
            problems
        );
    }
}

/// A file is refused for each way its front matter breaks the record's form: no front matter, an
/// unclosed one, a field missing, out of order, renamed or added, a value YAML would read as
/// something else, a quote left alone, a day off the calendar, a fix that is no pull request's
/// address, and a guard of no form.
#[test]
fn test_a_file_is_refused_for_each_rule_of_its_front_matter() {
    let good = file("2026-11-14", "checker", "No lint.", LINT);
    let escape = escapes::parse("walker-hid", &good).unwrap();
    assert_eq!(escape.date, (2026, 11, 14));
    assert_eq!(escape.pull, 129);
    assert_eq!(escape.guard, escapes::Guard::Test(LINT[3..].to_string()));
    let with_prose = format!("{}\n# The walker\n\nIt hid --- the operands.\n", good);
    assert!(escapes::parse("walker-hid", &with_prose).is_ok());
    let quoted = file(
        "2026-11-14",
        "checker",
        "'No lint: a walker''s `_` arm.'",
        "'node test \"a: b\"'",
    );
    let escape = escapes::parse("walker-hid", &quoted).unwrap();
    assert_eq!(escape.missed, "No lint: a walker's `_` arm.");
    assert_eq!(escape.guard, escapes::Guard::Node("a: b".to_string()));
    for (bad, why) in [
        (good.replacen("---\n", "", 1), "no front matter"),
        (good.trim_end_matches("---\n").to_string(), "does not close"),
        (good.replace("area: checker\n", ""), "not the fields"),
        (
            good.replace(
                "area: checker\nmissed-by: No lint.\n",
                "missed-by: No lint.\narea: checker\n",
            ),
            "not the fields",
        ),
        (good.replace("fixed-by:", "fixed:"), "not the fields"),
        (
            good.replace("guard:", "kind: test\nguard:"),
            "not the fields",
        ),
        (
            good.replace("area: checker", "area:checker"),
            "not the fields",
        ),
        (
            good.replace("No lint.", "No lint: a wildcard."),
            "the value of `missed-by`",
        ),
        (
            good.replace("No lint.", "No lint # a wildcard."),
            "the value of `missed-by`",
        ),
        (
            good.replace("No lint.", "`_` hid the operands."),
            "the value of `missed-by`",
        ),
        (
            good.replace("No lint.", "'A walker's arm.'"),
            "the value of `missed-by`",
        ),
        (
            good.replace("No lint.", "'No lint."),
            "the value of `missed-by`",
        ),
        (
            good.replace("No lint.", " No lint."),
            "the value of `missed-by`",
        ),
        (good.replace("2026-11-14", "2026-02-30"), "no day"),
        (good.replace("2026-11-14", "14-11-2026"), "no day"),
        (
            good.replace("pull/129", "pull/12x"),
            "no pull request's address",
        ),
        (
            good.replace("pull/129", "pull/0"),
            "no pull request's address",
        ),
        (
            good.replace("pull/129", "pull/0129"),
            "no pull request's address",
        ),
        (good.replace("\n", "\r\n"), "a tab or a carriage return"),
        (
            good.replace("pull/129", "pull/+129"),
            "no pull request's address",
        ),
        (
            good.replace("No lint.", "No lint:"),
            "the value of `missed-by`",
        ),
        (
            good.replace("missed-by: No lint.", "missed-by: "),
            "the value of `missed-by`",
        ),
        (
            good.replace("pull/129", "pull/"),
            "no pull request's address",
        ),
        (
            good.replace("Menteion/Mente/pull/129", "Menteion/Menteion/pull/129"),
            "no pull request's address",
        ),
        (good.replace(LINT, "test_x"), "a guard that is none"),
        (good.replace(LINT, "fn helper"), "a guard that is none"),
        (good.replace(LINT, "fn test_a(b)"), "a guard that is none"),
        (
            good.replace(LINT, "policy check name"),
            "a guard that is none",
        ),
        (good.replace(LINT, "node test \"\""), "a guard that is none"),
    ] {
        let found = escapes::parse("walker-hid", &bad).expect_err(&bad);
        assert!(found.contains(why), "{}\n{}", bad, found);
    }
}

/// An escape that reads is refused for each rule of the record it breaks: an area outside the
/// list, nothing named as what missed it, and a guard of each kind that is declared nowhere; a
/// guard of each kind that is declared is found.
#[test]
fn test_an_escape_is_refused_for_each_rule_of_the_record() {
    for (text, problem) in [
        (
            file("2026-11-14", "parser", "No lint.", LINT),
            "the area `parser` is none of",
        ),
        (
            file("2026-11-14", "checker", "''", LINT),
            "it names no instrument that missed it",
        ),
        (
            file(
                "2026-11-14",
                "checker",
                "No lint.",
                "fn test_no_such_test_is_declared_anywhere",
            ),
            "is declared nowhere",
        ),
        (
            file(
                "2026-11-14",
                "policy",
                "No check.",
                "policy check \"no such check is declared anywhere\"",
            ),
            "is declared nowhere",
        ),
        (
            file(
                "2026-11-14",
                "harness",
                "No test.",
                "node test \"no such test is declared anywhere\"",
            ),
            "is declared nowhere",
        ),
        (
            file(
                "2026-11-14",
                "local checks",
                "No check.",
                "quick check \"no such check is declared anywhere\"",
            ),
            "is declared nowhere",
        ),
    ] {
        let row = escapes::parse("walker-hid", &text).unwrap();
        let found = escapes::problems(&root(), &[row]);
        assert_eq!(found.len(), 1, "{}: {:?}", text, found);
        assert!(found[0].contains(problem), "{}: {:?}", text, found);
    }
    for guard in [
        LINT,
        "policy check \"every file of .github/workflows is a workflow the policy reads\"",
        "node test \"every script that calls the API plans its runs with no broker and no key, as far as its first call\"",
        "quick check \"MIR mutation gate\"",
    ] {
        let row = escapes::parse(
            "walker-hid",
            &file("2026-11-14", "policy", "Something.", guard),
        )
        .unwrap();
        assert!(escapes::problems(&root(), &[row]).is_empty(), "{}", guard);
    }
}

/// A repository of this test binary's own, named `name`, holding `files`, of which git tracks
/// those `tracked` names; with `tracked` `None`, a checkout whose `.git` is an empty directory, as
/// in CI's test sandbox.
fn repository(name: &str, files: &[(&str, &str)], tracked: Option<&[&str]>) -> io::Result<PathBuf> {
    let dir = planted(name, files)?;
    let Some(tracked) = tracked else {
        fs::create_dir(dir.join(".git"))?;
        return Ok(dir);
    };
    let git = |args: &[&str]| -> io::Result<()> {
        let status = std::process::Command::new("git")
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .current_dir(&dir)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("git {:?}: {}", args, status)))
        }
    };
    git(&["init", "-q"])?;
    if !tracked.is_empty() {
        git(&[&["add", "--"][..], tracked].concat())?;
    }
    Ok(dir)
}

/// A guard is declared only in code that git tracks: a Rust test under `#[test]`, run and not
/// ignored, in `compiler/`; a policy check or a Node test written in code. A name in a comment, in
/// a string, in an untracked copy, in a spike, in a function that lost `#[test]` or in an ignored
/// test declares none. In CI's sandbox, whose `.git` is empty, the checkout is read, but what a
/// build or an install leaves under `target` and `node_modules`.
#[test]
fn test_a_guard_is_read_from_the_code_git_tracks() {
    let test = escapes::parse(
        "walker-hid",
        &file("2026-11-14", "checker", "No lint.", "fn test_planted_guard"),
    )
    .unwrap();
    let check = escapes::parse(
        "walker-hid",
        &file(
            "2026-11-14",
            "policy",
            "No check.",
            "policy check \"a planted check\"",
        ),
    )
    .unwrap();
    let node = escapes::parse(
        "walker-hid",
        &file(
            "2026-11-14",
            "harness",
            "No test.",
            "node test \"a planted test\"",
        ),
    )
    .unwrap();
    let rust = "compiler/mente-x/src/lib.rs";
    let policy = ".github/scripts/policy.test.mjs";
    let harness = "benchmark/harness/harness.test.ts";
    for (name, path, text, row, found) in [
        (
            "rust-test",
            rust,
            "#[test]\nfn test_planted_guard() {}\n",
            &test,
            true,
        ),
        (
            "rust-attributes",
            rust,
            "#[test]\n#[cfg(unix)]\npub fn test_planted_guard() {\n}\n",
            &test,
            true,
        ),
        (
            "rust-no-test",
            rust,
            "fn test_planted_guard() {}\n",
            &test,
            false,
        ),
        (
            "rust-ignored",
            rust,
            "#[test]\n#[ignore = \"slow\"]\nfn test_planted_guard() {}\n",
            &test,
            false,
        ),
        (
            "rust-ignored-first",
            rust,
            "#[ignore]\n#[test]\nfn test_planted_guard() {}\n",
            &test,
            false,
        ),
        (
            "rust-line-comment",
            rust,
            "#[test]\n// fn test_planted_guard() {}\nfn other() {}\n",
            &test,
            false,
        ),
        (
            "rust-block-comment",
            rust,
            "/* #[test]\nfn test_planted_guard() {} /* nested */ */\n",
            &test,
            false,
        ),
        (
            "rust-doc-comment",
            rust,
            "/// #[test]\n/// fn test_planted_guard() {}\nfn other() {}\n",
            &test,
            false,
        ),
        (
            "rust-string",
            rust,
            "const S: &str = \"#[test]\\nfn test_planted_guard() {}\";\n",
            &test,
            false,
        ),
        (
            "rust-raw-string",
            rust,
            "const S: &str = r#\"#[test] fn test_planted_guard() {}\"#;\n",
            &test,
            false,
        ),
        (
            "rust-longer-name",
            rust,
            "#[test]\nfn xtest_planted_guard() {}\n",
            &test,
            false,
        ),
        (
            "rust-spike",
            "spikes/x/src/lib.rs",
            "#[test]\nfn test_planted_guard() {}\n",
            &test,
            false,
        ),
        (
            "rust-extension",
            "compiler/mente-x/src/lib.txt",
            "#[test]\nfn test_planted_guard() {}\n",
            &test,
            false,
        ),
        (
            "policy-check",
            policy,
            "check(\"a planted check\", () => {});\n",
            &check,
            true,
        ),
        (
            "policy-line-comment",
            policy,
            "// check(\"a planted check\", () => {});\n",
            &check,
            false,
        ),
        (
            "policy-block-comment",
            policy,
            "/*\ncheck(\"a planted check\", () => {});\n*/\n",
            &check,
            false,
        ),
        (
            "policy-after-url",
            policy,
            "const u = \"https://x\"; check(\"a planted check\", () => {});\n",
            &check,
            true,
        ),
        (
            "policy-after-regex",
            policy,
            "const r = /\\/\\//; check(\"a planted check\", () => {});\n",
            &check,
            true,
        ),
        (
            "node-test",
            harness,
            "test(\"a planted test\", () => {});\n",
            &node,
            true,
        ),
        (
            "node-commented",
            harness,
            "// test(\"a planted test\", () => {});\n",
            &node,
            false,
        ),
    ] {
        let dir = repository(name, &[(path, text)], Some(&[path])).unwrap();
        let problems = escapes::problems(&dir, std::slice::from_ref(row));
        assert_eq!(problems.is_empty(), found, "{}: {:?}", name, problems);
    }
    // An untracked copy declares nothing; a tracked one does.
    let declared = "#[test]\nfn test_planted_guard() {}\n";
    let copy = "compiler/old/lib.rs";
    let files = [(rust, "fn other() {}\n"), (copy, declared)];
    let untracked = repository("untracked-copy", &files, Some(&[rust])).unwrap();
    assert!(!escapes::problems(&untracked, std::slice::from_ref(&test)).is_empty());
    let both = repository("tracked-copy", &files, Some(&[rust, copy])).unwrap();
    assert!(escapes::problems(&both, std::slice::from_ref(&test)).is_empty());
    // CI's sandbox: the checkout is read, but what a build or an install leaves.
    for (name, path, found) in [
        ("sandbox-source", rust, true),
        ("sandbox-target", "compiler/target/debug/lib.rs", false),
        ("sandbox-modules", "compiler/node_modules/x/lib.rs", false),
    ] {
        let dir = repository(name, &[(path, declared)], None).unwrap();
        let problems = escapes::problems(&dir, std::slice::from_ref(&test));
        assert_eq!(problems.is_empty(), found, "{}: {:?}", name, problems);
    }
    // A repository git cannot read fails the check, never passes it.
    let broken = planted(
        "broken-git",
        &[(rust, declared), (".git", "gitdir: nowhere\n")],
    )
    .unwrap();
    let problems = escapes::problems(&broken, std::slice::from_ref(&test));
    assert_eq!(problems.len(), 1, "{:?}", problems);
    assert!(
        problems[0].starts_with("the guards cannot be read: git ls-files"),
        "{:?}",
        problems
    );
}

/// `mn escapes` counts the escapes per ISO week, the Thursday's year deciding the week's, and per
/// area.
#[test]
fn test_escapes_are_counted_per_iso_week_and_per_area() {
    for (day, week) in [
        ((2026, 11, 15), "2026-W46"),
        ((2026, 11, 16), "2026-W47"),
        ((2026, 1, 1), "2026-W01"),
        ((2027, 1, 1), "2026-W53"),
        ((2024, 12, 30), "2025-W01"),
        ((2021, 1, 3), "2020-W53"),
        // Across the 400-year cycles of the calendar: 2000-01-01, a Saturday, is in 1999-W52.
        ((2000, 1, 1), "1999-W52"),
        ((2400, 1, 1), "2399-W52"),
        ((2001, 1, 1), "2001-W01"),
    ] {
        assert_eq!(escapes::week(day), week, "{:?}", day);
    }
    let dir = planted(
        "counted",
        &[
            ("first.md", &file("2026-11-14", "checker", "A.", LINT)),
            ("second.md", &file("2026-11-16", "MIR", "C.", LINT)),
            ("third.md", &file("2026-11-15", "MIR", "B.", LINT)),
        ],
    )
    .unwrap();
    let rows = escapes::read(&dir).unwrap();
    assert_eq!(
        escapes::counts(&rows),
        "Escapes per week:\n  2026-W46  2\n  2026-W47  1\nEscapes per area:\n  MIR  2\n  checker  1\n"
    );
}

/// `mn escapes` run in `dir` with `args`: its exit code, its standard output and its standard
/// error. A failure to start it fails the test that asked.
fn mn(dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let out = match std::process::Command::new(env!("CARGO_BIN_EXE_mn"))
        .arg("escapes")
        .args(args)
        .current_dir(dir)
        .output()
    {
        Ok(out) => out,
        Err(e) => panic!("run mn: {}", e),
    };
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `mn escapes` prints the record's counts, `mn escapes --check` checks it as the test does, and
/// each fails with what is wrong: an argument it does not take, a directory that is missing, an
/// entry that is no escape, and, with `--check`, a guard declared nowhere.
#[test]
fn test_mn_escapes_counts_and_checks_the_record() {
    let rows = escapes::read(&root().join(escapes::RECORD)).unwrap();
    assert_eq!(
        mn(&root(), &[]),
        (Some(0), escapes::counts(&rows), String::new())
    );
    let (code, out, err) = mn(&root(), &["--check"]);
    assert_eq!((code, err.as_str()), (Some(0), ""));
    assert_eq!(
        out,
        "[PASS] mn escapes: every escape of docs/escapes holds\n"
    );
    for args in [&["--count"][..], &["--check", "--check"][..]] {
        let (code, out, err) = mn(&root(), args);
        assert_eq!((code, out.as_str()), (Some(1), ""), "{:?}", args);
        assert_eq!(err, "Usage: mn escapes [--check]\n", "{:?}", args);
    }
    let empty = planted("cli-empty", &[]).unwrap();
    let (code, out, err) = mn(&empty, &[]);
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(
        err.starts_with("[FAIL] ") && err.contains("escapes"),
        "{}",
        err
    );
    let nowhere = file(
        "2026-11-14",
        "checker",
        "No lint.",
        "fn test_declared_nowhere",
    );
    let dir = planted(
        "cli-record",
        &[
            ("docs/escapes/walker-hid.md", &nowhere),
            ("docs/escapes/ESC-001.md", &nowhere),
        ],
    )
    .unwrap();
    let (code, out, err) = mn(&dir, &[]);
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(err.starts_with("[FAIL] ESC-001.md: "), "{}", err);
    fs::remove_file(dir.join("docs/escapes/ESC-001.md")).unwrap();
    let rows = escapes::read(&dir.join(escapes::RECORD)).unwrap();
    assert_eq!(
        mn(&dir, &[]),
        (Some(0), escapes::counts(&rows), String::new())
    );
    let (code, out, err) = mn(&dir, &["--check"]);
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(
        err.starts_with("[FAIL] walker-hid: its guard") && err.contains("declared nowhere"),
        "{}",
        err
    );
}
