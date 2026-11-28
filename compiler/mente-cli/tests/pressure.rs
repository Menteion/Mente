//! The report of the pressure on the repository's files (`src/pressure.rs`), from a log as
//! `pressure::LOG` prints it, and `mn pressure`, which reads one on its standard input. One test
//! runs `pressure::LOG` itself on a repository it makes, so that the command and the reader agree.

use mente_cli::pressure::{self, Merge};

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod support;

/// A log as `pressure::LOG` prints it: a merge of pull request 170 and of 168 on two days, each
/// with its two parents, a commit that merged no pull request, and each merge's files with their
/// statuses, a deletion among them.
const LOG: &str = "\0\
2026-11-17\taaaa bbbb\t\tMerge pull request #170 from Menteion/scheduler-model-check\n\
\n\
M\tdocs/design-review.md\n\
M\tcompiler/mente-cli/src/main.rs\n\
\0\
2026-11-16\tcccc\t\tFix a typo on main\n\
\n\
M\tREADME.md\n\
\0\
2026-11-15\tdddd eeee\t\tMerge pull request #168 from Menteion/workflow-files\n\
\n\
M\tdocs/design-review.md\n\
A\tdocs/escapes/walker-wildcard-hid-operands.md\n\
D\t.github/scripts/policy.test.mjs\n";

fn merge(day: &str, pull: u32, files: &[&str], deleted: &[&str]) -> Merge {
    Merge {
        day: day.to_string(),
        pull,
        files: files.iter().map(|f| f.to_string()).collect(),
        deleted: deleted.iter().map(|f| f.to_string()).collect(),
    }
}

/// The log is read merge by merge, a commit that merged no pull request passed over, CRLF line
/// ends read as LF. An empty log, as git leaves one when it fails, a log of another form, a merge
/// with fewer than two parents, as a shallow clone shows one, and a file with no status are
/// refused, each with what to do, never read as a log with no merge (the review of PR 176).
#[test]
fn test_a_log_is_read_merge_by_merge() {
    let expected = vec![
        merge(
            "2026-11-17",
            170,
            &["docs/design-review.md", "compiler/mente-cli/src/main.rs"],
            &[],
        ),
        merge(
            "2026-11-15",
            168,
            &[
                "docs/design-review.md",
                "docs/escapes/walker-wildcard-hid-operands.md",
                ".github/scripts/policy.test.mjs",
            ],
            &[".github/scripts/policy.test.mjs"],
        ),
    ];
    assert_eq!(pressure::merges(LOG).unwrap(), expected);
    assert_eq!(
        pressure::merges(&LOG.replace('\n', "\r\n")).unwrap(),
        expected
    );
    let merge_of = |parents: &str| {
        format!(
            "\x002026-11-17\t{}\t\tMerge pull request #1 from a/b\n\nM\tREADME.md\n",
            parents
        )
    };
    for (log, why) in [
        (
            "".to_string(),
            "the log is empty, as git leaves it when it fails: run `git fetch origin main`",
        ),
        ("\n".to_string(), "the log is empty"),
        (
            "commit 0f3a\nAuthor: x\n".to_string(),
            "does not start with a commit",
        ),
        (
            "\0Merge pull request #1 from a/b\n".to_string(),
            "commit 1 of the log starts with",
        ),
        (
            "\x002026-11-31\taaaa bbbb\t\tMerge pull request #1 from a/b\n".to_string(),
            "not a day",
        ),
        (
            "\x002026-11-17 aaaa bbbb Merge pull request #1 from a/b\n".to_string(),
            "its marks and a subject apart by tabs",
        ),
        (merge_of("aaaa"), "has 1 parent where a merge has two"),
        (merge_of(""), "has 0 parents where a merge has two"),
        (merge_of("aaaa"), "run `git fetch --unshallow`"),
        (
            "\x002026-11-17\taaaa\tgrafted\tFix a typo on main\n".to_string(),
            "commit 1 of the log is grafted: the history is shallow there",
        ),
        (
            "\x002026-11-17\taaaa\tHEAD -> main\tFix a typo on main\n".to_string(),
            "is marked \"HEAD -> main\"",
        ),
        (
            merge_of("aaaa bbbb").replace("M\tREADME.md", "README.md"),
            "holds \"README.md\", not a status and a path",
        ),
        (
            merge_of("aaaa bbbb").replace("M\tREADME.md", "R100\tREADME.md"),
            "not a status and a path",
        ),
    ] {
        let found = pressure::merges(&log).expect_err(&log);
        assert!(found.contains(why), "{:?}: {}", log, found);
    }
    assert_eq!(
        pressure::merges("\x002026-11-17\t\t\tFirst commit\n\nA\tREADME.md\n").unwrap(),
        vec![]
    );
    for (subject, pull) in [
        ("Merge pull request #170 from Menteion/x", Some(170)),
        ("Merge pull request #7", Some(7)),
        ("Merge pull request #17x from Menteion/x", None),
        ("Merge pull request # from Menteion/x", None),
        ("Merge pull request #99999999999 from Menteion/x", None),
        ("Merge branch 'main' into x", None),
        ("Revert \"Merge pull request #170 from Menteion/x\"", None),
    ] {
        let log = format!("\x002026-11-17\taaaa bbbb\t\t{}\n\nM\tREADME.md\n", subject);
        let found: Vec<u32> = pressure::merges(&log)
            .unwrap()
            .iter()
            .map(|m| m.pull)
            .collect();
        assert_eq!(found, pull.into_iter().collect::<Vec<_>>(), "{}", subject);
    }
}

