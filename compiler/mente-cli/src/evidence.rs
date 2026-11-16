//! What the runners run (Norman, 2026-11-17). A test stands as the guard of an escaped bug, or as
//! the evidence of a claim, only if a runner runs it. Reading the sources for a name was found to
//! leak round after round, as reading spelling always does: a `#[test]` under `#[cfg(any())]`, a
//! file no `mod` declares, a test inside another function, a macro never used, a check hidden in
//! a template string or after a regular expression. So the runners' own lists decide:
//!
//! - the tests nextest lists, [`NEXTEST`], each with whether it is ignored;
//! - the tests a Node run ran, from its report in JUnit's form ([`NODE`]), each with its file and
//!   whether it was skipped;
//! - the quick checks, by their names in `tools/quick-checks.json`, which is data, not code.
//!
//! `mn evidence` reads the lists it is given and checks every guard of the record of escaped bugs
//! and every test an `@assert-count` names as evidence in the documents spec-guard reads. A list
//! that cannot be read, or holds no test, fails the check, never passes it.

use std::path::Path;

use crate::escapes::{Escape, Guard};
use crate::json::{self, Json};

/// The command whose output is nextest's list.
pub const NEXTEST: &str =
    "cargo-nextest nextest list --workspace --run-ignored all --message-format json";

/// How a Node run writes the report of the tests it ran, beside its usual reporter.
pub const NODE: &str =
    "node --test --test-reporter=junit --test-reporter-destination=<file> <files>";

/// The documents whose `@assert-count` directives spec-guard runs, as the script `spec-guard` of
/// `package.json` names them: a pattern `dir/*.md` is the Markdown files of that directory.
pub const DOCUMENTS: &[&str] = &[
    "spec/*.md",
    "docs/*.md",
    "README.md",
    "ROADMAP.md",
    "CLAUDE.md",
];

/// The file whose tests are the workflow policy's checks.
pub const POLICY: &str = ".github/scripts/policy.test.mjs";

/// A test nextest lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustTest {
    /// Its binary, `mente-cli`, `mente-cli::bin/mn` or `mente-cli::escapes`.
    pub binary: String,
    /// Its package.
    pub package: String,
    /// Its path in the binary, `tier1::tests::test_x`.
    pub path: String,
    pub ignored: bool,
}

/// A test a Node run ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeTest {
    /// Its file, with `/` between names.
    pub file: String,
    pub name: String,
    pub skipped: bool,
}

/// What the runners run, as their lists say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Runs {
    pub rust: Vec<RustTest>,
    pub node: Vec<NodeTest>,
    pub quick: Vec<String>,
}

fn field<'a>(fields: &'a [(String, Json)], name: &str) -> Option<&'a Json> {
    fields.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// The tests of nextest's list, the output of [`NEXTEST`].
pub fn nextest(text: &str) -> Result<Vec<RustTest>, String> {
    let Json::Object(top) = json::parse(text)? else {
        return Err("nextest's list is no JSON object".to_string());
    };
    let Some(Json::Object(suites)) = field(&top, "rust-suites") else {
        return Err("nextest's list holds no `rust-suites`".to_string());
    };
    let mut out = Vec::new();
    for (id, suite) in suites {
        let Json::Object(suite) = suite else {
            return Err(format!("{}: no suite", id));
        };
        let (Some(Json::Str(package)), Some(Json::Str(binary)), Some(Json::Object(cases))) = (
            field(suite, "package-name"),
            field(suite, "binary-id"),
            field(suite, "testcases"),
        ) else {
            return Err(format!(
                "{}: a suite with no `package-name`, `binary-id` or `testcases`",
                id
            ));
        };
        for (path, case) in cases {
            let Some(Json::Bool(ignored)) = (match case {
                Json::Object(case) => field(case, "ignored"),
                _ => None,
            }) else {
                return Err(format!("{} {}: a test with no `ignored`", binary, path));
            };
            out.push(RustTest {
                binary: binary.clone(),
                package: package.clone(),
                path: path.clone(),
                ignored: *ignored,
            });
        }
    }
    if out.is_empty() {
        return Err("nextest's list holds no test".to_string());
    }
    Ok(out)
}

/// `text` with XML's entities read: `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` and the numeric ones.
fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let end = rest[at..]
            .find(';')
            .ok_or_else(|| format!("an entity that does not end: {}", &rest[at..]))?;
        let entity = &rest[at + 1..at + end];
        let c = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .map(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').map(|d| d.parse().ok()))
                .flatten()
                .and_then(char::from_u32),
        };
        out.push(c.ok_or_else(|| format!("an entity XML has not: &{};", entity))?);
        rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The value of attribute `name` in the attributes of a tag, `name="..."`.
