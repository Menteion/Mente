//! The forms the project's records are written in, defined once and read by every reader of a
//! record (Norman, 2026-11-17): the design register's tests (`tests/register.rs`), the record of
//! escaped bugs (`escapes.rs`, `tests/escapes.rs`, `mn escapes`), and the forms' own tests
//! (`tests/records.rs`), which also hold that no document names a record by a number.
//!
//! A kind of record is kept one file a record, in a directory of its own, each file named by its
//! record's key (Norman, 2026-11-17): two changes in flight that each add a record then never touch
//! one file, and a record is found by its name. [`KINDS`] declares every kind kept so, and
//! [`SHARED`] every file that still holds many records, each with why it is not yet a kind.

use std::fs;
use std::path::{Path, PathBuf};

/// A kind of record kept one file a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    /// What one record of the kind is, `an escaped bug`.
    pub record: &'static str,
    /// The kind's directory, from the repository's root.
    pub dir: &'static str,
    /// The extension of every record's file.
    pub extension: &'static str,
    /// The subdirectory, of the same form, that holds the kind's finished records, if it has one.
    pub archive: Option<&'static str>,
}

/// The record of escaped bugs, `docs/escapes/<key>.md` (`escapes.rs`).
pub const ESCAPES: Kind = Kind {
    record: "an escaped bug",
    dir: "docs/escapes",
    extension: "md",
    archive: None,
};

/// The task briefs, `briefs/<key>.md`, which spec-brief reads and archives to
/// `briefs/archive/<key>.md`. `.spec-brief.json` takes each brief's id from its front matter, where
/// it is the file's key.
pub const BRIEFS: Kind = Kind {
    record: "a task brief",
    dir: "briefs",
    extension: "md",
    archive: Some("archive"),
};

/// Every kind of record kept one file a record.
pub const KINDS: &[Kind] = &[ESCAPES, BRIEFS];

/// A file that holds many records, and why it is not yet a kind of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shared {
    /// The file, from the repository's root.
    pub file: &'static str,
    /// The records it holds.
    pub records: &'static str,
    /// Why it is still one file.
    pub why: &'static str,
}

/// Every file that still holds many records. A file leaves this list in the change that makes its
/// records a kind, since the test that reads the list requires each file to exist.
pub const SHARED: &[Shared] = &[
    Shared {
        file: "docs/design-review.md",
        records: "the design register's entries",
        why: "spec-graph reads a register only as the table or the sections of one file, and checks its statuses, citations and keys there; the register becomes a kind when spec-graph reads a register kept one file an entry, a feature asked of the tools' owner",
    },
    Shared {
        file: "compiler/mente-cli/tests/golden/mir.txt",
        records: "the MIR of each program of the corpus",
        why: "it becomes one file a program in a pull request of its own",
    },
    Shared {
        file: "compiler/mente-cli/tests/golden/diagnostics.txt",
        records: "the diagnostics of each program of the corpus",
        why: "it becomes one file a program in a pull request of its own",
    },
    Shared {
        file: "compiler/mente-cli/tests/golden/projections.txt",
        records: "the projections of each program of the corpus",
        why: "it becomes one file a program in a pull request of its own",
    },
    Shared {
        file: "compiler/mente-cli/tests/mutants/equivalent.txt",
        records: "the equivalent mutants of the MIR mutation gate, each with its reason",
        why: "it becomes one file a mutant, its reason in the front matter, in a pull request of its own",
    },
];

/// The file that keeps a directory in git while it holds no record.
pub const KEEP: &str = ".gitkeep";

/// Each record of `kind` in its directory `dir`, by key, with its file, in the order of their keys,
/// the finished records of its archive among them; or each entry that is none: a file whose name is
/// not `<key>.<extension>` by [`is_key`], a subdirectory but the archive, and a key both the
/// directory and its archive hold. [`KEEP`] may stand in either.
pub fn entries(kind: &Kind, dir: &Path) -> Result<Vec<(String, PathBuf)>, Vec<String>> {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    read_entries(kind, dir, "", kind.archive, &mut found, &mut problems);
    found.sort();
    for pair in found.windows(2) {
        if pair[0].0 == pair[1].0 {
            problems.push(format!(
                "{}: a key two files hold, {} and {}",
                pair[0].0,
                pair[0].1.display(),
                pair[1].1.display()
            ));
        }
    }
    if problems.is_empty() {
        Ok(found)
    } else {
        Err(problems)
    }
}

