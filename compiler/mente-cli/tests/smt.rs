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
    // The list closes on a line of its own, at the indentation of its `const`.
    let close = rest
        .find("\n];")
        .or_else(|| rest.find("\n    ];"))
        .expect("the end of CONSTRUCTS");
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
/// headed by its source position, lists those no path reaches, counts both, and wants `--out`,
/// before its inputs or after them.
#[test]
fn test_mn_smt_writes_a_query_for_each_discharged_obligation() {
    let dir = scratch("constructs");
    let programs = dir.join("programs");
    let n = write_constructs(&programs);
    let out = dir.join("out");
    let run = mn(&[
        "smt",
        "--out",
        out.to_str().unwrap(),
        programs.to_str().unwrap(),
    ]);
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}{}", stdout, text(&run.stderr));
    // What mente-sema exports of the same programs, which `mn smt` counts.
    let (mut functions, mut written, mut unreached) = (0, 0, 0);
    for p in construct_programs() {
        for (_, _, queries) in mente_sema::tier1_queries(&p) {
            functions += 1;
            for q in queries {
                match q.text {
                    Some(_) => written += 1,
                    None => unreached += 1,
                }
            }
        }
    }
    assert!(unreached > 0);
    assert!(
        stdout.contains(&format!(
            "Wrote {} queries for {} functions of {} files to `",
            written, functions, n
        )),
        "{}",
        stdout
    );
    assert!(
        stdout.contains(&format!(
            "; {} discharged obligations are reached by no path.",
            unreached
        )),
        "{}",
        stdout
    );
    let listed = stdout
        .lines()
        .filter(|l| l.starts_with("unreached: "))
        .count();
    assert_eq!(listed, unreached, "{}", stdout);
    assert!(stdout.contains("range of `add_todo`"), "{}", stdout);
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
    assert!(written >= 50, "only {} queries", written);
    assert_eq!(queries.len(), written);
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

/// `mn smt --independent` writes a query for each check Tier 1 erases, headed by its source, its
/// function, its obligation and the trap a model must raise; `mn smt --confirm` reads a solver's
/// answers: `unsat` passes, a timeout decides nothing, and a model that does not satisfy its
/// query, or a known bug no model confirms, fails the job. Each wants its arguments.
#[test]
fn test_mn_smt_independent_writes_queries_and_confirm_reads_their_answers() {
    let dir = scratch("independent");
    let file = dir.join("m.mn");
    fs::write(
        &file,
        "module m\npub fn safe(d: u8[1..=9]) -> u8 = 7 % d\npub fn kept(d: u8) -> u8 = 7 % d\n",
    )
    .unwrap();
    let out = dir.join("out");
    let run = mn(&[
        "smt",
        "--independent",
        file.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}{}", stdout, text(&run.stderr));
    // `safe`'s divisor Tier 1 proves, and so erases; `kept`'s it does not, and has no query.
    // Both results, `7 % d`, lie in `u8`, so their narrowings are erased too.
    assert!(
        stdout.contains("Wrote 3 independent queries for 2 functions of 1 files to `"),
        "{}",
        stdout
    );
    assert!(
        stdout.ends_with("0 files do not check, 0 obligations have no site.\n"),
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
    queries.sort();
    let names: Vec<String> = queries
        .iter()
        .map(|q| q.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "kept-3-range.smt2",
            "safe-2-divisor.smt2",
            "safe-2-range.smt2"
        ],
        "{:?}",
        queries
    );
    let query = &queries[1];
    let body = fs::read_to_string(query).unwrap();
    assert!(
        body.starts_with(&format!(
            "; source: {}:2:39\n; function: safe\n; obligation: divisor at 2:39\n; trap: division-by-zero at 2:35\n(declare-const p.d Int)\n",
            file.display()
        )),
        "{}",
        body
    );
    assert!(
        body.ends_with("(check-sat)\n(get-value (p.d))\n"),
        "{}",
        body
    );
    let answers = dir.join("answers.txt");
    let q = query.display();
    let confirm = |lines: String, expect: Option<&Path>| {
        fs::write(&answers, lines).unwrap();
        let mut args = vec!["smt", "--confirm", answers.to_str().unwrap()];
        if let Some(e) = expect {
            args.extend(["--expect", e.to_str().unwrap()]);
        }
        mn(&args)
    };
    let run = confirm(
        format!("unsat (error \"no model\")\t{}\ntimeout\t{}\n", q, q),
        None,
    );
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}{}", stdout, text(&run.stderr));
    assert!(
        stdout.contains(&format!("undecided: {}: timeout", q)),
        "{}",
        stdout
    );
    assert!(
        stdout.ends_with("1 unsat, 1 undecided, 0 confirmed soundness bugs of Tier 1, 0 known, 0 not confirmed, 0 known bugs missing.\n"),
        "{}",
        stdout
    );
    // d = 0 lies outside `safe`'s refinement: no input of the query, so the answer is wrong.
    let run = confirm(format!("sat ((p.d 0))\t{}\n", q), None);
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    assert!(
        stdout.contains("unconfirmed: ")
            && stdout.contains("the model safe(0) does not satisfy its query"),
        "{}",
        stdout
    );
    // A known bug that no answer confirms.
    let expect = dir.join("known.mn");
    fs::write(&expect, "// smt: kept division-by-zero at 3:30\n").unwrap();
    let run = confirm(format!("unsat\t{}\n", q), Some(&expect));
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    assert!(
        stdout.contains("missing: ")
            && stdout.contains("no query of `kept` was confirmed at its division-by-zero at 3:30"),
        "{}",
        stdout
    );
    for usage in [
        mn(&["smt", "--independent", file.to_str().unwrap()]),
        mn(&["smt", "--confirm"]),
        mn(&["smt", "--confirm", answers.to_str().unwrap(), "--expect"]),
    ] {
        assert_eq!(usage.status.code(), Some(2), "{}", text(&usage.stderr));
    }
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
