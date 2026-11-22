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

use std::path::{Path, PathBuf};

use crate::escapes::{Escape, Guard};
use crate::json::{self, Json};

/// The command whose output is nextest's list, as CI's profile takes the tests.
pub const NEXTEST: &str =
    "cargo-nextest nextest list --profile ci --workspace --run-ignored all --message-format json";

/// How a Node run writes the tests it ran, beside its usual reporter.
pub const NODE: &str = "node --test --test-reporter=./tools/evidence-reporter.mjs --test-reporter-destination=<file> <files>";

/// The command whose output is spec-guard's report of the directives it runs.
pub const SPEC_GUARD: &str = "npm run -s spec-guard -- --json";

/// The directory of the workflow policy's modules, whose top-level tests are its checks: each file
/// `<name>.test.mjs` directly in it, as `npm run test:ci` runs them. The policy and the gates
/// test hold the directory to the files its `modules.json` lists.
pub const POLICY_DIR: &str = ".github/scripts/policy/";

/// Whether `file`, a path with `/` between names, is a module of the workflow policy: a file
/// `<name>.test.mjs` directly in [`POLICY_DIR`], not one of the libraries the modules share.
pub fn is_policy_module(file: &str) -> bool {
    let dir = format!("/{}", POLICY_DIR);
    file.rfind(&dir).is_some_and(|at| {
        let name = &file[at + dir.len()..];
        !name.contains('/') && name.ends_with(".test.mjs")
    })
}

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

/// The npm script whose documents spec-guard's report must cover, in `package.json`.
pub const SPEC_GUARD_SCRIPT: &str = "spec-guard";