/// The report counts each file once a pull request, ranks the files by how many changed them and
/// then by path, keeps the `top` of them, says what each is among the records, and marks a file
/// the newest of its pull requests deleted; `since` keeps the merges of that day and after.
#[test]
fn test_the_report_ranks_files_by_the_pull_requests_that_changed_them() {
    let merges = pressure::merges(LOG).unwrap();
    assert_eq!(
        pressure::report(&merges, None, 20),
        "Pull requests merged from 2026-11-15 to 2026-11-17: 2.\n\
         The files the most of them changed:\n  \
         2  docs/design-review.md  (shared: the design register's entries)\n  \
         1  .github/scripts/policy.test.mjs  (deleted)\n  \
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
        pressure::report(&merges, Some("2026-11-18"), 20),
        "No pull request merged in the log.\n"
    );
    let twice = vec![
        merge("2026-11-17", 1, &["a.md", "a.md", "b.md"], &[]),
        merge("2026-11-17", 2, &["b.md"], &["b.md"]),
    ];
    let back = vec![
        merge("2026-11-18", 3, &["b.md"], &[]),
        merge("2026-11-17", 2, &["b.md"], &["b.md"]),
    ];
    assert_eq!(
        pressure::report(&twice, None, 20),
        "Pull requests merged from 2026-11-17 to 2026-11-17: 2.\n\
         The files the most of them changed:\n  \
         2  b.md\n  \
         1  a.md\n"
    );
    assert_eq!(
        pressure::report(&back, None, 20),
        "Pull requests merged from 2026-11-17 to 2026-11-18: 2.\n\
         The files the most of them changed:\n  \
         2  b.md\n"
    );
    let gone = vec![merge(
        "2026-11-17",
        4,
        &["docs/escapes/old.md"],
        &["docs/escapes/old.md"],
    )];
    assert!(
        pressure::report(&gone, None, 20).contains("(deleted; a kind: an escaped bug)"),
        "{}",
        pressure::report(&gone, None, 20)
    );
}

/// `git`, isolated from this machine (`support::isolated_git`), with a fixed identity and day for
/// the commits it makes, run in `dir`: what it prints, or why it failed.
fn git(dir: &Path, args: &[&str]) -> io::Result<String> {
    let out = support::isolated_git(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-11-17T12:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-11-17T12:00:00Z")
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    String::from_utf8(out.stdout).map_err(io::Error::other)
}

/// A repository of this test binary's own, `name`, with no commit yet.
fn repository(name: &str) -> io::Result<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("pressure")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    git(&dir, &["init", "-q", "-b", "main"])?;
    Ok(dir)
}

