//! The rules of the design register, `docs/design-review.md`, that a test can hold. They are an
//! integration test of their own, apart from main.rs, which changes with every feature, so that
//! nothing a feature changes can unhook them (`docs/design-review.md`, `reversal-conditions`).

use std::fs;
use std::path::Path;

/// The day from which a decision names the evidence that would reopen it (Norman, 2026-11-13).
const REOPEN_RULE_FROM: &str = "2026-11-13";

/// The marker that opens an entry's reopen condition, which ends its resolution.
const REOPEN_MARKER: &str = "**Reopen if:**";

/// The entries applied before the register recorded the day of each decision, which name none.
/// The number may only fall: an entry decided from now on records its day, so that the reopen
/// rule can tell when it was decided, and no entry escapes the rule by leaving its day out.
const UNDATED_DONE_ENTRIES: usize = 61;

/// One row of the register's tables.
struct Entry {
    key: String,
    resolution: String,
    status: String,
}

/// Splits a table row into its cells. A `|` inside backticks, or escaped as `\|`, stays in its cell.
fn cells(row: &str) -> Vec<String> {
    let inner = row.trim();
    let inner = inner.strip_prefix('|').unwrap_or(inner);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut found = Vec::new();
    let mut cell = String::new();
    let mut in_code = false;
    let mut escaped = false;
    for c in inner.chars() {
        if c == '|' && !in_code && !escaped {
            found.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(c);
        }
        if c == '`' {
            in_code = !in_code;
        }
        escaped = c == '\\' && !escaped;
    }
    found.push(cell.trim().to_string());
    found
}

/// Reads every entry of the register: the rows of its tables whose first cell is a key in
/// backticks. A row of another shape is a violation, since the rules below could not read it.
fn entries(register: &str) -> (Vec<Entry>, Vec<String>) {
    let mut found = Vec::new();
    let mut malformed = Vec::new();
    for (index, row) in register.lines().enumerate() {
        if !row.starts_with("| `") {
            continue;
        }
        let cells = cells(row);
        let key = cells[0].trim_matches('`');
        if cells.len() != 4 || key.is_empty() || key.contains('`') {
            malformed.push(format!(
                "line {}: an entry has a key in backticks, a problem, a resolution and a status; read {} cells",
                index + 1,
                cells.len()
            ));
            continue;
        }
        found.push(Entry {
            key: key.to_string(),
            resolution: cells[2].clone(),
            status: cells[3].clone(),
        });
    }
    (found, malformed)
}

/// The days of the decisions an entry records, written `(Norman, YYYY-MM-DD` in its resolution
/// or its status.
fn decision_days(entry: &Entry) -> Vec<String> {
    let mut days = Vec::new();
    for text in [&entry.resolution, &entry.status] {
        for (at, _) in text.match_indices("(Norman, ") {
            let day: String = text[at + "(Norman, ".len()..].chars().take(10).collect();
            let shape = day.len() == 10
                && day.chars().enumerate().all(|(i, c)| {
                    if i == 4 || i == 7 {
                        c == '-'
                    } else {
                        c.is_ascii_digit()
                    }
                });
            if shape {
                days.push(day);
            }
        }
    }
    days
}

/// The status's word: `open`, `proposed`, `decided` or `done`.
fn status_word(entry: &Entry) -> &str {
    let end = entry
        .status
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(entry.status.len());
    &entry.status[..end]
}

/// The register's rules for decisions ([`reversal-conditions`] in the register):
/// - a `decided` entry records the day it was decided, `Decided (Norman, YYYY-MM-DD)`;
/// - an entry decided on or after [`REOPEN_RULE_FROM`], `decided` or `done`, ends its resolution
///   with [`REOPEN_MARKER`] and the evidence that would reopen it: the marker once, words after
///   it, and no `Rejected:` after it;
/// - at most [`UNDATED_DONE_ENTRIES`] entries are `done` without a recorded day.
fn reopen_violations(register: &str) -> Vec<String> {
    let (entries, mut found) = entries(register);
    let mut undated_done = Vec::new();
    for entry in &entries {
        let status = status_word(entry);
        if !matches!(status, "decided" | "done") {
            continue;
        }
        let days = decision_days(entry);
        if status == "decided" && !entry.resolution.contains("Decided (Norman, ") {
            found.push(format!(
                "`{}` is decided, and its resolution does not record the day: `**Decided (Norman, YYYY-MM-DD).**`",
                entry.key
            ));
        }
        if days.is_empty() {
            if status == "done" {
                undated_done.push(entry.key.clone());
            }
            continue;
        }
        if days.iter().all(|day| day.as_str() < REOPEN_RULE_FROM) {
            continue;
        }
        let marks: Vec<usize> = entry
            .resolution
            .match_indices(REOPEN_MARKER)
            .map(|(at, _)| at)
            .collect();
        match marks.as_slice() {
            [] => found.push(format!(
                "`{}` was decided on or after {}, and its resolution does not end with `{}` and the evidence that would reopen it",
                entry.key, REOPEN_RULE_FROM, REOPEN_MARKER
            )),
            [at] => {
                let condition = entry.resolution[at + REOPEN_MARKER.len()..].trim();
                if !condition.chars().any(char::is_alphabetic) {
                    found.push(format!(
                        "`{}` names no evidence after `{}`",
                        entry.key, REOPEN_MARKER
                    ));
                } else if condition.contains("Rejected:") {
                    found.push(format!(
                        "`{}`'s reopen condition is not at the end of its resolution: `Rejected:` follows it",
                        entry.key
                    ));
                }
            }
            _ => found.push(format!(
                "`{}` has `{}` {} times; a resolution ends with one condition",
                entry.key,
                REOPEN_MARKER,
                marks.len()
            )),
        }
    }
    if undated_done.len() > UNDATED_DONE_ENTRIES {
        found.push(format!(
            "{} entries are done without the day of their decision, more than the {} applied before days were recorded; record `(Norman, YYYY-MM-DD)` in each new one: {}",
            undated_done.len(),
            UNDATED_DONE_ENTRIES,
            undated_done.join(", ")
        ));
    }
    found
}

