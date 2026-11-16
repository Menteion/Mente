//! The forms the project's records are written in (`src/records.rs`), and the rule that no record
//! is named by a number: CLAUDE.md's invariant 1 forbids `DEC-XXX` numbers in the specs, and
//! Norman extended it on 2026-11-17 to every kind of record, the register, the record of escaped
//! bugs, the specs and the task briefs. A number carries no meaning, needs an allocation that races
//! between changes in flight, and suggests an order the records do not have; a key, as the
//! register's entries have, does none of that.
//!
//! The rule here refuses a record's identifier by number, `ESC-001`. A spec's items cited by their
//! place, "item 3 of core-calculus.md", are another kind of number: the stack-budget work moves
//! those citations to the specs' keys, and their refusal comes with it.
//!
//! The check guards against a slip of the convention, not an adversary. It reads the text as a
//! reader sees it, through markup, unseen characters and other hyphens, and inside compounds, but
//! an identifier split across two lines on purpose, `ESC-` at a line's end and `001` on the next, is
//! held by review only: joining words at a line's end would invent identifiers, `on-` then `2026`,
//! where none was written.

use mente_cli::records;

use std::fs;
use std::path::{Path, PathBuf};

/// The rule a record's number breaks, with its fix.
const NUMBERED: &str = "a record is named by a key, never by a number (CLAUDE.md, invariant 1, for every kind of record): use a kebab-case key, as the register does";

/// The names of a record's form that are no record's, each exactly as it is spelled, in any case:
/// encodings, hashes, a vector extension and a floating-point standard; the licence; Tier 1 and
/// Tier 2, Mente's two tiers of proof; the four stages of the roadmap; and a runner's image.
const NAMES: &[&str] = &[
    "UTF-8",
    "UTF-16",
    "UTF-32",
    "SHA-1",
    "SHA-256",
    "SHA-512",
    "AVX-512",
    "IEEE-754",
    "Apache-2",
    "Tier-1",
    "Tier-2",
    "stage-1",
    "stage-2",
    "stage-3",
    "stage-4",
    "ubuntu-24",
];

/// The directories whose documents name records: the register and the other documents, the specs,
/// and the task briefs; and the documents at the root.
const DOCUMENTS: &[&str] = &[
    "docs",
    "spec",
    "briefs",
    "ROADMAP.md",
    "README.md",
    "CLAUDE.md",
];

/// Each hyphen of another code point than `-`: the hyphen, the non-breaking hyphen, the figure
/// dash, the en and em dashes, the horizontal bar, the minus sign, and the small and full-width
/// hyphen-minus.
fn hyphen(c: char) -> bool {
    matches!(
        c,
        '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{FE58}' | '\u{FE63}' | '\u{FF0D}'
    )
}

/// Whether `c` is written but not seen: a zero-width space, joiner or non-joiner, the word joiner,
/// the byte-order mark, or the soft hyphen.
fn unseen(c: char) -> bool {
    matches!(
        c,
        '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}'
    )
}

/// The characters of `text` as a reader sees the words, each with its line: the markup of
/// emphasis and code (`*`, `_`, `~`, a backtick) and the characters no one sees taken out, and
/// every hyphen made `-`.
fn normalised(text: &str) -> Vec<(char, usize)> {
    let mut out: Vec<(char, usize)> = Vec::new();
    let mut line = 1;
    for c in text.chars() {
        if matches!(c, '*' | '_' | '~' | '`') || unseen(c) {
            continue;
        }
        out.push((if hyphen(c) { '-' } else { c }, line));
        if c == '\n' {
            line += 1;
        }
    }
    out
}

/// Whether `c` ends a bare address in prose: a space, a character no address holds unescaped, or
/// punctuation that closes a sentence or a clause around it.
fn ends_address(c: char) -> bool {
    c.is_whitespace() || "<>\"'[]{}|\\^,;".contains(c)
}

