//! What the runners run (`src/evidence.rs`): nextest's list and the Node runs' reports, read as the
//! runners write them, and `mn evidence`, which checks the guards of the escaped bugs and the tests
//! named as evidence against them. The lists here are planted in the runners' own forms.

use mente_cli::escapes;
use mente_cli::evidence::{self, NodeTest, Runs, RustTest};
use mente_cli::json::{self, Json};

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// nextest's list, as `cargo-nextest nextest list --message-format json` writes it, of `suites`:
/// each a binary, its package and its tests, each with whether it is ignored.
fn list(suites: &[Suite]) -> String {
    let suites: Vec<String> = suites
        .iter()
        .map(|(binary, package, tests)| {
            let tests: Vec<String> = tests
                .iter()
                .map(|(path, ignored)| {
                    format!(
                        "\"{}\": {{\"ignored\": {}, \"filter-match\": {{\"status\": \"matches\"}}}}",
                        path, ignored
                    )
                })
                .collect();
            format!(
                "\"{b}\": {{\"package-name\": \"{p}\", \"binary-id\": \"{b}\", \"kind\": \"lib\", \"testcases\": {{{t}}}}}",
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

/// A binary of nextest's list: its id, its package, and its tests with whether each is ignored.
type Suite<'a> = (&'a str, &'a str, &'a [(&'a str, bool)]);

const SUITES: &[Suite] = &[
    (
        "mente-cli",
        "mente-cli",
        &[("records::tests::test_in_records", false)],
    ),
    (
        "mente-cli::bin/mn",
        "mente-cli",
        &[
            ("tests::test_in_main", false),
            ("mir_run::tests::test_in_mir_run", false),
        ],
    ),
    (
        "mente-cli::escapes",
        "mente-cli",
        &[("test_in_escapes", false), ("test_ignored", true)],
    ),
    (
        "mente-sema",
        "mente-sema",
        &[
            ("tier1::smt::tests::test_in_smt", false),
            ("tests::test_in_lib", false),
        ],
    ),
];

/// A Node run's report in JUnit's form, as `node --test --test-reporter=junit` writes it.
const REPORT: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<testsuites>\n\
\t<testcase name=\"a check &amp; its &quot;quotes&quot; &#35;1\" time=\"0.1\" classname=\"test\" file=\"C:\\w\\.github\\scripts\\policy.test.mjs\"/>\n\
\t<testcase name=\"a plain check\" time=\"0.1\" classname=\"test\" file=\"/w/.github/scripts/policy.test.mjs\"/>\n\
\t<testcase name=\"a run test\" time=\"0.1\" classname=\"test\" file=\"/w/.github/scripts/policy-run.test.mjs\"/>\n\
\t<testcase name=\"a harness test\" time=\"0.1\" classname=\"test\" file=\"/w/benchmark/harness/harness.test.ts\"/>\n\
\t<testcase name=\"a skipped test\" time=\"0\" classname=\"test\" file=\"/w/site/site.test.mjs\">\n\
\t\t<skipped type=\"skipped\" message=\"later\"/>\n\
\t</testcase>\n\
\t<testsuite name=\"a group\">\n\
\t\t<testcase name=\"a nested test\" time=\"0.1\" classname=\"test\" file=\"/w/site/site.test.mjs\"></testcase>\n\
\t</testsuite>\n\
</testsuites>\n";

fn runs() -> Result<Runs, String> {
    Ok(Runs {
        rust: evidence::nextest(&list(SUITES))?,
        node: evidence::junit(REPORT)?,
        quick: vec!["MIR mutation gate".to_string()],
    })
}

/// nextest's list is read test by test, with its binary, package, path and whether it is ignored;
/// a list of another form, or one that holds no test, is refused.
#[test]
fn test_nextest_list_is_read_as_nextest_writes_it() {
    let tests = evidence::nextest(&list(SUITES)).unwrap();
    assert_eq!(tests.len(), 7);
    assert!(tests.contains(&RustTest {
        binary: "mente-cli::escapes".to_string(),
        package: "mente-cli".to_string(),
        path: "test_ignored".to_string(),
        ignored: true,
    }));
    for (text, why) in [
        ("[]".to_string(), "no JSON object"),
        ("{}".to_string(), "no `rust-suites`"),
        ("{\"rust-suites\": {}}".to_string(), "holds no test"),
        (
            "{\"rust-suites\": {\"a\": {\"binary-id\": \"a\", \"testcases\": {}}}}".to_string(),
            "a suite with no",
        ),
        (
            "{\"rust-suites\": {\"a\": {\"package-name\": \"a\", \"binary-id\": \"a\", \"testcases\": {\"t\": {}}}}}"
                .to_string(),
            "a test with no `ignored`",
        ),
        ("{\"rust-suites\": ".to_string(), "ends before a value"),
    ] {
        let found = evidence::nextest(&text).expect_err(&text);
        assert!(found.contains(why), "{}: {}", text, found);
    }
}

/// A Node report is read test by test, nested ones too, with its file, its name as XML spells it
/// and whether it was skipped; a report of another form, or one that holds no test, is refused.
#[test]
fn test_a_node_report_is_read_as_node_writes_it() {
    let tests = evidence::junit(REPORT).unwrap();
    let names: Vec<(&str, bool)> = tests.iter().map(|t| (t.name.as_str(), t.skipped)).collect();
    assert_eq!(
        names,
        [
            ("a check & its \"quotes\" #1", false),
            ("a plain check", false),
            ("a run test", false),
            ("a harness test", false),
            ("a skipped test", true),
            ("a nested test", false),
        ]
    );
    assert_eq!(tests[0].file, "C:/w/.github/scripts/policy.test.mjs");
    for (text, why) in [
        ("<testcase name=\"a\" file=\"b\"/>", "no `<testsuites>`"),
        ("<testsuites></testsuites>", "holds no test"),
        (
            "<testsuites><testcase name=\"a\"/></testsuites>",
            "no `name` or `file`",
        ),
        (
            "<testsuites><testcase name=\"a &bogus; b\" file=\"c\"/>",
            "an entity XML has not",
        ),
        (
            "<testsuites><testcase name=\"a &amp\" file=\"c\"/>",
            "does not end",
        ),
        ("<testsuites><testcase name=a file=\"c\"/>", "has no quotes"),
        (
            "<testsuites><testcase name=\"a file=\"c\"/>",
            "no `name` or `file`",
        ),
        (
            "<testsuites><testcase name=\"a\" file=\"c\">",
            "no `</testcase>`",
        ),
        (
            "<testsuites><testcase name=\"a\" file=\"c\"",
            "does not close",
        ),
    ] {
        let found = evidence::junit(text).expect_err(text);
        assert!(found.contains(why), "{}: {}", text, found);
    }
    let other =
        evidence::junit("<testsuites><testcases/><testcase name='a' file='b'/></testsuites>");
    assert_eq!(
        other.unwrap(),
        [NodeTest {
            file: "b".to_string(),
            name: "a".to_string(),
            skipped: false
        }]
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

/// A test runs from a target file when nextest lists it, not ignored, in the package of the file:
/// a file of `tests/` is the binary of its name, `src/lib.rs` the library, `src/main.rs` a binary,
/// and another file of `src/` the tests under its module's path. A file nextest builds no test
/// from is an error.
#[test]
fn test_a_test_runs_from_its_target_file() {
    let runs = runs().unwrap();
    for (target, name, found) in [
        (
            "compiler/mente-cli/tests/escapes.rs",
            "test_in_escapes",
            true,
        ),
        ("compiler/mente-cli/tests/escapes.rs", "test_ignored", false),
        ("compiler/mente-cli/tests/cli.rs", "test_in_escapes", false),
        ("compiler/mente-cli/src/records.rs", "test_in_records", true),
        (
            "compiler/mente-cli/src/escapes.rs",
            "test_in_records",
            false,
        ),
        ("compiler/mente-cli/src/main.rs", "test_in_main", true),
        ("compiler/mente-cli/src/lib.rs", "test_in_main", false),
        ("compiler/mente-cli/src/mir_run.rs", "test_in_mir_run", true),
        (
            "compiler/mente-cli/src/mir_run/mod.rs",
            "test_in_mir_run",
            true,
        ),
        ("compiler/mente-sema/src/tier1/smt.rs", "test_in_smt", true),
        ("compiler/mente-sema/src/tier1.rs", "test_in_smt", true),
        ("compiler/mente-sema/src/lib.rs", "test_in_lib", true),
        ("compiler/mente-sema/src/lib.rs", "test_in_escapes", false),
        ("compiler/mente-cli/src/records.rs", "in_records", false),
    ] {
        assert_eq!(
            runs.runs_rust_in(target, name),
            Ok(found),
            "{} {}",
            target,
            name
        );
    }
    for target in [
        "spikes/x/src/lib.rs",
        "compiler/mente-cli/Cargo.toml",
        "compiler/mente-cli/tests/stack/x.rs",
        "compiler/mente-cli/benches/x.rs",
        "compiler/mente-cli",
    ] {
        let found = runs
            .runs_rust_in(target, "test_in_records")
            .expect_err(target);
        assert!(
            found.contains("a file nextest builds no test from"),
            "{}",
            found
        );
    }
    assert!(runs.runs_rust("test_in_smt"));
    assert!(!runs.runs_rust("test_ignored"));
    assert!(!runs.runs_rust("smt::tests::test_in_smt"));
}

/// Each kind of guard is found in its runner's list: a Rust test nextest lists, not ignored; a
/// check the policy's own file ran; a test any Node run ran, not skipped; a quick check by name.
#[test]
fn test_each_guard_is_found_in_its_runners_list() {
    let runs = runs().unwrap();
    let guard = |guard: &str| {
        let text = format!(
            "---\ndate: 2026-11-14\nfixed-by: https://github.com/Menteion/Mente/pull/129\narea: checker\nmissed-by: No lint.\nguard: {}\n---\n",
            guard
        );
        escapes::parse("walker-hid", &text).unwrap()
    };
    for (written, found) in [
        ("fn test_in_smt", true),
        ("fn test_ignored", false),
        ("fn test_nowhere", false),
        ("policy check \"a plain check\"", true),
        ("'policy check \"a check & its #1\"'", false),
        ("policy check \"a run test\"", false),
        ("policy check \"a harness test\"", false),
        ("node test \"a harness test\"", true),
        ("node test \"a nested test\"", true),
        ("node test \"a skipped test\"", false),
        ("node test \"a check\"", false),
        ("quick check \"MIR mutation gate\"", true),
        ("quick check \"MIR mutation\"", false),
    ] {
        let rows = [guard(written)];
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
}

/// A test named as evidence is an `@assert-count` on a line of its own, outside fenced code, whose
/// symbol is `fn test_...`; each is checked against the tests nextest runs from its target.
#[test]
fn test_each_test_named_as_evidence_runs_from_its_target() {
    let text = "# A spec\n\
        - `Implemented` a rule.\n\
        \x20 <!-- @assert-count target=\"compiler/mente-cli/tests/escapes.rs\" symbol=\"fn test_in_escapes\" min=\"1\" -->\n\
        \x20 <!-- @assert-count target='compiler/mente-sema/src/tier1/smt.rs' symbol='fn test_in_smt(' min='1' -->\n\
        \x20 <!-- @assert-count target=\"compiler/mente-cli/tests/escapes.rs\" symbol=\"fn test_ignored\" min=\"1\" -->\n\
        \x20 <!-- @assert-count target=\"compiler/mente-cli/src/records.rs\" symbol=\"pub fn is_key\" min=\"1\" -->\n\
        \x20 <!-- @assert-count target=\"spikes/x/src/lib.rs\" symbol=\"fn test_in_spike\" min=\"1\" -->\n\
        Prose that writes <!-- @assert-count target=\"a\" symbol=\"fn test_inline\" --> inside a line.\n\
        ```md\n\
        <!-- @assert-count target=\"compiler/mente-cli/tests/escapes.rs\" symbol=\"fn test_fenced\" min=\"1\" -->\n\
        ```\n";
    let found = evidence::directives(text);
    let names: Vec<(usize, &str)> = found.iter().map(|(l, _, n)| (*l, n.as_str())).collect();
    assert_eq!(
        names,
        [
            (3, "test_in_escapes"),
            (4, "test_in_smt"),
            (5, "test_ignored"),
            (7, "test_in_spike")
        ]
    );
    let problems = evidence::evidence(
        &[("spec/a.md".to_string(), text.to_string())],
        &runs().unwrap(),
    );
    assert_eq!(problems.len(), 2, "{:?}", problems);
    assert!(
        problems[0].starts_with("spec/a.md:5: `fn test_ignored`"),
        "{:?}",
        problems
    );
    assert!(
        problems[1]
            .starts_with("spec/a.md:7: spikes/x/src/lib.rs: a file nextest builds no test from"),
        "{:?}",
        problems
    );
}

/// The documents read are those the script `spec-guard` of `package.json` names, and each of them.
#[test]
fn test_the_documents_are_those_spec_guard_reads() {
    let text = fs::read_to_string(root().join("package.json")).unwrap();
    let Ok(Json::Object(top)) = json::parse(&text) else {
        panic!("package.json is no object")
    };
    let Some((_, Json::Object(scripts))) = top.iter().find(|(k, _)| k == "scripts") else {
        panic!("package.json has no scripts")
    };
    let Some((_, Json::Str(script))) = scripts.iter().find(|(k, _)| k == "spec-guard") else {
        panic!("package.json has no script spec-guard")
    };
    let patterns: Vec<String> = script
        .split_whitespace()
        .skip(1)
        .map(|p| p.trim_matches('"').to_string())
        .collect();
    assert_eq!(patterns, evidence::DOCUMENTS);
    let found = evidence::documents(&root());
    for path in [
        "README.md",
        "ROADMAP.md",
        "CLAUDE.md",
        "docs/design-review.md",
        "spec/core-calculus.md",
    ] {
        assert!(
            found.iter().any(|f| f == path),
            "{} not among {:?}",
            path,
            found
        );
    }
    assert!(found.iter().all(|f| f.ends_with(".md")), "{:?}", found);
}

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
    let escape = "---\ndate: 2026-11-14\nfixed-by: https://github.com/Menteion/Mente/pull/129\narea: harness\nmissed-by: No test.\nguard: node test \"a harness test\"\n---\n";
    let spec = "- `Implemented` a rule.\n  <!-- @assert-count target=\"compiler/mente-cli/tests/escapes.rs\" symbol=\"fn test_in_escapes\" min=\"1\" -->\n";
    let quick = "{\"checks\": [{\"name\": \"MIR mutation gate\"}]}";
    let files = [
        ("docs/escapes/broker-priced.md", escape),
        ("spec/a.md", spec),
        ("tools/quick-checks.json", quick),
        ("README.md", ""),
        ("ROADMAP.md", ""),
        ("CLAUDE.md", ""),
        ("nextest.json", &list(SUITES)),
        ("node.xml", REPORT),
    ];
    let dir = planted("cli", &files).unwrap();
    let both = ["--nextest", "nextest.json", "--junit", "node.xml"];
    assert_eq!(
        mn(&dir, &both),
        (
            Some(0),
            "[PASS] mn evidence: every guard of docs/escapes and every test named as evidence runs\n".to_string(),
            String::new()
        )
    );
    let (code, out, err) = mn(&dir, &["--nextest", "nextest.json"]);
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert_eq!(
        err,
        "[FAIL] broker-priced: its guard node test \"a harness test\" is no test a Node run ran\n"
    );
    fs::write(
        dir.join("spec/a.md"),
        spec.replace("test_in_escapes", "test_ignored"),
    )
    .unwrap();
    let (code, _, err) = mn(&dir, &both);
    assert_eq!(code, Some(1));
    assert!(
        err.starts_with("[FAIL] spec/a.md:2: `fn test_ignored`"),
        "{}",
        err
    );
    fs::write(dir.join("spec/a.md"), spec).unwrap();
    for (args, starts) in [
        (&["--nextest", "missing.json"][..], "[FAIL] missing.json: "),
        (&["--nextest", "node.xml"][..], "[FAIL] node.xml: "),
        (
            &["--nextest", "nextest.json", "--junit", "nextest.json"][..],
            "[FAIL] nextest.json: ",
        ),
    ] {
        let (code, out, err) = mn(&dir, args);
        assert_eq!((code, out.as_str()), (Some(1), ""), "{:?}", args);
        assert!(err.starts_with(starts), "{:?}: {}", args, err);
    }
    fs::remove_file(dir.join("ROADMAP.md")).unwrap();
    let (code, _, err) = mn(&dir, &both);
    assert_eq!(code, Some(1));
    assert!(
        err.starts_with("[FAIL] ") && err.contains("ROADMAP.md"),
        "{}",
        err
    );
    for args in [
        &[][..],
        &["--junit", "node.xml"][..],
        &["--nextest"][..],
        &["--nextest", "nextest.json", "--junit"][..],
        &["--list", "nextest.json"][..],
    ] {
        let (code, out, err) = mn(&dir, args);
        assert_eq!((code, out.as_str()), (Some(1), ""), "{:?}", args);
        assert!(
            err.starts_with("Usage: mn evidence --nextest <file>... [--junit <file>]..."),
            "{:?}: {}",
            args,
            err
        );
        assert!(
            err.contains(evidence::NEXTEST) && err.contains(evidence::NODE),
            "{}",
            err
        );
    }
}
