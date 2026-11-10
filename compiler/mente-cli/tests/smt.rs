//! `mn smt`, which writes each obligation Tier 1 discharges as an SMT-LIB 2 query for the SMT
//! cross-check (`mente_sema::tier1::smt`), and the construct programs of `src/mir_run.rs`, which
//! the CI job "SMT cross-check" (`.github/workflows/smt.yml`) gives it along with the other
//! programs that check. The job, not these tests, runs the solver.

// These helpers are test code: a failed setup step is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The source of `src/mir_run.rs`, whose `CONSTRUCTS` reach each construct the lowering handles.
const MIR_RUN: &str = include_str!("../src/mir_run.rs");

fn mn(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mn"))
        .args(args)
        .output()
        .expect("run mn")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A fresh scratch directory for one test, removed by the caller.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mente-smt-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The text of the Rust string literal that starts after its opening quote, which `chars` has
/// just read; `chars` ends after its closing quote.
fn literal(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return out,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some('\n') => {
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                }
                Some('u') => {
                    let hex: String = chars
                        .by_ref()
                        .skip_while(|c| *c == '{')
                        .take_while(|c| *c != '}')
                        .collect();
                    let code = u32::from_str_radix(&hex, 16).expect("a \\u escape");
                    out.push(char::from_u32(code).expect("a char"));
                }
                Some('x') => {
                    let hex: String = chars.by_ref().take(2).collect();
                    let code = u32::from_str_radix(&hex, 16).expect("a \\x escape");
                    out.push(char::from_u32(code).expect("a char"));
                }
                Some(other) => out.push(other),
                None => break,
            },
            other => out.push(other),
        }
    }
    panic!("a string literal in CONSTRUCTS does not end");
}

/// The programs of `CONSTRUCTS` in `src/mir_run.rs`, each as the module its tests check: the
/// first string literal of each entry, after `module m`. Read from the source, since that file
/// belongs to the tests of the lowering.
fn construct_programs() -> Vec<String> {
    let start = MIR_RUN
        .find("const CONSTRUCTS")
        .expect("CONSTRUCTS in src/mir_run.rs");
    let rest = &MIR_RUN[start..];
    let open = rest.find("= &[").expect("the list of CONSTRUCTS") + "= &[".len();
    let close = rest.find("\n    ];").expect("the end of CONSTRUCTS");
    let list = &rest[open..close];
    assert!(
        !list.contains("r#\"") && !list.contains("r\""),
        "a raw string in CONSTRUCTS, which this reader does not read"
    );
    let mut programs = Vec::new();
    let mut chars = list.chars().peekable();
    // Within the list; an entry `(program, &[calls])` is one deeper, its calls two.
    let mut depth = 1;
    let mut first = false;
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '(' | '[' => {
                depth += 1;
                first = c == '(' && depth == 2;
            }
            ')' | ']' => depth -= 1,
            '"' => {
                let s = literal(&mut chars);
                if first && depth == 2 {
                    programs.push(format!("module m\n{}\n", s));
                }
                first = false;
            }
            _ => {}
        }
    }
    programs
}

/// Writes each construct program into `dir` as `construct-<k>.mn`.
fn write_constructs(dir: &Path) -> usize {
    let programs = construct_programs();
    fs::create_dir_all(dir).unwrap();
    for (k, p) in programs.iter().enumerate() {
        fs::write(dir.join(format!("construct-{:02}.mn", k)), p).unwrap();
    }
    programs.len()
}

/// Every entry of `CONSTRUCTS` gives its program, escapes and lines as the Rust compiler reads
/// them, and the calls are left out.
#[test]
fn test_the_construct_programs_are_read_from_mir_run() {
    let programs = construct_programs();
    assert!(programs.len() >= 20, "only {} programs", programs.len());
    assert!(programs[0].starts_with("module m\npub fn bind(x: u16) -> u8 {\n  val y: u8 = x\n"));
    assert!(programs
        .iter()
        .all(|p| p.contains("fn ") && !p.contains("bind(200)")));
    assert!(programs
        .iter()
        .any(|p| p.contains("if c { todo(\"not yet\") } else { 5 }")));
    assert!(programs
        .iter()
        .any(|p| p.contains("pub fn grid(n: u8) -> u16 {\n  var i: u8 = 0\n")));
    assert!(programs
        .iter()
        .all(|p| mente_syntax::parse_module(p).is_ok()));
}

/// `mn smt` writes a query a discharged obligation, named by function, line and kind and
/// headed by its source position, lists those no path reaches, and wants `--out`.
#[test]
fn test_mn_smt_writes_a_query_for_each_discharged_obligation() {
    let dir = scratch("constructs");
    let programs = dir.join("programs");
    let n = write_constructs(&programs);
    let out = dir.join("out");
    let run = mn(&[
        "smt",
        programs.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}{}", stdout, text(&run.stderr));
    assert!(
        stdout.contains(&format!("functions of {} files", n)),
        "{}",
        stdout
    );
    assert!(
        stdout.contains("unreached: ") && stdout.contains("range of `add_todo`"),
        "{}",
        stdout
    );
    let mut queries = Vec::new();
    let mut stack = vec![out.clone()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                queries.push(p);
            }
        }
    }
    assert!(queries.len() >= 50, "only {} queries", queries.len());
    for q in &queries {
        let body = fs::read_to_string(q).unwrap();
        assert!(body.starts_with("; source: "), "{}", body);
        assert!(body.contains("(set-logic QF_LIA)") && body.ends_with("(check-sat)\n(exit)\n"));
        assert!(q.extension().is_some_and(|e| e == "smt2"));
    }
    let names: Vec<String> = queries
        .iter()
        .map(|q| q.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.contains(&"half-4-divisor.smt2".to_string()),
        "{:?}",
        names
    );
    let index = queries
        .iter()
        .find(|q| q.ends_with("at-4-index.smt2"))
        .expect("the query of `xs[k]` in `at`");
    let body = fs::read_to_string(index).unwrap();
    assert!(body.contains("construct-"), "{}", body);
    assert!(
        body.contains("(assert (or (< |k| 0) (>= |k| 3)))"),
        "{}",
        body
    );
    let usage = mn(&["smt", programs.to_str().unwrap()]);
    assert_eq!(usage.status.code(), Some(2));
    fs::remove_dir_all(&dir).ok();
}

/// Writes the construct programs into the directory `MENTE_SMT_CONSTRUCTS` names, for the CI job
/// to give `mn smt`:
/// `MENTE_SMT_CONSTRUCTS=<dir> cargo test -p mente-cli --test smt -- --ignored write_construct_programs`.
#[test]
#[ignore = "writes files for the CI job \"SMT cross-check\""]
fn write_construct_programs() {
    let dir = std::env::var("MENTE_SMT_CONSTRUCTS").expect("MENTE_SMT_CONSTRUCTS");
    let n = write_constructs(Path::new(&dir));
    println!("wrote {} construct programs to {}", n, dir);
}
