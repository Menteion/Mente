//! The rules of the design register, `docs/design-review.md`, that a test can hold, as an
//! integration test of their own, apart from main.rs, which changes with every feature
//! (`docs/design-review.md`, `reversal-conditions`).

use std::fs;
use std::path::Path;

/// The day from which a decision names the evidence that would reopen it (Norman, 2026-11-13).
const REOPEN_RULE_FROM: &str = "2026-11-13";

/// The marker that opens an entry's reopen condition, the last thing in its resolution.
const REOPEN_MARKER: &str = "**Reopen if:**";

/// The one form in which an entry records the day of its decision: `(Norman, ` and a day written
/// `YYYY-MM-DD`, as in `**Decided (Norman, 2026-11-13).**`.
const DAY_OPENS: &str = "(Norman, ";

/// Every entry decided or applied before [`REOPEN_RULE_FROM`], as the register stood that day, with
/// its day: the latest it records in the one form, or the day its prose gives, or `None` where it
/// gives none. An entry decided since then records its day in the one form and is not in the list,
/// so that the reopen rule can tell when it was decided: an entry dated before the rule, or not
/// dated at all, that the list does not hold is a new decision written with an old day or none.
/// The list only shrinks: an entry that records a later day in the one form leaves it.
const BEFORE_THE_RULE: &[(&str, Option<&str>)] = &[
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
    ("error-conversion", Some("2026-11-11")),
    ("overflow-check-cost", Some("2026-11-08")),
    ("integer-conversions", Some("2026-11-08")),
    ("integer-inference", Some("2026-11-08")),
    ("no-shadowing", Some("2026-11-08")),
    ("struct-enum-syntax", Some("2026-11-11")),
    ("match-patterns", Some("2026-11-11")),
    ("match-scrutinee-narrowing", Some("2026-11-12")),
    ("match-arm-freeze", Some("2026-11-12")),
    ("optional-semantics", Some("2026-11-11")),
    ("error-union-semantics", Some("2026-11-11")),
    ("str-text-model", Some("2026-11-11")),
    ("slice-ranges", Some("2026-11-11")),
    ("list-string-m2", Some("2026-11-11")),
    ("struct-invariants-m2", Some("2026-11-11")),
    ("mut-projection-soa", None),
    ("mut-argument-type", Some("2026-11-09")),
    ("strict-defaults", Some("2026-11-09")),
    ("philosophy-polish", Some("2026-11-09")),
    ("language-evolution", Some("2026-11-09")),
    ("shared-upgrade", None),
    ("arena-concurrency-and-drop", None),
    ("parallel-exclusivity", None),
    ("layout-alignment-endianness", None),
    ("view-source-freeze", Some("2026-11-11")),
    ("view-provenance-sources", Some("2026-11-11")),
    ("read-only-views", Some("2026-11-11")),
    ("own-invalidation", Some("2026-11-11")),
    ("copy-and-drop", Some("2026-11-11")),
    ("exclusivity-projections", Some("2026-11-11")),
    ("struct-field-order", Some("2026-11-11")),
    ("enum-layout", Some("2026-11-11")),
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
    ("loop-condition-candidates", Some("2026-11-09")),
    ("loop-decreases", Some("2026-11-09")),
    ("refinement-inference", Some("2026-11-09")),
    ("codegen-placeholders", Some("2026-11-08")),
    ("implemented-evidence", Some("2026-11-08")),
    ("reference-interpreter-crate", None),
    ("interpreter-checks-contracts", Some("2026-11-09")),
    ("wide-intermediates", Some("2026-11-10")),
    ("mechanised-model", None),
    ("refine-binding-fix", Some("2026-11-10")),
    ("nesting-limit", Some("2026-11-10")),
    ("array-size-limit", Some("2026-11-10")),
    ("call-without-body", Some("2026-11-11")),
    ("tier1-variable-cap", Some("2026-11-11")),
    ("tier1-path-cap", Some("2026-11-12")),
    ("mutation-timeouts", Some("2026-11-10")),
    ("quality-per-change", Some("2026-11-10")),
    ("guard-per-problem", Some("2026-11-12")),
    ("guard-audit", Some("2026-11-12")),
    ("roadmap-type-checker", Some("2026-11-04")),
    ("roadmap-targets", None),
    ("roadmap-scope", None),
    ("roadmap-vertical-slice", Some("2026-11-08")),
    ("success-workload", Some("2026-11-08")),
    ("long-term-order", Some("2026-11-06")),
    ("foundation-gate", Some("2026-11-08")),
    ("canonical-forms", Some("2026-11-09")),
    ("foundation-milestones", Some("2026-11-08")),
    ("core-calculus-home", None),
    ("grammar-home", None),
    ("loop-exits", Some("2026-11-08")),
    ("benchmark-corpus", Some("2026-11-09")),
    ("milestone-tags", Some("2026-11-09")),
    ("dependency-approval", Some("2026-11-09")),
    ("example-bodies", Some("2026-11-09")),
    ("m2-design-closure", Some("2026-11-11")),
    ("example-01-m2", Some("2026-11-11")),
    ("cranelift-simd-width", None),
    ("cranelift-float-select", None),
    ("cranelift-cf-abi", None),
    ("cranelift-optimizer-scope", None),
    ("mir-control-flow", Some("2026-11-10")),
    ("native-runtime", Some("2026-11-10")),
    ("cranelift-debug-spike", Some("2026-11-09")),
    ("debug-info", Some("2026-11-08")),
    ("error-return-spike", Some("2026-11-11")),
    ("ai-first-purpose", Some("2026-11-08")),
    ("ai-coding-problems", Some("2026-11-09")),
    ("todo-marker", Some("2026-11-08")),
    ("unreachable-marker", Some("2026-11-09")),
    ("sympathetic-defaults", Some("2026-11-11")),
    ("cost-report", Some("2026-11-11")),
    ("docs-site", Some("2026-11-08")),
    ("foreign-idiom-diagnostics", Some("2026-11-08")),
    ("diagnostics-as-api", Some("2026-11-08")),
    ("language-card", Some("2026-11-08")),
    ("syntax-probe", Some("2026-11-09")),
    ("early-core-benchmark", Some("2026-11-09")),
    ("benchmark-difficulty", Some("2026-11-10")),
    ("gradual-verification", Some("2026-11-08")),
    ("value-copy-cost", Some("2026-11-11")),
    ("roadmap-ai-priorities", Some("2026-11-08")),
    ("control-language", None),
];

