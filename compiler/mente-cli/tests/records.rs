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

/// A name that is no key is told which part of the rule it breaks, in the words of the rule's one
/// definition, and how to rename it where a key comes of it: a number up to twenty, or its
/// ordinal, spelled in letters; a numbering, such as `esc-001`, never spelled, but the record named
/// by what it records (the review of PR 175).
#[test]
fn test_a_refused_name_says_which_part_of_the_rule_it_breaks() {
    let asks = |part: usize| format!("a key is {}", records::KEY_PARTS[part].asks);
    for (name, parts) in [
        ("fix-2nd-pass", vec![1]),
        ("esc-001", vec![1]),
        ("Guard_per", vec![0, 1]),
        ("guard--per", vec![0, 1]),
        ("con", vec![3]),
        ("readme", vec![4]),
    ] {
        let expected: Vec<String> = parts.iter().map(|&p| asks(p)).collect();
        assert_eq!(records::key_problems(name), expected, "{}", name);
    }
    let long = "a".repeat(records::KEY_LENGTH + 1);
    assert_eq!(records::key_problems(&long), [asks(2)]);
    assert!(records::KEY_PARTS[1]
        .asks
        .contains("starting with a letter"));
    for (name, suggestion) in [
        ("fix-2nd-pass", Some("fix-second-pass")),
        ("Fix_2nd Pass", Some("fix-second-pass")),
        ("pick-2", Some("pick-two")),
        ("pick-0", Some("pick-zero")),
        ("ring-20th", Some("ring-twentieth")),
        ("esc-001", None),
        ("pick-21", None),
        ("pick-2x", None),
        ("con", None),
    ] {
        assert_eq!(
            records::suggest_key(name).as_deref(),
            suggestion,
            "{}",
            name
        );
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("kinds")
        .join("messages");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for name in ["fix-2nd-pass.md", "esc-001.md", "walker-hid.txt"] {
        fs::write(dir.join(name), "").unwrap();
    }
    let found = records::entries(&records::ESCAPES, &dir).expect_err("three refusals");
    assert_eq!(
        found,
        [
            format!(
                "esc-001.md: not an escaped bug, since its name is no key: {}; name it by what it records, in lowercase words joined by hyphens",
                asks(1)
            ),
            format!(
                "fix-2nd-pass.md: not an escaped bug, since its name is no key: {}; rename it `fix-second-pass.md`",
                asks(1)
            ),
            "walker-hid.txt: not an escaped bug, a file `<key>.md`; rename it `walker-hid.md`"
                .to_string(),
        ]
    );
    let missing = records::entries(&records::ESCAPES, &dir.join("gone")).expect_err("no directory");
    assert!(
        missing[0].contains("keeps its records in this directory, which is to exist"),
        "{:?}",
        missing
    );
}

/// Every part gives a word of its own, and every word is a key's: checked over every part of up to
/// three characters drawn from letters on either side of `z`, `z`, digits, a capital, `_` and a
/// letter beyond ASCII, and on the spellings the doc gives.
#[test]
fn test_a_key_word_stands_for_one_part_alone() {
    let alphabet = ['a', 'y', 'z', '0', '9', 'A', '_', 'é'];
    let mut parts = vec![String::new()];
    for length in 1..=3 {
        let shorter: Vec<String> = parts
            .iter()
            .filter(|p| p.chars().count() == length - 1)
            .cloned()
            .collect();
        for p in shorter {
            for c in alphabet {
                parts.push(format!("{}{}", p, c));
            }
        }
    }
    let mut seen = std::collections::BTreeMap::new();
    for part in &parts {
        let word = records::key_word(part);
        assert!(
            (records::KEY_PARTS[0].holds)(&word) && (records::KEY_PARTS[1].holds)(&word),
            "{:?} gives {:?}",
            part,
            word
        );
        if let Some(other) = seen.insert(word.clone(), part.clone()) {
            panic!("{:?} and {:?} both give {:?}", other, part, word);
        }
    }
    for (part, word) in [
        ("pick", "pick"),
        ("zero", "zzero"),
        ("4", "z4"),
        ("01", "z01"),
        ("", "zh"),
        ("Foo", "zh466f6f"),
        ("a_b", "zh615f62"),
    ] {
        assert_eq!(records::key_word(part), word, "{:?}", part);
    }
    assert!(
        seen.keys().all(|word| word != "z"),
        "a part gives the word z"
    );
    assert_eq!(
        records::key_of(&[&["array", "beyond", "4", "gib"]]).as_deref(),
        Some("array-beyond-z4-gib")
    );
    let ab_c: &[&[&str]] = &[&["a", "b"], &["c"]];
    let a_bc: &[&[&str]] = &[&["a"], &["b", "c"]];
    assert_eq!(records::key_of(ab_c).as_deref(), Some("a-b-z-c"));
    assert_eq!(records::key_of(a_bc).as_deref(), Some("a-z-b-c"));
    assert_eq!(records::key_of(&[&["con"]]), None);
    assert_eq!(records::key_of(&[&["a"; 51]]), None);
}

/// A kind holds its records' files themselves: a link in its directory is refused, never read as
/// what it points to. Windows lets only an administrator make a link, so the Linux runners hold
/// this case.
#[cfg(unix)]
#[test]
fn test_a_link_in_a_kind_is_refused() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("kinds")
        .join("link");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("elsewhere")).unwrap();
    fs::write(dir.join("elsewhere/x.md"), "").unwrap();
    let kind = dir.join("briefs");
    fs::create_dir_all(&kind).unwrap();
    std::os::unix::fs::symlink(dir.join("elsewhere/x.md"), kind.join("linked-brief.md")).unwrap();
    std::os::unix::fs::symlink(dir.join("elsewhere"), kind.join("archive")).unwrap();
    let found = records::entries(&records::BRIEFS, &kind).expect_err("two links");
    assert_eq!(found.len(), 2, "{:?}", found);
    assert!(
        found.iter().all(|p| p.contains("but a link")),
        "{:?}",
        found
    );
}