fn attribute(tag: &str, name: &str) -> Result<Option<String>, String> {
    let mut rest = tag;
    while let Some(at) = rest.find('=') {
        let key = rest[..at]
            .rsplit(|c: char| c.is_whitespace())
            .next()
            .unwrap_or_default();
        let value = &rest[at + 1..];
        let Some(quote) = value.chars().next().filter(|q| *q == '"' || *q == '\'') else {
            return Err(format!("an attribute whose value has no quotes: {}", key));
        };
        let close = value[1..]
            .find(quote)
            .ok_or_else(|| format!("an attribute that does not close: {}", key))?;
        if key == name {
            return unescape(&value[1..1 + close]).map(Some);
        }
        rest = &value[1 + close + 1..];
    }
    Ok(None)
}

/// The tests of a Node run's report in JUnit's form ([`NODE`]): each `<testcase>` with its `name`
/// and `file`, skipped where it holds `<skipped`.
pub fn junit(text: &str) -> Result<Vec<NodeTest>, String> {
    if !text.contains("<testsuites") {
        return Err("no report of Node's tests in JUnit's form: no `<testsuites>`".to_string());
    }
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<testcase") {
        let after = &rest[at + "<testcase".len()..];
        if !after.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            rest = after;
            continue;
        }
        let close = after.find('>').ok_or("a `<testcase` that does not close")?;
        let (tag, closed) = match after[..close].strip_suffix('/') {
            Some(tag) => (tag, true),
            None => (&after[..close], false),
        };
        let body_from = close + 1;
        let (body, next) = if closed {
            ("", body_from)
        } else {
            let end = after[body_from..]
                .find("</testcase>")
                .ok_or("a `<testcase>` with no `</testcase>`")?;
            (
                &after[body_from..body_from + end],
                body_from + end + "</testcase>".len(),
            )
        };
        let (Some(name), Some(file)) = (attribute(tag, "name")?, attribute(tag, "file")?) else {
            return Err(format!(
                "a `<testcase` with no `name` or `file`: {}",
                tag.trim()
            ));
        };
        out.push(NodeTest {
            file: file.replace('\\', "/"),
            name,
            skipped: body.contains("<skipped"),
        });
        rest = &after[next..];
    }
    if out.is_empty() {
        return Err("a Node report that holds no test".to_string());
    }
    Ok(out)
}

/// The names of the quick checks of `tools/quick-checks.json`.
pub fn quick_checks(text: &str) -> Result<Vec<String>, String> {
    let Json::Object(top) = json::parse(text)? else {
        return Err("tools/quick-checks.json is no JSON object".to_string());
    };
    let Some(Json::Array(checks)) = field(&top, "checks") else {
        return Err("tools/quick-checks.json holds no `checks`".to_string());
    };
    checks
        .iter()
        .map(|c| match c {
            Json::Object(c) => match field(c, "name") {
                Some(Json::Str(name)) => Ok(name.clone()),
                _ => Err("a quick check with no `name`".to_string()),
            },
            _ => Err("a quick check that is no object".to_string()),
        })
        .collect()
}

/// Whether `path`, a test's path in its binary, names the test `name`: its last segment.
fn names(path: &str, name: &str) -> bool {
    path.rsplit("::").next() == Some(name)
}

impl Runs {
    /// Whether nextest lists a test `name` that is not ignored, in any binary.
    pub fn runs_rust(&self, name: &str) -> bool {
        self.rust.iter().any(|t| !t.ignored && names(&t.path, name))
    }