/// The decision markers the register wrote before it settled on `Decided (Norman, YYYY-MM-DD)`,
/// each pinned to its entry: a marker in another form is accepted only where it is listed here.
const HISTORIC_MARKERS: &[(&str, &str)] = &[
    ("overflow-check-cost", "decided (2026-11-08"),
    ("tier1-loops", "decided (2026-11-08"),
    ("tier1-counterexample", "decided (2026-11-08"),
    ("tier2-trusted", "decided (2026-11-08"),
    ("loop-condition-candidates", "Decided (2026-11-09"),
    ("refinement-inference", "Decided (2026-11-09"),
    ("codegen-placeholders", "decided (2026-11-08"),
    ("implemented-evidence", "decided (2026-11-08"),
    ("wide-intermediates", "Decided (2026-11-10"),
    ("refine-binding-fix", "Decided (2026-11-10"),
    ("success-workload", "decided (2026-11-08"),
    ("foundation-gate", "decided (2026-11-08"),
    ("foundation-milestones", "decided (2026-11-08"),
    ("debug-info", "decided (2026-11-08"),
    ("ai-first-purpose", "decided (2026-11-08"),
    ("todo-marker", "decided (2026-11-08"),
    ("docs-site", "decided (2026-11-08"),
    ("foreign-idiom-diagnostics", "decided (2026-11-08"),
    ("diagnostics-as-api", "decided (2026-11-08"),
    ("language-card", "decided (2026-11-08"),
    ("gradual-verification", "decided (2026-11-08"),
];

/// Words that end with a full stop inside a sentence.
const ABBREVIATIONS: &[&str] = &["i.e", "e.g", "etc", "cf", "vs"];

/// What the rules read of the register: its entries and its markers, as constants for the register
/// itself and as arguments for the planted registers that test the rules.
struct Pins<'a> {
    before: &'a [(&'a str, Option<&'a str>)],
    markers: &'a [(&'a str, &'a str)],
}

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

/// Whether `day` is a day of the calendar written `YYYY-MM-DD`.
fn is_day(day: &str) -> bool {
    let shape = day.len() == 10
        && day.char_indices().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    if !shape {
        return false;
    }
    let number = |range: std::ops::Range<usize>| day[range].parse::<u32>().unwrap_or(0);
    let (year, month, date) = (number(0..4), number(5..7), number(8..10));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=length).contains(&date)
}