/// Every Markdown file of the tree belongs to exactly one declaration: a kind of record, a shared
/// file or a set of documents, each with its reason; every exact pattern names a file that exists.
#[test]
fn test_every_markdown_file_is_declared_once() {
    let problems =
        records::undeclared(&root(), records::KINDS, records::SHARED, records::DOCUMENTS);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    for set in records::DOCUMENTS {
        assert!(!set.patterns.is_empty() && !set.what.is_empty() && !set.why.is_empty());
        for pattern in set.patterns.iter().filter(|p| !p.contains('*')) {
            assert!(
                root().join(pattern).is_file(),
                "{}: declared among {}, but no such file",
                pattern,
                set.what
            );
        }
    }
}

/// A Markdown file nothing declares is refused with what to do, and so is one two declarations
/// claim; a nested repository, `target` and `node_modules` are not read. A record of a new kind in
/// a directory nothing declares, as the review of PR 175 planted, is refused by the real
/// declarations.
#[test]
fn test_a_markdown_file_nothing_declares_is_refused() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("kinds")
        .join("closed");
    let _ = fs::remove_dir_all(&dir);
    for file in [
        "README.md",
        "Upper.MD",
        "docs/adr/0001-use-keys.md",
        "docs/log.md",
        "docs/page.md",
        "notes/a-note.md",
        "nested/.git/HEAD",
        "nested/x.md",
        "target/y.md",
        "node_modules/z.md",
        "notes.txt",
    ] {
        let path = dir.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
    let kinds = [records::Kind {
        record: "a note",
        dir: "notes",
        extension: "md",
        archive: None,
    }];
    let shared = [records::Shared {
        file: "docs/log.md",
        records: "the log",
        why: "one file",
    }];
    let documents = [records::Documents {
        patterns: &["README.md", "docs/*.md"],
        what: "pages",
        why: "pages",
    }];
    let found = records::undeclared(&dir, &kinds, &shared, &documents);
    let nothing = "a Markdown file nothing declares: declare its directory in records.rs as a kind of record, whose files are then named by keys (`KINDS`), or as a set of documents with why they are none (`DOCUMENTS`)";
    assert_eq!(
        found,
        [
            format!("Upper.MD: {}", nothing),
            format!("docs/adr/0001-use-keys.md: {}", nothing),
            "docs/log.md: claimed by the shared file of the log and the documents of pages, where one declaration is to claim it".to_string(),
        ]
    );
    let adr = dir.join("adr");
    fs::create_dir_all(adr.join("docs/adr")).unwrap();
    fs::write(adr.join("docs/adr/0001-use-keys.md"), "").unwrap();
    let real = records::undeclared(&adr, records::KINDS, records::SHARED, records::DOCUMENTS);
    assert_eq!(real, [format!("docs/adr/0001-use-keys.md: {}", nothing)]);
}

