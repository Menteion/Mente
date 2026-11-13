//! The rules of the design register, `docs/design-review.md`, that a test can hold, as an
//! integration test of their own, apart from main.rs, which changes with every feature
//! (`docs/design-review.md`, `reversal-conditions`).

use std::fs;
use std::path::Path;

/// The day from which a decision names the evidence that would reopen it (Norman, 2026-11-13).
const REOPEN_RULE_FROM: &str = "2026-11-13";

/// The marker that opens an entry's reopen condition, the last sentence of its resolution.
const REOPEN_MARKER: &str = "**Reopen if:**";

/// The one form in which an entry records the day of its decision: `(Norman, ` and the day as
/// `YYYY-MM-DD`, as in `**Decided (Norman, 2026-11-13).**`.
const DAY_OPENS: &str = "(Norman, ";

/// The entries decided or applied before the register recorded each day in that form, as they stood
/// on 2026-11-13, with the day where the entry gives it in prose and `None` where it gives none.
/// The list may only shrink: an entry decided since then records its day in the one form, so that
/// the reopen rule can tell when it was decided and no entry escapes the rule by leaving its day
/// out; and an entry of the list that is given its day in that form leaves the list.
const LEGACY: &[(&str, Option<&str>)] = &[
    ("interface-from-signature", None),
    ("units-scale", None),
    ("interval-subtyping", None),
    ("octagon-midpoint", None),
    ("requires-fragment", None),
    ("generic-angle-ambiguity", None),
    ("overflow-release", None),
    ("string-types", None),
    ("comptime-params", None),
    ("pool-indexing", None),
    ("slice-syntax", None),
    ("literal-suffixes", None),
    ("bitfield-widths", None),
    ("mut-projection-soa", None),
    ("shared-upgrade", None),
    ("arena-concurrency-and-drop", None),
    ("parallel-exclusivity", None),
    ("layout-alignment-endianness", None),
    ("effect-polymorphism", None),
    ("with-overloaded", None),
    ("handler-semantics", None),
    ("dst-scheduling", None),
    ("disjoint-proof-tier", None),
    ("fiber-stacks", None),
    ("branchless-definition", None),
    ("simd-kernel-model", None),
    ("transitive-contracts", None),
    ("cf-abi-scope", None),
    ("mutation-kill-rate", None),
    ("mn-bind-scope", None),
    ("ffi-boundary-failure", None),
    ("comptime-termination", None),
    ("assert-substring", None),
    ("tier1-loops", Some("2026-11-08")),
    ("tier1-counterexample", Some("2026-11-08")),
    ("tier2-trusted", Some("2026-11-08")),
    ("tier1-latency-spike", None),
    ("codegen-placeholders", Some("2026-11-08")),
    ("implemented-evidence", Some("2026-11-08")),
    ("reference-interpreter-crate", None),
    ("wide-intermediates", Some("2026-11-10")),
    ("mechanised-model", None),
    ("roadmap-type-checker", Some("2026-11-04")),
    ("roadmap-targets", None),
    ("roadmap-scope", None),
    ("core-calculus-home", None),
    ("grammar-home", None),
    ("cranelift-simd-width", None),
    ("cranelift-float-select", None),
    ("cranelift-cf-abi", None),
    ("cranelift-optimizer-scope", None),
    ("cranelift-debug-spike", Some("2026-11-09")),
    ("debug-info", Some("2026-11-08")),
    ("error-return-spike", Some("2026-11-11")),
    ("todo-marker", Some("2026-11-08")),
    ("docs-site", Some("2026-11-08")),
    ("foreign-idiom-diagnostics", Some("2026-11-08")),
    ("language-card", Some("2026-11-08")),
    ("benchmark-difficulty", Some("2026-11-10")),
    ("gradual-verification", Some("2026-11-08")),
    ("control-language", None),
];

/// One row of a register table.
struct Entry {
    key: String,
    line: usize,
    resolution: String,
    status: String,
}

