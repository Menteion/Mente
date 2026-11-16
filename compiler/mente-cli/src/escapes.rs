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
            "a guard that is none of `fn test_...`, `policy check \"...\"`, `node test \"...\"`, `quick check \"...\"`",
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
    if let Some(name) = text.strip_prefix("fn ") {
        let ok = name.starts_with("test_")
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        return ok.then(|| Guard::Test(name.to_string()));
    }
    quoted("policy check ")
        .map(Guard::Policy)
        .or_else(|| quoted("node test ").map(Guard::Node))
        .or_else(|| quoted("quick check ").map(Guard::Quick))
}

/// Each way the escapes break the record's rules, with the repository at `root`: each area is one
/// of [`AREAS`]; each escape says what missed it; and each guard stands where it is declared, so
/// that deleting it fails the record. A key is a file's name, so it is unique, and since no file
/// is removed, never reused. That the fix merged is a fact of the history, which the Commits check
/// reads; this reads the tree.
pub fn problems(root: &Path, rows: &[Escape]) -> Vec<String> {
    let sources = match Sources::read(root) {
        Ok(sources) => sources,
        Err(why) => return vec![format!("the guards cannot be read: {}", why)],
    };
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
        if !sources.holds(&r.guard) {
            out.push(format!(
                "{}: its guard {:?} is declared nowhere",
                r.key, r.guard
            ));
        }
    }
    out
}

/// The code a guard may be declared in, read once from the files git tracks: the Rust sources of
/// `compiler/` with their comments and literals taken out, the JavaScript and TypeScript of the
/// workflow policy and of the Node tests with their comments taken out, and the quick checks' map.
/// A name in a comment, in an untracked copy or in a spike declares no guard.
struct Sources {
    rust: Vec<String>,
    policy: Vec<String>,
    node: Vec<String>,
    quick: String,
}

impl Sources {
    fn read(root: &Path) -> Result<Sources, String> {
        let files = tracked(root)?;
        let read = |dirs: &[&str], exts: &[&str], code: fn(&str) -> String| -> Vec<String> {
            files
                .iter()
                .filter(|f| dirs.iter().any(|d| f.starts_with(&format!("{}/", d))))
                .filter(|f| {
                    Path::new(f)
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| exts.contains(&e))
                })
                .filter_map(|f| fs::read_to_string(root.join(f)).ok())
                .map(|text| code(&text))
                .collect()
        };
        Ok(Sources {
            rust: read(&["compiler"], &["rs"], rust_code),
            policy: read(&[".github/scripts"], &["mjs"], js_code),
            node: read(
                &["benchmark", "tools", "site", ".github/scripts"],
                &["mjs", "ts"],
                js_code,
            ),
            quick: fs::read_to_string(root.join("tools/quick-checks.json")).unwrap_or_default(),
        })
    }

    fn holds(&self, guard: &Guard) -> bool {
        let any = |texts: &[String], needle: &str| texts.iter().any(|t| t.contains(needle));
        match guard {
            Guard::Test(name) => self.rust.iter().any(|code| runs_test(code, name)),
            Guard::Policy(name) => any(&self.policy, &format!("check(\"{}\"", name)),
            Guard::Node(name) => any(&self.node, &format!("test(\"{}\"", name)),
            Guard::Quick(name) => self.quick.contains(&format!("\"name\": \"{}\"", name)),
        }
    }
}

/// The files git tracks in the repository at `root`, by their paths from it, `/` between names.
/// In CI's test sandbox `.git` is an empty directory, so that a test cannot read the history; there
/// the checkout is the tracked tree, and its files are read, but those a build or an install
/// leaves under `target` and `node_modules`. That is sound because CI's checkout is fresh: nothing
/// untracked stands in it but what its own steps build and install, under the two skipped. Anywhere else git is asked, with no program of the
/// repository's configuration run, and a git that cannot answer fails the check.
pub fn tracked(root: &Path) -> Result<Vec<String>, String> {
    let git = root.join(".git");
    if git.is_dir() && fs::read_dir(&git).is_ok_and(|mut d| d.next().is_none()) {
        let mut out = Vec::new();
        walk(root, "", &mut out);
        return Ok(out);
    }
    let output = std::process::Command::new("git")
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "ls-files",
            "-z",
        ])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .current_dir(root)
        .output()
        .map_err(|e| format!("git ls-files: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect())
}

/// Each file under `dir`, by its path from the root after `prefix`, but none under `.git`, `target`
/// or `node_modules`.
fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = format!("{}{}", prefix, name);
        if entry.path().is_dir() {
            if name != ".git" && name != "target" && name != "node_modules" {
                walk(&entry.path(), &format!("{}/", path), out);
            }
        } else {
            out.push(path);
        }
    }
}

/// Whether `code`, a Rust source as [`rust_code`] leaves it, declares a test `name` that runs: a
/// function `fn name(` under `#[test]` and under no attribute that ignores it.
fn runs_test(code: &str, name: &str) -> bool {
    let needle = format!("fn {}(", name);
    code.match_indices(&needle).any(|(at, _)| {
        let before = &code[..at];
        if before.ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            return false;
        }
        let attributes = attributes_before(before);
        attributes.iter().any(|a| a == "test") && !attributes.iter().any(|a| a.contains("ignore"))
    })
}