/// A pattern matches name by name: `*` within a name, `**` across any number of directories.
#[test]
fn test_a_pattern_matches_name_by_name() {
    for (pattern, file, matches) in [
        ("README.md", "README.md", true),
        ("README.md", "docs/README.md", false),
        ("spec/*.md", "spec/grammar.md", true),
        ("spec/*.md", "spec/sub/grammar.md", false),
        ("spikes/*/README.md", "spikes/tier1-latency/README.md", true),
        ("spikes/*/README.md", "spikes/README.md", false),
        (
            "benchmark/prelim/**/*.md",
            "benchmark/prelim/m1/a.reply.md",
            true,
        ),
        ("benchmark/prelim/**/*.md", "benchmark/prelim/a.md", true),
        ("benchmark/prelim/**/*.md", "benchmark/probe/a.md", false),
        ("a*b*c.md", "axxbyyc.md", true),
        ("a*b*c.md", "axxcyyb.md", false),
        ("*.md", "x.txt", false),
    ] {
        assert_eq!(
            records::matches_pattern(pattern, file),
            matches,
            "{} {}",
            pattern,
            file
        );
    }
}

/// The id a brief's front matter gives it, which spec-brief reads (`.spec-brief.json`).
fn brief_id(text: &str) -> Option<String> {
    let text = text.replace("\r\n", "\n");
    let front = text.strip_prefix("---\n")?.split_once("\n---")?.0;
    front
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .map(|id| id.trim().to_string())
}

/// Every kind of record is a directory of files named by their keys, and nothing else; a brief's
/// id, which spec-brief reads from its front matter, is its file's key; and spec-brief is
/// configured to read the briefs so.
#[test]
fn test_every_kind_is_a_directory_of_files_named_by_a_key() {
    for kind in records::KINDS {
        let found = records::entries(kind, &root().join(kind.dir))
            .unwrap_or_else(|p| panic!("{}:\n{}", kind.dir, p.join("\n")));
        if *kind == records::BRIEFS {
            for (key, path) in found {
                let text = fs::read_to_string(&path).unwrap();
                let id = brief_id(&text);
                assert_eq!(
                    id.as_deref(),
                    Some(key.as_str()),
                    "{}: a brief's id is its file's key: rename the file {}/{}.md, since spec-brief new names it after the title (records::BRIEFS)",
                    path.display(),
                    records::BRIEFS.dir,
                    id.as_deref().unwrap_or("<its id>")
                );
            }
        }
    }
    let config = fs::read_to_string(root().join(".spec-brief.json")).unwrap();
    for line in [
        format!("\"briefs\": \"{}\",", records::BRIEFS.dir),
        format!(
            "\"archive\": \"{}/{}\",",
            records::BRIEFS.dir,
            records::BRIEFS.archive.unwrap()
        ),
        format!("\"files\": \"*.{}\",", records::BRIEFS.extension),
        "\"source\": \"frontmatter\"".to_string(),
    ] {
        assert!(
            config.contains(&line),
            ".spec-brief.json does not say {}",
            line
        );
    }
}

/// Every file that still holds many records exists and says why it is not yet a kind, and lies
/// in no kind's directory; no two kinds share a directory.
#[test]
fn test_every_shared_record_file_exists_and_says_why() {
    for shared in records::SHARED {
        assert!(
            root().join(shared.file).is_file(),
            "{}: no file; a file that became a kind leaves records::SHARED",
            shared.file
        );
        assert!(!shared.records.is_empty() && !shared.why.is_empty());
        for kind in records::KINDS {
            assert!(
                !shared.file.starts_with(&format!("{}/", kind.dir)),
                "{} lies in the directory of {}",
                shared.file,
                kind.record
            );
        }
    }
    let mut dirs: Vec<&str> = records::KINDS.iter().map(|k| k.dir).collect();
    dirs.sort_unstable();
    dirs.dedup();
    assert_eq!(
        dirs.len(),
        records::KINDS.len(),
        "two kinds share a directory"
    );
}

