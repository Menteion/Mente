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
/// order of their keys, and CRLF line ends read as LF.
#[test]
fn test_the_record_holds_only_files_named_by_a_key() {
    let good = file("2026-11-14", "checker", "No lint.", LINT);
    let dir = planted(
        "good",
        &[
            ("walker-hid-operands.md", &good),
            ("generator-gap.md", &good.replace('\n', "\r\n")),
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
