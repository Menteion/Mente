//! What the runners run (`src/evidence.rs`): nextest's list, our reporter's report of each Node
//! run and spec-guard's report of its directives, each read as its tool writes it, and
//! `mn evidence`, which checks the guards of the escaped bugs and the tests named as evidence
//! against them. The lists here are planted in the tools' own forms; the Node report is the one
//! `tools/evidence-reporter.mjs` writes in its own test.

use mente_cli::escapes;
use mente_cli::evidence::{self, Named, NodeTest, Runs, RustTest};

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A binary of nextest's list: its id, its package, and its tests, each with whether it is ignored
/// and whether the profile takes it.
type Suite<'a> = (&'a str, &'a str, &'a [(&'a str, bool, bool)]);

/// nextest's list, as `cargo-nextest nextest list --message-format json` writes it, of `suites`.
fn list(suites: &[Suite]) -> String {
    let suites: Vec<String> = suites
        .iter()
        .map(|(binary, package, tests)| {
            let tests: Vec<String> = tests
                .iter()
                .map(|(path, ignored, matches)| {
                    let filter = if *matches {
                        "{\"status\": \"matches\"}"
                    } else {
                        "{\"status\": \"mismatch\", \"reason\": \"default-filter\"}"
                    };
                    format!(
                        "\"{}\": {{\"ignored\": {}, \"filter-match\": {}}}",
                        path, ignored, filter
                    )
                })
                .collect();
            format!(
                "\"{b}\": {{\"package-name\": \"{p}\", \"binary-id\": \"{b}\", \"testcases\": {{{t}}}}}",
                b = binary,
                p = package,
                t = tests.join(", ")
            )
        })
        .collect();
    format!(
        "{{\"rust-build-meta\": {{}}, \"test-count\": 1, \"rust-suites\": {{{}}}}}",
        suites.join(", ")
    )
}

const SUITES: &[Suite] = &[
    (
        "mente-cli",
        "mente-cli",
        &[
            ("records::tests::test_in_records", false, true),
            ("tests::test_in_lib", false, true),
        ],
    ),
    (
        "mente-cli::bin/mn",
        "mente-cli",
        &[
            ("tests::test_in_main", false, true),
            ("mir_run::tests::test_in_mir_run", false, true),
            ("mir_run::coverage::tests::test_in_coverage", false, true),
            ("catalogue::test_in_catalogue", false, true),
        ],
    ),
    (
        "mente-cli::bin/extra",
        "mente-cli",
        &[
            ("test_in_extra", false, true),
            ("shared::test_in_shared", false, true),
        ],
    ),
    (
        "mente-cli::bin/tool",
        "mente-cli",
        &[
            ("test_in_tool", false, true),
            ("parse::test_in_tool_parse", false, true),
            ("inline::test_in_tool_inline", false, true),
        ],
    ),
    (
        "mente-cli::escapes",
        "mente-cli",
        &[
            ("test_in_escapes", false, true),
            ("test_ignored", true, true),
            ("test_filtered", false, false),
            ("common::test_in_common", false, true),
            ("inline::test_in_escapes_inline", false, true),
        ],
    ),
    (
        "mente-interp",
        "mente-interp",
        &[("tests::test_runs_stop_at_their_limits", false, true)],
    ),
    (
        "mente-mir",
        "mente-mir",
        &[("cfg::tests::test_runs_stop_at_their_limits", false, true)],
    ),
];

/// The crate trees the mapping reads: which modules have files of their own.
const TREE: &[&str] = &[
    "compiler/mente-cli/src/lib.rs",
    "compiler/mente-cli/src/records.rs",
    "compiler/mente-cli/src/main.rs",
    "compiler/mente-cli/src/mir_run.rs",
    "compiler/mente-cli/src/mir_run/coverage.rs",
    "compiler/mente-cli/src/catalogue.rs",
    "compiler/mente-cli/src/bin/extra.rs",
    "compiler/mente-cli/src/bin/shared/mod.rs",
    "compiler/mente-cli/src/bin/tool/main.rs",
    "compiler/mente-cli/src/bin/tool/parse.rs",
    "compiler/mente-cli/tests/escapes.rs",
    "compiler/mente-cli/tests/common/mod.rs",
    "compiler/mente-interp/src/lib.rs",
    "compiler/mente-interp/src/tests.rs",
    "compiler/mente-mir/src/lib.rs",
    "compiler/mente-mir/src/cfg/mod.rs",
];

/// A fresh directory of this test binary's own, named `name`, holding `files`.
fn planted(name: &str, files: &[(&str, &str)]) -> io::Result<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("evidence")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    for (path, text) in files {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, text)?;
    }
    Ok(dir)
}

