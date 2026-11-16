//! What the runners run (Norman, 2026-11-17). A test stands as the guard of an escaped bug, or as
//! the evidence of a claim, only if a runner runs it. Reading the sources for a name was found to
//! leak round after round, as reading spelling always does, and so does reading another tool's
//! format with a second parser. So every input here is the tools' own output, in JSON, read by the
//! one JSON reader (`json.rs`):
//!
//! - the tests nextest lists, [`NEXTEST`], each with whether it is ignored and whether the profile
//!   CI runs takes it;
//! - the tests a Node run ran, as our own reporter writes them ([`NODE`]): one JSON object a test,
//!   with its name, the suites around it, its file, and whether it was skipped or left to do;
//! - the directives spec-guard runs, as spec-guard itself reports them ([`SPEC_GUARD`]);
//! - the quick checks, by their names in `tools/quick-checks.json`, which is data, not code.
//!
//! `mn evidence` checks every guard of the record of escaped bugs, and every test an
//! `@assert-count` names as evidence, against them. A list that cannot be read, or holds nothing,
//! fails the check, never passes it.

use std::path::Path;

use crate::escapes::{Escape, Guard};
use crate::json::{self, Json};

/// The command whose output is nextest's list, as CI's profile takes the tests.
pub const NEXTEST: &str =
    "cargo-nextest nextest list --profile ci --workspace --run-ignored all --message-format json";

/// How a Node run writes the tests it ran, beside its usual reporter.
pub const NODE: &str = "node --test --test-reporter=./tools/evidence-reporter.mjs --test-reporter-destination=<file> <files>";

/// The command whose output is spec-guard's report of the directives it runs.
pub const SPEC_GUARD: &str = "npm run -s spec-guard -- --json";

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
    /// Whether the profile the list was taken with runs it: its `filter-match` says `matches`.
    pub matches: bool,
}

/// A test a Node run ran, as `tools/evidence-reporter.mjs` writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeTest {
    /// Its file, with `/` between names.
    pub file: String,
    /// The suites around it, outermost first.
    pub suite: Vec<String>,
    pub name: String,
    pub skipped: bool,
    pub todo: bool,
}

/// A test an `@assert-count` names as evidence, as spec-guard reports the directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    /// The document and the line of the directive.
    pub document: String,
    pub line: usize,
    /// The file the directive counts in.
    pub target: String,
    /// The test, `test_...`.
    pub name: String,
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

