//! The forms the project's records are written in, defined once and read by every reader of a
//! record (Norman, 2026-11-17): the design register's tests (`tests/register.rs`), the record of
//! escaped bugs (`escapes.rs`, `tests/escapes.rs`, `mn escapes`), and the forms' own tests
//! (`tests/records.rs`). A record is named by a key, never by a number, and the key rule,
//! [`is_key`], is that structure; a number written in prose is held by review only.
//!
//! A kind of record is kept one file a record, in a directory of its own, each file named by its
//! record's key (Norman, 2026-11-17): two changes in flight that each add a record then never touch
//! one file, and a record is found by its name. [`KINDS`] declares every kind kept so, [`SHARED`]
//! every file that still holds many records, each with why it is not yet a kind, and
//! [`DOCUMENTS`] every set of Markdown documents that are no records, each with why. Every Markdown
//! file of the tree belongs to exactly one of them ([`undeclared`]), so that a new directory of
//! records cannot stand undeclared, its names unread.

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
/// it is the file's key. spec-brief 0.15's `new` names the file it writes after the brief's title,
/// whatever `--id` says, so a brief is made in two steps: `spec-brief new "<title>" --id <key>`,
/// then the file it wrote renamed `briefs/<key>.md`.
pub const BRIEFS: Kind = Kind {
    record: "a task brief",
    dir: "briefs",
    extension: "md",
    archive: Some("archive"),
};

/// The tasks of the agent benchmark, `benchmark/tasks/<key>.md` (`benchmark/README.md`).
pub const BENCHMARK_TASKS: Kind = Kind {
    record: "a task of the agent benchmark",
    dir: "benchmark/tasks",
    extension: "md",
    archive: None,
};

/// The equivalent mutants of the MIR mutation gate,
/// `compiler/mente-cli/tests/mutants/equivalent/<key>.md`, each named by the key
/// `mir_mutants::mutant_key` derives from the mutant's name.
pub const EQUIVALENT_MUTANTS: Kind = Kind {
    record: "an equivalent mutant",
    dir: "compiler/mente-cli/tests/mutants/equivalent",
    extension: "md",
    archive: None,
};

/// The groups of the equivalent mutants, `compiler/mente-cli/tests/mutants/equivalence-groups/<key>.md`,
/// each stating once the reasoning its mutants share, which each mutant's record names.
pub const EQUIVALENCE_GROUPS: Kind = Kind {
    record: "a group of equivalent mutants",
    dir: "compiler/mente-cli/tests/mutants/equivalence-groups",
    extension: "md",
    archive: None,
};

/// Every kind of record kept one file a record.
pub const KINDS: &[Kind] = &[
    ESCAPES,
    BRIEFS,
    BENCHMARK_TASKS,
    EQUIVALENT_MUTANTS,
    EQUIVALENCE_GROUPS,
];

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
        file: "docs/STATUS.md",
        records: "the implementation status of each mechanism, a row each",
        why: "spec-graph reads it as a ledger, one file (`.spec-graph.json`); it becomes a kind when spec-graph reads a ledger kept one file an entry, which the feature asked of the tools' owner for the register would cover",
    },
    Shared {
        file: "ROADMAP.md",
        records: "the milestones' checklists and the measurement protocols",
        why: "spec-graph reads its checklists and each Target claim's protocol from this one file (`.spec-graph.json`); it becomes a kind when spec-graph reads them kept one file each, which the same feature asked of the tools' owner would cover",
    },
    Shared {
        file: "docs/milestone-reviews.md",
        records: "the review of each finished milestone",
        why: "a review is written once, when its milestone closes, and not edited after, and milestones close one at a time, so no two changes in flight write one review",
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
];

/// A set of Markdown documents that are no records: each file matches one of its patterns, where
/// `*` stands for any characters of one name and `**` for any number of directories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Documents {
    /// The files of the set, from the repository's root.
    pub patterns: &'static [&'static str],
    /// What the documents are.
    pub what: &'static str,
    /// Why they are no records of a kind.
    pub why: &'static str,
}