fn tree(name: &str) -> io::Result<PathBuf> {
    let files: Vec<(&str, &str)> = TREE.iter().map(|f| (*f, "")).collect();
    planted(name, &files)
}

/// The report `tools/evidence-reporter.mjs` writes in its own test.
fn report() -> io::Result<String> {
    fs::read_to_string(root().join("tools/evidence-reporter.jsonl"))
}

fn runs() -> Result<Runs, String> {
    Ok(Runs {
        rust: evidence::nextest(&list(SUITES))?,
        node: evidence::node_report(&report().map_err(|e| e.to_string())?)?,
        quick: vec!["MIR mutation gate".to_string()],
    })
}

/// nextest's list is read test by test, with its binary, package, path, whether it is ignored and
/// whether the profile takes it; a list of another form, or one that holds no test, is refused.
#[test]
fn test_nextest_list_is_read_as_nextest_writes_it() {
    let tests = evidence::nextest(&list(SUITES)).unwrap();
    assert_eq!(tests.len(), SUITES.iter().map(|s| s.2.len()).sum::<usize>());
    assert!(tests.contains(&RustTest {
        binary: "mente-cli::escapes".to_string(),
        package: "mente-cli".to_string(),
        path: "test_filtered".to_string(),
        ignored: false,
        matches: false,
    }));
    let case = |c: &str| {
        format!(
            "{{\"rust-suites\": {{\"a\": {{\"package-name\": \"a\", \"binary-id\": \"a\", \"testcases\": {{\"t\": {}}}}}}}}}",
            c
        )
    };
    for (text, why) in [
        ("[]".to_string(), "no JSON object"),
        ("{}".to_string(), "no `rust-suites`"),
        ("{\"rust-suites\": {}}".to_string(), "holds no test"),
        (
            "{\"rust-suites\": {\"a\": {\"binary-id\": \"a\", \"testcases\": {}}}}".to_string(),
            "a suite with no",
        ),
        (case("{}"), "a test with no `ignored`"),
        (
            case("{\"ignored\": false}"),
            "a test with no `filter-match`",
        ),
        (
            case("{\"ignored\": false, \"filter-match\": {}}"),
            "no `status`",
        ),
        (case("1"), "a test that is no object"),
        ("{\"rust-suites\": ".to_string(), "ends before a value"),
    ] {
        let found = evidence::nextest(&text).expect_err(&text);
        assert!(found.contains(why), "{}: {}", text, found);
    }
}

/// A Node report, as our reporter writes it in its own test, is read test by test with its file,
/// its suites, its name and whether it was skipped or left to do; a report of another form, or one
/// that holds no test, is refused.
#[test]
fn test_a_node_report_is_read_as_our_reporter_writes_it() {
    let tests = evidence::node_report(&report().unwrap()).unwrap();
    assert_eq!(tests.len(), 13);
    assert_eq!(
        tests[1],
        NodeTest {
            file: "/w/.github/scripts/policy.test.mjs".to_string(),
            suite: vec![],
            name: "a check with > < & \" ' and <!-- a comment -->".to_string(),
            skipped: false,
            todo: false,
        }
    );
    assert!(tests[2].skipped && tests[4].skipped);
    assert!(tests[3].todo && tests[5].todo);
    assert_eq!(
        tests[6],
        NodeTest {
            file: "/w/.github/scripts/policy-helpers.mjs".to_string(),
            suite: vec!["a policy suite".to_string()],
            name: "a check from a helper".to_string(),
            skipped: false,
            todo: false,
        }
    );
    assert_eq!(tests[7].suite, ["a policy suite"]);
    assert_eq!(tests[9].suite, ["a suite", "an inner suite"]);
    assert!(!tests
        .iter()
        .any(|t| t.name.contains("timed out") || t.name.contains("cancelled")));
    let line = "{\"name\": \"a\", \"suite\": [], \"file\": \"C:\\\\w\\\\a.mjs\", \"skipped\": false, \"todo\": false}";
    assert_eq!(evidence::node_report(line).unwrap()[0].file, "C:/w/a.mjs");
    for (text, why) in [
        ("", "holds no test"),
        ("\n\n", "holds no test"),
        ("ok 1 - a test", "line 1"),
        ("[]", "line 1: no JSON object"),
        (
            "{\"name\": \"a\", \"file\": \"b\", \"skipped\": false, \"todo\": false}",
            "no `suite`",
        ),
        (
            "{\"name\": \"a\", \"suite\": [1], \"file\": \"b\", \"skipped\": false, \"todo\": false}",
            "a suite that is no string",
        ),
        (
            "{\"name\": \"a\", \"suite\": [], \"file\": \"b\", \"skipped\": false}",
            "no `name`, `file`, `skipped` or `todo`",
        ),
        (
            "{\"name\": \"a\", \"suite\": [], \"file\": \"b\", \"skipped\": \"no\", \"todo\": false}",
            "no `name`, `file`, `skipped` or `todo`",
        ),
    ] {
        let found = evidence::node_report(text).expect_err(text);
        assert!(found.contains(why), "{:?}: {}", text, found);
    }
}

