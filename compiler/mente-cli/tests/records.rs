//! The forms the project's records are written in (`src/records.rs`). No record is named by a
//! number: CLAUDE.md's invariant 1 forbids `DEC-XXX` numbers in the specs, and Norman extended it
//! on 2026-11-17 to every kind of record. A number carries no meaning, needs an allocation that
//! races between changes in flight, and suggests an order the records do not have; a key, as the
//! register's entries have, does none of that. The specification's directory is held to the same
//! form: files named by a key, with no numeric prefix, no subdirectory and no pictograph.
//!
//! The rule is held by structure (Norman decided, 2026-11-17): a record's name is its file's name
//! or its register entry's key, and the one key rule, `records::is_key`, refuses a number there; a
//! citation of a record is a link whose anchor or file is such a key, which the link checks read.
//! A number written in prose, `ESC-001` in a sentence, is held by review only: a scanner of the
//! documents' text was retired, since reading spelling leaked round after round.

use mente_cli::records;

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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
        "readme",
    ] {
        assert!(!records::is_key(key), "{}", key);
    }
    let longest = format!("a{}", "b".repeat(records::KEY_LENGTH - 1));
    assert!(records::is_key(&longest));
    assert!(!records::is_key(&format!("{}c", longest)));
    assert!(records::is_key("readme-of-escapes"));
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
    Ok((fields, values, expand(input)?))
}

/// A case's input with its spellings expanded: `{CR}`, `{TAB}` and `{U+hhhh}`, four to six
/// hexadecimal digits in capitals naming a code point.
fn expand(input: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = input.replace("{CR}", "\r").replace("{TAB}", "\t");
    while let Some(at) = rest.find("{U+") {
        out.push_str(&rest[..at]);
        let close = rest[at..].find('}').ok_or("a `{U+` that does not close")?;
        let hex = &rest[at + 3..at + close];
        let ok = (4..=6).contains(&hex.len())
            && hex
                .chars()
                .all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c));
        let c = ok
            .then(|| u32::from_str_radix(hex, 16).ok().and_then(char::from_u32))
            .flatten()
            .ok_or_else(|| format!("{{U+{}}}: no code point in the case's spelling", hex))?;
        out.push(c);
        rest = rest[at + close + 1..].to_string();
    }
    out.push_str(&rest);
    Ok(out)
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
    assert_eq!(
        expand("a{U+0085}b{U+2028}{U+10FFFF}"),
        Ok("a\u{85}b\u{2028}\u{10FFFF}".to_string())
    );
    for bad in [
        "{U+85}",
        "{U+00e9}",
        "{U+1100000}",
        "{U+110000}",
        "{U+D800}",
        "{U+0085",
    ] {
        assert!(expand(bad).is_err(), "{}", bad);
    }
}

/// Whether `c` is a pictograph: an emoji, a symbol of the emoji blocks, a dingbat, or the selector
/// that asks for a character's emoji form.
fn pictograph(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE0F
    )
}

/// Each way the specification's directory breaks its layout (CLAUDE.md, invariant 1): an entry
/// that is no file, which a subdirectory is; a file that is not `README.md` and is no `<key>.md`,
/// so no numeric prefix, `00_core.md`, and no other kind of file; and a line that holds a
/// pictograph, which a decorative warning header is, by its file and line.
fn spec_layout(dir: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return vec![format!("{}: no directory", dir.display())];
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if !path.is_file() {
            problems.push(format!(
                "{}: a subdirectory, which spec/ holds none of",
                name
            ));
            continue;
        }
        let keyed = name.strip_suffix(".md").is_some_and(records::is_key);
        if name != "README.md" && !keyed {
            problems.push(format!(
                "{}: no spec, which is a file `<key>.md` whose key is lowercase words joined by hyphens, with no number before it",
                name
            ));
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            if line.chars().any(pictograph) {
                problems.push(format!(
                    "{}:{}: a pictograph, which no spec holds",
                    name,
                    n + 1
                ));
            }
        }
    }
    problems
}

/// The specification's directory holds only `README.md` and files named by a key, with no
/// subdirectory, no numeric prefix and no pictograph (CLAUDE.md, invariant 1).
#[test]
fn test_the_specs_are_files_named_by_a_key_with_no_pictograph() {
    let problems = spec_layout(&root().join("spec"));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The layout refuses each way a spec breaks it, and accepts a spec of the right shape.
#[test]
fn test_the_spec_layout_refuses_each_way_a_spec_breaks_it() {
    let good = "# Regions\n\nA region holds its values.\n";
    let planted = |name: &str, files: &[(&str, &str)]| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("spec-layout")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (path, text) in files {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    };
    let dir = planted(
        "good",
        &[("README.md", good), ("regions-and-pools.md", good)],
    );
    assert_eq!(spec_layout(&dir), Vec::<String>::new());
    for (name, path, text, problem) in [
        ("prefix", "00_core.md", good, "00_core.md: no spec"),
        ("digits", "00-core.md", good, "00-core.md: no spec"),
        ("other-kind", "regions.txt", good, "regions.txt: no spec"),
        (
            "subdirectory",
            "old/regions.md",
            good,
            "old: a subdirectory",
        ),
        (
            "warning",
            "regions.md",
            "# \u{26A0}\u{FE0F} Warning\n",
            "regions.md:1: a pictograph",
        ),
        (
            "emoji",
            "regions.md",
            "# Regions\n\nFast \u{1F680}\n",
            "regions.md:3: a pictograph",
        ),
        (
            "star",
            "regions.md",
            "## \u{2B50} Note\n",
            "regions.md:1: a pictograph",
        ),
    ] {
        let dir = planted(name, &[("README.md", good), (path, text)]);
        let found = spec_layout(&dir);
        assert_eq!(found.len(), 1, "{}: {:?}", name, found);
        assert!(found[0].starts_with(problem), "{}: {:?}", name, found);
    }
    let arrows = planted("arrows", &[("README.md", "# A \u{2192} B, x \u{2264} y\n")]);
    assert_eq!(spec_layout(&arrows), Vec::<String>::new());
}