/// A kind's directory is refused for each entry that is no record: a name that is no key, a
/// number among them, a name Windows reserves, a file of another extension, a subdirectory but
/// the archive, one inside the archive, and a key both the directory and its archive hold. A
/// `.gitkeep` is no record and no problem, and the archive's records are the kind's.
#[test]
fn test_a_kind_refuses_each_entry_that_is_no_record() {
    let briefs = records::BRIEFS;
    let escapes = records::ESCAPES;
    let planted = |name: &str, files: &[&str]| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("kinds")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for path in files {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "---\nid: x\n---\n").unwrap();
        }
        dir
    };
    let good = planted(
        "good",
        &[
            ".gitkeep",
            "rotate-tokens.md",
            "archive/.gitkeep",
            "split-the-judge.md",
        ],
    );
    let keys: Vec<String> = records::entries(&briefs, &good)
        .unwrap()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(keys, ["rotate-tokens", "split-the-judge"]);
    let archived = planted(
        "archived",
        &["rotate-tokens.md", "archive/split-the-judge.md"],
    );
    assert_eq!(records::entries(&briefs, &archived).unwrap().len(), 2);
    for (name, kind, path, problem) in [
        (
            "numbered",
            briefs,
            "001_rotate-tokens.md",
            "001_rotate-tokens.md: not a task brief",
        ),
        (
            "number",
            escapes,
            "ESC-001.md",
            "ESC-001.md: not an escaped bug",
        ),
        ("reserved", escapes, "con.md", "con.md: not an escaped bug"),
        (
            "reserved-port",
            briefs,
            "archive/lpt1.md",
            "archive/lpt1.md: not a task brief",
        ),
        (
            "extension",
            escapes,
            "walker-hid.txt",
            "walker-hid.txt: not an escaped bug",
        ),
        (
            "subdirectory",
            briefs,
            "old/rotate.md",
            "old: not a task brief",
        ),
        (
            "no-archive",
            escapes,
            "archive/walker-hid.md",
            "archive: not an escaped bug",
        ),
        (
            "nested",
            briefs,
            "archive/archive/x.md",
            "archive/archive: not a task brief",
        ),
        (
            "twice",
            briefs,
            "archive/rotate-tokens.md",
            "rotate-tokens: a key two files hold",
        ),
    ] {
        let dir = planted(name, &["rotate-tokens.md", path]);
        let found = records::entries(&kind, &dir).expect_err(name);
        assert_eq!(found.len(), 1, "{}: {:?}", name, found);
        assert!(found[0].starts_with(problem), "{}: {:?}", name, found);
    }
    assert_eq!(
        brief_id("---\nstatus: draft\nid: rotate-tokens\n---\n# R\n").as_deref(),
        Some("rotate-tokens")
    );
    assert_eq!(
        brief_id("---\r\nid: rotate-tokens\r\n---\r\n").as_deref(),
        Some("rotate-tokens")
    );
    assert_eq!(brief_id("# R\n\nid: rotate-tokens\n"), None);
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

/// Unicode's property Extended_Pictographic, as ranges of code points, from `emoji-data.txt` of
/// Unicode 16.0 (https://www.unicode.org/Public/16.0.0/ucd/emoji/emoji-data.txt), adjacent lines
/// merged: every character Unicode deems pictographic, emoji and those that may become emoji, so
/// that no hand-picked range lets one through.
const EXTENDED_PICTOGRAPHIC: &[(u32, u32)] = &[
    (0xA9, 0xA9),
    (0xAE, 0xAE),
    (0x203C, 0x203C),
    (0x2049, 0x2049),
    (0x2122, 0x2122),
    (0x2139, 0x2139),
    (0x2194, 0x2199),
    (0x21A9, 0x21AA),
    (0x231A, 0x231B),
    (0x2328, 0x2328),
    (0x2388, 0x2388),
    (0x23CF, 0x23CF),
    (0x23E9, 0x23F3),
    (0x23F8, 0x23FA),
    (0x24C2, 0x24C2),
    (0x25AA, 0x25AB),
    (0x25B6, 0x25B6),
    (0x25C0, 0x25C0),
    (0x25FB, 0x25FE),
    (0x2600, 0x2605),
    (0x2607, 0x2612),
    (0x2614, 0x2685),
    (0x2690, 0x2705),
    (0x2708, 0x2712),
    (0x2714, 0x2714),
    (0x2716, 0x2716),
    (0x271D, 0x271D),
    (0x2721, 0x2721),
    (0x2728, 0x2728),
    (0x2733, 0x2734),
    (0x2744, 0x2744),
    (0x2747, 0x2747),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2763, 0x2767),
    (0x2795, 0x2797),
    (0x27A1, 0x27A1),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2934, 0x2935),
    (0x2B05, 0x2B07),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x3030, 0x3030),
    (0x303D, 0x303D),
    (0x3297, 0x3297),
    (0x3299, 0x3299),
    (0x1F000, 0x1F0FF),
    (0x1F10D, 0x1F10F),
    (0x1F12F, 0x1F12F),
    (0x1F16C, 0x1F171),
    (0x1F17E, 0x1F17F),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F1AD, 0x1F1E5),
    (0x1F201, 0x1F20F),
    (0x1F21A, 0x1F21A),
    (0x1F22F, 0x1F22F),
    (0x1F232, 0x1F23A),
    (0x1F23C, 0x1F23F),
    (0x1F249, 0x1F3FA),
    (0x1F400, 0x1F53D),
    (0x1F546, 0x1F64F),
    (0x1F680, 0x1F6FF),
    (0x1F774, 0x1F77F),
    (0x1F7D5, 0x1F7FF),
    (0x1F80C, 0x1F80F),
    (0x1F848, 0x1F84F),
    (0x1F85A, 0x1F85F),
    (0x1F888, 0x1F88F),
    (0x1F8AE, 0x1F8FF),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1FAFF),
    (0x1FC00, 0x1FFFD),
];

