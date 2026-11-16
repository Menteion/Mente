//! The record of escaped bugs, `docs/escapes.md` (Norman, 2026-11-16): each bug that escaped its
//! author's own checks, the instrument that missed it and the guard its fix added
//! (`docs/design-review.md`, `guard-per-problem`). The table is read here, once: by `mn escapes`,
//! which counts the escapes per week and per area, and by the test that checks every row
//! (`tests/escapes.rs`), which includes this file. No count is written into a document.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The record, from the repository's root.
pub const RECORD: &str = "docs/escapes.md";

/// The table's header, exactly.
pub const HEADER: &str = "| Id | Date | Fixed by | Area | Missed by | Guard |";

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

/// The pull requests a row links to.
const PULLS: &str = "https://github.com/Menteion/Mente/pull/";

/// What now catches an escape's class, each found where it is declared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Guard {
    /// A Rust test, `fn test_...`, in `compiler/` or `spikes/`.
    Test(String),
    /// A check of the workflow policy, `check("...")`, in `.github/scripts/`.
    Policy(String),
    /// A Node test, `test("...")`, in `benchmark/`, `tools/`, `site/` or `.github/scripts/`.
    Node(String),
    /// A quick check before a push, by its name in `tools/quick-checks.json`.
    Quick(String),
}

/// One row of the record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escape {
    pub id: String,
    /// The day the fix merged.
    pub date: (i32, u32, u32),
    /// The pull request that merged the fix.
    pub pull: u32,
    pub area: String,
    pub missed: String,
    pub guard: Guard,
}

/// The rows of the record's table, in order, or each line that cannot be read as one.
pub fn parse(text: &str) -> Result<Vec<Escape>, Vec<String>> {
    let mut lines = text.lines().skip_while(|l| l.trim() != HEADER);
    if lines.next().is_none() {
        return Err(vec![format!("{} has no table headed `{}`", RECORD, HEADER)]);
    }
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    for (k, line) in lines.skip(1).take_while(|l| l.starts_with('|')).enumerate() {
        match row(line) {
            Ok(r) => rows.push(r),
            Err(why) => problems.push(format!("row {}: {}: {}", k + 1, why, line)),
        }
    }
    if problems.is_empty() {
        Ok(rows)
    } else {
        Err(problems)
    }
}

fn row(line: &str) -> Result<Escape, String> {
    let cells: Vec<&str> = line
        .trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect();
    let [id, date, pull, area, missed, guard] = cells[..] else {
        return Err(format!("{} cells, not 6", cells.len()));
    };
    Ok(Escape {
        id: id.trim_matches('`').to_string(),
        date: day(date).ok_or("a date that is no day `YYYY-MM-DD`")?,
        pull: pull_number(pull).ok_or("a fix that is no link `[#N](...pull/N)`")?,
        area: area.to_string(),
        missed: missed.to_string(),
        guard: guard_of(guard).ok_or(
            "a guard that is none of `fn test_...`, policy check \"...\", node test \"...\", quick check \"...\"",
        )?,
    })
}

fn day(text: &str) -> Option<(i32, u32, u32)> {
    let mut parts = text.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=12)
        .contains(&m)
        .then_some(())
        .filter(|()| (1..=days[(m - 1) as usize]).contains(&d))
        .map(|()| (y, m, d))
}

/// `[#N](https://github.com/Menteion/Mente/pull/N)`, the same `N` twice.
fn pull_number(text: &str) -> Option<u32> {
    let rest = text.strip_prefix("[#")?;
    let (n, link) = rest.split_once("](")?;
    let link = link.strip_suffix(')')?.strip_prefix(PULLS)?;
    (n == link).then(|| n.parse().ok()).flatten()
}

fn guard_of(text: &str) -> Option<Guard> {
    let quoted = |prefix: &str| {
        let name = text
            .strip_prefix(prefix)?
            .strip_prefix('"')?
            .strip_suffix('"')?;
        (!name.is_empty() && !name.contains('"')).then(|| name.to_string())
    };
    if let Some(name) = text.strip_prefix("`fn ").and_then(|t| t.strip_suffix('`')) {
        let ok = name.starts_with("test_")
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        return ok.then(|| Guard::Test(name.to_string()));
    }
    quoted("policy check ")
        .map(Guard::Policy)
        .or_else(|| quoted("node test ").map(Guard::Node))
        .or_else(|| quoted("quick check ").map(Guard::Quick))
}

/// Each way the rows break the record's rules, with the repository at `root`: each id is the next
/// of `ESC-001`, `ESC-002`, ..., so that ids are unique and none is reused; the days never go
/// back; each area is one of [`AREAS`]; each row says what missed it; and each guard stands where
/// it is declared, so that deleting it fails the record. That the fix merged is a fact of the
/// history, which the Commits check reads (`Escaped: <id>`); this reads the tree.
pub fn problems(root: &Path, rows: &[Escape]) -> Vec<String> {
    let sources = Sources::read(root);
    let mut out = Vec::new();
    let mut last = (0, 0, 0);
    for (k, r) in rows.iter().enumerate() {
        let expected = format!("ESC-{:03}", k + 1);
        if r.id != expected {
            out.push(format!("{}: the id of row {} is {}", r.id, k + 1, expected));
        }
        if r.date < last {
            out.push(format!("{}: its day comes before the row above's", r.id));
        }
        last = r.date;
        if !AREAS.contains(&r.area.as_str()) {
            out.push(format!(
                "{}: the area `{}` is none of {}",
                r.id,
                r.area,
                AREAS.join(", ")
            ));
        }
        if r.missed.is_empty() {
            out.push(format!("{}: it names no instrument that missed it", r.id));
        }
        if !sources.holds(&r.guard) {
            out.push(format!(
                "{}: its guard {:?} is declared nowhere",
                r.id, r.guard
            ));
        }
    }
    out
}

/// The files a guard may be declared in, read once.
struct Sources {
    rust: Vec<String>,
    policy: Vec<String>,
    node: Vec<String>,
    quick: String,
}

impl Sources {
    fn read(root: &Path) -> Sources {
        let files = |dirs: &[&str], exts: &[&str]| {
            let mut out = Vec::new();
            for d in dirs {
                collect(&root.join(d), exts, &mut out);
            }
            out
        };
        Sources {
            rust: files(&["compiler", "spikes"], &["rs"]),
            policy: files(&[".github/scripts"], &["mjs"]),
            node: files(
                &["benchmark", "tools", "site", ".github/scripts"],
                &["mjs", "ts"],
            ),
            quick: fs::read_to_string(root.join("tools/quick-checks.json")).unwrap_or_default(),
        }
    }

    fn holds(&self, guard: &Guard) -> bool {
        let any = |texts: &[String], needle: &str| texts.iter().any(|t| t.contains(needle));
        match guard {
            Guard::Test(name) => any(&self.rust, &format!("fn {}(", name)),
            Guard::Policy(name) => any(&self.policy, &format!("check(\"{}\"", name)),
            Guard::Node(name) => any(&self.node, &format!("test(\"{}\"", name)),
            Guard::Quick(name) => self.quick.contains(&format!("\"name\": \"{}\"", name)),
        }
    }
}

/// The text of each file under `dir` with one of `exts`, but none under `target` or
/// `node_modules`.
fn collect(dir: &Path, exts: &[&str], out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name != "target" && name != "node_modules" {
                collect(&path, exts, out);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.extend(fs::read_to_string(&path).ok());
        }
    }
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