/// The documents of the planted reports, as [`SCRIPT`] names them over [`DOCUMENT_FILES`].
const DOCUMENTS: [&str; 2] = ["CLAUDE.md", "spec/a.md"];

/// A `package.json` whose spec-guard script names [`DOCUMENTS`].
const SCRIPT: &str = "{\"scripts\": {\"spec-guard\": \"spec-guard \\\"spec/*.md\\\" CLAUDE.md\"}}";

/// The files of a tree [`SCRIPT`] expands to [`DOCUMENTS`].
const DOCUMENT_FILES: [&str; 2] = ["CLAUDE.md", "spec/a.md"];

fn documents() -> Vec<String> {
    DOCUMENTS.iter().map(|d| d.to_string()).collect()
}

/// spec-guard's report of the directives it runs, `--json`, from its `check` of [`DOCUMENTS`],
/// with `results`.
fn spec_guard(results: &[&str]) -> String {
    format!(
        "{{\"tool\": \"spec-guard\", \"toolVersion\": \"0.35.0\", \"command\": \"check\", \"formatVersion\": 1, \"ok\": true, \"specFiles\": [\"CLAUDE.md\", \"spec/a.md\"], \"errors\": [], \"results\": [{}]}}",
        results.join(", ")
    )
}

fn count(document: &str, line: usize, symbol: &str, targets: &[&str]) -> String {
    let targets: Vec<String> = targets.iter().map(|t| format!("\"{}\"", t)).collect();
    format!(
        "{{\"ok\": true, \"kind\": \"assert-count\", \"spec\": {{\"file\": \"{}\", \"line\": {}, \"column\": 6}}, \"symbol\": \"{}\", \"targets\": [{}]}}",
        document,
        line,
        symbol,
        targets.join(", ")
    )
}

