//! `mn fuzz` end to end: the worker process isolates a planted panic, stack overflow and hang,
//! and the fuzzer classifies, minimizes and writes each one; and every crash the fuzzer found,
//! minimized under `tests/fuzz`, replays through every phase without a failure once its fix
//! lands, on the edit seed its record names. A file whose fix has not landed is listed in
//! `PENDING` and must still fail, so that the main test stays green and each fix moves its file
//! into it.

// These helpers are test code: a failed setup step is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn mn(args: &[&str], plant: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mn"));
    command.args(args).env_remove("MENTE_FUZZ_PLANT");
    if let Some(p) = plant {
        command.env("MENTE_FUZZ_PLANT", p);
    }
    command.output().expect("run mn")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A fresh scratch directory for one test, removed by the caller.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mente-fuzz-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("seeds")).unwrap();
    dir
}

/// The description and the minimized input of the one failure `mn fuzz` wrote to `out`.
fn the_failure(out: &Path) -> (String, String) {
    let about: Vec<PathBuf> = fs::read_dir(out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "found"))
        .collect();
    assert_eq!(
        about.len(),
        1,
        "one failure expected in {}: {:?}",
        out.display(),
        about
    );
    let input = about[0].with_extension("mn");
    (
        fs::read_to_string(&about[0]).unwrap(),
        fs::read_to_string(input).unwrap(),
    )
}

