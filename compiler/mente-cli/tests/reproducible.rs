//! Compiling twice gives identical output, on every platform (`docs/design-brief.md`, "The
//! quality system"; foundation gate condition 8).
//!
//! Each output below is produced twice and must match byte for byte, and then must match the
//! golden files in `tests/golden/`, which the CI matrix checks on x86-64 Linux, x86-64 Windows
//! and AArch64 Linux alike:
//!
//! - each example's projection (`mn view --spec`);
//! - each conformance program's diagnostics (`mn check --json`);
//! - the MIR each conformance program, each performance kernel and the debugger demo lowers to
//!   (`mn dump --mir`);
//! - the object `mn build --object` writes for each kernel and the demo, for every target: each
//!   machine compiles for all three, so an object that depends on the machine compiling it
//!   differs from the golden digest on some runner. The objects name a fixed compilation
//!   directory, since the debug information otherwise records the directory they were built in.
//!
//! On Linux an executable `mn build` links twice must match as well. On Windows it cannot:
//! MSVC's `link.exe` gives each program database a new identifier, which the executable records,
//! so two links differ even under `/Brepro`, though their objects are identical.
//!
//! When an output changes on purpose, run with `MENTE_BLESS=1` to rewrite the golden files, and
//! review their diff.

// A failed setup step here is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

/// Runs `mn` in `dir` and returns its standard output.
fn mn(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_mn"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run mn");
    String::from_utf8(out.stdout).expect("mn prints UTF-8")
}

/// Runs `mn` in `dir` and returns its exit status and its standard output.
fn mn_status(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mn"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run mn");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).expect("mn prints UTF-8"),
    )
}

/// The 64-bit FNV-1a digest of `bytes`, in hexadecimal: enough to tell two objects apart.
fn digest(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", h)
}

/// The programs that build natively, each with the call its `main` makes: the performance
/// kernels, relative to the repository's root, and the debugger demo.
fn native_programs() -> Vec<(String, String)> {
    let root = repo_root();
    let mut programs: Vec<(String, String)> = sorted_files(&root.join("perf/kernels"), "mn")
        .into_iter()
        .map(|file| {
            let kernel = file.file_stem().unwrap().to_string_lossy().into_owned();
            (
                format!("perf/kernels/{}.mn", kernel),
                format!("{}(3)", kernel),
            )
        })
        .collect();
    programs.push((
        "compiler/mente-cli/tests/debuggers/demo.mn".to_string(),
        "sum_squares(3, 4)".to_string(),
    ));
    programs
}

/// A new empty directory for this test's files.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The same output twice, or a failure naming what differed.
fn twice(what: &str, run: impl Fn() -> String) -> String {
    let first = run();
    let second = run();
    assert_eq!(first, second, "{} differs between two runs", what);
    first
}

/// Compares `actual` with a golden file, or rewrites it under `MENTE_BLESS=1`.
fn check_golden(name: &str, actual: &str) {
    let path = golden(name);
    if std::env::var_os("MENTE_BLESS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_default();
    if expected != actual {
        let line = expected
            .lines()
            .zip(actual.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
        panic!(
            "{} differs from tests/golden/{} at line {}:\n  golden: {:?}\n  now:    {:?}\nRun with MENTE_BLESS=1 if the change is intended.",
            name,
            name,
            line + 1,
            expected.lines().nth(line).unwrap_or(""),
            actual.lines().nth(line).unwrap_or("")
        );
    }
}

fn sorted_files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == extension))
        .collect();
    files.sort();
    files
}

#[test]
fn test_projections_are_reproducible() {
    let root = repo_root();
    let mut all = String::new();
    for file in sorted_files(&root.join("examples"), "mn") {
        let rel = format!("examples/{}", file.file_name().unwrap().to_string_lossy());
        let out = twice(&rel, || {
            // The last two lines report the time the projection took; everything else is output.
            let full = mn(&root, &["view", "--spec", &rel]);
            let lines: Vec<&str> = full.lines().collect();
            lines[..lines.len().saturating_sub(2)].join("\n")
        });
        assert!(out.contains("// spec_hash: "), "{}: {}", rel, out);
        all.push_str(&format!("== {} ==\n{}\n", rel, out));
    }
    check_golden("projections.txt", &all);
}