/// The tests named as evidence are read from spec-guard's own report, each `@assert-count` whose
/// symbol is `fn test_...`, with its document, line and target; a report of another tool, format
/// or command, one over other documents than the script names, one with errors, one that names no
/// test, and a directive over other than one file, are refused.
#[test]
fn test_the_evidence_is_read_from_spec_guards_report() {
    let present =
        "{\"ok\": true, \"kind\": \"assert-present\", \"spec\": {\"file\": \"spec/a.md\", \"line\": 1}}";
    let text = spec_guard(&[
        present,
        &count(
            "spec/a.md",
            3,
            "fn test_in_escapes",
            &["compiler/mente-cli/tests/escapes.rs"],
        ),
        &count(
            "CLAUDE.md",
            9,
            "fn test_in_records(",
            &["compiler/mente-cli/src/records.rs"],
        ),
        &count(
            "spec/a.md",
            4,
            "pub fn is_key",
            &["compiler/mente-cli/src/records.rs"],
        ),
    ]);
    assert_eq!(
        evidence::spec_guard(&text, &documents()).unwrap(),
        [
            Named {
                document: "spec/a.md".to_string(),
                line: 3,
                target: "compiler/mente-cli/tests/escapes.rs".to_string(),
                name: "test_in_escapes".to_string(),
            },
            Named {
                document: "CLAUDE.md".to_string(),
                line: 9,
                target: "compiler/mente-cli/src/records.rs".to_string(),
                name: "test_in_records".to_string(),
            },
        ]
    );
    let one = count(
        "spec/a.md",
        3,
        "fn test_x",
        &["compiler/mente-cli/tests/escapes.rs"],
    );
    for (text, why) in [
        ("[]".to_string(), "no JSON object"),
        (
            text.replace("\"spec-guard\"", "\"spec-graph\""),
            "its `tool` is not `spec-guard`",
        ),
        (
            text.replace("\"formatVersion\": 1", "\"formatVersion\": 2"),
            "other than its version 1",
        ),
        // The round-5 review's plant: a check of one document alone.
        (
            text.replace("\"command\": \"check\"", "\"command\": \"prove\""),
            "a spec-guard command other than `check`",
        ),
        (
            text.replace("\"command\": \"check\", ", ""),
            "a spec-guard command other than `check`",
        ),
        (
            text.replace("\"CLAUDE.md\", \"spec/a.md\"", "\"spec/a.md\""),
            "covers other documents than its script names: missing [CLAUDE.md], beside them []",
        ),
        (
            text.replace(
                "\"CLAUDE.md\", \"spec/a.md\"",
                "\"CLAUDE.md\", \"spec/a.md\", \"docs/x.md\"",
            ),
            "missing [], beside them [docs/x.md]",
        ),
        (
            text.replace("\"specFiles\"", "\"files\""),
            "names no `specFiles`",
        ),
        (
            text.replace("\"CLAUDE.md\", \"spec/a.md\"", "\"CLAUDE.md\", 1"),
            "a document of `specFiles` that is no string",
        ),
        (
            text.replace("\"errors\": []", "\"errors\": [{}]"),
            "holds errors",
        ),
        (text.replace("\"errors\": [], ", ""), "holds errors"),
        (
            text.replace("\"results\"", "\"outcomes\""),
            "holds no `results`",
        ),
        (spec_guard(&[present]), "names no test as evidence"),
        (
            spec_guard(&[&count("spec/a.md", 3, "fn test_x", &["a.rs", "b.rs"])]),
            "spec/a.md:3: `fn test_x` names its evidence in other than one file",
        ),
        (
            spec_guard(&[&count("spec/a.md", 3, "fn test_x", &[])]),
            "in other than one file",
        ),
        (
            spec_guard(&[&one.replace("\"line\": 3", "\"line\": \"3\"")]),
            "no place",
        ),
        (
            spec_guard(&[&one.replace("\"symbol\": \"fn test_x\", ", "")]),
            "with no `symbol`",
        ),
        (spec_guard(&["1"]), "a result that is no object"),
    ] {
        let found = evidence::spec_guard(&text, &documents()).expect_err(&text);
        assert!(found.contains(why), "{}: {}", text, found);
    }
}