/// Every set of Markdown documents that are no records. No two sets, kinds or shared files claim
/// one file.
pub const DOCUMENTS: &[Documents] = &[
    Documents {
        patterns: &["README.md", "CLAUDE.md", "SECURITY.md"],
        what: "the repository's front pages",
        why: "each is the one page of its subject, read whole",
    },
    Documents {
        patterns: &["spec/*.md"],
        what: "the specification, one mechanism a file, and its index `spec/README.md`",
        why: "a spec is a document of one decision, not a record; the spec layout's test holds its names",
    },
    Documents {
        patterns: &[
            "docs/design-brief.md",
            "docs/escapes.md",
            "docs/language-card.md",
            "docs/platform-layer.md",
        ],
        what: "the documentation's pages",
        why: "each is a page of one subject, edited as a whole",
    },
    Documents {
        patterns: &[
            ".github/scripts/policy/README.md",
            "benchmark/README.md",
            "compiler/mente-cli/src/front-matter/README.md",
            "conformance/README.md",
            "examples/README.md",
            "perf/README.md",
            "site/README.md",
            "spikes/*/README.md",
        ],
        what: "the descriptions of directories",
        why: "each describes its directory, and a spike's description is its report",
    },
    Documents {
        patterns: &["benchmark/control/*.md"],
        what: "the benchmark's language cards of its control languages",
        why: "each is the card an agent is shown for one language, compared with the language card",
    },
    Documents {
        patterns: &["benchmark/harness/fixtures/**/*.md"],
        what: "the benchmark harness's fixtures",
        why: "they stand in for the held-out set in the harness's tests, in the held-out set's own form",
    },
    Documents {
        patterns: &[
            "benchmark/prelim/**/*.md",
            "benchmark/probe/**/*.md",
            "benchmark/results/**/*.md",
        ],
        what: "the benchmark's runs: each agent's replies, named by the harness, and each run's description",
        why: "a run's output is written once by the harness and never edited, so no two changes touch it",
    },
];

/// Every Markdown file under `root`, its path from there with `/` between names, that no kind,
/// shared file or set of documents claims, or that more than one claims: each with what to do.
/// The walk leaves out `.git`, `target`, `node_modules` and every directory that holds a `.git`
/// of its own, a worktree or another repository.
pub fn undeclared(
    root: &Path,
    kinds: &[Kind],
    shared: &[Shared],
    documents: &[Documents],
) -> Vec<String> {
    let mut files = Vec::new();
    let mut problems = Vec::new();
    markdown_files(root, "", &mut files, &mut problems);
    files.sort();
    for file in files {
        let mut claims: Vec<String> = Vec::new();
        for kind in kinds {
            if file.starts_with(&format!("{}/", kind.dir)) {
                claims.push(format!("the kind of {}", kind.record));
            }
        }
        for s in shared {
            if file == s.file {
                claims.push(format!("the shared file of {}", s.records));
            }
        }
        for set in documents {
            if set.patterns.iter().any(|p| matches_pattern(p, &file)) {
                claims.push(format!("the documents of {}", set.what));
            }
        }
        match claims.len() {
            1 => {}
            0 => problems.push(format!(
                "{}: a Markdown file nothing declares: declare its directory in records.rs as a kind of record, whose files are then named by keys (`KINDS`), or as a set of documents with why they are none (`DOCUMENTS`)",
                file
            )),
            _ => problems.push(format!(
                "{}: claimed by {}, where one declaration is to claim it",
                file,
                claims.join(" and ")
            )),
        }
    }
    problems
}

/// The Markdown files under `dir`, whose path from the root starts with `prefix`, into `out`, and
/// each directory that cannot be read into `problems`.
fn markdown_files(dir: &Path, prefix: &str, out: &mut Vec<String>, problems: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            problems.push(format!("{}: {}", dir.display(), e));
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            let skipped = matches!(name.as_str(), ".git" | "target" | "node_modules")
                || path.join(".git").exists();
            if !skipped {
                markdown_files(&path, &format!("{}{}/", prefix, name), out, problems);
            }
        } else if Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            out.push(format!("{}{}", prefix, name));
        }
    }
}

/// Whether the path `file` matches `pattern`, name by name: `**` stands for any number of names,
/// and `*` within a name for any characters of it.
pub fn matches_pattern(pattern: &str, file: &str) -> bool {
    fn names(pattern: &[&str], file: &[&str]) -> bool {
        match (pattern.split_first(), file.split_first()) {
            (None, None) => true,
            (Some((&"**", rest)), _) => {
                names(rest, file) || file.split_first().is_some_and(|(_, f)| names(pattern, f))
            }
            (Some((p, rest)), Some((f, others))) => matches_name(p, f) && names(rest, others),
            _ => false,
        }
    }
    let pattern: Vec<&str> = pattern.split('/').collect();
    let file: Vec<&str> = file.split('/').collect();
    names(&pattern, &file)
}