/// The records of `kind` in `dir`, named in problems from `prefix`, with `archive` the one
/// subdirectory read as more records.
fn read_entries(
    kind: &Kind,
    dir: &Path,
    prefix: &str,
    archive: Option<&str>,
    found: &mut Vec<(String, PathBuf)>,
    problems: &mut Vec<String>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            problems.push(format!("{}: {}", dir.display(), e));
            return;
        }
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if path.is_dir() && archive == Some(name.as_str()) {
            let inner = format!("{}{}/", prefix, name);
            read_entries(kind, &path, &inner, None, found, problems);
            continue;
        }
        if path.is_file() && name == KEEP {
            continue;
        }
        let key = name
            .strip_suffix(&format!(".{}", kind.extension))
            .filter(|k| path.is_file() && is_key(k));
        match key {
            Some(key) => found.push((key.to_string(), path)),
            None => problems.push(format!(
                "{}{}: not {}, which is a file `<key>.{}` whose key is lowercase words of letters and digits joined by hyphens, and no name Windows reserves",
                prefix, name, kind.record, kind.extension
            )),
        }
    }
}

/// Whether `day` is a day of the calendar written `YYYY-MM-DD`.
pub fn is_day(day: &str) -> bool {
    let shape = day.len() == 10
        && day.char_indices().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    if !shape {
        return false;
    }
    let number = |range: std::ops::Range<usize>| day[range].parse::<u32>().unwrap_or(0);
    let (year, month, date) = (number(0..4), number(5..7), number(8..10));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=length).contains(&date)
}