/// The documents spec-guard's report must cover are read from the spec-guard script of
/// `package.json` and expanded as spec-guard expands them, never copied; a script of another shape
/// is refused. The repository's own script names the specs and the documents beside them. The
/// planted tree holds the names spec-guard 0.35's own expansion was seen to take, a dotfile, a
/// space, brackets and a leading hyphen, and to leave, a directory named `dir.md`, an extension in
/// capitals and a file of a subdirectory (the review of a136bf0).
#[test]
fn test_the_documents_are_read_from_the_spec_guard_script() {
    let script = |s: &str| format!("{{\"scripts\": {{\"spec-guard\": \"{}\"}}}}", s);
    let tree = [
        "spec/a.md",
        "spec/b.md",
        "spec/.hidden.md",
        "spec/with space.md",
        "spec/[x].md",
        "spec/-lead.md",
        "spec/UPPER.MD",
        "spec/dir.md/inside.md",
        "spec/c.txt",
        "spec/sub/d.md",
        "docs/x.md",
        "README.md",
    ];
    let plant = |name: &str, package: &str| {
        let mut files: Vec<(&str, &str)> = tree.iter().map(|f| (*f, "")).collect();
        files.push(("package.json", package));
        planted(name, &files).unwrap()
    };
    let dir = plant(
        "script",
        &script("spec-guard \\\"spec/*.md\\\" 'docs/*.md' README.md"),
    );
    assert_eq!(
        evidence::spec_guard_files(&dir).unwrap(),
        [
            "README.md",
            "docs/x.md",
            "spec/-lead.md",
            "spec/.hidden.md",
            "spec/[x].md",
            "spec/a.md",
            "spec/b.md",
            "spec/with space.md"
        ]
    );
    for (package, why) in [
        (
            script("npx spec-guard spec/a.md"),
            "does not run spec-guard",
        ),
        (
            script("spec-guard --json spec/a.md"),
            "an option of the script",
        ),
        (
            script("spec-guard spec/**/*.md"),
            "a pattern mn evidence does not expand",
        ),
        (
            script("spec-guard spec/a*.md"),
            "a pattern mn evidence does not expand",
        ),
        (script("spec-guard $DOCS"), "more than plain words"),
        (script("spec-guard a.md && rm b"), "more than plain words"),
        (script("spec-guard \\\"a.md"), "more than plain words"),
        (script(""), "is empty"),
        ("{\"scripts\": {}}".to_string(), "no script `spec-guard`"),
        ("{}".to_string(), "no `scripts`"),
        ("[]".to_string(), "no JSON object"),
    ] {
        let dir = plant("script-refused", &package);
        let found = evidence::spec_guard_files(&dir).expect_err(&package);
        assert!(found.contains(why), "{}: {}", package, found);
    }
    let missing = planted("script-missing", &[]).unwrap();
    assert!(evidence::spec_guard_files(&missing)
        .expect_err("no package.json")
        .contains("package.json"));
    let own = evidence::spec_guard_files(&root()).unwrap();
    for document in [
        "CLAUDE.md",
        "ROADMAP.md",
        "README.md",
        "spec/core-calculus.md",
        "docs/STATUS.md",
    ] {
        assert!(own.iter().any(|d| d == document), "{}: {:?}", document, own);
    }
    let planted_documents = planted("documents", &DOCUMENT_FILES.map(|f| (f, ""))).unwrap();
    fs::write(planted_documents.join("package.json"), SCRIPT).unwrap();
    assert_eq!(
        evidence::spec_guard_files(&planted_documents).unwrap(),
        documents()
    );
}

