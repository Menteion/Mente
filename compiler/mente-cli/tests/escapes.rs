//! The record of escaped bugs, `docs/escapes.md` (Norman, 2026-11-16), beside the register's
//! tests: every row is checked against the tree, through the one reader of the table, which
//! `mn escapes` uses too, so that a row and its guard cannot part unseen.

#[path = "../src/escapes.rs"]
mod escapes;

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The record's table with `rows`.
fn table(rows: &[&str]) -> String {
    format!(
        "# A record\n\n{}\n| :--- | :--- | :--- | :--- | :--- | :--- |\n{}\n",
        escapes::HEADER,
        rows.join("\n")
    )
}

/// A row that holds, with id `id`, day `day`, area `area`, what missed it `missed` and guard
/// `guard`.
fn row(id: &str, day: &str, area: &str, missed: &str, guard: &str) -> String {
    format!(
        "| `{}` | {} | [#129](https://github.com/Menteion/Mente/pull/129) | {} | {} | {} |",
        id, day, area, missed, guard
    )
}

const LINT: &str = "`fn test_exhaustive_crates_deny_wildcard_arms`";

/// Every row of the record has the next id, a day no earlier than the row above's, one of the
/// record's areas, what missed it, and a guard declared where its kind is: a Rust test, a check of
/// the workflow policy, a Node test or a quick check. Deleting a guard fails this test.
#[test]
fn test_every_escape_has_its_id_its_day_its_area_and_a_guard_that_stands() {
    let text = fs::read_to_string(root().join(escapes::RECORD)).unwrap();
    let rows = escapes::parse(&text).unwrap_or_else(|p| panic!("{}", p.join("\n")));
    assert!(!rows.is_empty(), "{} holds no row", escapes::RECORD);
    let problems = escapes::problems(&root(), &rows);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// A row is refused for each rule it breaks: a line that does not read as a row, an id out of
/// turn or repeated, a day before the row above's, an area outside the list, nothing named as
/// what missed it, and a guard of each kind that is declared nowhere.
#[test]
fn test_a_row_is_refused_for_each_rule_it_breaks() {
    let good = row("ESC-001", "2026-11-14", "checker", "No lint.", LINT);
    let rows = escapes::parse(&table(&[&good])).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(escapes::problems(&root(), &rows).is_empty());
    // Lines that do not read as a row.
    for bad in [
        row("ESC-001", "2026-02-30", "checker", "No lint.", LINT),
        row("ESC-001", "14-11-2026", "checker", "No lint.", LINT),
        row("ESC-001", "2026-11-14", "checker", "No lint.", "test_x"),
        row(
            "ESC-001",
            "2026-11-14",
            "checker",
            "No lint.",
            "`fn helper`",
        ),
        row(
            "ESC-001",
            "2026-11-14",
            "checker",
            "No lint.",
            "policy check name",
        ),
        good.replace("pull/129)", "pull/130)"),
        good.replace(" | checker |", " |"),
    ] {
        assert!(escapes::parse(&table(&[&bad])).is_err(), "{}", bad);
    }
    // Rows that read and break a rule.
    let second = |day: &str| row("ESC-002", day, "MIR", "The generator.", LINT);
    for (rows, problem) in [
        (
            vec![good.clone(), good.clone()],
            "the id of row 2 is ESC-002",
        ),
        (vec![second("2026-11-14")], "the id of row 1 is ESC-001"),
        (
            vec![good.clone(), second("2026-11-13")],
            "its day comes before the row above's",
        ),
        (
            vec![row("ESC-001", "2026-11-14", "parser", "No lint.", LINT)],
            "the area `parser` is none of",
        ),
        (
            vec![row("ESC-001", "2026-11-14", "checker", "", LINT)],
            "it names no instrument that missed it",
        ),
        (
            vec![row(
                "ESC-001",
                "2026-11-14",
                "checker",
                "No lint.",
                "`fn test_no_such_test_is_declared_anywhere`",
            )],
            "is declared nowhere",
        ),
        (
            vec![row(
                "ESC-001",
                "2026-11-14",
                "policy",
                "No check.",
                "policy check \"no such check is declared anywhere\"",
            )],
            "is declared nowhere",
        ),
        (
            vec![row(
                "ESC-001",
                "2026-11-14",
                "harness",
                "No test.",
                "node test \"no such test is declared anywhere\"",
            )],
            "is declared nowhere",
        ),
        (
            vec![row(
                "ESC-001",
                "2026-11-14",
                "local checks",
                "No check.",
                "quick check \"no such check is declared anywhere\"",
            )],
            "is declared nowhere",
        ),
    ] {
        let text: Vec<&str> = rows.iter().map(String::as_str).collect();
        let parsed = escapes::parse(&table(&text)).unwrap();
        let found = escapes::problems(&root(), &parsed);
        assert!(
            found.iter().any(|p| p.contains(problem)),
            "{:?}: {:?}",
            rows,
            found
        );
    }
    // A guard of each kind that is declared is found.
    for guard in [
        "policy check \"every file of .github/workflows is a workflow the policy reads\"",
        "node test \"every script that calls the API plans its runs with no broker and no key, as far as its first call\"",
        "quick check \"MIR mutation gate\"",
    ] {
        let parsed = escapes::parse(&table(&[&row(
            "ESC-001",
            "2026-11-14",
            "policy",
            "Something.",
            guard,
        )]))
        .unwrap();
        assert!(escapes::problems(&root(), &parsed).is_empty(), "{}", guard);
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
    let rows = escapes::parse(&table(&[
        &row("ESC-001", "2026-11-14", "checker", "A.", LINT),
        &row("ESC-002", "2026-11-15", "MIR", "B.", LINT),
        &row("ESC-003", "2026-11-16", "MIR", "C.", LINT),
    ]))
    .unwrap();
    assert_eq!(
        escapes::counts(&rows),
        "Escapes per week:\n  2026-W46  2\n  2026-W47  1\nEscapes per area:\n  MIR  2\n  checker  1\n"
    );
}