/// `text` as [`normalised`] with each link's target blanked: the address of a Markdown link,
/// `[text](address)`, and of an autolink, `<https://...>`, where it closes before a space or the
/// line's end; and a bare address, `https://...`, up to what ends it ([`ends_address`]), a `)` it
/// does not open, or a full stop, colon, `!` or `?` that ends it. A link's text stays, a link that
/// never closes blanks nothing past its address, and an identifier glued to an address is read.
fn unlinked(text: &str) -> Vec<(char, usize)> {
    let mut c = normalised(text);
    let at = |c: &[(char, usize)], k: usize, s: &str| {
        s.chars()
            .enumerate()
            .all(|(j, x)| c.get(k + j).map(|p| p.0) == Some(x))
    };
    let mut k = 0;
    while k < c.len() {
        let (from, end) = if at(&c, k, "](") || at(&c, k, "<http://") || at(&c, k, "<https://") {
            let (from, close) = if c[k].0 == ']' {
                (k + 2, ')')
            } else {
                (k + 1, '>')
            };
            let end = (from..c.len())
                .find(|&j| c[j].0.is_whitespace() || c[j].0 == close)
                .unwrap_or(c.len());
            if c.get(end).map(|p| p.0) != Some(close) {
                k = end.max(k + 1);
                continue;
            }
            (from, end)
        } else if at(&c, k, "http://") || at(&c, k, "https://") {
            let mut open = 0usize;
            let mut end = k;
            while let Some(&(ch, _)) = c.get(end) {
                if ends_address(ch) || (ch == ')' && open == 0) {
                    break;
                }
                match ch {
                    '(' => open += 1,
                    ')' => open -= 1,
                    _ => {}
                }
                end += 1;
            }
            while end > k && matches!(c[end - 1].0, '.' | ':' | '!' | '?') {
                end -= 1;
            }
            (k, end)
        } else {
            k += 1;
            continue;
        };
        for p in &mut c[from..end] {
            p.0 = ' ';
        }
        k = end.max(k + 1);
    }
    c
}