/// A test runs from a target file when nextest lists it, not ignored and taken by the profile, in
/// the target's package and from exactly its module: `tests/x.rs` and `tests/x/main.rs` are the
/// binary `x`, `src/lib.rs` the library's root, `src/main.rs` the root of the package's binary,
/// `src/bin/x.rs` and `src/bin/x/main.rs` the binary `x`, and another file its module and the
/// modules it declares inline, never a submodule with a file of its own. Each root keeps its
/// submodules' files beside it. A file nextest builds no test from is an error.
#[test]
fn test_a_test_runs_from_exactly_its_target_file() {
    let runs = runs().unwrap();
    let dir = tree("mapping").unwrap();
    for (target, name, found) in [
        (
            "compiler/mente-cli/tests/escapes.rs",
            "test_in_escapes",
            true,
        ),
        ("compiler/mente-cli/tests/escapes.rs", "test_ignored", false),
        (
            "compiler/mente-cli/tests/escapes.rs",
            "test_filtered",
            false,
        ),
        ("compiler/mente-cli/tests/cli.rs", "test_in_escapes", false),
        ("compiler/mente-cli/src/records.rs", "test_in_records", true),
        ("compiler/mente-cli/src/lib.rs", "test_in_lib", true),
        ("compiler/mente-cli/src/lib.rs", "test_in_records", false),
        ("compiler/mente-cli/src/main.rs", "test_in_main", true),
        ("compiler/mente-cli/src/main.rs", "test_in_mir_run", false),
        ("compiler/mente-cli/src/main.rs", "test_in_lib", false),
        ("compiler/mente-cli/src/main.rs", "test_in_extra", false),
        ("compiler/mente-cli/src/bin/extra.rs", "test_in_extra", true),
        ("compiler/mente-cli/src/bin/extra.rs", "test_in_main", false),
        // A root keeps its submodules' files beside it: `mod shared;` in src/bin/extra.rs is
        // src/bin/shared/mod.rs, and `mod common;` in tests/escapes.rs is tests/common/mod.rs.
        (
            "compiler/mente-cli/src/bin/extra.rs",
            "test_in_shared",
            false,
        ),
        (
            "compiler/mente-cli/src/bin/shared/mod.rs",
            "test_in_shared",
            true,
        ),
        (
            "compiler/mente-cli/tests/escapes.rs",
            "test_in_common",
            false,
        ),
        (
            "compiler/mente-cli/tests/common/mod.rs",
            "test_in_common",
            true,
        ),
        (
            "compiler/mente-cli/tests/escapes.rs",
            "test_in_escapes_inline",
            true,
        ),
        // src/bin/tool/main.rs is the root of the binary tool, and the files beside it its modules.
        (
            "compiler/mente-cli/src/bin/tool/main.rs",
            "test_in_tool",
            true,
        ),
        (
            "compiler/mente-cli/src/bin/tool/main.rs",
            "test_in_tool_inline",
            true,
        ),
        (
            "compiler/mente-cli/src/bin/tool/main.rs",
            "test_in_tool_parse",
            false,
        ),
        (
            "compiler/mente-cli/src/bin/tool/parse.rs",
            "test_in_tool_parse",
            true,
        ),
        ("compiler/mente-cli/src/main.rs", "test_in_tool", false),
        ("compiler/mente-cli/src/main.rs", "test_in_shared", false),
        ("compiler/mente-cli/src/mir_run.rs", "test_in_mir_run", true),
        (
            "compiler/mente-cli/src/mir_run.rs",
            "test_in_coverage",
            false,
        ),
        (
            "compiler/mente-cli/src/mir_run/coverage.rs",
            "test_in_coverage",
            true,
        ),
        (
            "compiler/mente-cli/src/catalogue.rs",
            "test_in_catalogue",
            true,
        ),
        (
            "compiler/mente-interp/src/tests.rs",
            "test_runs_stop_at_their_limits",
            true,
        ),
        (
            "compiler/mente-interp/src/lib.rs",
            "test_runs_stop_at_their_limits",
            false,
        ),
        (
            "compiler/mente-mir/src/cfg/mod.rs",
            "test_runs_stop_at_their_limits",
            true,
        ),
        (
            "compiler/mente-mir/src/cfg.rs",
            "test_runs_stop_at_their_limits",
            true,
        ),
        (
            "compiler/mente-mir/src/lib.rs",
            "test_runs_stop_at_their_limits",
            false,
        ),
        ("compiler/mente-cli/src/records.rs", "in_records", false),
    ] {
        assert_eq!(
            runs.runs_rust_in(&dir, target, name),
            Ok(found),
            "{} {}",
            target,
            name
        );
    }
    for target in [
        "spikes/x/src/lib.rs",
        "compiler/mente-cli/Cargo.toml",
        "compiler/mente-cli/benches/x.rs",
        "compiler/mente-cli/examples/x.rs",
        "compiler/mente-cli",
    ] {
        let found = runs
            .runs_rust_in(&dir, target, "test_in_records")
            .expect_err(target);
        assert!(
            found.contains("a file nextest builds no test from"),
            "{}",
            found
        );
    }
}

fn escape_with(guard: &str) -> String {
    format!(
        "---\ndate: 2026-11-14\nfixed-by: https://github.com/Menteion/Mente/pull/129\narea: checker\nmissed-by: No lint.\nguard: {}\n---\n",
        guard
    )
}

/// Each kind of guard is found in its runner's list: a Rust test of the package it names, which
/// nextest runs; a check at the top of the policy's own file; a test any Node run ran; a quick
/// check by name. Skipped, left to do, ignored and filtered tests do not count, and a Rust guard
/// that names no package does not read, since one name may stand in two crates.
#[test]
fn test_each_guard_is_found_in_its_runners_list() {
    let runs = runs().unwrap();
    for (written, found) in [
        ("fn test_runs_stop_at_their_limits in mente-interp", true),
        ("fn test_runs_stop_at_their_limits in mente-mir", true),
        ("fn test_runs_stop_at_their_limits in mente-sema", false),
        ("fn test_ignored in mente-cli", false),
        ("fn test_filtered in mente-cli", false),
        ("fn test_nowhere in mente-cli", false),
        ("policy check \"a plain check\"", true),
        ("policy check \"a test inside\"", false),
        ("policy check \"a nested check\"", false),
        ("node test \"a nested check\"", true),
        ("policy check \"a skipped check\"", false),
        ("policy check \"a check left to do\"", false),
        ("node test \"a deeper test\"", true),
        ("node test \"a subtest\"", true),
        ("node test \"a skipped check\"", false),
        ("node test \"a check left to do\"", false),
        ("node test \"a suite\"", false),
        ("quick check \"MIR mutation gate\"", true),
        ("quick check \"MIR mutation\"", false),
    ] {
        let rows = [escapes::parse("walker-hid", &escape_with(written)).unwrap()];
        let problems = evidence::guards(&rows, &runs);
        assert_eq!(problems.is_empty(), found, "{}: {:?}", written, problems);
        if !found {
            assert!(
                problems[0].starts_with("walker-hid: its guard "),
                "{:?}",
                problems
            );
        }
    }
    for written in [
        "fn test_runs_stop_at_their_limits",
        "fn test_x in",
        "fn test_x in serde",
        "fn test_x in mente-cli!",
    ] {
        assert!(
            escapes::parse("walker-hid", &escape_with(written)).is_err(),
            "{}",
            written
        );
    }
}