/// Every day written `YYYY-MM-DD` in a text, whatever precedes it.
fn days_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len().saturating_sub(9))
        .map(|at| chars[at..at + 10].iter().collect::<String>())
        .filter(|day| is_day(day))
        .collect()
}

/// The days an entry records in the one form, in its resolution and its status, and a violation
/// for each place where it records a decision in another form: a day after `(Norman, ` that is not
/// a day of the calendar written `YYYY-MM-DD`; a `Decided (` that `Norman, ` does not follow; or
/// a bold span of the resolution that speaks of a decision or an amendment and carries no day in
/// the one form. Only
/// the historic markers pinned to the entry are let through.
fn recorded_days(entry: &Entry, markers: &[(&str, &str)]) -> (Vec<String>, Vec<String>) {
    let (mut days, mut wrong) = (Vec::new(), Vec::new());
    let pinned = |text: &str| {
        markers
            .iter()
            .any(|(key, marker)| *key == entry.key && text.contains(marker))
    };
    for text in [&entry.resolution, &entry.status] {
        for (at, _) in text.match_indices(DAY_OPENS) {
            let day: String = text[at + DAY_OPENS.len()..].chars().take(10).collect();
            if is_day(&day) {
                days.push(day);
            } else {
                wrong.push(format!(
                    "`{}` (line {}) writes a day as `{}...`; the register writes `(Norman, YYYY-MM-DD`, a day of the calendar",
                    entry.key,
                    entry.line,
                    &text[at..(at + DAY_OPENS.len() + day.len()).min(text.len())]
                ));
            }
        }
        let lower = text.to_ascii_lowercase();
        for (at, _) in lower.match_indices("decided (") {
            let marker: String = text[at..].chars().take("decided (".len() + 10).collect();
            if !text[at + "decided (".len()..].starts_with("Norman, ") && !pinned(&marker) {
                wrong.push(format!(
                    "`{}` (line {}) records a decision as `{}...`; the register writes `Decided (Norman, YYYY-MM-DD)`",
                    entry.key, entry.line, marker
                ));
            }
        }
    }
    // A status cell's emphasis, as in `**Decided**`, is its word, not a decision's marker.
    for span in entry.resolution.split("**").skip(1).step_by(2) {
        let lower = span.to_ascii_lowercase();
        let speaks = lower.contains("decided") || lower.contains("amended");
        if speaks && !span.contains(DAY_OPENS) && !pinned(span) {
            wrong.push(format!(
                "`{}` (line {}) marks a decision as `**{}**`, with no day; the register writes `Decided (Norman, YYYY-MM-DD)`",
                entry.key, entry.line, span
            ));
        }
    }
    (days, wrong)
}