    /// Whether nextest lists a test `name` from the file `target`, not ignored: a file of a
    /// package's `tests/` is the binary of its name, and a file of its `src/` is a module of its
    /// library or of a binary, `src/tier1/smt.rs` the tests under `tier1::smt::`. A target nextest
    /// builds no test from is an error.
    pub fn runs_rust_in(&self, target: &str, name: &str) -> Result<bool, String> {
        let no = || format!("{}: a file nextest builds no test from", target);
        let rest = target.strip_prefix("compiler/").ok_or_else(no)?;
        let (package, rest) = rest.split_once('/').ok_or_else(no)?;
        let rest = rest.strip_suffix(".rs").ok_or_else(no)?;
        let in_binary: Box<dyn Fn(&RustTest) -> bool> =
            if let Some(stem) = rest.strip_prefix("tests/") {
                if stem.contains('/') {
                    return Err(no());
                }
                let binary = format!("{}::{}", package, stem);
                Box::new(move |t: &RustTest| t.binary == binary)
            } else if let Some(module) = rest.strip_prefix("src/") {
                let module = module.strip_suffix("/mod").unwrap_or(module);
                let library = package.to_string();
                let bins = format!("{}::bin/", package);
                match module {
                    "lib" => Box::new(move |t: &RustTest| t.binary == library),
                    "main" => Box::new(move |t: &RustTest| t.binary.starts_with(&bins)),
                    _ => {
                        let prefix = format!("{}::", module.replace('/', "::"));
                        Box::new(move |t: &RustTest| {
                            (t.binary == library || t.binary.starts_with(&bins))
                                && t.path.starts_with(&prefix)
                        })
                    }
                }
            } else {
                return Err(no());
            };
        Ok(self
            .rust
            .iter()
            .any(|t| !t.ignored && t.package == package && names(&t.path, name) && in_binary(t)))
    }

    /// Whether the workflow policy's run ran a check `name`, not skipped.
    pub fn runs_policy(&self, name: &str) -> bool {
        self.node
            .iter()
            .any(|t| !t.skipped && t.name == name && t.file.ends_with(&format!("/{}", POLICY)))
    }

    /// Whether a Node run ran a test `name`, not skipped.
    pub fn runs_node(&self, name: &str) -> bool {
        self.node.iter().any(|t| !t.skipped && t.name == name)
    }
}

/// Each guard of `rows` that no runner runs, by its escape's key.
pub fn guards(rows: &[Escape], runs: &Runs) -> Vec<String> {
    let mut out = Vec::new();
    for r in rows {
        let (runs_it, what) = match &r.guard {
            Guard::Test(name) => (
                runs.runs_rust(name),
                format!("`fn {}` is no test nextest lists, or it is ignored", name),
            ),
            Guard::Policy(name) => (
                runs.runs_policy(name),
                format!(
                    "policy check {:?} is no check the workflow policy's run ran",
                    name
                ),
            ),
            Guard::Node(name) => (
                runs.runs_node(name),
                format!("node test {:?} is no test a Node run ran", name),
            ),
            Guard::Quick(name) => (
                runs.quick.iter().any(|q| q == name),
                format!(
                    "quick check {:?} is no check of tools/quick-checks.json",
                    name
                ),
            ),
        };
        if !runs_it {
            out.push(format!("{}: its guard {}", r.key, what));
        }
    }
    out
}

/// Each `@assert-count` of `text` whose symbol names a test, `fn test_...`, by its line, its target
/// and the test's name. Only a directive that stands on a line of its own, outside fenced code, is
/// one spec-guard runs.
pub fn directives(text: &str) -> Vec<(usize, String, String)> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || !line.starts_with("<!-- @assert-count") {
            continue;
        }
        let (Ok(Some(target)), Ok(Some(symbol))) =
            (attribute(line, "target"), attribute(line, "symbol"))
        else {
            continue;
        };
        if let Some(rest) = symbol.strip_prefix("fn test_") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            out.push((n + 1, target, format!("test_{}", name)));
        }
    }
    out
}

/// The files of [`DOCUMENTS`] under `root`, by their paths from it.
pub fn documents(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for pattern in DOCUMENTS {
        match pattern.strip_suffix("/*.md") {
            Some(dir) => {
                let mut found: Vec<String> = std::fs::read_dir(root.join(dir))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| n.ends_with(".md"))
                    .map(|n| format!("{}/{}", dir, n))
                    .collect();
                found.sort();
                out.extend(found);
            }
            None => out.push(pattern.to_string()),
        }
    }
    out
}

/// Each test that a document's `@assert-count` names as evidence and nextest does not run from
/// its target, by the document, its line and the test; `documents` holds each document's path and
/// text.
pub fn evidence(documents: &[(String, String)], runs: &Runs) -> Vec<String> {
    let mut out = Vec::new();
    for (path, text) in documents {
        for (line, target, name) in directives(text) {
            match runs.runs_rust_in(&target, &name) {
                Ok(true) => {}
                Ok(false) => out.push(format!(
                    "{}:{}: `fn {}`, the evidence it names in {}, is no test nextest runs from there, or it is ignored",
                    path, line, name, target
                )),
                Err(why) => out.push(format!("{}:{}: {}", path, line, why)),
            }
        }
    }
    out
}
