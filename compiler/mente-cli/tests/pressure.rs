//! The report of the pressure on the repository's files (`src/pressure.rs`), from a log as
//! `pressure::LOG` prints it, and `mn pressure`, which reads one on its standard input.

use mente_cli::pressure::{self, Merge};

use std::io::Write;
use std::process::{Command, Stdio};

/// A log as `pressure::LOG` prints it: a merge of pull request 170 and of 168 on two days, a
/// commit that merged no pull request, and each merge's files.
const LOG: &str = "\0\
2026-11-17\tMerge pull request #170 from Menteion/scheduler-model-check\n\
\n\
docs/design-review.md\n\
compiler/mente-cli/src/main.rs\n\
\0\
2026-11-16\tFix a typo on main\n\
\n\
README.md\n\
\0\
2026-11-15\tMerge pull request #168 from Menteion/workflow-files\n\
\n\
docs/design-review.md\n\
docs/escapes/walker-wildcard-hid-operands.md\n\
.github/scripts/policy.test.mjs\n";

fn merge(day: &str, pull: u32, files: &[&str]) -> Merge {
    Merge {
        day: day.to_string(),
        pull,
        files: files.iter().map(|f| f.to_string()).collect(),
    }
}

/// The log is read merge by merge, a commit that merged no pull request passed over, CRLF line
/// ends read as LF; a log of another form is refused, never read as one with no merge.
#[test]
fn test_a_log_is_read_merge_by_merge() {
    let expected = vec![
        merge(
            "2026-11-17",
            170,
            &["docs/design-review.md", "compiler/mente-cli/src/main.rs"],
        ),
        merge(
            "2026-11-15",
            168,
            &[
                "docs/design-review.md",
                "docs/escapes/walker-wildcard-hid-operands.md",
                ".github/scripts/policy.test.mjs",
            ],
        ),
    ];
    assert_eq!(pressure::merges(LOG).unwrap(), expected);
    assert_eq!(
        pressure::merges(&LOG.replace('\n', "\r\n")).unwrap(),
        expected
    );
    assert_eq!(pressure::merges("").unwrap(), vec![]);
    assert_eq!(pressure::merges("\n").unwrap(), vec![]);
    for (log, why) in [
        ("commit 0f3a\nAuthor: x\n", "does not start with a commit"),
        (
            "\0Merge pull request #1 from a/b\n",
            "commit 1 of the log starts with",
        ),
        (
            "\x002026-11-31\tMerge pull request #1 from a/b\n",
            "not a day",
        ),
        (
            "\x002026-11-17 Merge pull request #1 from a/b\n",
            "apart by a tab",
        ),
    ] {
        let found = pressure::merges(log).expect_err(log);
        assert!(found.contains(why), "{:?}: {}", log, found);
    }
    for (subject, pull) in [
        ("Merge pull request #170 from Menteion/x", Some(170)),
        ("Merge pull request #7", Some(7)),
        ("Merge pull request #17x from Menteion/x", None),
        ("Merge pull request # from Menteion/x", None),
        ("Merge pull request #99999999999 from Menteion/x", None),
        ("Merge branch 'main' into x", None),
        ("Revert \"Merge pull request #170 from Menteion/x\"", None),
    ] {
        let log = format!("\x002026-11-17\t{}\n\nREADME.md\n", subject);
        let found: Vec<u32> = pressure::merges(&log)
            .unwrap()
            .iter()
            .map(|m| m.pull)
            .collect();
        assert_eq!(found, pull.into_iter().collect::<Vec<_>>(), "{}", subject);
    }
}