/// The documents spec-guard checks, read from the `spec-guard` script of `package.json` under
/// `root` and expanded as spec-guard 0.35 expands them: each argument a file, or `dir/*.ext`, the
/// files of `dir` whose names end in that extension, in that case, a name that starts with a dot
/// among them, and no directory or file of a subdirectory. A script that does not run
/// spec-guard, an option, a word a shell would read otherwise than as itself, or a pattern of
/// another shape, is refused, so that the list is never guessed.
pub fn spec_guard_files(root: &Path) -> Result<Vec<String>, String> {
    let path = root.join("package.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let Json::Object(top) = json::parse(&text)? else {
        return Err("package.json is no JSON object".to_string());
    };
    let Some(Json::Object(scripts)) = field(&top, "scripts") else {
        return Err("package.json has no `scripts`".to_string());
    };
    let Some(script) = string(scripts, SPEC_GUARD_SCRIPT) else {
        return Err(format!(
            "package.json has no script `{}`",
            SPEC_GUARD_SCRIPT
        ));
    };
    let words = shell_words(&script)
        .ok_or_else(|| format!("the script `{}` is more than plain words", script))?;
    let Some((command, patterns)) = words.split_first() else {
        return Err("the script `spec-guard` is empty".to_string());
    };
    if command != "spec-guard" {
        return Err(format!("the script `{}` does not run spec-guard", script));
    }
    let mut files = Vec::new();
    for pattern in patterns {
        let glob = |c: char| "*?[]{}!".contains(c);
        if pattern.starts_with('-') {
            return Err(format!(
                "`{}`: an option of the script, whose effect on the documents is not read",
                pattern
            ));
        }
        match pattern.rsplit_once("/*.") {
            Some((dir, extension))
                if !dir.contains(glob) && !extension.contains(glob) && !extension.contains('/') =>
            {
                let mut names: Vec<String> = std::fs::read_dir(root.join(dir))
                    .map_err(|e| format!("{}: {}", dir, e))?
                    .flatten()
                    .filter(|e| e.path().is_file())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(&format!(".{}", extension)))
                    .map(|n| format!("{}/{}", dir, n))
                    .collect();
                names.sort();
                files.extend(names);
            }
            _ if !pattern.contains(glob) => files.push(pattern.clone()),
            _ => {
                return Err(format!(
                    "`{}`: a pattern mn evidence does not expand",
                    pattern
                ))
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// The words of a script as a shell splits it, each unquoted or quoted whole in `"` or `'`, or
/// `None` where it holds anything a shell would read otherwise: a variable, an operator, an escape.
fn shell_words(script: &str) -> Option<Vec<String>> {
    let plain = |c: char| c.is_ascii_alphanumeric() || "._/*-".contains(c);
    let mut words = Vec::new();
    for word in script.split(' ').filter(|w| !w.is_empty()) {
        let inner = match word.chars().next() {
            Some(q @ ('"' | '\'')) => word.strip_prefix(q)?.strip_suffix(q)?,
            _ => word,
        };
        if inner.is_empty() || !inner.chars().all(plain) {
            return None;
        }
        words.push(inner.to_string());
    }
    Some(words)
}

/// The tests the `@assert-count` directives name as evidence, `fn test_...`, from spec-guard's
/// report of the directives it runs, the output of [`SPEC_GUARD`]: each with its document, line
/// and the one file it counts in. A report of another tool, format or command, one that read with
/// errors, one over other documents than `files` ([`spec_guard_files`]), or one that names no
/// test, is refused.
pub fn spec_guard(text: &str, files: &[String]) -> Result<Vec<Named>, String> {
    let Json::Object(top) = json::parse(text)? else {
        return Err("spec-guard's report is no JSON object".to_string());
    };
    if string(&top, "tool").as_deref() != Some("spec-guard") {
        return Err("no report of spec-guard: its `tool` is not `spec-guard`".to_string());
    }
    if !matches!(field(&top, "formatVersion"), Some(Json::Number(v)) if v == "1") {
        return Err("a report of spec-guard in a format other than its version 1".to_string());
    }
    if string(&top, "command").as_deref() != Some("check") {
        return Err("a report of a spec-guard command other than `check`".to_string());
    }
    let Some(Json::Array(checked)) = field(&top, "specFiles") else {
        return Err("spec-guard's report names no `specFiles`".to_string());
    };
    let mut checked: Vec<String> = checked
        .iter()
        .map(|f| match f {
            Json::Str(f) => Ok(f.clone()),
            _ => Err("a document of `specFiles` that is no string".to_string()),
        })
        .collect::<Result<_, _>>()?;
    checked.sort();
    if checked != files {
        let missing: Vec<&str> = files
            .iter()
            .filter(|f| !checked.contains(f))
            .map(String::as_str)
            .collect();
        let other: Vec<&str> = checked
            .iter()
            .filter(|f| !files.contains(f))
            .map(String::as_str)
            .collect();
        return Err(format!(
            "spec-guard's report covers other documents than its script names: missing [{}], beside them [{}]",
            missing.join(", "),
            other.join(", ")
        ));
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

/// Where the file `rest`, its path from `family` without `.rs`, stands in a family of binaries,
/// the integration tests of `tests/` or the binaries of `src/bin/`: the binary whose file it is,
/// none where it is a module of any binary of the family, the directory of that binary's root,
/// and its module's path from there, none for the root itself. `x` is the root of the binary `x`,
/// and so is `x/main` where that file stands, whose directory holds the binary's other files;
/// another file, `common/mod` say, is a module of whichever binary declares it.
fn in_family(family: &Path, rest: &str) -> (Option<String>, PathBuf, Option<String>) {
    match rest.split_once('/') {
        None => (Some(rest.to_string()), family.to_path_buf(), None),
        Some((x, inner)) if family.join(x).join("main.rs").is_file() => (
            Some(x.to_string()),
            family.join(x),
            (inner != "main").then(|| inner.to_string()),
        ),
        Some(_) => (None, family.to_path_buf(), Some(rest.to_string())),
    }
}

impl Runs {
    /// Whether nextest runs a test `name` of package `package`, in any of its binaries.
    pub fn runs_rust(&self, package: &str, name: &str) -> bool {
        self.rust
            .iter()
            .any(|t| runs(t) && t.package == package && names(&t.path, name))
    }

    /// Whether nextest runs a test `name` written in the file `target`, under the repository's
    /// root `root`. A file maps to exactly its own module and the modules it declares inline. A
    /// crate root keeps the files of its submodules beside it, in its own directory: `src/lib.rs` is
    /// the library's root and `src/main.rs` the root of the package's binary, whose modules are the
    /// other files of `src/`, `src/a/b.rs` or `src/a/b/mod.rs` the module `a::b`. In `tests/` and
    /// `src/bin/`, `x.rs` and `x/main.rs` are each the root of the binary `x`; the other files of
    /// `x/` beside a `main.rs` are that binary's modules, and another file, `common/mod.rs` say, is a
    /// module of any binary there that declares it. A test of a submodule with a file of its own is
    /// that file's, not the target's. A target nextest builds no test from is an error.
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
                let file = e.file_name().to_string_lossy().into_owned();
                match file.strip_suffix(".rs") {
                    Some(stem) => Some(stem.to_string()),
                    None => e.path().join("main.rs").is_file().then_some(file),
                }
            })
            .map(|stem| format!("{}{}", bins, stem))
            .collect();
        let (binary, root_dir, module): (BinaryFilter, PathBuf, Option<String>) =
            if let Some(rest) = rest.strip_prefix("tests/") {
                let (binary, root_dir, module) = in_family(&crate_dir.join("tests"), rest);
                let tests = format!("{}::", package);
                let filter: BinaryFilter = match binary {
                    Some(x) => {
                        let id = format!("{}{}", tests, x);
                        Box::new(move |b| b == id)
                    }
                    None => {
                        Box::new(move |b| b.strip_prefix(&tests).is_some_and(|t| !t.contains('/')))
                    }
                };
                (filter, root_dir, module)
            } else if let Some(rest) = rest.strip_prefix("src/bin/") {
                let (binary, root_dir, module) = in_family(&crate_dir.join("src/bin"), rest);
                let filter: BinaryFilter = match binary {
                    Some(x) => {
                        let id = format!("{}{}", bins, x);
                        Box::new(move |b| b == id)
                    }
                    None => Box::new(move |b| own_bins.iter().any(|o| o == b)),
                };
                (filter, root_dir, module)
            } else if let Some(module) = rest.strip_prefix("src/") {
                let main_bin =
                    move |b: &str| b.starts_with(&bins) && !own_bins.iter().any(|o| o == b);
                let src = crate_dir.join("src");
                match module {
                    "lib" => (Box::new(move |b| b == library), src, None),
                    "main" => (Box::new(main_bin), src, None),
                    _ => (
                        Box::new(move |b| b == library || main_bin(b)),
                        src,
                        Some(module.to_string()),
                    ),
                }
            } else {
                return Err(no());
            };
        let (prefix, dir) = match module {
            None => (Vec::new(), root_dir),
            Some(module) => {
                let module = module.strip_suffix("/mod").unwrap_or(&module);
                let prefix: Vec<String> = module.split('/').map(str::to_string).collect();
                (prefix, root_dir.join(module))
            }
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
                && is_policy_module(&t.file)
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
