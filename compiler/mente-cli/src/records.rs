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
/// letters and digits, the first a letter, joined by single hyphens, `guard-per-problem` or
/// `bench-retired-misplaced-paren`.
pub fn is_key(key: &str) -> bool {
    key.split('-').enumerate().all(|(k, word)| {
        !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            && (k > 0 || word.starts_with(|c: char| c.is_ascii_lowercase()))
    })
}

/// The values of a record file's front matter, which names exactly `fields`, in that order, one
/// `name: value` line each, between a first line `---` and the next. Each value is a string a YAML
/// reader reads as this one does, so that the file renders where YAML is read: a plain value, which
/// starts with no YAML indicator and holds no `: ` and no ` #`, or a value in single quotes, where
/// `''` stands for one quote. Line ends may be CRLF. What follows the front matter is the record's
/// prose, free.
pub fn front_matter(text: &str, fields: &[&str]) -> Result<Vec<String>, String> {
    let text = text.replace("\r\n", "\n");
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
                "the value of `{}` is no plain value and no value in single quotes: quote it, `'...'`, with `''` for a quote",
                name
            ))
        })
        .collect()
}

/// The string a YAML reader reads in `value`, if it reads one this reader agrees on.
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
        && !value.ends_with(':');
    plain.then(|| value.to_string())
}