/// The report counts each file once a pull request, ranks the files by how many changed them and
/// then by path, keeps the `top` of them, and says what each is among the records; `since` keeps
/// the merges of that day and after.
#[test]
fn test_the_report_ranks_files_by_the_pull_requests_that_changed_them() {
    let merges = pressure::merges(LOG).unwrap();
    assert_eq!(
        pressure::report(&merges, None, 20),
        "Pull requests merged from 2026-11-15 to 2026-11-17: 2.\n\
         The files the most of them changed:\n  \
         2  docs/design-review.md  (shared: the design register's entries)\n  \
         1  .github/scripts/policy.test.mjs\n  \
         1  compiler/mente-cli/src/main.rs\n  \
         1  docs/escapes/walker-wildcard-hid-operands.md  (a kind: an escaped bug)\n"
    );
    assert_eq!(
        pressure::report(&merges, Some("2026-11-16"), 1),
        "Pull requests merged from 2026-11-17 to 2026-11-17: 1.\n\
         The files the most of them changed:\n  \
         1  compiler/mente-cli/src/main.rs\n"
    );
    assert_eq!(
        pressure::report(&merges, Some("2026-11-15"), 2),
        "Pull requests merged from 2026-11-15 to 2026-11-17: 2.\n\
         The files the most of them changed:\n  \
         2  docs/design-review.md  (shared: the design register's entries)\n  \
         1  .github/scripts/policy.test.mjs\n"
    );
    assert_eq!(
        pressure::report(&merges, Some("2026-11-18"), 20),
        "No pull request merged in the log.\n"
    );
    let twice = vec![
        merge("2026-11-17", 1, &["a.md", "a.md", "b.md"]),
        merge("2026-11-17", 2, &["b.md"]),
    ];
    let many: Vec<Merge> = (1..=10)
        .map(|n| merge("2026-11-17", n, &["c.md"]))
        .collect();
    assert_eq!(
        pressure::report(&twice, None, 20),
        "Pull requests merged from 2026-11-17 to 2026-11-17: 2.\n\
         The files the most of them changed:\n  \
         2  b.md\n  \
         1  a.md\n"
    );
    assert_eq!(
        pressure::report(&[many, twice].concat(), None, 20),
        "Pull requests merged from 2026-11-17 to 2026-11-17: 12.\n\
         The files the most of them changed:\n  \
         10  c.md\n   \
         2  b.md\n   \
         1  a.md\n"
    );
}

/// `mn pressure` run with `args` and `input` on its standard input: its exit code, its standard
/// output and its standard error. A failure to run it fails the test that asked.
fn mn(args: &[&str], input: &str) -> (Option<i32>, String, String) {
    match run(args, input) {
        Ok(out) => out,
        Err(e) => panic!("run mn pressure: {}", e),
    }
}

fn run(args: &[&str], input: &str) -> std::io::Result<(Option<i32>, String, String)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mn"))
        .arg("pressure")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    Ok((
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// `mn pressure` prints the report of the log it reads, with `--since` and `--top` as the report
/// takes them, and fails with what is wrong: an argument it does not take, a day that is none, a
/// number that is none, and a log of another form.
#[test]
fn test_mn_pressure_reports_the_log_on_its_input() {
    let merges = pressure::merges(LOG).unwrap();
    assert_eq!(
        mn(&[], LOG),
        (Some(0), pressure::report(&merges, None, 20), String::new())
    );
    assert_eq!(
        mn(&["--since", "2026-11-16", "--top", "1"], LOG),
        (
            Some(0),
            pressure::report(&merges, Some("2026-11-16"), 1),
            String::new()
        )
    );
    assert_eq!(
        mn(&["--top", "1", "--since", "2026-11-16"], LOG),
        mn(&["--since", "2026-11-16", "--top", "1"], LOG)
    );
    let usage = format!(
        "Usage: {} | mn pressure [--since YYYY-MM-DD] [--top N]\n",
        pressure::LOG
    );
    for args in [
        &["--since"][..],
        &["--since", "2026-02-30"][..],
        &["--top", "0"][..],
        &["--top", "x"][..],
        &["--top"][..],
        &["--days", "7"][..],
        &["2026-11-16"][..],
        &["--top", "1", "--top", "2"][..],
        &["--since", "2026-11-15", "--since", "2026-11-16"][..],
    ] {
        assert_eq!(
            mn(args, LOG),
            (Some(1), String::new(), usage.clone()),
            "{:?}",
            args
        );
    }
    let (code, out, err) = mn(&[], "commit 0f3a\n");
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(err.starts_with("[FAIL] the log does not start"), "{}", err);
}
