//! `mn fuzz` end to end: the worker process isolates a planted panic, stack overflow and hang,
//! and the fuzzer classifies, minimizes and writes each one; and every crash the fuzzer found,
//! minimized under `tests/fuzz`, replays through every phase without a failure once its fix
//! lands. A file whose fix has not landed is listed in `PENDING` and replayed by its own
//! ignored test, so that the main test stays green and each fix turns one test on.

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
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .filter(|p| p.file_name().is_some_and(|n| n != "summary.txt"))
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
        "module m
pub fn f(x: u8) -> u16 = x + 1
",
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
    assert!(
        summary.contains(
            "failures: 0
"
        ),
        "{}",
        summary
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The minimized crashes whose fixes have not landed yet, each replayed by its own ignored
/// test below. A fix removes its file's name here and its test, and the file joins the main
/// test.
const PENDING: [&str; 2] = [
    "incremental-module-without-a-name.mn",
    "incremental-position-in-a-message.mn",
];

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

/// Every minimized crash in `tests/fuzz` whose fix has landed replays without a failure.
#[test]
fn test_fuzz_regressions_replay_without_a_failure() {
    let mut files = Vec::new();
    for entry in fs::read_dir(regressions()).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(name.ends_with(".mn"), "{} is not a .mn file", name);
        if !PENDING.contains(&name.as_str()) {
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

/// Found by `mn fuzz`: a module line without a name, then `fn` on the next line. A whole check
/// says it expected a name and found `fn`; `mn check --watch`'s session, which reads `fn` as the
/// next item, says it found the end of the file, at another span.
#[test]
#[ignore = "pending a fix in mente-sema's incremental session"]
fn fuzz_incremental_module_without_a_name() {
    replay(&[regressions().join("incremental-module-without-a-name.mn")]);
}

/// Found by `mn fuzz`: an unclosed parameter list. The session's message names where the list
/// opened by its line in the item, 1:9, where a whole check names its line in the module, 3:9.
#[test]
#[ignore = "pending a fix in mente-sema's incremental session"]
fn fuzz_incremental_position_in_a_message() {
    replay(&[regressions().join("incremental-position-in-a-message.mn")]);
}

/// Found by `mn fuzz`: deep nesting overflowed the stack, in the body parser, the tree dump,
/// the checker or the incremental session, whichever recursed deepest first. Past the nesting
/// limit of 256 levels each way of nesting the fuzzer makes is `E0127`, and no phase crashes.
/// Each is written out here rather than as a file, at a depth far past the limit.
#[test]
fn test_fuzz_deep_nesting_is_e0127_and_no_crash() {
    // Twenty times the limit: enough to cross it in every shape, and quick in a debug build.
    const DEPTH: usize = 5_000;
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