/// The byte offset just past a code span that opens at `at`, a run of backticks closed by the next
/// run of the same length, or `None` where no run closes it and the backticks are text.
fn code_span_end(bytes: &[u8], at: usize) -> Option<usize> {
    let run = |from: usize| bytes[from..].iter().take_while(|&&b| b == b'`').count();
    let open = run(at);
    let mut scan = at + open;
    while scan < bytes.len() {
        if bytes[scan] == b'`' {
            let close = run(scan);
            if close == open {
                return Some(scan + close);
            }
            scan += close;
        } else {
            scan += 1;
        }
    }
    None
}

/// The cells of a table row, read as spec-family's table scanner reads them (spec-core's
/// `tables.js`): a pipe inside a code span, or escaped with a backslash, is content; a pipe that
/// opens the row is a border; and what follows the last pipe is a cell unless the row ends with
/// one. A line without a pipe has no cells.
fn split_row(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut bounds = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'`' => at = code_span_end(bytes, at).unwrap_or(at + 1),
            b'|' if at == 0 || bytes[at - 1] != b'\\' => {
                bounds.push(at);
                at += 1;
            }
            _ => at += 1,
        }
    }
    if bounds.is_empty() {
        return Vec::new();
    }
    let lead = line.len() - line.trim_start().len();
    let mut cursor = if bounds[0] == lead {
        bounds.remove(0) + 1
    } else {
        lead
    };
    let mut cells = Vec::new();
    for bound in bounds {
        cells.push(line[cursor..bound].trim().to_string());
        cursor = bound + 1;
    }
    if !line[cursor..].trim().is_empty() {
        cells.push(line[cursor..].trim().to_string());
    }
    cells
}

/// A cell's text without emphasis or code marks, as the tools read a key or a status.
fn flatten(cell: &str) -> String {
    cell.chars()
        .filter(|c| !matches!(c, '*' | '`'))
        .collect::<String>()
        .trim()
        .trim_matches('_')
        .to_string()
}

/// The word a status opens with, in lower case, as spec-family's `statusWord` reads it: `done` of
/// `done: ...`, of `**Done**` and of `Done, as two types`.
fn status_word(status: &str) -> String {
    flatten(status)
        .split(|c: char| c.is_whitespace() || matches!(c, ':' | ';' | ',' | '('))
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// Every entry of the register's tables, and a violation for each row the rules could not read.
/// A table is a header line, a delimiter row of as many cells, and the rows under them up to a line
/// that is blank or holds no pipe, outside fenced code; a register table is one with a `Key` and a
/// `Status` column.
fn entries(register: &str) -> (Vec<Entry>, Vec<String>) {
    let mut lines: Vec<Option<&str>> = Vec::new();
    let mut fence: Option<&str> = None;
    for line in register.lines() {
        let indent = line.len() - line.trim_start().len();
        let text = line.trim_start();
        let opens = ["```", "~~~"].into_iter().find(|f| text.starts_with(f));
        match (fence, opens) {
            (None, Some(f)) if indent < 4 => {
                fence = Some(f);
                lines.push(None);
            }
            (Some(f), Some(g)) if f == g && indent < 4 => {
                fence = None;
                lines.push(None);
            }
            (Some(_), _) => lines.push(None),
            _ if text.is_empty() || indent >= 4 || text.starts_with('#') => lines.push(None),
            _ => lines.push(Some(line)),
        }
    }
    let delimiter = |cell: &String| {
        let core = cell.trim_start_matches(':').trim_end_matches(':');
        !core.is_empty() && core.chars().all(|c| c == '-')
    };
    let (mut found, mut malformed) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i + 1 < lines.len() {
        let (Some(header), Some(marks)) = (lines[i], lines[i + 1]) else {
            i += 1;
            continue;
        };
        let headers: Vec<String> = split_row(header).iter().map(|h| flatten(h)).collect();
        let marks = split_row(marks);
        if headers.is_empty() || marks.len() != headers.len() || !marks.iter().all(delimiter) {
            i += 1;
            continue;
        }
        let column = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
        let register = (column("Key"), column("Status"));
        let mut row = i + 2;
        while let Some(line) = lines.get(row).copied().flatten() {
            let cells = split_row(line);
            if cells.is_empty() {
                break;
            }
            if let (Some(key), Some(status)) = register {
                let name = flatten(&cells[key.min(cells.len() - 1)]);
                if cells.len() != headers.len() {
                    malformed.push(format!(
                        "line {}: a register row has one cell under each of the {} headers; read {} cells",
                        row + 1,
                        headers.len(),
                        cells.len()
                    ));
                } else if name.chars().any(|c| c.is_ascii_alphanumeric()) {
                    found.push(Entry {
                        key: name,
                        line: row + 1,
                        resolution: column("Proposed resolution")
                            .map(|c| cells[c].clone())
                            .unwrap_or_default(),
                        status: cells[status].clone(),
                    });
                }
            }
            row += 1;
        }
        i = row;
    }
    (found, malformed)
}

/// Whether `day` is written `YYYY-MM-DD`.
fn is_day(day: &str) -> bool {
    day.len() == 10
        && day.char_indices().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// The days an entry records in the one form, in its resolution and its status; a violation for
/// each day written in another form after `(Norman, `; and one for each `Decided (` that the form
/// does not follow, which only an entry decided on or after the rule must avoid: older entries
/// wrote `Decided (2026-11-08)` beside a day in the one form, and are history.
fn recorded_days(entry: &Entry) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut days, mut wrong, mut markers) = (Vec::new(), Vec::new(), Vec::new());
    for text in [&entry.resolution, &entry.status] {
        for (at, _) in text.match_indices(DAY_OPENS) {
            let day: String = text[at + DAY_OPENS.len()..].chars().take(10).collect();
            if is_day(&day) {
                days.push(day);
            } else {
                wrong.push(format!(
                    "`{}` (line {}) writes a day as `{}...`; the register writes `(Norman, YYYY-MM-DD`",
                    entry.key,
                    entry.line,
                    &text[at..(at + DAY_OPENS.len() + day.len()).min(text.len())]
                ));
            }
        }
        let lower = text.to_ascii_lowercase();
        for (at, _) in lower.match_indices("decided (") {
            let after = &text[at + "decided (".len()..];
            if !after.starts_with("Norman, ") {
                let shown: String = after.chars().take(20).collect();
                markers.push(format!(
                    "`{}` (line {}) records a decision as `Decided ({}...`; the register writes `Decided (Norman, YYYY-MM-DD)`",
                    entry.key, entry.line, shown
                ));
            }
        }
    }
    (days, wrong, markers)
}