/// A commit on the branch checked out in `dir` that writes each of `files` and removes each of
/// `removed`.
fn commit(dir: &Path, message: &str, files: &[&str], removed: &[&str]) -> io::Result<()> {
    for f in files {
        let path = dir.join(f);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, format!("{} {}\n", message, f))?;
        git(dir, &["add", "--", f])?;
    }
    for f in removed {
        git(dir, &["rm", "-q", "--", f])?;
    }
    git(dir, &["commit", "-q", "-m", message]).map(|_| ())
}

/// The merge of the branch `branch` into `main`, as the merge queue writes it.
fn merge_pull(dir: &Path, pull: u32, branch: &str) -> io::Result<()> {
    git(dir, &["checkout", "-q", "main"])?;
    let message = format!("Merge pull request #{} from Menteion/{}", pull, branch);
    git(dir, &["merge", "-q", "--no-ff", "-m", &message, branch]).map(|_| ())
}

/// `pressure::LOG`, word by word, run in `dir`.
fn log(dir: &Path) -> io::Result<String> {
    let words: Vec<&str> = pressure::LOG.split(' ').collect();
    match words.split_first() {
        Some((&"git", args)) => git(dir, args),
        _ => Err(io::Error::other("pressure::LOG does not run git")),
    }
}

/// `pressure::LOG` itself, run on a repository with two merged pull requests, a file renamed, one
/// deleted and one named beyond ASCII, a commit on `main` between them and `origin/main` at its
/// head, reads as the merges were made; a shallow clone of it is refused with what to do, since
/// the merges before where it stops are missing (the review of PR 176).
#[test]
fn test_the_log_command_reads_a_real_history() -> io::Result<()> {
    let dir = repository("history")?;
    commit(&dir, "Start", &["README.md", "old.md", "gone.md"], &[])?;
    git(&dir, &["checkout", "-q", "-b", "first"])?;
    commit(&dir, "Add a page", &["docs/caf\u{e9}.md"], &[])?;
    git(&dir, &["mv", "old.md", "new.md"])?;
    commit(&dir, "Rename a page", &[], &[])?;
    merge_pull(&dir, 11, "first")?;
    commit(&dir, "Fix a typo on main", &["README.md"], &[])?;
    git(&dir, &["checkout", "-q", "-b", "second"])?;
    commit(&dir, "Drop a page", &[], &["gone.md"])?;
    merge_pull(&dir, 12, "second")?;
    let head = git(&dir, &["rev-parse", "HEAD"])?;
    git(
        &dir,
        &["update-ref", "refs/remotes/origin/main", head.trim()],
    )?;
    let merges = pressure::merges(&log(&dir)?).unwrap();
    assert_eq!(
        merges,
        vec![
            merge("2026-11-17", 12, &["gone.md"], &["gone.md"]),
            merge(
                "2026-11-17",
                11,
                &["docs/caf\u{e9}.md", "new.md", "old.md"],
                &["old.md"]
            ),
        ]
    );
    let parent = dir.parent().unwrap();
    let _ = fs::remove_dir_all(parent.join("shallow"));
    let url = format!("file://{}", dir.display().to_string().replace('\\', "/"));
    git(parent, &["clone", "-q", "--depth", "2", &url, "shallow"])?;
    let found = pressure::merges(&log(&parent.join("shallow"))?).expect_err("a shallow history");
    assert!(found.contains("git fetch --unshallow"), "{}", found);
    Ok(())
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
/// number that is none, a log of another form, and an empty log, as git leaves one when it fails.
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
        "Usage: {} && {} | mn pressure [--since YYYY-MM-DD] [--top N]\n",
        pressure::FETCH,
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
    let (code, out, err) = mn(&[], "");
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(err.starts_with("[FAIL] the log is empty"), "{}", err);
}
