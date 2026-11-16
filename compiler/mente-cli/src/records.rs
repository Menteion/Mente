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
/// No word is digits alone, so that no key is a number in a key's dress, `esc-001` or `adr-0258`,
/// and the first starts with a letter. Since a key names a file, it is no name Windows reserves.
pub fn is_key(key: &str) -> bool {
    !reserved(key)
        && key.split('-').enumerate().all(|(k, word)| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && word.contains(|c: char| c.is_ascii_lowercase())
                && (k > 0 || word.starts_with(|c: char| c.is_ascii_lowercase()))
        })
}

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
/// a null, a boolean or a number; or a value in single quotes, where `''` stands for one quote. No
/// tab and no carriage return stands anywhere in the file, since YAML reads either otherwise than
/// as written. What follows the front matter is the record's prose, free.
///
/// The cases of `src/front-matter/` hold this behaviour as data, one file a case, so that every
/// reader of the form is tested against the same cases (`tests/records.rs`); the `README.md` there
/// gives a case's form.
pub fn front_matter(text: &str, fields: &[&str]) -> Result<Vec<String>, String> {
    if text.contains(['\t', '\r']) {
        return Err(
            "a tab or a carriage return, which YAML reads otherwise than as written".to_string(),
        );
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
            scalar(value).ok_or(format!(
                "the value of `{}` is no plain string and no value in single quotes: quote it, `'...'`, with `''` for a quote",
                name
            ))
        })
        .collect()
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