/// Whether `key` is a record's key, as the register's entries are named: lowercase words of
/// letters and digits joined by single hyphens, `guard-per-problem` or `bench-retired-misplaced-paren`.
/// Every word starts with a letter, so that no key is a number in a key's dress: not `esc-001`,
/// `adr-0258` or `esc-001a`. Since a key names a file, it is no name Windows reserves, not
/// `readme`, which a directory's description is named, and at most [`KEY_LENGTH`] characters, so
/// that its file's path fits every file system the repository is checked out on.
pub fn is_key(key: &str) -> bool {
    !reserved(key)
        && key != "readme"
        && key.len() <= KEY_LENGTH
        && key.split('-').all(|word| {
            word.starts_with(|c: char| c.is_ascii_lowercase())
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// The most characters a key may hold.
pub const KEY_LENGTH: usize = 100;

/// Whether Windows reserves `name` for a device, whatever extension follows it, so that no file of
/// that name can be made or checked out there: `con`, `prn`, `aux`, `nul`, and `com` or `lpt`
/// with one digit.
fn reserved(name: &str) -> bool {
    let numbered = |device: &str| {
        name.strip_prefix(device)
            .is_some_and(|n| n.len() == 1 && n.chars().all(|c| c.is_ascii_digit()))
    };
    matches!(name, "con" | "prn" | "aux" | "nul") || numbered("com") || numbered("lpt")
}

/// The values of a record file's front matter, which names exactly `fields`, in that order, one
/// `name: value` line each, between a first line `---` and the next. Each value is a string a YAML
/// reader reads as this one does, so that the file renders where YAML is read: a plain value, which
/// starts with no YAML indicator, holds no `: ` and no ` #`, and is none of YAML's other scalars,
/// a null, a boolean, a number or a timestamp; or a value in single quotes, where `''` stands for
/// one quote. A field named `date` holds a day, `YYYY-MM-DD`, plain, which YAML reads as a date. No
/// character YAML reads otherwise than as written, or refuses, stands anywhere in the file: no
/// control character but the line feed (so no tab and no carriage return), no DEL, no next line
/// (U+0085), no line or paragraph separator (U+2028, U+2029) and no U+FFFE. What follows the front
/// matter is the record's prose, free.
///
/// The cases of `src/front-matter/` hold this behaviour as data, one file a case, so that every
/// reader of the form is tested against the same cases (`tests/records.rs`); the `README.md` there
/// gives a case's form.
pub fn front_matter(text: &str, fields: &[&str]) -> Result<Vec<String>, String> {
    let unread = |c: char| {
        (c.is_control() && c != '\n') || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FFFE}')
    };
    if let Some(c) = text.chars().find(|&c| unread(c)) {
        return Err(format!(
            "a tab, a carriage return or another character YAML reads otherwise than as written: U+{:04X}",
            u32::from(c)
        ));
    }
    let rest = text
        .strip_prefix("---\n")
        .ok_or("no front matter: the file does not start with a line `---`")?;
    let front = match rest.split_once("\n---\n") {
        Some((front, _)) => front,
        None => rest
            .strip_suffix("\n---")
            .ok_or("its front matter does not close with a line `---`")?,
    };
    let lines: Vec<(&str, &str)> = front
        .lines()
        .map(|l| l.split_once(": ").unwrap_or((l, "")))
        .collect();
    let names: Vec<&str> = lines.iter().map(|(n, _)| *n).collect();
    if names != fields {
        return Err(format!(
            "its front matter names {:?}, not the fields {:?} in that order",
            names, fields
        ));
    }
    lines
        .iter()
        .map(|(name, value)| {
            let plain = !value.starts_with('\'');
            if *name == "date" {
                return (plain && is_day(value))
                    .then(|| value.to_string())
                    .ok_or("the value of `date` is no day `YYYY-MM-DD` written plain".to_string());
            }
            if plain && timestamp(value) {
                return Err(format!(
                    "the value of `{}` is a timestamp YAML reads as a date: quote it, `'...'`",
                    name
                ));
            }
            scalar(value).ok_or(format!(
                "the value of `{}` is no plain string and no value in single quotes: quote it, `'...'`, with `''` for a quote",
                name
            ))
        })
        .collect()
}

/// Whether YAML reads the plain `value` as a timestamp: four digits, `-`, one or two, `-`, one or
/// two, and then nothing, or a `T`, a `t` or a run of spaces or tabs before a digit.
fn timestamp(value: &str) -> bool {
    let mut parts = value.splitn(3, '-');
    let (Some(year), Some(month), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let digits = |s: &str, lengths: &[usize]| {
        lengths.contains(&s.len()) && s.chars().all(|c| c.is_ascii_digit())
    };
    let day: String = rest.chars().take_while(char::is_ascii_digit).collect();
    // What follows the day starts with no digit, so a time starts with one only after a `T`, a
    // `t` or spaces.
    let after = &rest[day.len()..];
    let time = after
        .strip_prefix(['T', 't'])
        .unwrap_or_else(|| after.trim_start_matches([' ', '\t']));
    digits(year, &[4])
        && digits(month, &[1, 2])
        && digits(&day, &[1, 2])
        && (after.is_empty() || time.starts_with(|c: char| c.is_ascii_digit()))
}

/// The string a YAML reader reads in `value`, if it reads a string this reader agrees on.
fn scalar(value: &str) -> Option<String> {
    if let Some(inner) = value.strip_prefix('\'') {
        let inner = inner.strip_suffix('\'')?;
        // Every quote inside is one of a pair.
        if inner.replace("''", "").contains('\'') {
            return None;
        }
        return Some(inner.replace("''", "'"));
    }
    let plain = !value.is_empty()
        && value.trim() == value
        && !value.starts_with(|c: char| "-?:,[]{}#&*!|>'\"%@`".contains(c))
        && !value.contains(": ")
        && !value.contains(" #")
        && !value.ends_with(':')
        && !other_scalar(value);
    plain.then(|| value.to_string())
}

/// Whether YAML reads the plain `value` as a scalar other than a string, in version 1.2 or 1.1: a
/// null (`~`, `null`), a boolean (`y`, `yes`, `true`, `on` and their opposites) in any case, or a
/// number: an integer, decimal, binary, octal or hexadecimal, a float, `.inf` and `.nan` among
/// them, a number with `_` between its digits, or a sexagesimal one, `12:30`.
fn other_scalar(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let word = matches!(
        lower.as_str(),
        "~" | "null" | "y" | "n" | "yes" | "no" | "true" | "false" | "on" | "off"
    );
    let number = lower.replace('_', "");
    let unsigned = number.strip_prefix(['+', '-']).unwrap_or(&number);
    let digits = |s: &str, radix: u32| !s.is_empty() && s.chars().all(|c| c.is_digit(radix));
    let special = matches!(unsigned, ".inf" | ".nan");
    let radix = [("0b", 2), ("0o", 8), ("0x", 16)]
        .iter()
        .any(|(prefix, radix)| {
            unsigned
                .strip_prefix(prefix)
                .is_some_and(|s| digits(s, *radix))
        });
    let sexagesimal = unsigned.starts_with(|c: char| c.is_ascii_digit())
        && unsigned.contains(':')
        && unsigned
            .chars()
            .all(|c| c.is_ascii_digit() || c == ':' || c == '.');
    let (mantissa, exponent) = match unsigned.split_once('e') {
        Some((m, e)) => (m, Some(e.strip_prefix(['+', '-']).unwrap_or(e))),
        None => (unsigned, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let float = (digits(whole, 10) || (whole.is_empty() && digits(fraction, 10)))
        && (fraction.is_empty() || digits(fraction, 10))
        && exponent.is_none_or(|e| digits(e, 10));
    word || special || radix || sexagesimal || float
}