/// The characters that make a pictograph of what Extended_Pictographic leaves out: the selector
/// that asks for a character's emoji form, U+FE0F, the combining keycap that makes a keycap of a
/// digit or a sign, U+20E3, and the regional indicators whose pairs are flags, U+1F1E6 to U+1F1FF.
const EMOJI_MAKERS: &[(u32, u32)] = &[(0xFE0F, 0xFE0F), (0x20E3, 0x20E3), (0x1F1E6, 0x1F1FF)];

/// Whether `c` is or makes a pictograph: Unicode's property Extended_Pictographic, or one of
/// [`EMOJI_MAKERS`].
fn pictograph(c: char) -> bool {
    let c = u32::from(c);
    EMOJI_MAKERS.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
        || EXTENDED_PICTOGRAPHIC
            .binary_search_by(|&(lo, hi)| {
                if hi < c {
                    std::cmp::Ordering::Less
                } else if lo > c {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
}

/// Each way the specification's directory breaks its layout (CLAUDE.md, invariant 1): an entry
/// that is no file, which a subdirectory is; a file that is not `README.md` and is no `<key>.md`,
/// so no numeric prefix, `00_core.md`, and no other kind of file; a line that holds a pictograph,
/// which a decorative warning header is, by its file and line; and, so that nothing passes unread,
/// an entry or a file that cannot be read and a spec that is not UTF-8 text.
fn spec_layout(dir: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => return vec![format!("{}: {}", dir.display(), e)],
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                problems.push(format!(
                    "{}: an entry that cannot be read: {}",
                    dir.display(),
                    e
                ));
                continue;
            }
        };
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
        let text = match fs::read(&path).map(String::from_utf8) {
            Ok(Ok(text)) => text,
            Ok(Err(_)) => {
                problems.push(format!("{}: a spec that is not UTF-8 text", name));
                continue;
            }
            Err(e) => {
                problems.push(format!("{}: {}", name, e));
                continue;
            }
        };
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
    // The characters a hand-picked set let through, each pictographic by Unicode's property, and
    // the keycaps and flags the property leaves out.
    for heading in [
        "# A \u{23F3} B\n",
        "# A \u{231B} B\n",
        "# A \u{23F0} B\n",
        "# A \u{25B6} B\n",
        "# A \u{2139} B\n",
        "# A \u{2194} B\n",
        "# A \u{A9} B\n",
        "## 1\u{FE0F}\u{20E3} One\n",
        "## #\u{FE0F}\u{20E3} Sign\n",
        "## 1\u{20E3} Keycap\n",
        "## \u{1F1FA}\u{1F1F8} Flag\n",
    ] {
        let dir = planted("property", &[("README.md", good), ("regions.md", heading)]);
        let found = spec_layout(&dir);
        assert_eq!(found.len(), 1, "{:?}: {:?}", heading, found);
        assert!(
            found[0].starts_with("regions.md:1: a pictograph"),
            "{:?}: {:?}",
            heading,
            found
        );
    }
    // A spec that cannot be read as UTF-8, UTF-16 with a warning sign in it, is reported, never
    // passed unread.
    let dir = planted("utf16", &[("README.md", good)]);
    let utf16: Vec<u8> = "# \u{26A0} Warning\n"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    fs::write(dir.join("regions.md"), [&[0xFF, 0xFE][..], &utf16].concat()).unwrap();
    assert_eq!(
        spec_layout(&dir),
        ["regions.md: a spec that is not UTF-8 text"]
    );
    let arrows = planted("arrows", &[("README.md", "# A \u{2192} B, x \u{2264} y\n")]);
    assert_eq!(spec_layout(&arrows), Vec::<String>::new());
}