/// Whether `name` matches `pattern`, a pattern of one name, where `*` stands for any characters.
fn matches_name(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((head, rest)) => name.strip_prefix(head).is_some_and(|after| {
            (0..=after.len())
                .filter(|&k| after.is_char_boundary(k))
                .any(|k| matches_name(rest, &after[k..]))
        }),
    }
}

/// The file that keeps a directory in git while it holds no record.
pub const KEEP: &str = ".gitkeep";

/// Each record of `kind` in its directory `dir`, by key, with its file, in the order of their keys,
/// the finished records of its archive among them; or each entry that is none: a file whose name is
/// not `<key>.<extension>` by [`is_key`], a subdirectory but the archive, a link, and a key both
/// the directory and its archive hold, each with what to do. [`KEEP`] may stand in either.
pub fn entries(kind: &Kind, dir: &Path) -> Result<Vec<(String, PathBuf)>, Vec<String>> {
    let (found, problems) = scan(kind, dir);
    if problems.is_empty() {
        Ok(found)
    } else {
        Err(problems)
    }
}

/// The records of `kind` in `dir` that are records, as [`entries`] reads them, and every problem
/// beside them, so that a reader of the records can report each problem of a file, too, in the
/// same run.
pub fn scan(kind: &Kind, dir: &Path) -> (Vec<(String, PathBuf)>, Vec<String>) {
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
    (found, problems)
}

/// The records of `kind` in `dir`, named in problems from `prefix`, with `archive` the one
/// subdirectory read as more records. A link is read as itself, never as what it points to.
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
            problems.push(format!(
                "{}: {}; {} keeps its records in this directory, which is to exist, holding `{}` while it is empty",
                dir.display(),
                e,
                kind.record,
                KEEP
            ));
            return;
        }
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            problems.push(format!("{}{}: unreadable", prefix, name));
            continue;
        };
        if meta.file_type().is_symlink() {
            problems.push(format!(
                "{}{}: not {}, but a link; a kind holds its records' files themselves",
                prefix, name, kind.record
            ));
            continue;
        }
        if meta.is_dir() {
            if archive == Some(name.as_str()) {
                let inner = format!("{}{}/", prefix, name);
                read_entries(kind, &path, &inner, None, found, problems);
            } else {
                problems.push(format!(
                    "{}{}: not {}, but a directory; the kind keeps its records in files `<key>.{}`{}",
                    prefix,
                    name,
                    kind.record,
                    kind.extension,
                    kind.archive
                        .map(|a| format!(", the finished ones in `{}/`", a))
                        .unwrap_or_default()
                ));
            }
            continue;
        }
        if name == KEEP {
            continue;
        }
        let suffix = format!(".{}", kind.extension);
        match name.strip_suffix(&suffix) {
            Some(key) if is_key(key) => found.push((key.to_string(), path)),
            Some(key) => problems.push(format!(
                "{}{}: not {}, since its name is no key: {}{}",
                prefix,
                name,
                kind.record,
                key_problems(key).join("; "),
                rename(key, &suffix)
            )),
            None => problems.push(format!(
                "{}{}: not {}, a file `<key>{}`{}",
                prefix,
                name,
                kind.record,
                suffix,
                rename(
                    Path::new(&name)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                        .as_str(),
                    &suffix
                )
            )),
        }
    }
}