/// Every entry decided from 2026-11-13 names the evidence that would reopen it.
#[test]
fn test_decided_entries_name_what_would_reopen_them() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let register = fs::read_to_string(root.join("docs/design-review.md")).unwrap();
    let (read, _) = entries(&register);
    assert!(
        read.len() >= 100,
        "expected the register's entries, read {}",
        read.len()
    );
    let found = reopen_violations(&register);
    assert!(
        found.is_empty(),
        "the register's rules for decisions (docs/design-review.md, its introduction):\n{}",
        found.join("\n")
    );
}

/// The rule itself: what it lets through and what it stops.
#[test]
fn test_the_reopen_rule_rejects_a_decision_without_its_condition() {
    let row = |key: &str, resolution: &str, status: &str| {
        format!("| `{}` | A problem. | {} | {} |\n", key, resolution, status)
    };
    let mut register = String::from(
        "| Key | Problem | Proposed resolution | Status |\n| :--- | :--- | :--- | :--- |\n",
    );
    register += &row("open", "", "open");
    register += &row("proposed", "Do it.", "proposed");
    register += &row(
        "before",
        "**Decided (Norman, 2026-11-12).** Do it.",
        "decided",
    );
    register += &row(
        "with",
        "**Decided (Norman, 2026-11-13).** Do it. Rejected: the other. **Reopen if:** a tenth of the repairs.",
        "decided",
    );
    register += &row(
        "code",
        "**Decided (Norman, 2026-11-13).** Do `a | b`. **Reopen if:** none: it follows from an invariant.",
        "decided",
    );
    register += &row(
        "without",
        "**Decided (Norman, 2026-11-13).** Do it. Rejected: the other.",
        "decided",
    );
    register += &row("undated", "Do it.", "decided");
    register += &row(
        "applied",
        "Do it.",
        "done: [`x.md`](../spec/x.md) (Norman, 2026-11-14; commit \"Do it\")",
    );
    register += &row(
        "empty",
        "**Decided (Norman, 2026-11-20).** Do it. **Reopen if:** .",
        "decided",
    );
    register += &row(
        "early",
        "**Decided (Norman, 2026-11-20).** Do it. **Reopen if:** a tenth. Rejected: the other.",
        "decided",
    );
    register += &row(
        "twice",
        "**Decided (Norman, 2026-11-20).** **Reopen if:** one. **Reopen if:** two.",
        "decided",
    );
    register += "| `short` | A problem. | decided |\n";
    let (read, malformed) = entries(&register);
    assert_eq!(read.len(), 11);
    assert_eq!(malformed.len(), 1, "{:?}", malformed);
    let found = reopen_violations(&register);
    assert_eq!(found.len(), 7, "{}", found.join("\n"));
    assert!(found[0].contains("read 3 cells"), "{}", found[0]);
    let keys: Vec<&str> = found[1..]
        .iter()
        .map(|message| message.split('`').nth(1).unwrap_or(message))
        .collect();
    assert_eq!(
        keys,
        vec!["without", "undated", "applied", "empty", "early", "twice"],
        "{}",
        found.join("\n")
    );
    for (message, says) in found[1..].iter().zip([
        "does not end with",
        "does not record the day",
        "does not end with",
        "names no evidence",
        "`Rejected:` follows it",
        "2 times",
    ]) {
        assert!(message.contains(says), "{}", message);
    }

    // Undated applied entries may not grow past the number applied before days were recorded.
    let mut many = String::new();
    for index in 0..UNDATED_DONE_ENTRIES {
        many += &row(
            &format!("old-{}", index),
            "Do it.",
            "done: (commit \"Do it\")",
        );
    }
    assert!(reopen_violations(&many).is_empty());
    many += &row("new", "Do it.", "done: (commit \"Do it\")");
    let found = reopen_violations(&many);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].contains("without the day of their decision"),
        "{}",
        found[0]
    );
}