/// Whether a reopen condition is one sentence that ends its cell: it ends with a full stop, and no
/// full stop before it, outside code spans, is followed by a space and a capital letter.
fn one_closing_sentence(condition: &str) -> bool {
    let bytes = condition.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'`' => at = code_span_end(bytes, at).unwrap_or(at + 1),
            b'.' if at + 2 < bytes.len()
                && bytes[at + 1] == b' '
                && bytes[at + 2].is_ascii_uppercase() =>
            {
                return false;
            }
            _ => at += 1,
        }
    }
    condition.ends_with('.')
}

/// The register's rules for decisions (`reversal-conditions` in the register), with `legacy` the
/// entries that may record no day in the one form:
/// - an entry `decided` or `done` records the day of its decision as `(Norman, YYYY-MM-DD`, a
///   `decided` one as `Decided (Norman, YYYY-MM-DD)`, and writes no day after `(Norman, ` in
///   another form; one decided on or after [`REOPEN_RULE_FROM`] writes no `Decided (` in another
///   form either;
/// - an entry decided on or after [`REOPEN_RULE_FROM`] ends its resolution with one
///   [`REOPEN_MARKER`] and the evidence that would reopen it, one sentence with nothing after it;
/// - `legacy` holds only entries that are decided or done and record no day in the one form, each
///   day it gives being before [`REOPEN_RULE_FROM`].
fn reopen_violations(register: &str, legacy: &[(&str, Option<&str>)]) -> Vec<String> {
    let (entries, mut found) = entries(register);
    let mut legacy_seen = Vec::new();
    for entry in &entries {
        let status = status_word(&entry.status);
        if !matches!(status.as_str(), "decided" | "done") {
            continue;
        }
        let old = legacy.iter().find(|(key, _)| *key == entry.key);
        let (mut days, wrong, markers) = recorded_days(entry);
        if let Some((key, day)) = old {
            legacy_seen.push(*key);
            if !days.is_empty() {
                found.push(format!(
                    "`{}` records its day in the one form now, so it leaves the list of entries that record none",
                    entry.key
                ));
            }
            days.extend(day.map(str::to_string));
        } else {
            found.extend(wrong);
            if days.is_empty() || days.iter().any(|day| day.as_str() >= REOPEN_RULE_FROM) {
                found.extend(markers);
            }
            if days.is_empty() {
                found.push(format!(
                    "`{}` (line {}) records no day of its decision: write `(Norman, YYYY-MM-DD`",
                    entry.key, entry.line
                ));
                continue;
            }
            if status == "decided" && !entry.resolution.contains("Decided (Norman, ") {
                found.push(format!(
                    "`{}` (line {}) is decided, and its resolution does not open the decision with `**Decided (Norman, YYYY-MM-DD).**`",
                    entry.key, entry.line
                ));
            }
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
                "`{}` (line {}) was decided on or after {}, and its resolution does not end with `{}` and the evidence that would reopen it",
                entry.key, entry.line, REOPEN_RULE_FROM, REOPEN_MARKER
            )),
            [at] => {
                let condition = entry.resolution[at + REOPEN_MARKER.len()..].trim();
                if !condition.chars().any(char::is_alphabetic) {
                    found.push(format!(
                        "`{}` (line {}) names no evidence after `{}`",
                        entry.key, entry.line, REOPEN_MARKER
                    ));
                } else if !one_closing_sentence(condition) {
                    found.push(format!(
                        "`{}` (line {}): the reopen condition is one sentence, and nothing follows it in the cell",
                        entry.key, entry.line
                    ));
                }
            }
            _ => found.push(format!(
                "`{}` (line {}) has `{}` {} times; a resolution ends with one condition",
                entry.key,
                entry.line,
                REOPEN_MARKER,
                marks.len()
            )),
        }
    }
    for (key, day) in legacy {
        if !legacy_seen.contains(key) {
            found.push(format!(
                "`{}` is in the list of entries that record no day, and the register holds no such entry decided or done",
                key
            ));
        }
        if day.is_some_and(|day| day >= REOPEN_RULE_FROM) {
            found.push(format!(
                "`{}` is in the list of entries before the rule, with a day on or after {}",
                key, REOPEN_RULE_FROM
            ));
        }
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
    let found = reopen_violations(&register, LEGACY);
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
    let header =
        "| Key | Problem | Proposed resolution | Status |\n| :--- | :--- | :--- | :--- |\n";
    let reads = |rows: &str, legacy: &[(&str, Option<&str>)]| {
        reopen_violations(&format!("{}{}", header, rows), legacy)
    };
    let condition = "**Reopen if:** a tenth of the repairs.";

    // What the rule lets through.
    let mut good = String::new();
    good += &row("open", "", "open");
    good += &row("proposed", "Do it.", "proposed");
    good += &row(
        "before",
        "**Decided (Norman, 2026-11-12).** Do it.",
        "decided",
    );
    good += &row(
        "with",
        &format!(
            "**Decided (Norman, 2026-11-13).** Do it. Rejected: the other. {}",
            condition
        ),
        "decided",
    );
    good += &row(
        "code",
        "**Decided (Norman, 2026-11-13).** Do `a | b` and a \\| b. **Reopen if:** none: it follows from an invariant.",
        "decided",
    );
    good += "|`tight`|A problem.|**Decided (Norman, 2026-11-20).** **Reopen if:** `x. Y` is seen.|decided|\n";
    good += &format!(
        "  {}",
        row(
            "indented",
            &format!("**Decided (Norman, 2026-11-20).** {}", condition),
            "**Decided**"
        )
    );
    good += &row("old", "Do it.", "done: (commit \"Do it\")");
    good += &row(
        "older",
        "**When decided (2026-11-08):** do it.",
        "Done: (commit \"Do it\")",
    );
    let legacy = [("old", None), ("older", Some("2026-11-08"))];
    assert_eq!(reads(&good, &legacy), Vec::<String>::new());
    let (read, malformed) = entries(&format!("{}{}", header, good));
    assert_eq!(read.len(), 9);
    assert!(malformed.is_empty());
    assert_eq!(read[5].key, "tight");
    assert_eq!(read[6].key, "indented");
    // A table inside fenced code is no register.
    let fenced = format!("```\n{}{}```\n", header, row("x", "Do it.", "decided"));
    assert_eq!(entries(&fenced).0.len(), 0);

    // What it stops, one case to a row.
    let cases = [
        (
            row("without", "**Decided (Norman, 2026-11-13).** Do it. Rejected: the other.", "decided"),
            "does not end with",
            1,
        ),
        (
            row("shouted", "**Decided (Norman, 2026-11-14).** Do it.", "**DECIDED**"),
            "does not end with",
            1,
        ),
        (row("undated", "Do it.", "decided"), "records no day", 1),
        (
            row("applied", "Do it.", "Done: [`x.md`](../spec/x.md) (Norman, 2026-11-14; commit \"Do it\")"),
            "does not end with",
            1,
        ),
        (
            row("spelled", &format!("**Decided (Norman, 20 November 2026).** {}", condition), "decided"),
            "writes a day as",
            2,
        ),
        (
            row("unnamed", &format!("**Decided (2026-11-20).** {}", condition), "decided"),
            "records a decision as",
            2,
        ),
        (
            row("empty", "**Decided (Norman, 2026-11-20).** Do it. **Reopen if:** .", "decided"),
            "names no evidence",
            1,
        ),
        (
            row("early", &format!("**Decided (Norman, 2026-11-20).** {} Rejected: the other.", condition), "decided"),
            "one sentence",
            1,
        ),
        (
            row(
                "trailing",
                "**Decided (Norman, 2026-11-20).** **Reopen if:** never. Do it. Alternatives rejected: the other.",
                "decided",
            ),
            "one sentence",
            1,
        ),
        (
            row("unended", "**Decided (Norman, 2026-11-20).** **Reopen if:** a tenth of the repairs", "decided"),
            "one sentence",
            1,
        ),
        (
            row("twice", "**Decided (Norman, 2026-11-20).** **Reopen if:** one. **Reopen if:** two.", "decided"),
            "2 times",
            1,
        ),
        ("| `short` | A problem. | decided |\n".to_string(), "read 3 cells", 1),
    ];
    // A decision on or after the rule writes no `Decided (` in another form, though it records its
    // day in the one form elsewhere; an older one may, as the register's history does.
    let mixed = |day: &str| {
        row(
            "mixed",
            &format!("**Decided ({}).** Do it. {}", day, condition),
            &format!(
                "done: [`x.md`](../spec/x.md) (Norman, {}; commit \"Do it\")",
                day
            ),
        )
    };
    let found = reads(&mixed("2026-11-20"), &[]);
    assert_eq!(found.len(), 1, "{:?}", found);
    assert!(found[0].contains("records a decision as"), "{}", found[0]);
    assert_eq!(reads(&mixed("2026-11-08"), &[]), Vec::<String>::new());
    for (rows, says, count) in &cases {
        let found = reads(rows, &[]);
        assert_eq!(found.len(), *count, "{}: {:?}", rows, found);
        assert!(found[0].contains(says), "{}: {}", rows, found[0]);
    }

    // The list of entries before the rule only shrinks, and holds only what it says.
    let found = reads(
        &row("old", "**Decided (Norman, 2026-11-10).** Do it.", "done"),
        &[("old", None)],
    );
    assert_eq!(found.len(), 1, "{:?}", found);
    assert!(found[0].contains("leaves the list"), "{}", found[0]);
    let found = reads(&row("old", "Do it.", "open"), &[("old", None)]);
    assert_eq!(found.len(), 1, "{:?}", found);
    assert!(found[0].contains("holds no such entry"), "{}", found[0]);
    let found = reads(
        &row("old", "Do it.", "done"),
        &[("old", Some("2026-11-13"))],
    );
    assert!(
        found.iter().any(|f| f.contains("before the rule")),
        "{:?}",
        found
    );
}