/// Each test named as evidence is checked against the tests nextest runs from its target.
#[test]
fn test_each_test_named_as_evidence_runs_from_its_target() {
    let dir = tree("named").unwrap();
    let named = |line: usize, target: &str, name: &str| Named {
        document: "spec/a.md".to_string(),
        line,
        target: target.to_string(),
        name: name.to_string(),
    };
    let problems = evidence::evidence(
        &dir,
        &[
            named(3, "compiler/mente-cli/tests/escapes.rs", "test_in_escapes"),
            named(4, "compiler/mente-cli/tests/escapes.rs", "test_ignored"),
            named(5, "spikes/x/src/lib.rs", "test_in_spike"),
        ],
        &runs().unwrap(),
    );
    assert_eq!(problems.len(), 2, "{:?}", problems);
    assert!(
        problems[0].starts_with("spec/a.md:4: `fn test_ignored`"),
        "{:?}",
        problems
    );
    assert!(
        problems[1]
            .starts_with("spec/a.md:5: spikes/x/src/lib.rs: a file nextest builds no test from"),
        "{:?}",
        problems
    );
}

/// The quick checks are read by name from `tools/quick-checks.json`.
#[test]
fn test_the_quick_checks_are_read_by_name() {
    let text = fs::read_to_string(root().join("tools/quick-checks.json")).unwrap();
    let names = evidence::quick_checks(&text).unwrap();
    assert!(
        names.iter().any(|n| n == "MIR mutation gate"),
        "{:?}",
        names
    );
    for (text, why) in [
        ("[]", "no JSON object"),
        ("{}", "holds no `checks`"),
        ("{\"checks\": [{\"run\": []}]}", "with no `name`"),
        ("{\"checks\": [\"a\"]}", "is no object"),
    ] {
        let found = evidence::quick_checks(text).expect_err(text);
        assert!(found.contains(why), "{}: {}", text, found);
    }
}

