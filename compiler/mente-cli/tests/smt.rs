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
        stdout.contains("Of the checks Tier 1 erased that no input reaches, 0 are unreachable, 0 lie past the bounds and 0 past a construct the encoding lacks; 0 have no site, lying within such a construct.")
            && stdout.ends_with("0 files do not check, 0 obligations have no site where they should.\n"),
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
    let other = queries[2].display();
    // The queries asked, each of which must have one answer.
    let asked = dir.join("asked.txt");
    fs::write(&asked, format!("{}\n{}\n", q, other)).unwrap();
    let confirm = |lines: String, expect: Option<&Path>| {
        fs::write(&answers, lines).unwrap();
        let mut args = vec![
            "smt",
            "--confirm",
            answers.to_str().unwrap(),
            "--queries",
            asked.to_str().unwrap(),
        ];
        if let Some(e) = expect {
            args.extend(["--expect", e.to_str().unwrap()]);
        }
        mn(&args)
    };
    let run = confirm(
        format!("unsat (error \"no model\")\t{}\ntimeout\t{}\n", q, other),
        None,
    );
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}{}", stdout, text(&run.stderr));
    assert!(
        stdout.contains(&format!("undecided: {}: timeout", other))
            && stdout.contains("fragment: 1 queries linear, 0 of them undecided")
            && stdout.contains("fragment: 1 queries nonlinear, 1 of them undecided"),
        "{}",
        stdout
    );
    assert!(
        stdout.ends_with("1 unsat, 1 undecided, 0 confirmed soundness bugs of Tier 1, 0 known, 0 not confirmed, 0 known bugs missing, 0 errors.\n"),
        "{}",
        stdout
    );
    // A query of the list with no answer fails the job.
    let run = confirm(format!("unsat\t{}\n", q), None);
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    assert!(
        stdout.contains(&format!("error: {}: asked and not answered", other)),
        "{}",
        stdout
    );
    // d = 0 lies outside `safe`'s refinement: no input of the query, so the answer is wrong.
    let run = confirm(format!("sat ((p.d 0))\t{}\nunsat\t{}\n", q, other), None);
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
    fs::write(&expect, "// smt: kept divisor at 3:32\n").unwrap();
    let run = confirm(format!("unsat\t{}\nunsat\t{}\n", q, other), Some(&expect));
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    assert!(
        stdout.contains("missing: ")
            && stdout
                .contains("no query of `kept` was confirmed at its obligation `divisor at 3:32`"),
        "{}",
        stdout
    );
    // A directory names the known bugs of each of its files; a path that is none cannot be read.
    let known = dir.join("known");
    fs::create_dir_all(&known).unwrap();
    fs::write(known.join("a.mn"), "// smt: kept divisor at 3:32\n").unwrap();
    fs::write(known.join("b.mn"), "// smt: safe divisor at 2:39\n").unwrap();
    let run = confirm(format!("unsat\t{}\nunsat\t{}\n", q, other), Some(&known));
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    assert!(
        stdout.contains("no query of `kept` was confirmed at its obligation `divisor at 3:32`")
            && stdout
                .contains("no query of `safe` was confirmed at its obligation `divisor at 2:39`"),
        "{}",
        stdout
    );
    for usage in [
        mn(&["smt", "--independent", file.to_str().unwrap()]),
        mn(&["smt", "--confirm"]),
        mn(&["smt", "--confirm", answers.to_str().unwrap(), "--expect"]),
        confirm(
            format!("unsat\t{}\nunsat\t{}\n", q, other),
            Some(&dir.join("no-such-file.mn")),
        ),
    ] {
        assert_eq!(usage.status.code(), Some(2), "{}", text(&usage.stderr));
    }
    fs::remove_dir_all(&dir).ok();
}