/// The attributes written just before the end of `code`, each without its `#[`, its `]` and its
/// spaces, over the words `pub`, `async`, `unsafe` and `const` that may stand between them and
/// the function.
fn attributes_before(code: &str) -> Vec<String> {
    let mut rest = code.trim_end();
    let mut out = Vec::new();
    loop {
        if let Some(word) = ["pub", "async", "unsafe", "const"].into_iter().find(|w| {
            rest.ends_with(w)
                && !rest[..rest.len() - w.len()]
                    .ends_with(|c: char| c.is_alphanumeric() || c == '_')
        }) {
            rest = rest[..rest.len() - word.len()].trim_end();
            continue;
        }
        if !rest.ends_with(']') {
            return out;
        }
        let mut depth = 0usize;
        let mut open = None;
        for (k, c) in rest.char_indices().rev() {
            match c {
                ']' => depth += 1,
                '[' => {
                    depth -= 1;
                    if depth == 0 {
                        open = Some(k);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(open) = open.filter(|&k| rest[..k].ends_with('#')) else {
            return out;
        };
        out.push(
            rest[open + 1..rest.len() - 1]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect(),
        );
        rest = rest[..open - 1].trim_end();
    }
}

fn starts(c: &[char], k: usize, s: &str) -> bool {
    s.chars().enumerate().all(|(j, x)| c.get(k + j) == Some(&x))
}

fn identifier(c: Option<&char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || *c == '_')
}

/// A Rust source as its tokens read: each comment, block comments nested, made one space, and
/// each string, raw string, byte string or character literal made `""`, so that a name written in
/// one is no code. A lifetime stays as it is.
pub fn rust_code(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut k = 0;
    while k < c.len() {
        if starts(&c, k, "//") {
            while k < c.len() && c[k] != '\n' {
                k += 1;
            }
            out.push(' ');
            continue;
        }
        if starts(&c, k, "/*") {
            let mut depth = 0usize;
            while k < c.len() {
                if starts(&c, k, "/*") {
                    depth += 1;
                    k += 2;
                } else if starts(&c, k, "*/") {
                    depth -= 1;
                    k += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    k += 1;
                }
            }
            out.push(' ');
            continue;
        }
        let token = k == 0 || !identifier(c.get(k - 1));
        let prefix = usize::from(token && matches!(c[k], 'b' | 'c'));
        if token && c.get(k + prefix) == Some(&'r') {
            let hashes = c[k + prefix + 1..]
                .iter()
                .take_while(|&&h| h == '#')
                .count();
            if c.get(k + prefix + 1 + hashes) == Some(&'"') {
                let close: String = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                let mut j = k + prefix + 2 + hashes;
                while j < c.len() && !starts(&c, j, &close) {
                    j += 1;
                }
                k = (j + close.chars().count()).min(c.len());
                out.push_str("\"\"");
                continue;
            }
        }
        if c.get(k + prefix) == Some(&'"') && (prefix == 0 || token) {
            let mut j = k + prefix + 1;
            while j < c.len() && c[j] != '"' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            k = (j + 1).min(c.len());
            out.push_str("\"\"");
            continue;
        }
        if c[k] == '\'' && (c.get(k + 1) == Some(&'\\') || c.get(k + 2) == Some(&'\'')) {
            let mut j = k + 1;
            while j < c.len() && c[j] != '\'' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            k = (j + 1).min(c.len());
            out.push_str("\"\"");
            continue;
        }
        out.push(c[k]);
        k += 1;
    }
    out
}

/// A JavaScript or TypeScript source with each comment made one space. Its strings stay, since a
/// test is named by one. A `/` that starts a regular expression, as the token before it says, is
/// read as one, so that a `//` in it starts no comment.
pub fn js_code(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut k = 0;
    while k < c.len() {
        if starts(&c, k, "//") {
            while k < c.len() && c[k] != '\n' {
                k += 1;
            }
            out.push(' ');
            continue;
        }
        if starts(&c, k, "/*") {
            k += 2;
            while k < c.len() && !starts(&c, k, "*/") {
                k += 1;
            }
            k = (k + 2).min(c.len());
            out.push(' ');
            continue;
        }
        let quote = matches!(c[k], '"' | '\'' | '`');
        let regex = c[k] == '/' && {
            let before = out.trim_end();
            before.is_empty()
                || before.ends_with(|p: char| "(,=:[!&|?{};+-*%<>~^".contains(p))
                || [
                    "return", "typeof", "case", "in", "of", "delete", "void", "throw", "new",
                ]
                .iter()
                .any(|w| {
                    before.ends_with(w)
                        && !before[..before.len() - w.len()]
                            .ends_with(|p: char| p.is_alphanumeric() || p == '_' || p == '$')
                })
        };
        if quote || regex {
            let end = c[k];
            let mut j = k + 1;
            let mut class = false;
            while j < c.len() && (c[j] != end || class) {
                if c[j] == '\\' {
                    j += 1;
                } else if regex && c[j] == '[' {
                    class = true;
                } else if regex && c[j] == ']' {
                    class = false;
                }
                j += 1;
            }
            let j = (j + 1).min(c.len());
            out.extend(&c[k..j]);
            k = j;
            continue;
        }
        out.push(c[k]);
        k += 1;
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