/// `mn evidence` run in `dir` with `args`: its exit code, its standard output and its standard
/// error. A failure to start it fails the test that asked.
fn mn(dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let out = match std::process::Command::new(env!("CARGO_BIN_EXE_mn"))
        .arg("evidence")
        .args(args)
        .current_dir(dir)
        .output()
    {
        Ok(out) => out,
        Err(e) => panic!("run mn: {}", e),
    };
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// `mn evidence` passes where every guard and every test named as evidence runs, and fails with
/// each that does not, with a list that cannot be read, and with an argument it does not take.
#[test]
fn test_mn_evidence_checks_guards_and_evidence_against_the_lists() {
    let escape = "---\ndate: 2026-11-14\nfixed-by: https://github.com/Menteion/Mente/pull/129\narea: harness\nmissed-by: No test.\nguard: node test \"a subtest\"\n---\n";
    let quick = "{\"checks\": [{\"name\": \"MIR mutation gate\"}]}";
    let guard_report = spec_guard(&[&count(
        "spec/a.md",
        2,
        "fn test_in_escapes",
        &["compiler/mente-cli/tests/escapes.rs"],
    )]);
    let mut files: Vec<(&str, String)> = TREE.iter().map(|f| (*f, String::new())).collect();
    files.extend([
        ("docs/escapes/broker-priced.md", escape.to_string()),
        ("tools/quick-checks.json", quick.to_string()),
        ("nextest.json", list(SUITES)),
        ("node.jsonl", report().unwrap()),
        ("spec-guard.json", guard_report.clone()),
        ("package.json", SCRIPT.to_string()),
    ]);
    files.extend(DOCUMENT_FILES.map(|f| (f, String::new())));
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    let dir = planted("cli", &refs).unwrap();
    let all = [
        "--spec-guard",
        "spec-guard.json",
        "--nextest",
        "nextest.json",
        "--node",
        "node.jsonl",
    ];
    assert_eq!(
        mn(&dir, &all),
        (
            Some(0),
            "[PASS] mn evidence: every guard of docs/escapes and every test named as evidence runs\n"
                .to_string(),
            String::new()
        )
    );
    let (code, out, err) = mn(&dir, &all[..4]);
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert_eq!(
        err,
        "[FAIL] broker-priced: its guard node test \"a subtest\" is no test a Node run ran\n"
    );
    fs::write(
        dir.join("spec-guard.json"),
        guard_report.replace("test_in_escapes", "test_ignored"),
    )
    .unwrap();
    let (code, _, err) = mn(&dir, &all);
    assert_eq!(code, Some(1));
    assert!(
        err.starts_with("[FAIL] spec/a.md:2: `fn test_ignored`"),
        "{}",
        err
    );
    fs::write(dir.join("spec-guard.json"), &guard_report).unwrap();
    // A report of a check of one document alone, and no script to read the documents from.
    fs::write(
        dir.join("spec-guard.json"),
        guard_report.replace("\"CLAUDE.md\", \"spec/a.md\"", "\"spec/a.md\""),
    )
    .unwrap();
    let (code, _, err) = mn(&dir, &all);
    assert_eq!(code, Some(1));
    assert!(
        err.starts_with(
            "[FAIL] spec-guard.json: spec-guard's report covers other documents than its script names"
        ),
        "{}",
        err
    );
    fs::write(dir.join("spec-guard.json"), &guard_report).unwrap();
    fs::rename(dir.join("package.json"), dir.join("package.off")).unwrap();
    let (code, _, err) = mn(&dir, &all);
    assert_eq!(code, Some(1));
    assert!(
        err.starts_with("[FAIL] ") && err.contains("package.json"),
        "{}",
        err
    );
    fs::rename(dir.join("package.off"), dir.join("package.json")).unwrap();
    for (args, starts) in [
        (
            &["--spec-guard", "missing.json", "--nextest", "nextest.json"][..],
            "[FAIL] missing.json: ",
        ),
        (
            &["--spec-guard", "nextest.json", "--nextest", "nextest.json"][..],
            "[FAIL] nextest.json: ",
        ),
        (
            &["--spec-guard", "spec-guard.json", "--nextest", "node.jsonl"][..],
            "[FAIL] node.jsonl: ",
        ),
        (
            &[
                "--spec-guard",
                "spec-guard.json",
                "--nextest",
                "nextest.json",
                "--node",
                "nextest.json",
            ][..],
            "[FAIL] nextest.json: ",
        ),
    ] {
        let (code, out, err) = mn(&dir, args);
        assert_eq!((code, out.as_str()), (Some(1), ""), "{:?}", args);
        assert!(err.starts_with(starts), "{:?}: {}", args, err);
    }
    fs::remove_file(dir.join("tools/quick-checks.json")).unwrap();
    let (code, _, err) = mn(&dir, &all);
    assert_eq!(code, Some(1));
    assert!(
        err.starts_with("[FAIL] ") && err.contains("quick-checks.json"),
        "{}",
        err
    );
    for args in [
        &[][..],
        &["--nextest", "nextest.json"][..],
        &["--spec-guard", "spec-guard.json"][..],
        &[
            "--spec-guard",
            "spec-guard.json",
            "--spec-guard",
            "spec-guard.json",
            "--nextest",
            "nextest.json",
        ][..],
        &["--spec-guard"][..],
        &["--junit", "node.jsonl"][..],
    ] {
        let (code, out, err) = mn(&dir, args);
        assert_eq!((code, out.as_str()), (Some(1), ""), "{:?}", args);
        assert!(
            err.starts_with(
                "Usage: mn evidence --spec-guard <file> --nextest <file>... [--node <file>]..."
            ),
            "{:?}: {}",
            args,
            err
        );
        for command in [evidence::SPEC_GUARD, evidence::NEXTEST, evidence::NODE] {
            assert!(err.contains(command), "{}", err);
        }
    }
}
