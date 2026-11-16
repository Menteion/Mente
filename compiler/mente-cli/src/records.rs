//! The forms the project's records are written in, defined once and read by every reader of a
//! record (Norman, 2026-11-17): the design register's tests (`tests/register.rs`), the record of
//! escaped bugs (`escapes.rs`, `tests/escapes.rs`, `mn escapes`), and the forms' own tests
//! (`tests/records.rs`), which also hold that no document names a record by a number.

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
