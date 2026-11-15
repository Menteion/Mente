//! The stack budget of item 25 of `core-calculus.md` on the reference interpreter and on MIR.
//!
//! Each program under `tests/stack/` states, in `// call: <call> -> <output>` lines, what a call
//! of it prints, a value or the trap that ends it, each figure derived by hand from item 25's
//! equations; this runner runs every such call with `mn run --call`, on the reference interpreter
//! and with `--mir`, and holds each to the line and the two to each other. Native code joins when
//! it counts frames and reserves its stack, and these programs move into the conformance suite
//! then (`ROADMAP.md`, the stack budget's line): until then a recursion that fits the budget may
//! run past the system's stack natively, which no conformance program may do.

// A failed setup step here is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn mn() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mn"))
}

/// What `mn run` prints of a call, its value or its trap, without the file's path before a trap.
fn printed(file: &Path, call: &str, mir: bool) -> String {
    let mut cmd = Command::new(mn());
    cmd.arg("run").arg(file).arg("--call").arg(call);
    if mir {
        cmd.arg("--mir");
    }
    let out = cmd.output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let prefix = format!("{}: ", file.display());
    text.trim().trim_start_matches(&prefix).to_string()
}

#[test]
fn test_both_interpreters_count_frames_against_the_budget() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/stack");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mn"))
        .collect();
    files.sort();
    let mut failures = Vec::new();
    let mut calls = 0;
    for file in &files {
        let src = fs::read_to_string(file).unwrap();
        for line in src.lines().filter_map(|l| l.strip_prefix("// call: ")) {
            let (call, expected) = line.split_once(" -> ").unwrap();
            calls += 1;
            let reference = printed(file, call, false);
            let mir = printed(file, call, true);
            let name = file.file_name().unwrap().to_string_lossy();
            if !reference.starts_with(expected) {
                failures.push(format!(
                    "{} {}: the reference interpreter prints {}, stated {}",
                    name, call, reference, expected
                ));
            }
            if mir != reference {
                failures.push(format!(
                    "{} {}: MIR prints {}, the reference interpreter {}",
                    name, call, mir, reference
                ));
            }
        }
    }
    assert!(calls > 0, "no program under tests/stack states a call");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