/// Each identifier of a record by a number in `text`, with its line: two or more letters, in any
/// case, a hyphen and digits, with no letter or digit on either side, so also inside a compound,
/// `pre-ESC-001` or `ESC-001-fix`, unless more digits follow after a hyphen, as in a date or a
/// version (`menteion-paused-2026-11-15`, `claude-sonnet-5-5`), in code or not, `**ESC**-001`, but a name of [`NAMES`], and none
/// in a link's target, which names a page by its address. A diagnostic code, `E0310`, has no hyphen
/// and is none.
fn numbered(text: &str) -> Vec<(usize, String)> {
    let chars = unlinked(text);
    let word = |c: char| c.is_alphanumeric();
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let starts = chars[at].0.is_alphabetic() && (at == 0 || !word(chars[at - 1].0));
        if !starts {
            at += 1;
            continue;
        }
        let letters = chars[at..]
            .iter()
            .take_while(|p| p.0.is_alphabetic())
            .count();
        let hyphen = at + letters;
        let digits = chars.get(hyphen + 1..).map_or(0, |rest| {
            rest.iter().take_while(|p| p.0.is_ascii_digit()).count()
        });
        let end = hyphen + 1 + digits;
        let ends = chars.get(end).is_none_or(|p| !word(p.0))
            && !(chars.get(end).map(|p| p.0) == Some('-')
                && chars.get(end + 1).is_some_and(|p| p.0.is_ascii_digit()));
        if letters >= 2 && chars.get(hyphen).map(|p| p.0) == Some('-') && digits > 0 && ends {
            let id: String = chars[at..end].iter().map(|p| p.0).collect();
            if !NAMES.iter().any(|n| n.eq_ignore_ascii_case(&id)) {
                found.push((chars[at].1, id));
            }
        }
        at += letters.max(1);
    }
    found
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every Markdown file at `path`, or under it.
fn documents(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
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

/// No document of the register, the specs, the briefs or the root names a record by a number.
#[test]
fn test_no_document_names_a_record_by_a_number() {
    let mut files = Vec::new();
    for d in DOCUMENTS {
        documents(&root().join(d), &mut files);
    }
    for read in ["design-review.md", "ROADMAP.md", "README.md", "CLAUDE.md"] {
        assert!(
            files.iter().any(|f| f.ends_with(read)),
            "{} is among the documents read",
            read
        );
    }
    let mut found = Vec::new();
    for f in &files {
        let text = fs::read_to_string(f).unwrap();
        for (line, id) in numbered(&text) {
            found.push(format!("{}:{}: {}", f.display(), line, id));
        }
    }
    assert!(found.is_empty(), "{}:\n{}", NUMBERED, found.join("\n"));
}

/// A record's number is refused wherever it stands and however it is written: in code, in
/// emphasis, in a link's text, in any case, with any hyphen or an unseen character in it, inside a
/// compound, glued to an address, and after a link that never closes. A diagnostic code, a name of
/// [`NAMES`], a word with a hyphen before its digits, a word that runs on after them, a link's
/// target and words at the ends of two lines are not.
#[test]
fn test_a_record_number_is_refused_and_a_diagnostic_code_is_not() {
    for text in [
        "Fixed in ESC-001.",
        "as DEC-007 says",
        "`ADR-12`",
        "(RFC-9110)",
        "ESC-1000",
        "Menteion's ADR-0258",
        "Esc-001",
        "esc-001",
        "**ESC**-001",
        "`ESC`-001",
        "_ESC_-001",
        "ESC\u{2011}001",
        "ESC\u{2013}001",
        "ESC\u{2212}001",
        "ES\u{200B}C-001",
        "ESC\u{00AD}-001",
        "\u{FEFF}ESC-0\u{2060}01",
        "pre-ESC-001",
        "ESC-001-fix",
        "my-ESC-001",
        "SHA-001",
        "Tier-3",
        "[ADR-0258](https://example.org/pause)",
        "https://example.org/a ADR-0258",
        "<https://example.org/a> ADR-0258 (https://example.org/b)",
        "[unclosed](https://example.org ADR-0258",
        "[unclosed](ADR-0258",
        "<https://example.org ADR-0258",
        "https://example.org)ADR-0258",
        "https://example.org,ADR-0258",
        "https://example.org;ADR-0258",
        "https://example.org/\"ADR-0258",
        "ADR-0258https://example.org",
        "[a](https://example.org)ADR-0258",
        "---\nADR-0258",
    ] {
        assert_eq!(numbered(text).len(), 1, "{:?}: {:?}", text, numbered(text));
    }
    for text in [
        "`E0310` is a diagnostic code",
        "Tier-1 and x86-64",
        "DEC-XXX numbers",
        "an N-1 split",
        "ESC-001x",
        "Apache-2.0 and ubuntu-24.04",
        "stage-3 work and a Tier-1-style proof",
        "a word broken at its hyphen-\nand then 2026",
        "turned on-\n2026-11-17",
        "the tag `menteion-paused-2026-11-15` and `claude-sonnet-5-5`",
        "[the record that pauses it](https://example.org/ADR-0258)",
        "<https://example.org/ADR-0258>",
        "see https://example.org/ADR-0258, then",
        "see https://example.org/ADR-0258.",
        "(https://example.org/wiki/A_(B)/ADR-0258)",
        "[a](https://example.org/ADR-1) and [b](http://example.org/DEC-2)",
    ] {
        assert_eq!(numbered(text), Vec::<(usize, String)>::new(), "{:?}", text);
    }
    assert_eq!(
        numbered("one\nthen ESC-001 here"),
        vec![(2, "ESC-001".to_string())]
    );
    // An identifier split across two lines on purpose is held by review only.
    assert_eq!(
        numbered("one\nESC-\n001 here"),
        Vec::<(usize, String)>::new()
    );
}

/// Each name of [`NAMES`] is allowed exactly, in any case, and nothing beside it: the same letters
/// with other digits are a number again. The list holds these names and no other.
#[test]
fn test_each_allowed_name_is_exact() {
    let names = [
        "UTF-8",
        "UTF-16",
        "UTF-32",
        "SHA-1",
        "SHA-256",
        "SHA-512",
        "AVX-512",
        "IEEE-754",
        "Apache-2",
        "Tier-1",
        "Tier-2",
        "stage-1",
        "stage-2",
        "stage-3",
        "stage-4",
        "ubuntu-24",
    ];
    assert_eq!(names, NAMES);
    for name in names {
        for spelled in [name.to_string(), name.to_uppercase(), name.to_lowercase()] {
            assert_eq!(
                numbered(&spelled),
                Vec::<(usize, String)>::new(),
                "{}",
                spelled
            );
        }
        assert_eq!(numbered(&format!("{}0", name)).len(), 1, "{}0", name);
        let (letters, _) = name.split_once('-').unwrap();
        assert_eq!(
            numbered(&format!("{}-001", letters)).len(),
            1,
            "{}-001",
            letters
        );
    }
}

/// A key is lowercase words of letters and digits, the first a letter, joined by single hyphens;
/// no word is digits alone, so that no number passes as a key; and no key is a name Windows
/// reserves for a device, though a longer word that starts with one is a key.
#[test]
fn test_a_key_is_lowercase_words_joined_by_hyphens() {
    for key in [
        "guard-per-problem",
        "bench-retired-misplaced-paren",
        "e0310-message",
        "m2",
        "m2-review",
        "console",
        "nul-check",
        "com10",
        "lpt",
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
        "esc-001",
        "dec-7",
        "adr-0258",
        "escape-1",
        "a-1-b",
        "esc-001a",
        "adr-0258x",
        "guard-2x",
        "con",
        "prn",
        "aux",
        "nul",
        "com0",
        "com1",
        "lpt9",
    ] {
        assert!(!records::is_key(key), "{}", key);
    }
}

/// A day is a day of the calendar written `YYYY-MM-DD`: each length of month, February of a leap
/// year and of a common one, and no day 0 or month 0.
#[test]
fn test_a_day_is_a_day_of_the_calendar() {
    for day in [
        "2026-11-17",
        "2028-02-29",
        "2000-02-29",
        "2026-02-28",
        "2100-02-28",
        "2026-01-31",
        "2026-12-31",
        "2026-04-30",
        "2026-01-01",
    ] {
        assert!(records::is_day(day), "{}", day);
    }
    for day in [
        "2026-02-29",
        "2100-02-29",
        "2026-13-01",
        "2026-11-1",
        "17-11-2026",
        "2026-04-31",
        "2026-01-32",
        "2026-00-10",
        "2026-11-00",
        "2026/11/17",
    ] {
        assert!(!records::is_day(day), "{}", day);
    }
}

/// One case of `src/front-matter/`, in the form its `README.md` gives: its fields,
/// the values it is to be read as, `None` if it is to be refused, and its input.
type Case = (Vec<String>, Option<Vec<String>>, String);

fn case(text: &str) -> Result<Case, String> {
    let (head, input) = text.split_once("\ninput\n").ok_or("no line `input`")?;
    let mut lines = head.lines();
    let fields: Vec<String> = lines
        .next()
        .and_then(|l| l.strip_prefix("fields "))
        .ok_or("no line `fields`")?
        .split(' ')
        .map(str::to_string)
        .collect();
    let values = match lines.next() {
        Some("expect accepted") => {
            let mut values = Vec::new();
            for field in &fields {
                let line = lines.next().ok_or("a field with no line `value`")?;
                let rest = line
                    .strip_prefix("value ")
                    .and_then(|l| l.strip_prefix(field.as_str()))
                    .ok_or(format!("{}: no line `value {}`", line, field))?;
                values.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
            }
            Some(values)
        }
        Some("expect refused") => None,
        other => {
            return Err(format!(
                "{:?}: no line `expect accepted` or `expect refused`",
                other
            ))
        }
    };
    if let Some(extra) = lines.next() {
        return Err(format!(
            "{}: a line the case's form has no place for",
            extra
        ));
    }
    Ok((
        fields,
        values,
        input.replace("{CR}", "\r").replace("{TAB}", "\t"),
    ))
}

/// Every case of `src/front-matter/` is read by `records::front_matter` as the case says: its
/// values, or refused. Each case is a file `<key>.txt` of the form the directory's `README.md`
/// gives, the one other file there, so that another reader of the form is tested against the
/// same cases.
#[test]
fn test_front_matter_reads_every_case_of_its_fixtures() {
    let dir = root().join("compiler/mente-cli/src/front-matter");
    let mut accepted = 0;
    let mut refused = 0;
    for entry in fs::read_dir(&dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "README.md" {
            continue;
        }
        let key = name.strip_suffix(".txt").filter(|k| records::is_key(k));
        assert!(
            key.is_some(),
            "{}: no case, which is a file `<key>.txt`",
            name
        );
        let text = fs::read_to_string(entry.path()).unwrap();
        let (fields, values, input) = case(&text).unwrap_or_else(|e| panic!("{}: {}", name, e));
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        let read = records::front_matter(&input, &fields);
        match values {
            Some(values) => {
                assert_eq!(read, Ok(values), "{}", name);
                accepted += 1;
            }
            None => {
                assert!(read.is_err(), "{}: read as {:?}", name, read);
                refused += 1;
            }
        }
    }
    assert!(
        accepted > 0 && refused > 0,
        "{} accepted, {} refused",
        accepted,
        refused
    );
    for (text, why) in [
        ("fields a\nexpect accepted\nvalue a x\n", "no line `input`"),
        ("expect refused\ninput\n", "no line `fields`"),
        ("fields a\nexpect maybe\ninput\n", "no line `expect"),
        (
            "fields a b\nexpect accepted\nvalue a x\ninput\n",
            "a field with no line `value`",
        ),
        (
            "fields a\nexpect accepted\nvalue b x\ninput\n",
            "no line `value a`",
        ),
        (
            "fields a\nexpect refused\nvalue a x\ninput\n",
            "a line the case's form has no place for",
        ),
    ] {
        let found = case(text).expect_err(text);
        assert!(found.contains(why), "{:?}: {}", text, found);
    }
    assert_eq!(
        case("fields a b\nexpect accepted\nvalue a x y\nvalue b\ninput\n---{CR}\n{TAB}\n"),
        Ok((
            vec!["a".to_string(), "b".to_string()],
            Some(vec!["x y".to_string(), String::new()]),
            "---\r\n\t\n".to_string()
        ))
    );
}
