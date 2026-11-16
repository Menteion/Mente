//! The record of escaped bugs (Norman, 2026-11-16): each bug that escaped its author's own checks,
//! the instrument that missed it and the guard its fix added (`docs/design-review.md`,
//! `guard-per-problem`). One file an escape, `docs/escapes/<key>.md` (Norman, 2026-11-17), so that
//! two fixes in flight never touch one file: the file's name is the escape's key, by the one rule
//! of [`crate::records`], and its front matter holds the escape's fields. The directory is read
//! here, once: by `mn escapes`, which counts the escapes per week and per area, and by the test
//! that checks every file (`tests/escapes.rs`). No count is written into a document.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::records;

/// The record's directory, from the repository's root.
pub const RECORD: &str = "docs/escapes";

/// The fields of an escape's front matter, each once, in this order.
pub const FIELDS: [&str; 5] = ["date", "fixed-by", "area", "missed-by", "guard"];

/// The areas an escape may be in, so that the counts per area are not split by a spelling.
pub const AREAS: &[&str] = &[
    "checker",
    "Tier 1",
    "interpreter",
    "MIR",
    "native",
    "generator",
    "local checks",
    "policy",
    "harness",
];

/// The address of the pull requests that fix an escape.
pub const PULLS: &str = "https://github.com/Menteion/Mente/pull/";

/// What now catches an escape's class, each found in the list of what its runner runs
/// (`evidence.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Guard {
    /// A Rust test of a package, `fn test_... in mente-interp`, that nextest runs. The package is
    /// named, since one test's name may stand in two crates.
    Test { name: String, package: String },
    /// A check of the workflow policy, `check("...")`, that the policy's run ran.
    Policy(String),
    /// A Node test, `test("...")`, that a Node run ran.
    Node(String),
    /// A quick check before a push, by its name in `tools/quick-checks.json`.
    Quick(String),
}

/// One escape of the record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escape {
    /// The escape's key, its file's name.
    pub key: String,
    /// The day the escape was found.
    pub date: (i32, u32, u32),
    /// The pull request that merged the fix.
    pub pull: u32,
    pub area: String,
    pub missed: String,
    pub guard: Guard,
}

/// Every escape in the record's directory `dir`, in the order of their keys, or each problem in
/// reading it: an entry that is no file `<key>.md` whose key the key rule admits, and a file whose
/// front matter is not the record's. Keys are unique since files' names are.
pub fn read(dir: &Path) -> Result<Vec<Escape>, Vec<String>> {
    let entries = fs::read_dir(dir).map_err(|e| vec![format!("{}: {}", dir.display(), e)])?;
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let key = name
            .strip_suffix(".md")
            .filter(|k| path.is_file() && records::is_key(k));
        let Some(key) = key else {
            problems.push(format!(
                "{}: no escape, which is a file `<key>.md` whose key is lowercase words of letters and digits joined by hyphens",
                name
            ));
            continue;
        };
        match fs::read_to_string(&path) {
            Ok(text) => match parse(key, &text) {
                Ok(row) => rows.push(row),
                Err(why) => problems.push(format!("{}: {}", name, why)),
            },
            Err(e) => problems.push(format!("{}: {}", name, e)),
        }
    }
    if problems.is_empty() {
        Ok(rows)
    } else {
        Err(problems)
    }
}

/// The escape of key `key` that a file's text records: a front matter of exactly [`FIELDS`], in
/// the one form of [`records::front_matter`], and then any prose.
pub fn parse(key: &str, text: &str) -> Result<Escape, String> {
    let values = records::front_matter(text, &FIELDS)?;
    let value = |k: usize| values[k].as_str();
    Ok(Escape {
        key: key.to_string(),
        date: day(value(0)).ok_or("a date that is no day `YYYY-MM-DD`")?,
        pull: pull_number(value(1)).ok_or(
            "a fix that is no pull request's address, `https://github.com/Menteion/Mente/pull/N`",
        )?,
        area: value(2).to_string(),
        missed: value(3).to_string(),
        guard: guard_of(value(4)).ok_or(
            "a guard that is none of `fn test_... in <package>`, `policy check \"...\"`, `node test \"...\"`, `quick check \"...\"`",
        )?,
    })
}