#[test]
fn test_diagnostics_are_reproducible() {
    let root = repo_root().join("conformance");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut all = String::new();
    for dir in dirs {
        for file in sorted_files(&dir, "mn") {
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            let rel = format!("{}/{}", dir.file_name().unwrap().to_string_lossy(), name);
            // Run from the program's own directory, so the file named in the JSON has no
            // platform-specific separators.
            let out = twice(&rel, || mn(&dir, &["check", "--json", &name]));
            all.push_str(&format!("== {} ==\n{}", rel, out));
        }
    }
    check_golden("diagnostics.txt", &all);
}

#[test]
fn test_mir_is_reproducible() {
    let root = repo_root();
    let mut programs: Vec<(PathBuf, String)> = Vec::new();
    let mut dirs: Vec<PathBuf> = fs::read_dir(root.join("conformance"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        for file in sorted_files(&dir, "mn") {
            programs.push((
                dir.clone(),
                file.file_name().unwrap().to_string_lossy().into_owned(),
            ));
        }
    }
    for (path, _) in native_programs() {
        let file = root.join(&path);
        let dir = file.parent().unwrap().to_path_buf();
        programs.push((
            dir,
            file.file_name().unwrap().to_string_lossy().into_owned(),
        ));
    }
    let mut all = String::new();
    let mut lowered = 0;
    for (dir, name) in programs {
        let rel =
            format!("{}/{}", dir.strip_prefix(&root).unwrap().display(), name).replace('\\', "/");
        // A program that does not check, or does not lower yet, is recorded by its status alone.
        let (status, out) = twice(&rel, || {
            let (status, out) = mn_status(&dir, &["dump", "--mir", &name]);
            format!("{}\n{}", status, out)
        })
        .split_once('\n')
        .map(|(s, o)| (s.to_string(), o.to_string()))
        .unwrap();
        if status == "0" {
            lowered += 1;
            all.push_str(&format!("== {} ==\n{}", rel, out));
        } else {
            all.push_str(&format!("== {} == (exit {})\n", rel, status));
        }
    }
    assert!(lowered >= 10, "only {} programs lowered:\n{}", lowered, all);
    check_golden("mir.txt", &all);
}

#[test]
fn test_objects_are_reproducible_and_independent_of_the_machine() {
    let root = repo_root();
    let dirs = [scratch("objects-1"), scratch("objects-2")];
    let mut all = String::new();
    for (path, call) in native_programs() {
        for triple in [
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu",
            "x86_64-pc-windows-msvc",
        ] {
            // The same file name in both directories: CodeView records the object's name.
            let objects: Vec<Vec<u8>> = dirs
                .iter()
                .map(|dir| {
                    let out = dir.join("program.o");
                    let (status, printed) = mn_status(
                        &root,
                        &[
                            "build",
                            &path,
                            "--call",
                            &call,
                            "--object",
                            "--target",
                            triple,
                            "--compilation-dir",
                            "/mente",
                            "-o",
                            out.to_str().unwrap(),
                        ],
                    );
                    assert_eq!(status, 0, "{} for {}: {}", path, triple, printed);
                    fs::read(&out).unwrap()
                })
                .collect();
            assert!(
                objects[0] == objects[1],
                "{}: two objects for {} differ",
                path,
                triple
            );
            all.push_str(&format!(
                "{} {} {} bytes {}\n",
                path,
                triple,
                objects[0].len(),
                digest(&objects[0])
            ));
        }
    }
    check_golden("objects.txt", &all);
}

#[test]
#[cfg(target_os = "linux")]
fn test_executables_are_reproducible_on_linux() {
    let root = repo_root();
    let dirs = [scratch("executables-1"), scratch("executables-2")];
    for (path, call) in native_programs() {
        let built: Vec<Vec<u8>> = dirs
            .iter()
            .map(|dir| {
                let exe = dir.join("program");
                let (status, printed) = mn_status(
                    &root,
                    &["build", &path, "--call", &call, "-o", exe.to_str().unwrap()],
                );
                assert_eq!(status, 0, "{}: {}", path, printed);
                fs::read(&exe).unwrap()
            })
            .collect();
        assert!(built[0] == built[1], "{}: two executables differ", path);
    }
}