/// `mn smt --independent --keys` writes each check Tier 1 erased with what the encoding made of
/// it, and `mn smt --compare` holds a run to its base's: a check verified less, unless a waiver
/// names it, a waiver nothing needs, a program both hold unchanged left unread or no longer
/// checked, each fails it. `mn smt --encodes` tells whether a tree's workspace holds the encoding.
#[test]
fn test_mn_smt_compare_holds_a_run_to_its_base() {
    let dir = scratch("compare");
    let file = dir.join("m.mn");
    fs::write(&file, "module m\npub fn safe(d: u8[1..=9]) -> u8 = 7 % d\n").unwrap();
    let export = |name: &str| {
        let keys = dir.join(format!("{}-keys.txt", name));
        let answers = dir.join(format!("{}-answers.txt", name));
        let run = mn(&[
            "smt",
            "--independent",
            file.to_str().unwrap(),
            "--out",
            dir.join(name).to_str().unwrap(),
            "--keys",
            keys.to_str().unwrap(),
        ]);
        assert!(run.status.success(), "{}", text(&run.stderr));
        let written = fs::read_to_string(&keys).unwrap();
        let lines: String = written
            .lines()
            .filter_map(|l| l.rsplit_once("\tquery "))
            .map(|(_, path)| format!("unsat\t{}\n", path))
            .collect();
        fs::write(&answers, lines).unwrap();
        (keys, answers, written)
    };
    let (base_keys, base_answers, base_text) = export("base");
    let (head_keys, head_answers, head_text) = export("head");
    let shown = file.display().to_string();
    let file_line = head_text.lines().next().unwrap().to_string();
    assert!(
        file_line.starts_with(&format!("file\t{}\t", shown)) && file_line.ends_with("\tchecked"),
        "{}",
        head_text
    );
    assert!(
        head_text.contains(&format!("check\t{}\t", shown))
            && head_text.contains("\tsafe\tdivisor at 2:39\tquery "),
        "{}",
        head_text
    );
    // A base tree whose workspace does not hold the encoding, and the waivers of no commit.
    let tree = dir.join("base-tree");
    let package = |at: &str, name: &str| {
        fs::create_dir_all(tree.join(at)).unwrap();
        fs::write(
            tree.join(at).join("Cargo.toml"),
            format!("[package]\nname = \"{}\"\n", name),
        )
        .unwrap();
    };
    let workspace = |members: &[&str]| {
        let listed: String = members
            .iter()
            .map(|m| format!("    \"{}\",\n", m))
            .collect();
        fs::write(
            tree.join("Cargo.toml"),
            format!("[workspace]\nmembers = [\n{}]\n", listed),
        )
        .unwrap();
    };
    package("compiler/mente-syntax", "mente-syntax");
    package("tools/smt", "mente-smt");
    workspace(&["compiler/mente-syntax"]);
    let waivers = dir.join("waivers.txt");
    fs::write(&waivers, "").unwrap();
    let compare = || {
        mn(&[
            "smt",
            "--compare",
            "--base-tree",
            tree.to_str().unwrap(),
            "--base",
            base_keys.to_str().unwrap(),
            base_answers.to_str().unwrap(),
            "--head",
            head_keys.to_str().unwrap(),
            head_answers.to_str().unwrap(),
            "--waivers",
            waivers.to_str().unwrap(),
        ])
    };
    let run = compare();
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}{}", stdout, text(&run.stderr));
    assert!(
        stdout.ends_with("0 checks are verified less than on the base and 0 more waived, 0 programs were not read and 0 no longer check, and 0 checks moved otherwise; 0 waivers are unused or malformed; 0 programs were added, changed, renamed or deleted. Undecided: 0 on the base and 0 here over the checks both query, 0 of this run's 2 queries.\n"),
        "{}",
        stdout
    );
    // The divisor's query cut off by the bound here: verified less than on the base, unless a
    // commit's trailer names it.
    let cut: String = head_text
        .lines()
        .map(|l| match l.split_once("\tsafe\tdivisor at 2:39\tquery ") {
            Some((front, _)) => format!("{}\tsafe\tdivisor at 2:39\tbeyond-bound\n", front),
            None => format!("{}\n", l),
        })
        .collect();
    fs::write(&head_keys, &cut).unwrap();
    let run = compare();
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    let dropped = format!(
        "{}: `safe`'s obligation `divisor at 2:39` is a query on the base and cut off by the bound here",
        shown
    );
    assert!(
        stdout.contains(&format!("dropped: {}\n", dropped)),
        "{}",
        stdout
    );
    fs::write(
        &waivers,
        format!("\n{} safe divisor at 2:39: a test of the waiver\n\n", shown),
    )
    .unwrap();
    let run = compare();
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}", stdout);
    assert!(
        stdout.contains(&format!("waived: {}; a test of the waiver\n", dropped)),
        "{}",
        stdout
    );
    // The waiver, with nothing verified less, is unused; and a trailer that is no waiver fails.
    fs::write(&head_keys, &head_text).unwrap();
    let run = compare();
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    assert!(
        stdout.contains(&format!(
            "unused: Smt-Drop: {} safe divisor at 2:39: a test of the waiver: no check",
            shown
        )),
        "{}",
        stdout
    );
    fs::write(&waivers, "m.mn safe\n").unwrap();
    let run = compare();
    assert_eq!(run.status.code(), Some(1));
    assert!(text(&run.stdout).contains("unused: `Smt-Drop: m.mn safe` is no waiver"));
    fs::write(&waivers, "").unwrap();
    // The program, unchanged in this tree, not read here; read, but no longer checked.
    fs::write(&head_keys, "").unwrap();
    let run = compare();
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    assert!(
        stdout.contains(&format!(
            "lost: {}: both trees hold it unchanged, but this export did not read it",
            shown
        )),
        "{}",
        stdout
    );
    fs::write(
        &head_keys,
        format!("{}unchecked\n", file_line.strip_suffix("checked").unwrap()),
    )
    .unwrap();
    let run = compare();
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    assert!(
        stdout.contains(&format!(
            "unchecked: {}: the base's checker accepts its module and this run's does not",
            shown
        )),
        "{}",
        stdout
    );
    // Changed in this tree, it is listed and not held.
    fs::write(&head_keys, &head_text).unwrap();
    let hash = file_line.split('\t').nth(2).unwrap();
    fs::write(&base_keys, base_text.replace(hash, "0000000000000000")).unwrap();
    let run = compare();
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}", stdout);
    assert!(
        stdout.contains(&format!(
            "program: changed: {}: 2 checks: 2 with a query, 0 unreachable, 0 cut off by the bound, 0 by a construct the encoding lacks, 0 without a site on the base; 2 checks: 2 with a query",
            shown
        )),
        "{}",
        stdout
    );
    // A program of the base this tree no longer holds is deleted, not lost.
    let gone = dir.join("gone.mn");
    fs::write(
        &base_keys,
        format!(
            "{}file\t{}\t0123456789abcdef\tchecked\n",
            base_text,
            gone.display()
        ),
    )
    .unwrap();
    let run = compare();
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}", stdout);
    assert!(
        stdout.contains(&format!("program: deleted: {}: 0 checks", gone.display()))
            && !stdout.contains("lost: "),
        "{}",
        stdout
    );
    // With no export of the base, every program is new while the base's workspace lacks the
    // encoding; once it holds it, wherever it lies, a program both trees hold unchanged that the
    // base's export did not read fails the run.
    fs::write(&base_keys, "").unwrap();
    let run = compare();
    let stdout = text(&run.stdout);
    assert!(run.status.success(), "{}", stdout);
    assert!(
        stdout.starts_with("The base at `")
            && stdout.contains(
                "` does not hold mente-smt, so every program is new to the comparison.\n"
            )
            && stdout.contains(&format!(
                "program: added: {}: 2 checks: 2 with a query",
                shown
            )),
        "{}",
        stdout
    );
    let encodes = || {
        mn(&["smt", "--encodes", tree.to_str().unwrap()])
            .status
            .code()
    };
    assert_eq!(encodes(), Some(1));
    workspace(&["compiler/mente-syntax", "tools/smt"]);
    assert_eq!(encodes(), Some(0));
    let run = compare();
    let stdout = text(&run.stdout);
    assert_eq!(run.status.code(), Some(1), "{}", stdout);
    assert!(
        !stdout.contains("does not hold mente-smt")
            && stdout.contains(&format!(
                "lost: {}: both trees hold it unchanged, but the base's export did not read it",
                shown
            )),
        "{}",
        stdout
    );
    fs::remove_file(tree.join("Cargo.toml")).unwrap();
    assert_eq!(encodes(), Some(2));
    assert_eq!(compare().status.code(), Some(2));
    workspace(&["compiler/mente-syntax"]);
    // Each flag misspelled alone, the command is a usage error.
    for k in 0..4 {
        let mut flags = ["--base-tree", "--base", "--head", "--waivers"];
        let wrong = format!("{}s", flags[k]);
        flags[k] = &wrong;
        let run = mn(&[
            "smt",
            "--compare",
            flags[0],
            tree.to_str().unwrap(),
            flags[1],
            base_keys.to_str().unwrap(),
            base_answers.to_str().unwrap(),
            flags[2],
            head_keys.to_str().unwrap(),
            head_answers.to_str().unwrap(),
            flags[3],
            waivers.to_str().unwrap(),
        ]);
        assert_eq!(run.status.code(), Some(2), "{}", wrong);
        assert!(text(&run.stderr).starts_with("Usage: mn smt --compare"));
    }
    fs::write(&head_keys, "check\tm.mn\n").unwrap();
    for usage in [
        compare(),
        mn(&["smt", "--compare", "--base", base_keys.to_str().unwrap()]),
        mn(&["smt", "--encodes"]),
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