/// A worker that panics, overflows its stack or stops answering costs the fuzzer one worker:
/// the failure is named, put in the phase it happened in, minimized down to the text that
/// triggers it, and written once.
#[test]
fn test_fuzz_worker_isolates_panics_stack_overflows_and_hangs() {
    for (plant, kind, message, seed) in [
        (
            "panic",
            "panic",
            "a planted panic [at ",
            "module m\npub fn f() -> i32 = 1\n// fuzz-plant\n",
        ),
        (
            "overflow",
            "crash",
            "stack overflow",
            "module m\npub fn f() -> i32 = 1\n// fuzz-plant\n",
        ),
        // A hang costs the phase budget and a grace each time, so its seed is all marker.
        ("hang", "hang", "", "fuzz-plant"),
    ] {
        let dir = scratch(plant);
        fs::write(dir.join("seeds").join("seed.mn"), seed).unwrap();
        let out = dir.join("out");
        let run = mn(
            &[
                "fuzz",
                "--seed",
                "1",
                "--runs",
                "1",
                "--phase-seconds",
                "1",
                "--corpus",
                dir.join("seeds").to_str().unwrap(),
                "--out",
                out.to_str().unwrap(),
            ],
            Some(plant),
        );
        assert_eq!(
            run.status.code(),
            Some(1),
            "{}: {}{}",
            plant,
            text(&run.stdout),
            text(&run.stderr)
        );
        let (about, minimized) = the_failure(&out);
        assert!(about.contains("phase: planted"), "{}: {}", plant, about);
        assert!(
            about.contains(&format!("kind: {}\n", kind)),
            "{}: {}",
            plant,
            about
        );
        assert!(
            about.contains(&format!("message: {}", message)),
            "{}: {}",
            plant,
            about
        );
        assert_eq!(minimized, "fuzz-plant", "{}: not minimized", plant);
        let summary = fs::read_to_string(out.join("summary.txt")).unwrap();
        assert!(
            summary.contains("runs: 1\n") && summary.contains("failures: 1\n"),
            "{}",
            summary
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

/// Without a plant, the same seed passes every phase, and `--replay` says so.
#[test]
fn test_fuzz_replay_passes_a_clean_module() {
    let dir = scratch("replay");
    let file = dir.join("seeds").join("clean.mn");
    fs::write(&file, "module m\npub fn f(x: u8) -> u16 = x + 1\n").unwrap();
    let run = mn(&["fuzz", "--replay", file.to_str().unwrap()], None);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}{}",
        text(&run.stdout),
        text(&run.stderr)
    );
    assert!(text(&run.stdout).contains("1 file(s), 0 with a failure"));
    let _ = fs::remove_dir_all(&dir);
}

/// A seed runs thorough through the worker: the phases compared run twice, the typed dump and
/// Tier 1's reports run, and so do the objects and both interpreters.
#[test]
fn test_fuzz_runs_a_seed_through_every_phase_thoroughly() {
    let dir = scratch("thorough");
    fs::write(
        dir.join("seeds").join("clean.mn"),
        "module m\npub fn f(x: u8) -> u16 = x + 1\n",
    )
    .unwrap();
    let out = dir.join("out");
    let run = mn(
        &[
            "fuzz",
            "--runs",
            "1",
            "--corpus",
            dir.join("seeds").to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}{}",
        text(&run.stdout),
        text(&run.stderr)
    );
    let summary = fs::read_to_string(out.join("summary.txt")).unwrap();
    for phase in [
        "dump-tokens: 2,",
        "check: 2,",
        "dump-typed: 2,",
        "tier1: 1,",
        "incremental: 1,",
        "lower: 2,",
        "lower-without-proofs: 2,",
        "object x86_64-unknown-linux-gnu: 2,",
        "object aarch64-unknown-linux-gnu: 2,",
        "object x86_64-pc-windows-msvc: 2,",
        "interp: 1,",
        "mir: 1,",
        "mir-without-proofs: 1,",
    ] {
        assert!(
            summary.contains(&format!("  {}", phase)),
            "no {} in {}",
            phase,
            summary
        );
    }
    assert!(summary.contains("failures: 0\n"), "{}", summary);
    assert!(summary.contains("worker starts: 1\n"), "{}", summary);
    let _ = fs::remove_dir_all(&dir);
}

/// `--replay` prints each failure of a file once, over all of its edits, and counts the files
/// with one; a file that hangs is not run again with the next edit.
#[test]
fn test_fuzz_replay_reports_each_failure_once() {
    let dir = scratch("replay-once");
    let seeds = dir.join("seeds");
    fs::write(seeds.join("clean.mn"), "module m\npub fn f() -> u8 = 7\n").unwrap();
    fs::write(seeds.join("marked.mn"), "module m\n// fuzz-plant\n").unwrap();
    let run = mn(
        &["fuzz", "--replay", seeds.to_str().unwrap()],
        Some("panic"),
    );
    let printed = text(&run.stdout);
    assert_eq!(
        run.status.code(),
        Some(1),
        "{}{}",
        printed,
        text(&run.stderr)
    );
    assert_eq!(
        printed.matches("panic in planted").count(),
        1,
        "{}",
        printed
    );
    assert!(
        printed.contains("marked.mn: panic in planted: a planted panic [at "),
        "{}",
        printed
    );
    assert!(
        printed.contains("2 file(s), 1 with a failure"),
        "{}",
        printed
    );
    let _ = fs::remove_dir_all(&dir);

    // Each hang costs the phase budget and a grace: once, not once for each of four edits.
    let dir = scratch("replay-hang");
    let file = dir.join("seeds").join("hang.mn");
    fs::write(&file, "fuzz-plant").unwrap();
    let start = std::time::Instant::now();
    let run = mn(
        &[
            "fuzz",
            "--phase-seconds",
            "1",
            "--replay",
            file.to_str().unwrap(),
        ],
        Some("hang"),
    );
    let took = start.elapsed();
    let printed = text(&run.stdout);
    assert_eq!(
        run.status.code(),
        Some(1),
        "{}{}",
        printed,
        text(&run.stderr)
    );
    assert_eq!(printed.matches("hang in planted").count(), 1, "{}", printed);
    assert!(
        took < std::time::Duration::from_secs(12),
        "replay took {:?}",
        took
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A found failure replays on the edit seed its record names, and on it alone: the planted
/// failure `edit` fires only on edit seed 4242424242, which the edits 0 to 3 of a file with no
/// record, or of a record whose seed is lost, never meet.
#[test]
fn test_fuzz_replay_takes_the_recorded_edit_seed() {
    let dir = scratch("replay-edit");
    let seeds = dir.join("seeds");
    let found = seeds.join("found.mn");
    fs::write(&found, "module m\n// fuzz-plant\n").unwrap();
    let replay = || {
        let run = mn(&["fuzz", "--replay", found.to_str().unwrap()], Some("edit"));
        (run.status.code(), text(&run.stdout), text(&run.stderr))
    };
    fs::write(
        seeds.join("found.found"),
        "mn fuzz found\nphase: planted\nedit seed: 4242424242\n",
    )
    .unwrap();
    let (code, printed, stderr) = replay();
    assert_eq!(code, Some(1), "{}{}", printed, stderr);
    assert!(
        printed.contains("found.mn: replayed on its recorded edit seed 4242424242"),
        "{}",
        printed
    );
    assert_eq!(
        printed
            .matches("found.mn: panic in planted: a planted panic on edit seed 4242424242 [at ")
            .count(),
        1,
        "{}",
        printed
    );
    // The same input with no record, or with its seed lost, replays on the edits 0 to 3.
    fs::write(
        seeds.join("found.found"),
        "mn fuzz found\nphase: planted\nedit seed: unknown\n",
    )
    .unwrap();
    let (code, printed, stderr) = replay();
    assert_eq!(code, Some(0), "{}{}", printed, stderr);
    assert!(
        printed.contains(
            "found.mn: its record names no edit seed, so it is replayed on the edits 0 to 3"
        ),
        "{}",
        printed
    );
    fs::remove_file(seeds.join("found.found")).unwrap();
    let (code, printed, stderr) = replay();
    assert_eq!(code, Some(0), "{}{}", printed, stderr);
    assert!(
        printed.contains("1 file(s), 0 with a failure, 0 with a bad record"),
        "{}",
        printed
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A bad record is reported, with what to write, and its file left out, while every other file
/// of the replay runs; the exit shows a failure and a bad record apart, and both together.
#[test]
fn test_fuzz_replay_goes_on_past_a_bad_record() {
    let dir = scratch("replay-bad");
    let seeds = dir.join("seeds");
    fs::write(seeds.join("a.mn"), "module m\n// fuzz-plant\n").unwrap();
    fs::write(
        seeds.join("a.found"),
        "mn fuzz found\nedit seed: 4242424242\n",
    )
    .unwrap();
    fs::write(seeds.join("b.mn"), "module m\n// fuzz-plant\n").unwrap();
    fs::write(seeds.join("c.mn"), "module m\npub fn f() -> u8 = 7\n").unwrap();
    let replay = |b: &str| {
        fs::write(seeds.join("b.found"), b).unwrap();
        let run = mn(&["fuzz", "--replay", seeds.to_str().unwrap()], Some("edit"));
        (run.status.code(), text(&run.stdout))
    };
    let (code, printed) = replay("mn fuzz found\nedit seed: -1\n");
    assert_eq!(code, Some(5), "{}", printed);
    assert!(
        printed.contains("a.mn: panic in planted: a planted panic on edit seed 4242424242"),
        "{}",
        printed
    );
    assert!(
        printed.contains("bad record: ")
            && printed.contains("b.found: the edit seed `-1` is not a decimal number from 0 to 18446744073709551615; write the seed as mn fuzz wrote it, or `unknown` where it is lost"),
        "{}",
        printed
    );
    assert!(
        printed.contains("3 file(s), 1 with a failure, 1 with a bad record"),
        "{}",
        printed
    );
    // A bad record alone, and a failure alone.
    fs::remove_file(seeds.join("a.found")).unwrap();
    let (code, printed) = replay("mn fuzz found\nphase: planted\n");
    assert_eq!(code, Some(4), "{}", printed);
    assert!(
        printed.contains("b.found: no `edit seed:` field"),
        "{}",
        printed
    );
    let (code, printed) = replay("mn fuzz found\nedit seed: 4242424242\n");
    assert_eq!(code, Some(1), "{}", printed);
    assert!(
        printed.contains("3 file(s), 1 with a failure, 0 with a bad record"),
        "{}",
        printed
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Jobs share the runs and the seeds, a campaign without runs goes on for its seconds, and a
/// corpus without seeds is a usage error.
#[test]
fn test_fuzz_shares_runs_among_jobs_and_keeps_to_its_seconds() {
    let dir = scratch("jobs");
    let seeds = dir.join("seeds");
    fs::write(seeds.join("a.mn"), "module a\npub fn f() -> u8 = 7\n").unwrap();
    fs::write(seeds.join("b.mn"), "module b\npub fn g() -> u8 = 8\n").unwrap();
    let out = dir.join("out");
    let fuzz = |extra: &[&str]| {
        let mut args = vec![
            "fuzz",
            "--corpus",
            seeds.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        let run = mn(&args, None);
        let summary = fs::read_to_string(out.join("summary.txt")).unwrap_or_default();
        (run, summary)
    };
    let (run, summary) = fuzz(&["--jobs", "2", "--runs", "2"]);
    assert_eq!(
        run.status.code(),
        Some(0),
        "{}{}",
        text(&run.stdout),
        text(&run.stderr)
    );
    assert!(summary.contains("runs: 2\n"), "{}", summary);
    assert!(summary.contains("worker starts: 2\n"), "{}", summary);
    assert!(summary.contains("  lex: 2,"), "{}", summary);
    // A campaign of one second may find a failure, but it runs, and it ends.
    let (run, summary) = fuzz(&["--seconds", "1"]);
    assert!(
        matches!(run.status.code(), Some(0 | 1)),
        "{}{}",
        text(&run.stdout),
        text(&run.stderr)
    );
    let runs: u64 = summary
        .lines()
        .find_map(|l| l.strip_prefix("runs: "))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    assert!(runs >= 2, "{}", summary);
    let run = mn(
        &["fuzz", "--corpus", dir.join("missing").to_str().unwrap()],
        None,
    );
    assert_eq!(run.status.code(), Some(2));
    assert!(
        text(&run.stderr).contains("no seed files"),
        "{}",
        text(&run.stderr)
    );
    let run = mn(&["fuzz", "--bogus"], None);
    assert_eq!(run.status.code(), Some(2));
    assert!(
        text(&run.stderr).contains("Usage: mn fuzz"),
        "{}",
        text(&run.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The minimized failures whose fixes have not landed yet, which still fail on the edit seed
/// their records name (`test_fuzz_pending_regressions_still_fail`). None is now: the last,
/// `incremental-layout-byte-range-past-usize.mn`, the incremental failure of the nightly run of
/// 2026-11-15, joined the main test with its fix, pull request 187. A fix removes its file's name
/// here, and the file joins the main test, as root cause 7's did with pull request 186.
const PENDING: [&str; 0] = [];

fn regressions() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fuzz")
}

/// Replays `files` through every phase and returns what `mn fuzz --replay` printed, failing
/// the test on any failure.
fn replay(files: &[PathBuf]) {
    if files.is_empty() {
        return;
    }
    let mut args = vec!["fuzz".to_string(), "--replay".to_string()];
    args.extend(files.iter().map(|f| f.to_string_lossy().into_owned()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let run = mn(&args, None);
    assert_eq!(
        run.status.code(),
        Some(0),
        "a minimized crash fails again:\n{}{}",
        text(&run.stdout),
        text(&run.stderr)
    );
}

/// Every minimized crash in `tests/fuzz` whose fix has landed replays without a failure, each
/// on the edits its record gives. Which regressions need a record, and which file is a
/// regression's record, the replay's own functions decide, in the unit test
/// `test_every_regression_of_a_phase_that_edits_carries_its_record` of `src/fuzz.rs`.
#[test]
fn test_fuzz_regressions_replay_without_a_failure() {
    let mut files = Vec::new();
    for entry in fs::read_dir(regressions()).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".mn") && !PENDING.contains(&name.as_str()) {
            files.push(entry.path());
        }
    }
    for name in PENDING {
        assert!(
            regressions().join(name).is_file(),
            "{} is pending but missing",
            name
        );
    }
    replay(&files);
}

/// Each pending regression still fails, on the edit seed its record names: its record
/// reproduces it, and the fix that lands makes this test fail until the file leaves `PENDING`.
#[test]
fn test_fuzz_pending_regressions_still_fail() {
    for name in PENDING {
        let file = regressions().join(name);
        let run = mn(&["fuzz", "--replay", file.to_str().unwrap()], None);
        let printed = text(&run.stdout);
        assert_eq!(
            run.status.code(),
            Some(1),
            "{} replays without a failure: its fix has landed, so remove it from PENDING\n{}{}",
            name,
            printed,
            text(&run.stderr)
        );
        assert!(
            printed.contains(": replayed on its recorded edit seed ")
                && printed.contains(" in incremental: "),
            "{}",
            printed
        );
    }
}

/// Found by `mn fuzz`: an array type of 2^32 elements or more checked, but did not lower ("an
/// array of 4294967296 elements"). An array holds at most 4,294,967,295 bytes, its length times
/// its element's size, and a larger array type or literal is `E0223`: as a parameter, as a
/// local and as a repeat literal, with no crash in any phase.
#[test]
fn test_fuzz_array_past_four_gigabytes_is_e0223() {
    let dir = scratch("e0223");
    let local = dir.join("seeds").join("local.mn");
    fs::write(
        &local,
        "module m\npub fn f() -> u8 {\n  val a: [4294967296]u8 = [0; 4294967296]\n  a[0]\n}\n",
    )
    .unwrap();
    let wide = dir.join("seeds").join("wide.mn");
    fs::write(&wide, "module m\npub fn f(a: [1073741824]i64) -> i64 = 0\n").unwrap();
    let files = [
        regressions().join("lower-array-of-2-to-the-32-elements.mn"),
        local,
        wide,
    ];
    replay(&files);
    for file in &files {
        let run = mn(&["check", file.to_str().unwrap()], None);
        let printed = text(&run.stdout) + &text(&run.stderr);
        assert_eq!(
            run.status.code(),
            Some(1),
            "{}: {}",
            file.display(),
            printed
        );
        assert!(
            printed.contains("E0223"),
            "{}: no E0223 in {}",
            file.display(),
            printed
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Found by `mn fuzz`: an element read whose index never gives a value, `a[unreachable("")]`,
/// checked but did not lower ("an index that is not an integer"); M1's lowering had it too. The
/// read ends where its index does, as a stored element's path already did.
#[test]
fn test_fuzz_unreachable_as_an_index_lowers() {
    let file = regressions().join("lower-unreachable-as-an-index.mn");
    replay(std::slice::from_ref(&file));
}

/// Found by `mn fuzz`: an assignment to what is no place (`E0114`) went on to read its value,
/// and noted the idiom `&` there while recovering from the error, which the body parser
/// asserts it never does. The error now ends the statement, as every error does.
#[test]
fn test_fuzz_reference_after_an_assignment_to_no_place_is_e0114() {
    let file = regressions().join("parse-reference-after-an-assignment-to-no-place.mn");
    replay(std::slice::from_ref(&file));
    let run = mn(&["check", file.to_str().unwrap()], None);
    let printed = text(&run.stdout) + &text(&run.stderr);
    assert_eq!(run.status.code(), Some(1), "{}", printed);
    assert!(printed.contains("E0114"), "no E0114 in {}", printed);
    assert!(!printed.contains("E0119"), "an idiom read in {}", printed);
}

/// Found by `mn fuzz`: a malformed return type passed a whole check and swallowed what followed,
/// here a lone `fn`, which `mn check --watch`'s session read as an item of its own (`E0101`). A
/// return type whose `]` or `>` closes nothing, or whose `[` or `<` is never closed, is `E0116`,
/// and the session and a whole check agree.
#[test]
fn test_fuzz_malformed_return_type_is_e0116() {
    let dir = scratch("e0116");
    let unclosed = dir.join("seeds").join("unclosed.mn");
    fs::write(&unclosed, "module m\npub fn f() -> [4 = 0\n").unwrap();
    let files = [
        regressions().join("incremental-malformed-return-type.mn"),
        unclosed,
    ];
    replay(&files);
    for file in &files {
        let run = mn(&["check", file.to_str().unwrap()], None);
        let printed = text(&run.stdout) + &text(&run.stderr);
        assert_eq!(
            run.status.code(),
            Some(1),
            "{}: {}",
            file.display(),
            printed
        );
        assert!(
            printed.contains("E0116"),
            "{}: no E0116 in {}",
            file.display(),
            printed
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Found by `mn fuzz`: a body with another parse error, an unclosed `[` or a trailing `+`, was
/// not held to the nesting limit, so `mn dump --tree` rendered its over-deep tree: quadratic, and
/// a stack overflow at a few hundred thousand links even on the stack `mn` runs on. A too-high
/// tree is `E0127` whether or not its body has another error, and no phase crashes or hangs.
/// Each shape is written out here rather than as a file.
#[test]
fn test_fuzz_unfinished_deep_chain_is_e0127_and_no_crash() {
    // Twenty times the limit: past it in both shapes, and quick through every phase in a debug
    // build, as the 300,000 links CI met are not.
    const DEPTH: usize = 5_000;
    let dir = scratch("unfinished");
    let index = dir.join("seeds").join("index.mn");
    let chain = format!(
        "module m\npub fn deep(x: i32) -> i32 = x{}[0\n",
        "[0]".repeat(DEPTH)
    );
    fs::write(&index, chain).unwrap();
    let plus = dir.join("seeds").join("plus.mn");
    let chain = format!(
        "module m\npub fn deep(x: i32) -> i32 = x{} +\n",
        " + 1".repeat(DEPTH)
    );
    fs::write(&plus, chain).unwrap();
    let files = [index, plus];
    replay(&files);
    for file in &files {
        let run = mn(&["check", file.to_str().unwrap()], None);
        let printed = text(&run.stdout) + &text(&run.stderr);
        assert_eq!(
            run.status.code(),
            Some(1),
            "{}: {}",
            file.display(),
            printed
        );
        assert!(printed.contains("E0127"), "{}: no E0127", file.display());
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Found by `mn fuzz`: deep nesting overflowed the stack, in the body parser, the tree dump,
/// the checker or the incremental session, whichever recursed deepest first. Past the nesting
/// limit of 256 levels each way of nesting the fuzzer makes is `E0127`, and no phase crashes.
/// Each is written out here rather than as a file, at a depth far past the limit. Issue 220
/// added the chains the parser reads in a loop, of each kind of operator and of fields, and
/// an `ensures` of one, whose tree the parser handed out past the limit and the incremental
/// session rendered in time growing with the square of its links.
#[test]
fn test_fuzz_deep_nesting_is_e0127_and_no_crash() {
    // Four times the limit: past it in every shape, and quick through every phase in a debug
    // build, which a per-test time limit asks for.
    const DEPTH: usize = 1_000;
    let expression = |nest: String| format!("module m\npub fn deep(x: i32) -> i32 = {}\n", nest);
    let modules = [
        expression(format!("{}x{}", "(".repeat(DEPTH), ")".repeat(DEPTH))),
        expression(format!("{}x", "- ".repeat(DEPTH))),
        expression(format!("{}x", "not ".repeat(DEPTH))),
        expression(format!("{}{{ x }}", "if x == 0 { 0 } else ".repeat(DEPTH))),
        expression(format!(
            "{}x{}",
            "if true { ".repeat(DEPTH),
            " } else { 0 }".repeat(DEPTH)
        )),
        expression(format!("x{}", " + 1".repeat(DEPTH))),
        expression(format!("x{}", " * 1".repeat(DEPTH))),
        expression(format!("x > 0{}", " and x > 0".repeat(DEPTH))),
        expression(format!("x > 0{}", " && x > 0".repeat(DEPTH))),
        expression(format!("x{}", ".a".repeat(DEPTH))),
        expression(format!("{}x", "x > 0 ? x : ".repeat(DEPTH))),
        expression(format!("{}x", "&".repeat(DEPTH))),
        format!(
            "module m
pub fn deep(n: u32) -> u32
  ensures result < n{}
= n
",
            " + 1".repeat(DEPTH)
        ),
        expression(format!("{}x{}", "[".repeat(DEPTH), "]".repeat(DEPTH))),
        expression(format!("{}x{}", "f(".repeat(DEPTH), ")".repeat(DEPTH))),
        expression(format!("x{}", "[0]".repeat(DEPTH))),
        format!(
            "module m\npub fn deep(x: i32) -> i32 {{\n  {}{}\n  x\n}}\n",
            "while x > 0 { ".repeat(DEPTH),
            " }".repeat(DEPTH)
        ),
        // No module line and no closing parenthesis: a release build overflowed in
        // `mn check --watch`'s session before the body parser, on `fn p=((((`.
        format!("fn p() -> i32 = {}", "(".repeat(DEPTH)),
    ];
    let dir = scratch("deep");
    let mut files = Vec::new();
    for (k, module) in modules.iter().enumerate() {
        let file = dir.join("seeds").join(format!("deep-{}.mn", k));
        fs::write(&file, module).unwrap();
        files.push(file);
    }
    replay(&files);
    for file in &files {
        let run = mn(&["check", file.to_str().unwrap()], None);
        let printed = text(&run.stdout) + &text(&run.stderr);
        // `mn check` exits with 1 when a module has a diagnostic.
        assert_eq!(
            run.status.code(),
            Some(1),
            "{}: {}",
            file.display(),
            printed
        );
        assert!(
            printed.contains("E0127"),
            "{}: no E0127 in {}",
            file.display(),
            printed
        );
    }
    let _ = fs::remove_dir_all(&dir);
}