/// A day of the calendar, by the one rule of [`records::is_day`].
fn day(text: &str) -> Option<(i32, u32, u32)> {
    if !records::is_day(text) {
        return None;
    }
    Some((
        text[0..4].parse().ok()?,
        text[5..7].parse().ok()?,
        text[8..10].parse().ok()?,
    ))
}

/// The number of a pull request at its address: a positive number, with no zero before it.
fn pull_number(text: &str) -> Option<u32> {
    let n = text.strip_prefix(PULLS)?;
    (n.starts_with(|c: char| ('1'..='9').contains(&c)) && n.chars().all(|c| c.is_ascii_digit()))
        .then(|| n.parse().ok())
        .flatten()
}

fn guard_of(text: &str) -> Option<Guard> {
    let quoted = |prefix: &str| {
        let name = text
            .strip_prefix(prefix)?
            .strip_prefix('"')?
            .strip_suffix('"')?;
        (!name.is_empty() && !name.contains('"')).then(|| name.to_string())
    };
    if let Some(rest) = text.strip_prefix("fn ") {
        let (name, package) = rest.split_once(" in ")?;
        let ok = name.starts_with("test_")
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && package.starts_with("mente-")
            && package
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        return ok.then(|| Guard::Test {
            name: name.to_string(),
            package: package.to_string(),
        });
    }
    quoted("policy check ")
        .map(Guard::Policy)
        .or_else(|| quoted("node test ").map(Guard::Node))
        .or_else(|| quoted("quick check ").map(Guard::Quick))
}

/// Each way the escapes break the record's rules: each area is one of [`AREAS`], and each escape
/// says what missed it. A key is a file's name, so it is unique, and since no file is removed,
/// never reused. Whether each guard runs is what the runners' lists say, which `mn evidence`
/// reads (`evidence.rs`); that the fix merged is a fact of the history, which the Commits check
/// reads.
pub fn problems(rows: &[Escape]) -> Vec<String> {
    let mut out = Vec::new();
    for r in rows {
        if !AREAS.contains(&r.area.as_str()) {
            out.push(format!(
                "{}: the area `{}` is none of {}",
                r.key,
                r.area,
                AREAS.join(", ")
            ));
        }
        if r.missed.is_empty() {
            out.push(format!("{}: it names no instrument that missed it", r.key));
        }
    }
    out
}

/// The ISO week of a day, as `YYYY-Www`.
pub fn week((y, m, d): (i32, u32, u32)) -> String {
    // Days since 1970-01-01 (the civil-from-days algorithm, inverted).
    let days = |y: i32, m: u32, d: u32| -> i64 {
        let y = i64::from(if m <= 2 { y - 1 } else { y });
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = i64::from((m + 9) % 12);
        let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    };
    let n = days(y, m, d);
    // 1970-01-01 was a Thursday: Monday is 1, Sunday 7.
    let weekday = (n + 3).rem_euclid(7) + 1;
    let thursday = n + 4 - weekday;
    // The ISO year is the Thursday's year.
    let year = (y - 1..=y + 1)
        .rev()
        .find(|&yy| days(yy, 1, 1) <= thursday)
        .unwrap_or(y);
    let week = (thursday - days(year, 1, 1)) / 7 + 1;
    format!("{}-W{:02}", year, week)
}

/// The escapes per ISO week and per area, as `mn escapes` prints them.
pub fn counts(rows: &[Escape]) -> String {
    let mut weeks: BTreeMap<String, usize> = BTreeMap::new();
    let mut areas: BTreeMap<&str, usize> = BTreeMap::new();
    for r in rows {
        *weeks.entry(week(r.date)).or_default() += 1;
        *areas.entry(r.area.as_str()).or_default() += 1;
    }
    let mut out = String::from("Escapes per week:\n");
    for (w, n) in &weeks {
        out.push_str(&format!("  {}  {}\n", w, n));
    }
    out.push_str("Escapes per area:\n");
    for (a, n) in &areas {
        out.push_str(&format!("  {}  {}\n", a, n));
    }
    out
}