/// What is wrong with a reopen condition, the text after [`REOPEN_MARKER`], or `None`. It is one
/// sentence that ends its cell and opens with a capital letter: nothing after it opens with `**`,
/// breaks the line with `<br>` or names what was rejected, and no full stop inside it, outside code
/// spans and after a word other than an abbreviation, is followed by a space and anything but a
/// lower-case letter.
fn condition_fault(condition: &str) -> Option<&'static str> {
    if !condition.chars().any(char::is_alphabetic) {
        return Some("names no evidence");
    }
    let lower = condition.to_ascii_lowercase();
    if condition.contains("**") || lower.contains("<br") || lower.contains("rejected") {
        return Some("is followed in its cell by more than the condition");
    }
    if !condition.chars().next().is_some_and(char::is_uppercase) {
        return Some("opens with a lower-case letter; a condition opens with a capital");
    }
    let bytes = condition.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'`' => at = code_span_end(bytes, at).unwrap_or(at + 1),
            b'.' => {
                let word = condition[..at]
                    .rsplit(|c: char| c.is_whitespace() || c == '(')
                    .next()
                    .unwrap_or("");
                let rest = &condition[at + 1..];
                let next = rest.trim_start().chars().next();
                let spaced = rest.starts_with(char::is_whitespace);
                if spaced
                    && next.is_some_and(|c| !c.is_lowercase())
                    && !ABBREVIATIONS.contains(&word.to_ascii_lowercase().as_str())
                {
                    return Some("is more than one sentence; nothing follows the condition");
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
    if !condition.ends_with('.') {
        return Some("does not end with a full stop");
    }
    None
}

/// The register's rules for decisions (`reversal-conditions` in the register):
/// - an entry `decided` or `done` records each decision as `(Norman, YYYY-MM-DD`, a `decided` one
///   opening it with `Decided (Norman, YYYY-MM-DD)`, and in no other form but the historic markers
///   pinned to it;
/// - an entry dated before [`REOPEN_RULE_FROM`], or not dated, is one the list of entries before
///   the rule holds, with the day the entry gives; and the list holds nothing else;
/// - an entry decided on or after [`REOPEN_RULE_FROM`] ends its resolution with one
///   [`REOPEN_MARKER`] and the evidence that would reopen it, one sentence that opens with a
///   capital letter, with nothing after it.
fn reopen_violations(register: &str, pins: &Pins) -> Vec<String> {
    let (entries, mut found) = entries(register);
    let mut seen = Vec::new();
    for entry in &entries {
        let status = status_word(&entry.status);
        if !matches!(status.as_str(), "decided" | "done") {
            continue;
        }
        let (mut days, wrong) = recorded_days(entry, pins.markers);
        found.extend(wrong);
        days.sort();
        let latest = days.last().cloned();
        let listed = pins.before.iter().find(|(key, _)| *key == entry.key);
        if let Some((key, _)) = listed {
            seen.push(*key);
        }
        let after_the_rule = latest.as_deref().is_some_and(|day| day >= REOPEN_RULE_FROM);
        match (listed, after_the_rule) {
            (Some(_), true) => found.push(format!(
                "`{}` (line {}) records a decision on or after {}, so it leaves the list of entries decided before the rule",
                entry.key, entry.line, REOPEN_RULE_FROM
            )),
            (Some((_, day)), false) => {
                let prose = days_in(&format!("{} {}", entry.resolution, entry.status));
                let agrees = match (latest.as_deref(), day) {
                    (Some(latest), Some(day)) => latest == *day,
                    (Some(_), None) => false,
                    (None, Some(day)) => prose.iter().any(|given| given == day),
                    (None, None) => prose.is_empty(),
                };
                if !agrees {
                    found.push(format!(
                        "`{}` (line {}): the list of entries before the rule gives it {:?}, and the register gives {}",
                        entry.key,
                        entry.line,
                        day,
                        latest.as_deref().map_or(
                            format!("the days {:?} in prose", prose),
                            |latest| format!("`(Norman, {}`", latest)
                        )
                    ));
                }
                continue;
            }
            (None, false) => {
                found.push(format!(
                    "`{}` (line {}) is {}, and the list of entries decided before {} does not hold it: a new decision records the day it is made, as `(Norman, YYYY-MM-DD`",
                    entry.key,
                    entry.line,
                    latest.map_or("not dated".to_string(), |day| format!("dated {}", day)),
                    REOPEN_RULE_FROM
                ));
                continue;
            }
            (None, true) => {}
        }
        if status == "decided" && !entry.resolution.contains("Decided (Norman, ") {
            found.push(format!(
                "`{}` (line {}) is decided, and its resolution does not open the decision with `**Decided (Norman, YYYY-MM-DD).**`",
                entry.key, entry.line
            ));
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
                if let Some(fault) =
                    condition_fault(entry.resolution[at + REOPEN_MARKER.len()..].trim())
                {
                    found.push(format!(
                        "`{}` (line {}): its reopen condition {}",
                        entry.key, entry.line, fault
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
    for (key, day) in pins.before {
        if !seen.contains(key) {
            found.push(format!(
                "`{}` is in the list of entries before the rule, and the register holds no such entry decided or done",
                key
            ));
        }
        if day.is_some_and(|day| !is_day(day) || day >= REOPEN_RULE_FROM) {
            found.push(format!(
                "`{}` is in the list of entries before the rule with {:?}, which is no day before {}",
                key, day, REOPEN_RULE_FROM
            ));
        }
    }
    for (key, _) in pins.markers {
        if !pins.before.iter().any(|(before, _)| before == key) {
            found.push(format!(
                "`{}` has a historic marker pinned, and is not among the entries before the rule",
                key
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
    let pins = Pins {
        before: BEFORE_THE_RULE,
        markers: HISTORIC_MARKERS,
    };
    let found = reopen_violations(&register, &pins);
    assert!(
        found.is_empty(),
        "the register's rules for decisions (docs/design-review.md, its introduction):\n{}",
        found.join("\n")
    );
}

/// Days of the calendar, and nothing else.
#[test]
fn test_a_recorded_day_is_a_day_of_the_calendar() {
    for day in ["2026-11-13", "2028-02-29", "2026-12-31", "2026-01-01"] {
        assert!(is_day(day), "{}", day);
    }
    for day in [
        "2026-19-45",
        "2026-11-31",
        "2027-02-29",
        "2026-00-10",
        "2026-11-00",
        "2026-1-130",
        "20261113",
    ] {
        assert!(!is_day(day), "{}", day);
    }
}

/// The rule itself: what it lets through and what it stops.
#[test]
fn test_the_reopen_rule_rejects_a_decision_without_its_condition() {
    let row = |key: &str, resolution: &str, status: &str| {
        format!("| `{}` | A problem. | {} | {} |\n", key, resolution, status)
    };
    let header =
        "| Key | Problem | Proposed resolution | Status |\n| :--- | :--- | :--- | :--- |\n";
    let none = Pins {
        before: &[],
        markers: &[],
    };
    let reads = |rows: &str, pins: &Pins| reopen_violations(&format!("{}{}", header, rows), pins);
    let condition = "**Reopen if:** A tenth of the repairs.";

    // What the rule lets through.
    let mut good = String::new();
    good += &row("open", "", "open");
    good += &row("proposed", "Do it.", "proposed");
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
        "**Decided (Norman, 2026-11-13).** Do `a | b` and a \\| b. **Reopen if:** None: it follows from an invariant.",
        "decided",
    );
    good += "|`tight`|A problem.|**Decided (Norman, 2026-11-20).** **Reopen if:** The `x. Y` check fails, i.e. M3's run.|decided|\n";
    good += &format!(
        "  {}",
        row(
            "indented",
            &format!("**Decided (Norman, 2026-11-20).** {}", condition),
            "**Decided**"
        )
    );
    good += &row(
        "before",
        "**Decided (Norman, 2026-11-12).** Do it.",
        "decided",
    );
    good += &row("old", "Do it.", "done: (commit \"Do it\")");
    good += &row(
        "older",
        "**When decided (2026-11-08):** do it.",
        "Done: (commit \"Do it\")",
    );
    let pins = Pins {
        before: &[
            ("before", Some("2026-11-12")),
            ("old", None),
            ("older", Some("2026-11-08")),
        ],
        markers: &[("older", "decided (2026-11-08")],
    };
    assert_eq!(reads(&good, &pins), Vec::<String>::new());
    let (read, malformed) = entries(&format!("{}{}", header, good));
    assert_eq!(read.len(), 9);
    assert!(malformed.is_empty());
    assert_eq!(read[4].key, "tight");
    assert_eq!(read[5].key, "indented");
    // A table inside fenced code is no register.
    let fenced = format!("```\n{}{}```\n", header, row("x", "Do it.", "decided"));
    assert_eq!(entries(&fenced).0.len(), 0);

    // What it stops: each case, the faults it draws, and the first fault's words.
    let after = |text: &str| {
        row(
            "after",
            &format!("**Decided (Norman, 2026-11-20).** {}", text),
            "decided",
        )
    };
    let cases = [
        (
            row(
                "without",
                "**Decided (Norman, 2026-11-13).** Do it. Rejected: the other.",
                "decided",
            ),
            1,
            "does not end with",
        ),
        (
            row(
                "shouted",
                "**Decided (Norman, 2026-11-14).** Do it.",
                "**DECIDED**",
            ),
            1,
            "does not end with",
        ),
        (row("undated", "Do it.", "decided"), 1, "not dated"),
        (
            row(
                "backdated",
                "**Decided (Norman, 2026-11-10).** Do it.",
                "decided",
            ),
            1,
            "dated 2026-11-10",
        ),
        (
            row(
                "applied",
                "Do it.",
                "Done: [`x.md`](../spec/x.md) (Norman, 2026-11-14; commit \"Do it\")",
            ),
            1,
            "does not end with",
        ),
        (
            row(
                "spelled",
                &format!("**Decided (Norman, 20 November 2026).** {}", condition),
                "decided",
            ),
            2,
            "writes a day as",
        ),
        (
            row(
                "impossible",
                &format!("**Decided (Norman, 2026-19-45).** {}", condition),
                "decided",
            ),
            2,
            "writes a day as",
        ),
        (
            row(
                "unnamed",
                &format!("**Decided (2026-11-20).** {}", condition),
                "decided",
            ),
            3,
            "records a decision as",
        ),
        (after("**Reopen if:** ."), 1, "names no evidence"),
        (
            after("**Reopen if:** a tenth of the repairs."),
            1,
            "capital",
        ),
        (
            after("**Reopen if:** A tenth. Rejected: the other."),
            1,
            "more than the condition",
        ),
        (
            after("**Reopen if:** A tenth. **Amended (x):** y."),
            2,
            "with no day",
        ),
        (
            after("**Reopen if:** A tenth.  Rejected"),
            1,
            "more than the condition",
        ),
        (
            after("**Reopen if:** A tenth. rejected"),
            1,
            "more than the condition",
        ),
        (
            after("**Reopen if:** A tenth.<br>Then"),
            1,
            "more than the condition",
        ),
        (
            after("**Reopen if:** A tenth; Rejected: x."),
            1,
            "more than the condition",
        ),
        (
            after("**Reopen if:** A tenth. [`y`](z.md) says so."),
            1,
            "more than one sentence",
        ),
        (
            after("**Reopen if:** Never. Do it."),
            1,
            "more than one sentence",
        ),
        (
            after("**Reopen if:** A tenth of the repairs"),
            1,
            "full stop",
        ),
        (
            after("**Reopen if:** One. **Reopen if:** Two."),
            1,
            "2 times",
        ),
        (
            "| `short` | A problem. | decided |\n".to_string(),
            1,
            "read 3 cells",
        ),
    ];
    for (rows, count, says) in &cases {
        let found = reads(rows, &none);
        assert_eq!(found.len(), *count, "{}: {:?}", rows, found);
        assert!(found[0].contains(says), "{}: {}", rows, found[0]);
    }

    // An entry before the rule is held to the day it gives and to the markers pinned to it.
    let old = |resolution: &str, status: &str| row("old", resolution, status);
    let undated = Pins {
        before: &[("old", None)],
        markers: &[],
    };
    let held = [
        // Amended with a day in another form, a marker in another form, or a marker with no day.
        (
            old(
                "Do it. **Decided (Norman, 20 November 2026).** Then.",
                "done",
            ),
            &undated,
            "writes a day as",
        ),
        (
            old("Do it. **Decided (2026-11-20).** Then.", "done"),
            &undated,
            "records a decision as",
        ),
        (
            old("Do it. **Amended:** then.", "done"),
            &undated,
            "with no day",
        ),
        (
            old(
                "**When decided (2026-11-08):** do it. **Decided (2026-11-20).** Then.",
                "done",
            ),
            &Pins {
                before: &[("old", Some("2026-11-08"))],
                markers: &[("old", "decided (2026-11-08")],
            },
            "records a decision as",
        ),
        // Amended with a later day in the one form: it leaves the list, and names its condition.
        (
            old(
                "**Decided (Norman, 2026-11-10).** Do it. **Decided (Norman, 2026-11-20).** Then.",
                "done",
            ),
            &Pins {
                before: &[("old", Some("2026-11-10"))],
                markers: &[],
            },
            "leaves the list",
        ),
        // A day the list gives that the register does not, or the other way about.
        (
            old("**Decided (Norman, 2026-11-10).** Do it.", "done"),
            &Pins {
                before: &[("old", Some("2026-11-08"))],
                markers: &[],
            },
            "gives it",
        ),
        (old("Decided on 2026-11-10.", "done"), &undated, "gives it"),
        (old("Do it.", "open"), &undated, "holds no such entry"),
        (
            old("Do it.", "done"),
            &Pins {
                before: &[("old", Some("2026-11-13"))],
                markers: &[],
            },
            "no day before",
        ),
    ];
    for (rows, pins, says) in &held {
        let found = reads(rows, pins);
        assert!(
            found.iter().any(|f| f.contains(says)),
            "{}: {:?}",
            rows,
            found
        );
    }
    let stray = Pins {
        before: &[],
        markers: &[("old", "decided (2026-11-08")],
    };
    assert!(reads("", &stray)[0].contains("historic marker pinned"));
}