fn string(fields: &[(String, Json)], name: &str) -> Option<String> {
    match field(fields, name) {
        Some(Json::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn boolean(fields: &[(String, Json)], name: &str) -> Option<bool> {
    match field(fields, name) {
        Some(Json::Bool(b)) => Some(*b),
        _ => None,
    }
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
        let (Some(package), Some(binary), Some(Json::Object(cases))) = (
            string(suite, "package-name"),
            string(suite, "binary-id"),
            field(suite, "testcases"),
        ) else {
            return Err(format!(
                "{}: a suite with no `package-name`, `binary-id` or `testcases`",
                id
            ));
        };
        for (path, case) in cases {
            let Json::Object(case) = case else {
                return Err(format!("{} {}: a test that is no object", binary, path));
            };
            let Some(ignored) = boolean(case, "ignored") else {
                return Err(format!("{} {}: a test with no `ignored`", binary, path));
            };
            let Some(Json::Object(filter)) = field(case, "filter-match") else {
                return Err(format!(
                    "{} {}: a test with no `filter-match`",
                    binary, path
                ));
            };
            let Some(status) = string(filter, "status") else {
                return Err(format!(
                    "{} {}: a `filter-match` with no `status`",
                    binary, path
                ));
            };
            out.push(RustTest {
                binary: binary.clone(),
                package: package.clone(),
                path: path.clone(),
                ignored,
                matches: status == "matches",
            });
        }
    }
    if out.is_empty() {
        return Err("nextest's list holds no test".to_string());
    }
    Ok(out)
}

/// The tests of a Node run's report, as `tools/evidence-reporter.mjs` writes it: one JSON object a
/// line, `{"name", "suite", "file", "skipped", "todo"}`.
pub fn node_report(text: &str) -> Result<Vec<NodeTest>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Json::Object(fields) =
            json::parse(line).map_err(|e| format!("line {}: {}", n + 1, e))?
        else {
            return Err(format!("line {}: no JSON object", n + 1));
        };
        let suite = match field(&fields, "suite") {
            Some(Json::Array(items)) => items
                .iter()
                .map(|i| match i {
                    Json::Str(s) => Ok(s.clone()),
                    _ => Err(format!("line {}: a suite that is no string", n + 1)),
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err(format!("line {}: no `suite`", n + 1)),
        };
        let (Some(name), Some(file), Some(skipped), Some(todo)) = (
            string(&fields, "name"),
            string(&fields, "file"),
            boolean(&fields, "skipped"),
            boolean(&fields, "todo"),
        ) else {
            return Err(format!(
                "line {}: a test with no `name`, `file`, `skipped` or `todo`",
                n + 1
            ));
        };
        out.push(NodeTest {
            file: file.replace('\\', "/"),
            suite,
            name,
            skipped,
            todo,
        });
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
            Json::Object(c) => {
                string(c, "name").ok_or_else(|| "a quick check with no `name`".to_string())
            }
            _ => Err("a quick check that is no object".to_string()),
        })
        .collect()
}

/// The tests the `@assert-count` directives name as evidence, `fn test_...`, from spec-guard's
/// report of the directives it runs, the output of [`SPEC_GUARD`]: each with its document, line
/// and the one file it counts in. A report of another tool or format, one that read with errors,
/// or one that names no test, is refused.
pub fn spec_guard(text: &str) -> Result<Vec<Named>, String> {
    let Json::Object(top) = json::parse(text)? else {
        return Err("spec-guard's report is no JSON object".to_string());
    };
    if string(&top, "tool").as_deref() != Some("spec-guard") {
        return Err("no report of spec-guard: its `tool` is not `spec-guard`".to_string());
    }
    if !matches!(field(&top, "formatVersion"), Some(Json::Number(v)) if v == "1") {
        return Err("a report of spec-guard in a format other than its version 1".to_string());
    }
    match field(&top, "errors") {
        Some(Json::Array(errors)) if errors.is_empty() => {}
        _ => {
            return Err(
                "spec-guard's report holds errors, so it may not hold every directive".to_string(),
            )
        }
    }
    let Some(Json::Array(results)) = field(&top, "results") else {
        return Err("spec-guard's report holds no `results`".to_string());
    };
    let mut out = Vec::new();
    for result in results {
        let Json::Object(result) = result else {
            return Err("a result that is no object".to_string());
        };
        if string(result, "kind").as_deref() != Some("assert-count") {
            continue;
        }
        let Some(symbol) = string(result, "symbol") else {
            return Err("an assert-count with no `symbol`".to_string());
        };
        let Some(rest) = symbol.strip_prefix("fn test_") else {
            continue;
        };
        let (Some(Json::Object(spec)), Some(Json::Array(targets))) =
            (field(result, "spec"), field(result, "targets"))
        else {
            return Err(format!(
                "{}: an assert-count with no `spec` or `targets`",
                symbol
            ));
        };
        let (Some(document), Some(Json::Number(line))) =
            (string(spec, "file"), field(spec, "line"))
        else {
            return Err(format!("{}: an assert-count with no place", symbol));
        };
        let line: usize = line
            .parse()
            .map_err(|_| format!("{}: a line that is no number", symbol))?;
        let [Json::Str(target)] = targets.as_slice() else {
            return Err(format!(
                "{}:{}: `{}` names its evidence in other than one file",
                document, line, symbol
            ));
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        out.push(Named {
            document,
            line,
            target: target.clone(),
            name: format!("test_{}", name),
        });
    }
    if out.is_empty() {
        return Err("spec-guard's report names no test as evidence".to_string());
    }
    Ok(out)
}

/// Which binaries, by their id, a target file's tests are built into.
type BinaryFilter = Box<dyn Fn(&str) -> bool>;

/// Whether `path`, a test's path in its binary, names the test `name`: its last segment.
fn names(path: &str, name: &str) -> bool {
    path.rsplit("::").next() == Some(name)
}

/// Whether nextest runs `test`: listed, not ignored, and taken by the profile.
fn runs(test: &RustTest) -> bool {
    !test.ignored && test.matches
}

/// Whether the test `path`, under the module whose own file is `file` and whose submodules' files
/// lie in `dir`, belongs to that file: it opens with `prefix`, and each module between that and
/// its name is declared inline, no file of its own standing for it, `<dir>/<m>.rs` or
/// `<dir>/<m>/mod.rs`.
fn in_file(path: &str, prefix: &[String], dir: &Path) -> bool {
    let segments: Vec<&str> = path.split("::").collect();
    let Some((_, modules)) = segments.split_last() else {
        return false;
    };
    if modules.len() < prefix.len() || modules[..prefix.len()] != prefix[..] {
        return false;
    }
    let mut dir = dir.to_path_buf();
    for m in &modules[prefix.len()..] {
        if dir.join(format!("{}.rs", m)).is_file() || dir.join(m).join("mod.rs").is_file() {
            return false;
        }
        dir = dir.join(m);
    }
    true
}

impl Runs {
    /// Whether nextest runs a test `name` of package `package`, in any of its binaries.
    pub fn runs_rust(&self, package: &str, name: &str) -> bool {
        self.rust
            .iter()
            .any(|t| runs(t) && t.package == package && names(&t.path, name))
    }

    /// Whether nextest runs a test `name` written in the file `target`, under the repository's
    /// root `root`. A file maps to exactly its own module and the modules it declares inline:
    /// `tests/x.rs` is the binary `x` of its package, `src/lib.rs` the library's root, `src/main.rs`
    /// the root of the package's binary and `src/bin/x.rs` the binary `x`, and another file of `src/`,
    /// `src/a/b.rs` or `src/a/b/mod.rs`, the module `a::b` of the library or of a binary. A test of
    /// a submodule with a file of its own is that file's, not the target's. A target nextest
    /// builds no test from is an error.
    pub fn runs_rust_in(&self, root: &Path, target: &str, name: &str) -> Result<bool, String> {
        let no = || format!("{}: a file nextest builds no test from", target);
        let rest = target.strip_prefix("compiler/").ok_or_else(no)?;
        let (package, rest) = rest.split_once('/').ok_or_else(no)?;
        let rest = rest.strip_suffix(".rs").ok_or_else(no)?;
        let crate_dir = root.join("compiler").join(package);
        let library = package.to_string();
        let bins = format!("{}::bin/", package);
        let own_bins: Vec<String> = std::fs::read_dir(crate_dir.join("src/bin"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_suffix(".rs")
                    .map(|stem| format!("{}{}", bins, stem))
            })
            .collect();
        let (binary, prefix, dir): (BinaryFilter, Vec<String>, std::path::PathBuf) =
            if let Some(stem) = rest.strip_prefix("tests/") {
                if stem.contains('/') {
                    return Err(no());
                }
                let id = format!("{}::{}", package, stem);
                (
                    Box::new(move |b| b == id),
                    Vec::new(),
                    crate_dir.join("tests").join(stem),
                )
            } else if let Some(stem) = rest.strip_prefix("src/bin/") {
                if stem.contains('/') {
                    return Err(no());
                }
                let id = format!("{}{}", bins, stem);
                (
                    Box::new(move |b| b == id),
                    Vec::new(),
                    crate_dir.join("src/bin").join(stem),
                )
            } else if let Some(module) = rest.strip_prefix("src/") {
                let main_bin =
                    move |b: &str| b.starts_with(&bins) && !own_bins.iter().any(|o| o == b);
                let src = crate_dir.join("src");
                match module {
                    "lib" => (Box::new(move |b| b == library), Vec::new(), src),
                    "main" => (Box::new(main_bin), Vec::new(), src),
                    _ => {
                        let module = module.strip_suffix("/mod").unwrap_or(module);
                        let prefix: Vec<String> = module.split('/').map(str::to_string).collect();
                        let dir = src.join(module);
                        (Box::new(move |b| b == library || main_bin(b)), prefix, dir)
                    }
                }
            } else {
                return Err(no());
            };
        Ok(self.rust.iter().any(|t| {
            runs(t)
                && t.package == package
                && names(&t.path, name)
                && binary(&t.binary)
                && in_file(&t.path, &prefix, &dir)
        }))
    }

    /// Whether the workflow policy's run ran a check `name`, at its top level, not skipped or left
    /// to do.
    pub fn runs_policy(&self, name: &str) -> bool {
        self.node.iter().any(|t| {
            !t.skipped
                && !t.todo
                && t.suite.is_empty()
                && t.name == name
                && t.file.ends_with(&format!("/{}", POLICY))
        })
    }

    /// Whether a Node run ran a test `name`, not skipped or left to do.
    pub fn runs_node(&self, name: &str) -> bool {
        self.node
            .iter()
            .any(|t| !t.skipped && !t.todo && t.name == name)
    }
}

/// Each guard of `rows` that no runner runs, by its escape's key.
pub fn guards(rows: &[Escape], runs: &Runs) -> Vec<String> {
    let mut out = Vec::new();
    for r in rows {
        let (runs_it, what) = match &r.guard {
            Guard::Test { name, package } => (
                runs.runs_rust(package, name),
                format!(
                    "`fn {} in {}` is no test nextest runs in that package, or it is ignored or filtered out",
                    name, package
                ),
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
                format!("quick check {:?} is no check of tools/quick-checks.json", name),
            ),
        };
        if !runs_it {
            out.push(format!("{}: its guard {}", r.key, what));
        }
    }
    out
}

/// Each test that `named` names as evidence and nextest does not run from its target, by the
/// directive's document and line, with the repository's root at `root`.
pub fn evidence(root: &Path, named: &[Named], runs: &Runs) -> Vec<String> {
    let mut out = Vec::new();
    for n in named {
        match runs.runs_rust_in(root, &n.target, &n.name) {
            Ok(true) => {}
            Ok(false) => out.push(format!(
                "{}:{}: `fn {}`, the evidence it names in {}, is no test nextest runs from there, or it is ignored or filtered out",
                n.document, n.line, n.name, n.target
            )),
            Err(why) => out.push(format!("{}:{}: {}", n.document, n.line, why)),
        }
    }
    out
}
