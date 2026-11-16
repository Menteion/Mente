//! The forms the project's records are written in (`src/records.rs`), and the rule that no record
//! is named by a number: CLAUDE.md's invariant 1 forbids `DEC-XXX` numbers in the specs, and
//! Norman extended it on 2026-11-17 to every kind of record, the register, the record of escaped
//! bugs, the specs and the task briefs. A number carries no meaning, needs an allocation that races
//! between changes in flight, and suggests an order the records do not have; a key, as the
//! register's entries have, does none of that.

use mente_cli::records;

use std::fs;
use std::path::{Path, PathBuf};

/// The rule a record's number breaks, with its fix.
const NUMBERED: &str = "a record is named by a key, never by a number (CLAUDE.md, invariant 1, for every kind of record): use a kebab-case key, as the register does";

/// Names of the form a record's number takes that are no record's: an encoding (`UTF-8`), a hash
/// (`SHA-256`), a vector extension (`AVX-512`) and a standard (`IEEE-754`).
const STANDARD_NAMES: &[&str] = &["UTF", "SHA", "AVX", "IEEE"];

/// Records of another repository, cited by the names they have there: Menteion's decision records
/// that pause it (`docs/design-review.md`, `menteion-pause`).
const CITED: &[&str] = &["ADR-0258", "ADR-0259"];

/// The directories whose documents name records: the register and the other documents, the specs,
/// and the task briefs.
const DOCUMENTS: &[&str] = &["docs", "spec", "briefs"];

/// Each identifier of a record by a number in `text`, with its line: two or more capital letters,
/// a hyphen and digits, as a word of its own, `ESC-001` or `DEC-7`, in code or not, but a standard
/// name or a cited record. A diagnostic code, `E0310`, has no hyphen and is none.
fn numbered(text: &str) -> Vec<(usize, String)> {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut found = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            let starts = chars[at].is_ascii_uppercase() && (at == 0 || !word(chars[at - 1]));
            if !starts {
                at += 1;
                continue;
            }
            let caps = chars[at..]
                .iter()
                .take_while(|c| c.is_ascii_uppercase())
                .count();
            let hyphen = at + caps;
            let digits = chars.get(hyphen + 1..).map_or(0, |rest| {
                rest.iter().take_while(|c| c.is_ascii_digit()).count()
            });
            let end = hyphen + 1 + digits;
            let ends = chars.get(end).is_none_or(|&c| !word(c));
            if caps >= 2 && chars.get(hyphen) == Some(&'-') && digits > 0 && ends {
                let id: String = chars[at..end].iter().collect();
                let prefix: String = chars[at..hyphen].iter().collect();
                if !STANDARD_NAMES.contains(&prefix.as_str()) && !CITED.contains(&id.as_str()) {
                    found.push((n + 1, id));
                }
            }
            at += caps.max(1);
        }
    }
    found
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every Markdown file under `dir`.
fn documents(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            documents(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// No document of the register, the specs or the briefs names a record by a number.
#[test]
fn test_no_document_names_a_record_by_a_number() {
    let mut files = Vec::new();
    for d in DOCUMENTS {
        documents(&root().join(d), &mut files);
    }
    assert!(
        files.iter().any(|f| f.ends_with("design-review.md")),
        "the register is among the documents read"
    );
    let mut found = Vec::new();
    for f in &files {
        let text = fs::read_to_string(f).unwrap();
        for (line, id) in numbered(&text) {
            found.push(format!("{}:{}: {}", f.display(), line, id));
        }
    }
    assert!(found.is_empty(), "{}:\n{}", NUMBERED, found.join("\n"));
}

/// A record's number is refused wherever it stands, in code or not; a diagnostic code, a standard
/// name, a cited record of another repository and a word with a hyphen before its digits are not.
#[test]
fn test_a_record_number_is_refused_and_a_diagnostic_code_is_not() {
    for text in [
        "Fixed in ESC-001.",
        "as DEC-007 says",
        "`ADR-12`",
        "(RFC-9110)",
        "ESC-1000",
    ] {
        assert_eq!(numbered(text).len(), 1, "{}", text);
    }
    for text in [
        "`E0310` is a diagnostic code",
        "UTF-8 and SHA-256, AVX-512, IEEE-754",
        "Menteion's ADR-0258",
        "Tier-1 and x86-64",
        "DEC-XXX numbers",
        "an N-1 split",
        "ESC-001x and my-ESC-001",
    ] {
        assert_eq!(numbered(text), Vec::<(usize, String)>::new(), "{}", text);
    }
    assert_eq!(
        numbered("one\nthen ESC-001 here"),
        vec![(2, "ESC-001".to_string())]
    );
}

/// A key is lowercase words of letters and digits, the first a letter, joined by single hyphens.
#[test]
fn test_a_key_is_lowercase_words_joined_by_hyphens() {
    for key in [
        "guard-per-problem",
        "bench-retired-misplaced-paren",
        "e0310-message",
        "m2",
    ] {
        assert!(records::is_key(key), "{}", key);
    }
    for key in [
        "",
        "ESC-001",
        "Guard-per-problem",
        "guard--per",
        "-guard",
        "guard-",
        "guard_per",
        "2-guard",
        "guard per",
    ] {
        assert!(!records::is_key(key), "{}", key);
    }
}

/// A day is a day of the calendar written `YYYY-MM-DD`.
#[test]
fn test_a_day_is_a_day_of_the_calendar() {
    for day in ["2026-11-17", "2028-02-29", "2000-02-29"] {
        assert!(records::is_day(day), "{}", day);
    }
    for day in [
        "2026-02-29",
        "2100-02-29",
        "2026-13-01",
        "2026-11-1",
        "17-11-2026",
    ] {
        assert!(!records::is_day(day), "{}", day);
    }
}