/// What to say of a file whose name `stem` is no key: the key [`suggest_key`] makes of it, or to
/// name the record by what it records.
fn rename(stem: &str, suffix: &str) -> String {
    let key = if is_key(stem) {
        Some(stem.to_string())
    } else {
        suggest_key(stem)
    };
    match key {
        Some(key) => format!("; rename it `{}{}`", key, suffix),
        None => "; name it by what it records, in lowercase words joined by hyphens".to_string(),
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

/// One part of the rule a key keeps: what it asks, and whether a name keeps it.
#[derive(Clone, Copy)]
pub struct KeyPart {
    pub asks: &'static str,
    pub holds: fn(&str) -> bool,
}

/// The rule a record's key keeps, part by part, the one place it is defined: [`is_key`] holds a
/// name to every part, and a refusal names each part a name breaks. A key is named as the
/// register's entries are, `guard-per-problem` or `bench-retired-misplaced-paren`.
pub const KEY_PARTS: &[KeyPart] = &[
    KeyPart {
        asks: "lowercase ASCII words of letters and digits, joined by single hyphens",
        holds: |k| {
            k.split('-').all(|w| {
                !w.is_empty()
                    && w.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            })
        },
    },
    KeyPart {
        asks: "every word starting with a letter, so that no word is a number, as in `esc-001` or `fix-2nd-pass`",
        holds: |k| {
            k.split('-')
                .all(|w| w.starts_with(|c: char| c.is_ascii_lowercase()))
        },
    },
    KeyPart {
        asks: "at most 100 characters, so that its file's path fits every file system the repository is checked out on",
        holds: |k| k.len() <= KEY_LENGTH,
    },
    KeyPart {
        asks: "no name Windows reserves for a device (`con`, `prn`, `aux`, `nul`, `com1` to `com9`, `lpt1` to `lpt9`)",
        holds: |k| !reserved(k),
    },
    KeyPart {
        asks: "not `readme`, the name of a directory's description",
        holds: |k| k != "readme",
    },
];

/// Whether `key` is a record's key: it keeps every part of [`KEY_PARTS`].
pub fn is_key(key: &str) -> bool {
    KEY_PARTS.iter().all(|part| (part.holds)(key))
}

/// What each part of [`KEY_PARTS`] that `key` breaks asks of it.
pub fn key_problems(key: &str) -> Vec<String> {
    KEY_PARTS
        .iter()
        .filter(|part| !(part.holds)(key))
        .map(|part| format!("a key is {}", part.asks))
        .collect()
}

/// The key a name that is none could be renamed: lowercase, each run of other characters a hyphen,
/// and each word that is a number from 0 to 20, or its ordinal, spelled in letters (`2nd` is
/// `second`). `None` when no key comes of it, as of a number with a leading zero, which numbers a
/// record (`esc-001`), or of a greater one: then the record is to be named by what it records.
pub fn suggest_key(name: &str) -> Option<String> {
    const NUMBERS: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    const ORDINALS: [&str; 21] = [
        "zeroth",
        "first",
        "second",
        "third",
        "fourth",
        "fifth",
        "sixth",
        "seventh",
        "eighth",
        "ninth",
        "tenth",
        "eleventh",
        "twelfth",
        "thirteenth",
        "fourteenth",
        "fifteenth",
        "sixteenth",
        "seventeenth",
        "eighteenth",
        "nineteenth",
        "twentieth",
    ];
    let lower = name.to_ascii_lowercase();
    let words: Vec<String> = lower
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .filter(|w| !w.is_empty())
        .map(|word| {
            if !word.starts_with(|c: char| c.is_ascii_digit()) {
                return Some(word.to_string());
            }
            let digits: String = word.chars().take_while(char::is_ascii_digit).collect();
            let rest = &word[digits.len()..];
            if digits.len() > 1 && digits.starts_with('0') {
                return None;
            }
            let n: usize = digits.parse().ok().filter(|n| *n <= 20)?;
            match rest {
                "" => Some(NUMBERS[n].to_string()),
                "st" | "nd" | "rd" | "th" => Some(ORDINALS[n].to_string()),
                _ => None,
            }
        })
        .collect::<Option<_>>()?;
    let key = words.join("-");
    is_key(&key).then_some(key)
}

/// The word of a key that stands for `part`, a name of a path or a word of an identifier, such that
/// no two parts give one word and every word is a key's: a part of lowercase letters and digits
/// that starts with a letter other than `z` is itself; one that starts with `z` or with a digit is
/// that part after a `z` (`zero` is `zzero`, `4` is `z4`, `01` is `z01`); any other, an empty part
/// or one with a capital or another character, is `zh` and its UTF-8 in hexadecimal (`Foo` is
/// `zh466f6f`). The second letter tells the three apart: a `z`, a digit or an `h`. Every kind
/// whose keys come from names derives them so ([`key_of`]).
pub fn key_word(part: &str) -> String {
    let plain = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    };
    match part.chars().next() {
        Some('a'..='y') if plain(part) => part.to_string(),
        Some('z' | '0'..='9') if plain(part) => format!("z{}", part),
        _ => part
            .bytes()
            .fold("zh".to_string(), |word, b| format!("{}{:02x}", word, b)),
    }
}

/// The key of `names`, each a sequence of parts: each part its [`key_word`], the parts of a name
/// joined by hyphens, and the names by the word `z`, which no part gives, so that one key comes of
/// each sequence of names and no two records' names give one key. `None` when the key breaks the
/// rule anyway, as one longer than [`KEY_LENGTH`] does, or a part alone that is a device's name.
pub fn key_of(names: &[&[&str]]) -> Option<String> {
    let key = names
        .iter()
        .map(|parts| {
            parts
                .iter()
                .map(|p| key_word(p))
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect::<Vec<_>>()
        .join("-z-");
    is_key(&key).then_some(key)
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
