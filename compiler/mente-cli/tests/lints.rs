//! The lints that keep the walkers of the compiler naming every variant of the enums they
//! match ([`guard-per-problem`]): `clippy::wildcard_enum_match_arm`, denied at the root of each
//! crate that walks the syntax tree, the types or the values, and a scan for what that lint
//! does not see. An integration test of its own, apart from main.rs, so that nothing a feature
//! changes there unhooks it.
//!
//! [`guard-per-problem`]: ../../../docs/design-review.md#guard-per-problem

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The crates whose matches name every variant: a `_` arm let walkers skip expression kinds,
/// so each denies `clippy::wildcard_enum_match_arm` at its root.
const EXHAUSTIVE_CRATES: [&str; 4] = ["mente-sema", "mente-interp", "mente-mir", "mente-codegen"];

const DENY_WILDCARD_ARMS: &str = "#![deny(clippy::wildcard_enum_match_arm)]";

/// The compiler's crates, beside this one.
fn compiler() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Every `.rs` file under `dir`, by its path from the compiler's directory, with its text.
fn sources(dir: &Path) -> Vec<(String, String)> {
    let base = compiler();
    let mut out = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for entry in fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let name = path
                    .strip_prefix(&base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((name, fs::read_to_string(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// What keeps a crate's matches from naming every variant: its root without the deny, or any
/// source that allows the lint, which an `#[expect]` with its reason must replace, since an
/// `expect` that nothing triggers fails the build and an `allow` does not.
fn wildcard_arm_violations(krate: &str, root: &str, sources: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    let inner: Vec<&str> = root
        .lines()
        .map(str::trim)
        .take_while(|l| l.is_empty() || l.starts_with("//") || l.starts_with("#!["))
        .collect();
    if !inner.contains(&DENY_WILDCARD_ARMS) {
        found.push(format!(
            "{}: src/lib.rs lacks `{}`",
            krate, DENY_WILDCARD_ARMS
        ));
    }
    for (path, text) in sources {
        for (n, line) in code_only(text).lines().enumerate() {
            if line.contains("allow(clippy::wildcard_enum_match_arm") {
                found.push(format!(
                    "{}:{}: allows the lint; write `#[expect(..., reason = \"...\")]`",
                    path,
                    n + 1
                ));
            }
        }
    }
    found
}

#[test]
fn test_exhaustive_crates_deny_wildcard_arms() {
    let mut found = Vec::new();
    for krate in EXHAUSTIVE_CRATES {
        let src = compiler().join(krate).join("src");
        let root = fs::read_to_string(src.join("lib.rs")).unwrap();
        found.extend(wildcard_arm_violations(krate, &root, &sources(&src)));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
    // The check itself: a root without the deny, the deny after the first item, and an `allow`
    // are each found; a root with it, an `expect` and a comment naming `allow` are not.
    let with = "//! Docs.\n\n// Why.\n#![deny(clippy::wildcard_enum_match_arm)]\n\npub mod a;\n";
    let without = "//! Docs.\n\npub mod a;\n";
    let late = "//! Docs.\npub mod a;\n#![deny(clippy::wildcard_enum_match_arm)]\n";
    let allowed = vec![(
        "a.rs".to_string(),
        "#[allow(clippy::wildcard_enum_match_arm)]\nfn f() {}\n".to_string(),
    )];
    let expected = vec![(
        "a.rs".to_string(),
        "#[expect(clippy::wildcard_enum_match_arm, reason = \"r\")]\n// no allow(clippy::wildcard_enum_match_arm) here\nfn f() {}\n".to_string(),
    )];
    assert!(wildcard_arm_violations("k", with, &expected).is_empty());
    assert_eq!(wildcard_arm_violations("k", without, &[]).len(), 1);
    assert_eq!(wildcard_arm_violations("k", late, &[]).len(), 1);
    assert_eq!(wildcard_arm_violations("k", with, &allowed).len(), 1);
}

// -------------------------------------------------------------------------------------------
// What the lint does not see
// -------------------------------------------------------------------------------------------

/// `src` with its comments, strings and characters blanked to spaces, line for line, so that
/// only code is scanned and a line number still points at the source.
fn code_only(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < b.len() && b[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(b[i]));
                    i += 1;
                }
            }
        } else if c == 'r' && (next == Some('"') || next == Some('#')) && raw_start(&b, i) {
            // A raw string: `r"..."` or `r#"..."#`, with as many `#` at its end.
            let mut j = i + 1;
            let mut hashes = 0;
            while b.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            out.push_str(&" ".repeat(j - i + 1));
            j += 1;
            loop {
                if j >= b.len() {
                    break;
                }
                if b[j] == '"' && (0..hashes).all(|k| b.get(j + 1 + k) == Some(&'#')) {
                    out.push_str(&" ".repeat(1 + hashes));
                    j += 1 + hashes;
                    break;
                }
                out.push(blank(b[j]));
                j += 1;
            }
            i = j;
        } else if c == '"' {
            out.push(' ');
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                if i < b.len() {
                    out.push(blank(b[i]));
                    i += 1;
                }
            }
            out.push(' ');
            i += 1;
        } else if c == '\'' && is_char_literal(&b, i) {
            out.push(' ');
            i += 1;
            while i < b.len() && b[i] != '\'' {
                if b[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                out.push(' ');
                i += 1;
            }
            out.push(' ');
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Whether the `r` at `i` starts a raw string rather than ending a name.
fn raw_start(b: &[char], i: usize) -> bool {
    let before = i.checked_sub(1).map(|k| b[k]);
    !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && {
        let mut j = i + 1;
        while b.get(j) == Some(&'#') {
            j += 1;
        }
        b.get(j) == Some(&'"')
    }
}

/// Whether the `'` at `i` opens a character literal, not a lifetime: `'x'`, `'\n'`.
fn is_char_literal(b: &[char], i: usize) -> bool {
    matches!(
        (b.get(i + 1), b.get(i + 2)),
        (Some('\\'), _) | (Some(_), Some('\''))
    )
}

/// The enums the exhaustive crates match: every enum declared in them or in `mente-syntax`,
/// and the names `use ... as` or `type ... =` gives one of them, or one of those names.
fn enum_names(files: &[(String, String)]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (_, text) in files {
        let code = code_only(text);
        let words: Vec<&str> = code
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .collect();
        for w in words.windows(2) {
            if w[0] == "enum" && w[1].starts_with(|c: char| c.is_ascii_uppercase()) {
                names.insert(w[1].to_string());
            }
        }
    }
    let pairs: Vec<(String, String)> = files
        .iter()
        .flat_map(|(_, text)| {
            let code = code_only(text);
            let mut pairs = aliases(&code);
            pairs.extend(type_aliases(&code));
            pairs
        })
        .collect();
    loop {
        let before = names.len();
        for (original, alias) in &pairs {
            if names.contains(original) {
                names.insert(alias.clone());
            }
        }
        if names.len() == before {
            return names;
        }
    }
}

/// Each `type Y = path::X;` of `code`, as `(X, Y)`: the last segment of the path, without its
/// generic arguments. A `type` without `=`, as a trait declares one, gives none.
fn type_aliases(code: &str) -> Vec<(String, String)> {
    let b: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    for i in 0..b.len() {
        if !word_at(&b, i, "type") {
            continue;
        }
        let end = (i..b.len()).find(|&k| b[k] == ';').unwrap_or(b.len());
        let declaration: String = b[i + 4..end].iter().collect();
        let Some((left, right)) = declaration.split_once('=') else {
            continue;
        };
        let alias: String = left
            .trim()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let path = right.trim().split('<').next().unwrap_or("");
        let original = path.rsplit("::").next().unwrap_or("").trim().to_string();
        if !alias.is_empty() && !original.is_empty() {
            out.push((original, alias));
        }
    }
    out
}

/// The bodies of the `impl` blocks of `code` whose type is one of `names`, a trait's included,
/// as ranges of `code`: in them `Self` is that enum.
fn enum_impls(code: &[char], names: &BTreeSet<String>) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 0..code.len() {
        if !word_at(code, i, "impl") {
            continue;
        }
        let Some(open) = (i..code.len()).find(|&k| code[k] == '{' || code[k] == ';') else {
            break;
        };
        if code[open] == ';' {
            continue;
        }
        let header: String = code[i + 4..open].iter().collect();
        // `impl<T> Trait for Type<T>` or `impl<T> Type<T>`: the type follows `for`, or the
        // generic parameters of the `impl`.
        let words: Vec<&str> = header.split_whitespace().collect();
        let ty = match words.iter().rposition(|w| *w == "for") {
            Some(k) => words[k + 1..].join(" "),
            None => {
                let rest = header.trim_start();
                match rest.strip_prefix('<') {
                    Some(r) => {
                        let mut depth = 1;
                        let at = r
                            .char_indices()
                            .find(|&(_, c)| {
                                match c {
                                    '<' => depth += 1,
                                    '>' => depth -= 1,
                                    _ => {}
                                }
                                depth == 0
                            })
                            .map_or(r.len(), |(k, _)| k + 1);
                        r[at..].to_string()
                    }
                    None => rest.to_string(),
                }
            }
        };
        let path = ty.trim().split(['<', ' ']).next().unwrap_or("");
        let name = path.rsplit("::").next().unwrap_or("");
        if names.contains(name) {
            out.push((open, close(code, open)));
        }
    }
    out
}

/// Each `X as Y` of the `use` declarations of `code`, a group's included and one over several
/// lines: `use a::{Int as SemaInt, b::{Ty as SemaTy}};` gives both.
fn aliases(code: &str) -> Vec<(String, String)> {
    let b: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    for i in 0..b.len() {
        if !word_at(&b, i, "use") {
            continue;
        }
        let end = (i..b.len()).find(|&k| b[k] == ';').unwrap_or(b.len());
        let declaration: String = b[i..end].iter().collect();
        let words: Vec<&str> = declaration
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty())
            .collect();
        for w in words.windows(3) {
            if w[1] == "as" {
                out.push((w[0].to_string(), w[2].to_string()));
            }
        }
    }
    out
}

/// Whether `text` names a variant of one of `names`: `Name::Variant`.
fn names_a_variant(text: &str, names: &BTreeSet<String>) -> bool {
    let b: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_uppercase() && (i == 0 || !(b[i - 1].is_alphanumeric() || b[i - 1] == '_'))
        {
            let start = i;
            while i < b.len() && (b[i].is_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            let word: String = b[start..i].iter().collect();
            if names.contains(&word) && b.get(i) == Some(&':') && b.get(i + 1) == Some(&':') {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// The index just past the bracket that closes the one at `open`.
fn close(b: &[char], open: usize) -> usize {
    let mut depth = 0;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

/// Whether the word `w` starts at `i`, on word boundaries.
fn word_at(b: &[char], i: usize, w: &str) -> bool {
    let wc: Vec<char> = w.chars().collect();
    b.get(i..i + wc.len()) == Some(&wc[..])
        && !(i > 0 && (b[i - 1].is_alphanumeric() || b[i - 1] == '_'))
        && !b
            .get(i + wc.len())
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
}

/// `text` split at its top-level `sep`s, outside brackets.
fn split_top(text: &str, sep: char) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if c == sep && depth == 0 {
            parts.push(String::new());
        } else if let Some(last) = parts.last_mut() {
            last.push(c);
        }
    }
    parts.into_iter().map(|p| p.trim().to_string()).collect()
}

/// Whether a pattern matches anything at all there: `_`, `..`, or a bare binding.
fn catch_all(p: &str) -> bool {
    let p = p.trim();
    let p = p.strip_prefix("ref ").unwrap_or(p).trim();
    let p = p.strip_prefix("mut ").unwrap_or(p).trim();
    p == "_"
        || p == ".."
        || (p.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
            && p.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !matches!(p, "true" | "false"))
        || p.split_once(" @ ").is_some_and(|(_, rest)| catch_all(rest))
}

/// Whether an arm's pattern is a fallback, where a variant added later would land unnamed: an
/// alternative that matches anything, or a tuple each of whose elements does, as `(v, _)`. A
/// tuple that names a variant in one place, as `(V::Never, _)`, says the rest does not matter
/// there, and is not one.
fn wildcard_pattern(pattern: &str) -> bool {
    let unguarded = match pattern.find(" if ") {
        Some(k) => &pattern[..k],
        None => pattern,
    };
    split_top(unguarded, '|').iter().any(|alt| {
        catch_all(alt)
            || (alt.starts_with('(')
                && alt.ends_with(')')
                && split_top(&alt[1..alt.len() - 1], ',')
                    .iter()
                    .filter(|e| !e.is_empty())
                    .all(|e| catch_all(e)))
    })
}

/// A place the lint does not see, where a variant of one of the exhaustive crates' enums is
/// left unnamed: its file, its function, its kind and its text, its spacing made one space, by
/// which its entry names it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    file: String,
    function: String,
    kind: &'static str,
    text: String,
    line: usize,
}

/// The function a byte of `code` lies in: the last `fn name` before it.
fn function_at(code: &[char], at: usize) -> String {
    let mut k = at.min(code.len());
    while k > 0 {
        k -= 1;
        if word_at(code, k, "fn") {
            let rest: String = code[k + 2..].iter().take(80).collect();
            return rest
                .trim_start()
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("")
                .to_string();
        }
    }
    String::new()
}

/// `code` with each `#[cfg(test)] mod` blanked: a test asserts what a value is, and walks
/// nothing.
fn without_tests(code: &str) -> Vec<char> {
    let mut b: Vec<char> = code.chars().collect();
    let marker: Vec<char> = "#[cfg(test)]".chars().collect();
    let mut i = 0;
    while i + marker.len() <= b.len() {
        if b[i..i + marker.len()] == marker[..] {
            let mut j = i + marker.len();
            while j < b.len() && b[j].is_whitespace() {
                j += 1;
            }
            if word_at(&b, j, "mod") {
                if let Some(open) = (j..b.len()).find(|&k| b[k] == '{' || b[k] == ';') {
                    if b[open] == '{' {
                        let end = close(&b, open);
                        for c in &mut b[i..end] {
                            if *c != '\n' {
                                *c = ' ';
                            }
                        }
                        i = end;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    b
}

/// `chars` as one line, each run of spaces one space.
fn one_line(chars: &[char]) -> String {
    let text: String = chars.iter().collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The top-level `=` of a `let` that starts at `i`, before any `;`: where its pattern ends.
fn let_equals(code: &[char], i: usize) -> Option<usize> {
    let mut depth = 0;
    let mut j = i + 3;
    while j < code.len() {
        match code[j] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '=' if depth == 0
                && !matches!(code.get(j + 1), Some('=') | Some('>'))
                && !matches!(
                    code.get(j - 1),
                    Some('<') | Some('>') | Some('!') | Some('=')
                ) =>
            {
                return Some(j)
            }
            ';' if depth == 0 => return None,
            _ => {}
        }
        if depth < 0 {
            return None;
        }
        j += 1;
    }
    None
}

/// The sites of one file: `matches!`, `let ... else`, `if let` and `while let` over a variant
/// of `names`, and fallback arms in a `match` whose arms name one.
fn blind_spots(file: &str, text: &str, names: &BTreeSet<String>) -> Vec<Site> {
    if file.ends_with("/tests.rs") {
        return Vec::new();
    }
    let code: Vec<char> = without_tests(&code_only(text));
    let line_of = |at: usize| {
        code[..at.min(code.len())]
            .iter()
            .filter(|&&c| c == '\n')
            .count()
            + 1
    };
    let site = |kind: &'static str, at: usize, text: String| Site {
        file: file.to_string(),
        function: function_at(&code, at),
        kind,
        text,
        line: line_of(at),
    };
    // Inside an `impl` of an enum, `Self::A` names its variant `A`.
    let impls = enum_impls(&code, names);
    let mut with_self = names.clone();
    with_self.insert("Self".to_string());
    let names_at = |at: usize| {
        if impls.iter().any(|&(a, b)| a <= at && at < b) {
            &with_self
        } else {
            names
        }
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < code.len() {
        if word_at(&code, i, "matches") && code.get(i + 7) == Some(&'!') {
            let open = i + 8;
            let end = close(&code, open);
            let inner: String = code[open + 1..end.saturating_sub(1)].iter().collect();
            let parts = split_top(&inner, ',');
            if parts.len() > 1 && names_a_variant(&parts[1..].join(","), names_at(i)) {
                out.push(site("matches!", i, one_line(&code[i..end])));
            }
            i = open;
        } else if word_at(&code, i, "let") {
            let Some(eq) = let_equals(&code, i) else {
                i += 3;
                continue;
            };
            let named = names_a_variant(&code[i + 3..eq].iter().collect::<String>(), names_at(i));
            match condition_keyword(&code, i) {
                // `if let PATTERN = VALUE { ... }` or `while let`: the value ends at the block,
                // which no struct literal comes before in a condition.
                Some(keyword) => {
                    let open = (eq..code.len())
                        .find(|&k| code[k] == '{')
                        .unwrap_or(code.len());
                    if named {
                        let text = format!(
                            "{} {}",
                            keyword.trim_end_matches(" let"),
                            one_line(&code[i..open])
                        );
                        out.push(site(keyword, i, text));
                    }
                }
                // `let PATTERN = VALUE else { ... };`: the value ends where a top-level `else`
                // or `;` comes, and the `else` of an `if` follows a `}`.
                None => {
                    let mut depth = 0;
                    let mut j = eq;
                    while j < code.len() {
                        match code[j] {
                            '(' | '[' | '{' => depth += 1,
                            ')' | ']' | '}' => depth -= 1,
                            _ => {}
                        }
                        if depth < 0 || (depth == 0 && code[j] == ';') {
                            break;
                        }
                        if depth == 0 && word_at(&code, j, "else") {
                            let before = code[..j].iter().rev().find(|c| !c.is_whitespace());
                            if before != Some(&'}') && named {
                                out.push(site("let-else", i, one_line(&code[i..j])));
                            }
                            break;
                        }
                        j += 1;
                    }
                }
            }
            i += 3;
        } else if word_at(&code, i, "match") {
            // The scrutinee, then the arms: each a pattern up to its `=>`, and a body that is a
            // block or runs to a top-level `,`.
            let Some(open) = (i..code.len()).find(|&k| code[k] == '{') else {
                break;
            };
            let scrutinee = one_line(&code[i + 5..open]);
            let end = close(&code, open);
            let mut arms = Vec::new();
            let mut k = open + 1;
            while k < end - 1 {
                while k < end - 1 && (code[k].is_whitespace() || code[k] == ',') {
                    k += 1;
                }
                if k >= end - 1 {
                    break;
                }
                let start = k;
                let mut depth = 0;
                while k < end - 1
                    && !(depth == 0 && code[k] == '=' && code.get(k + 1) == Some(&'>'))
                {
                    match code[k] {
                        '(' | '[' | '{' => depth += 1,
                        ')' | ']' | '}' => depth -= 1,
                        _ => {}
                    }
                    k += 1;
                }
                let pattern = one_line(&code[start..k]);
                k += 2;
                while k < end - 1 && code[k].is_whitespace() {
                    k += 1;
                }
                if k < end - 1 && code[k] == '{' {
                    k = close(&code, k);
                } else {
                    let mut depth = 0;
                    while k < end - 1 && !(depth == 0 && code[k] == ',') {
                        match code[k] {
                            '(' | '[' | '{' => depth += 1,
                            ')' | ']' | '}' => depth -= 1,
                            _ => {}
                        }
                        k += 1;
                    }
                }
                arms.push((start, pattern));
            }
            if arms.iter().any(|(_, p)| names_a_variant(p, names_at(i))) {
                for (at, p) in &arms {
                    if wildcard_pattern(p) {
                        let text = format!("match {} {{ {} }}", scrutinee, p);
                        out.push(site("wildcard arm", *at, text));
                    }
                }
            }
            i += 5;
        } else {
            i += 1;
        }
    }
    out
}

/// Every site of the four crates.
fn all_blind_spots() -> Vec<Site> {
    let mut files = Vec::new();
    for krate in EXHAUSTIVE_CRATES {
        let src = compiler().join(krate).join("src");
        // A module the root declares `#[cfg(test)] mod name;` holds tests only.
        let root = code_only(&fs::read_to_string(src.join("lib.rs")).unwrap());
        let words: Vec<&str> = root.split_whitespace().collect();
        let tests: Vec<String> = words
            .windows(3)
            .filter(|w| w[0] == "#[cfg(test)]" && w[1] == "mod" && w[2].ends_with(';'))
            .map(|w| format!("{}/src/{}.rs", krate, w[2].trim_end_matches(';')))
            .collect();
        files.extend(
            sources(&src)
                .into_iter()
                .filter(|(file, _)| !tests.contains(file)),
        );
    }
    let mut named = files.clone();
    named.extend(sources(&compiler().join("mente-syntax").join("src")));
    let names = enum_names(&named);
    files
        .iter()
        .flat_map(|(file, text)| blind_spots(file, text, &names))
        .collect()
}

/// Each place the lint does not see, by file, function, kind and text, with why every variant it
/// leaves unnamed belongs where it is. A site listed twice stands twice in its function.
const ALLOWED: &[(&str, &str, &str, &str, &str)] = &[
    // mente-codegen
    (
        "mente-codegen/src/lower.rs",
        "address",
        "wildcard arm",
        "match (step, shape) { _ }",
        "a step and a shape that do not go together is a lowering bug, reported as an error",
    ),
    (
        "mente-codegen/src/lower.rs",
        "aliases",
        "wildcard arm",
        "match tree.block(b).stmts.last().map(|s| &tree.stmt(*s).kind) { _ }",
        "a block whose last statement is no expression gives no value, so it aliases nothing",
    ),
    (
        "mente-codegen/src/lower.rs",
        "aliases",
        "wildcard arm",
        "match tree.block(b).stmts.last().map(|s| &tree.stmt(*s).kind) { _ }",
        "a block whose last statement is no expression gives no value, so it aliases nothing",
    ),
    (
        "mente-codegen/src/lower.rs",
        "binary",
        "matches!",
        "matches!(arith, ArithOp::Div | ArithOp::Rem)",
        "only a quotient and a remainder have a divisor to check; every other arithmetic operator has none",
    ),
    (
        "mente-codegen/src/lower.rs",
        "bound",
        "wildcard arm",
        "match slot(self, n).and_then(|i| self.s.locals[i].clone()) { _ }",
        "a bound names an integer value or an array place, which the checker requires; anything else is reported as an error",
    ),
    (
        "mente-codegen/src/lower.rs",
        "bound",
        "wildcard arm",
        "match slot(self, n).and_then(|i| self.s.locals[i].clone()) { _ }",
        "a bound names an integer value or an array place, which the checker requires; anything else is reported as an error",
    ),
    (
        "mente-codegen/src/lower.rs",
        "call",
        "if let",
        "if let Arg::Place(a, shape, Some(d), arg_span) = arg",
        "only a place argument declared with a type has a refinement to check where it comes back; a value argument and an undeclared place have none",
    ),
    (
        "mente-codegen/src/lower.rs",
        "entry",
        "wildcard arm",
        "match (is_mut, shape) { (_, shape) }",
        "a `mut` parameter or one in memory is a place, whatever its shape",
    ),
    (
        "mente-codegen/src/lower.rs",
        "expr",
        "wildcard arm",
        "match (l, r) { _ }",
        "operands of different kinds in a comparison are a lowering bug, reported as an error",
    ),
    (
        "mente-codegen/src/lower.rs",
        "function",
        "wildcard arm",
        "match (is_mut, shape) { (_, shape) }",
        "a `mut` parameter or one in memory is a place, whatever its shape",
    ),
    (
        "mente-codegen/src/lower.rs",
        "function",
        "wildcard arm",
        "match block.stmts.last().map(|&s| &tree.stmt(s).kind) { _ }",
        "a block that ends with no expression statement gives `()`",
    ),
    (
        "mente-codegen/src/lower.rs",
        "if_expr",
        "wildcard arm",
        "match (&shape, self.interval(e)) { _ }",
        "an `if` of `()` or of no type has no value to meet in its join",
    ),
    (
        "mente-codegen/src/lower.rs",
        "in_memory",
        "matches!",
        "matches!(self, Shape::Array(..) | Shape::Struct(_) | Shape::Enum(_))",
        "a value lies in memory when it is an aggregate; an integer, a `bool` and `()` are SSA values",
    ),
    (
        "mente-codegen/src/lower.rs",
        "literal_element",
        "if let",
        "if let ExprKind::Array(items) = &tree.expr(item).kind",
        "only an array literal element is built in place; any other is evaluated, which handles every kind",
    ),
    (
        "mente-codegen/src/lower.rs",
        "literal_element",
        "if let",
        "if let ExprKind::Repeat { value, .. } = &tree.expr(item).kind",
        "only a repeat element is built in place; any other is evaluated, which handles every kind",
    ),
    (
        "mente-codegen/src/lower.rs",
        "literal_element",
        "matches!",
        "matches!(elem, Shape::Array(..))",
        "only an element that is itself an array is built in place; any other element is evaluated",
    ),
    (
        "mente-codegen/src/lower.rs",
        "literal_element",
        "wildcard arm",
        "match (exact, v, elem) { (_, v, _) }",
        "every element but an exact integer is narrowed into the element's shape, which `narrow` does for each kind",
    ),
    (
        "mente-codegen/src/lower.rs",
        "match_expr",
        "if let",
        "if let (PatternKind::Variant { name, bindings, .. }, V::Mem(a), Shape::Enum(es)) = (&arm.pattern.kind, s, &read)",
        "only a variant pattern on an enum in memory binds payload fields; every other pattern binds none",
    ),
    (
        "mente-codegen/src/lower.rs",
        "match_expr",
        "wildcard arm",
        "match (&arm.pattern.kind, tested) { (_, _) if last }",
        "the last arm needs no test: the `match` is exhaustive",
    ),
    (
        "mente-codegen/src/lower.rs",
        "match_expr",
        "wildcard arm",
        "match (&arm.pattern.kind, tested) { _ }",
        "a pattern of another kind than its scrutinee is reported as an error",
    ),
    (
        "mente-codegen/src/lower.rs",
        "match_expr",
        "wildcard arm",
        "match (&shape, self.interval(e)) { _ }",
        "a `match` of `()` or of no type has no value to join",
    ),
    (
        "mente-codegen/src/lower.rs",
        "match_expr",
        "wildcard arm",
        "match (s, &read) { (v, _) }",
        "a scrutinee other than an enum in memory is tested as it is",
    ),
    (
        "mente-codegen/src/lower.rs",
        "meet",
        "wildcard arm",
        "match (v, ty) { _ }",
        "a branch of `()`, or one whose `if` has no value, passes nothing to the join",
    ),
    (
        "mente-codegen/src/lower.rs",
        "narrow",
        "wildcard arm",
        "match (v, from, to) { (v, _, _) }",
        "narrowing changes integers only, at every depth of arrays; a `bool`, a struct, an enum, `()` and no value land as they are",
    ),
    (
        "mente-codegen/src/lower.rs",
        "negate",
        "if let",
        "if let ExprKind::Int { value, .. } = self.tree().expr(operand).kind",
        "only a literal's negation is a constant; every other operand is negated where it runs",
    ),
    (
        "mente-codegen/src/lower.rs",
        "reaches_never",
        "matches!",
        "matches!(s, Step::Index(V::Never, _, _))",
        "only an index of no value ends a place's steps; every other step continues them",
    ),
    (
        "mente-codegen/src/lower.rs",
        "stmt",
        "matches!",
        "matches!(tree.stmt(s).kind, StmtKind::Break)",
        "`break` leaves the loop, and `continue`, the other statement that reaches here, goes to its head",
    ),
    (
        "mente-codegen/src/native.rs",
        "constant",
        "let-else",
        "let cfg::Ty::Int(t) = f.ty(v)",
        "only an integer MIR value can be a constant integer",
    ),
    (
        "mente-codegen/src/native.rs",
        "define_print",
        "matches!",
        "matches!(writer.target(), Target::X86_64Windows)",
        "Windows differs from the other targets in its system calls only",
    ),
    (
        "mente-codegen/src/native.rs",
        "emit_text",
        "matches!",
        "matches!(writer.target(), Target::X86_64Windows)",
        "Windows differs from the other targets in its system calls only",
    ),
    (
        "mente-codegen/src/native.rs",
        "first",
        "wildcard arm",
        "match (shape, arg) { _ }",
        "an argument of another shape than its parameter has no first value out of range; `refusal` reports it",
    ),
    (
        "mente-codegen/src/native.rs",
        "inst",
        "matches!",
        "matches!(arg_tys[0], cfg::Ty::Int(IntTy { signed: true, .. }))",
        "an order comparison is signed or unsigned by its operands' type; a `bool` or an address compares unsigned",
    ),
    (
        "mente-codegen/src/native.rs",
        "inst",
        "wildcard arm",
        "match (rty, out, results.first()) { _ }",
        "a call of a result type but `()` without a result or its slot is a code-generation bug, reported as an error",
    ),
    (
        "mente-codegen/src/native.rs",
        "make_call",
        "wildcard arm",
        "match (is_mut, shape, arg) { (_, shape, arg) }",
        "an argument that is not a scalar value lies in memory, whatever its shape; a result that is no integer or `bool` prints as `()`, which is all of `()`'s",
    ),
    (
        "mente-codegen/src/native.rs",
        "program",
        "matches!",
        "matches!(writer.target(), Target::X86_64Windows)",
        "Windows differs from the other targets in its imports only",
    ),
    (
        "mente-codegen/src/native.rs",
        "store_arg",
        "wildcard arm",
        "match (shape, arg) { _ }",
        "an argument of another shape than its parameter is reported as an error",
    ),
    // mente-interp
    (
        "mente-interp/src/eval.rs",
        "arith",
        "let-else",
        "let (Value::Int(a), Value::Int(b)) = (l, r)",
        "arithmetic on a value that is no integer is a broken state, which stops the run",
    ),
    (
        "mente-interp/src/eval.rs",
        "eval",
        "matches!",
        "matches!(op, BinaryOp::Shl | BinaryOp::Shr)",
        "a shift's amount takes no context type; every other operator's right operand does",
    ),
    (
        "mente-interp/src/eval.rs",
        "shaped",
        "wildcard arm",
        "match (v, ty) { _ }",
        "a value from outside the program of another shape than its type is refused as a bad argument",
    ),
    // mente-mir
    (
        "mente-mir/src/cfg.rs",
        "admits",
        "matches!",
        "matches!(t, Ty::Int(_) | Ty::Bool)",
        "what an operand admits is a list of MIR types; every type outside it is refused by the verifier",
    ),
    (
        "mente-mir/src/cfg.rs",
        "admits",
        "matches!",
        "matches!(t, Ty::Int(_))",
        "what an operand admits is a list of MIR types; every type outside it is refused by the verifier",
    ),
    (
        "mente-mir/src/cfg.rs",
        "inst",
        "let-else",
        "let Ty::Int(t) = ty",
        "an integer constant of another type is broken MIR, which stops the run",
    ),
    (
        "mente-mir/src/cfg.rs",
        "inst",
        "wildcard arm",
        "match (args.first(), args.get(1)) { _ }",
        "an operand of another type than its operation takes is broken MIR, which stops the run",
    ),
    (
        "mente-mir/src/cfg.rs",
        "inst",
        "wildcard arm",
        "match (args.first(), args.get(1), ty) { _ }",
        "an operand of another type than its operation takes is broken MIR, which stops the run",
    ),
    (
        "mente-mir/src/cfg.rs",
        "inst",
        "wildcard arm",
        "match (args.first(), ty) { _ }",
        "an operand of another type than its operation takes is broken MIR, which stops the run",
    ),
    (
        "mente-mir/src/cfg.rs",
        "inst",
        "wildcard arm",
        "match (n, ty) { _ }",
        "an operand of another type than its operation takes is broken MIR, which stops the run",
    ),
    (
        "mente-mir/src/cfg.rs",
        "inst",
        "wildcard arm",
        "match args.first() { _ }",
        "an operand of another type than its operation takes is broken MIR, which stops the run",
    ),
    (
        "mente-mir/src/cfg.rs",
        "inst",
        "wildcard arm",
        "match args.get(i) { _ }",
        "an operand of another type than its operation takes is broken MIR, which stops the run",
    ),
    (
        "mente-mir/src/cfg.rs",
        "inst",
        "wildcard arm",
        "match args.get(i) { _ }",
        "an operand of another type than its operation takes is broken MIR, which stops the run",
    ),
    (
        "mente-mir/src/cfg.rs",
        "run",
        "if let",
        "if let Op::Call(c) = f.op(v)",
        "only a call instruction enters another function; every other runs in place",
    ),
    (
        "mente-mir/src/cfg.rs",
        "store",
        "wildcard arm",
        "match (v, ty) { _ }",
        "a store of a value of another type than the slot's is broken MIR, which stops the run",
    ),
    (
        "mente-mir/src/cfg_mutate.rs",
        "apply",
        "let-else",
        "let Term::Branch { cond, then, otherwise, } = &block.term",
        "the branch mutants change only a block that ends in a branch, the one `sites` found",
    ),
    (
        "mente-mir/src/cfg_mutate.rs",
        "inst_changes",
        "let-else",
        "let Ty::Int(t) = f.ty(v)",
        "the arithmetic mutants change integer values only",
    ),
    (
        "mente-mir/src/cfg_mutate.rs",
        "loc",
        "wildcard arm",
        "match f.blocks.get(self.block.0 as usize).map(|b| &b.term) { _ }",
        "a terminator other than a branch has no condition to point at",
    ),
    (
        "mente-mir/src/cfg_mutate.rs",
        "mutants",
        "if let",
        "if let Term::Branch { .. } = b.term",
        "only a branch has targets for the branch mutants to change",
    ),
    // mente-sema
    (
        "mente-sema/src/check.rs",
        "arrays_in_one_piece",
        "if let",
        "if let ExprKind::Repeat { count, .. } = &e.kind",
        "only a repeat has a count to bound beside its type's",
    ),
    (
        "mente-sema/src/check.rs",
        "arrays_in_one_piece",
        "let-else",
        "let TypeKind::Array { elem, len } = &node.kind",
        "only an array type has a size to bound",
    ),
    (
        "mente-sema/src/check.rs",
        "binary",
        "matches!",
        "matches!( (lt, rt), (Ty::Int(_), Ty::Int(_)) | (Ty::Bool, Ty::Bool) | (Ty::Any, _) | (_, Ty::Any) )",
        "integers compare with integers and `bool`s with `bool`s, a value of any type with anything; every other pair is `E0203`",
    ),
    (
        "mente-sema/src/check.rs",
        "binary",
        "matches!",
        "matches!(t, Ty::Struct(_) | Ty::Enum(_))",
        "structs and enums compare later in M2 (`E0109`); every other type compares now",
    ),
    (
        "mente-sema/src/check.rs",
        "body",
        "if let",
        "if let Some(StmtKind::Expr(e)) = last.map(|s| &self.tree.stmt(s).kind)",
        "only an expression statement at the end gives a body its value; any other leaves none",
    ),
    (
        "mente-sema/src/check.rs",
        "coverage",
        "if let",
        "if let (Value::Int(x), Read::Int(lo, hi, text)) = (v, read)",
        "only an integer literal of an integer `match` narrows the range it covers",
    ),
    (
        "mente-sema/src/check.rs",
        "coverage",
        "if let",
        "if let Read::Int(lo, hi, _) = read",
        "only an integer `match` has a range to cover; an enum's and a `bool`'s values are counted one by one",
    ),
    (
        "mente-sema/src/check.rs",
        "discarded",
        "matches!",
        "matches!(ty, Ty::Unit | Ty::Any)",
        "only `()` and a value of no type are no value to discard; every other type is one",
    ),
    (
        "mente-sema/src/check.rs",
        "freeze",
        "matches!",
        "matches!(kind, BindingKind::Var | BindingKind::Param { is_mut: true })",
        "only a `var` or a `mut` parameter can change; every other binding is reported where it would be changed",
    ),
    (
        "mente-sema/src/check.rs",
        "int_read",
        "if let",
        "if let ( Bound::Int { negative: ln, value: lv, }, Bound::Int { negative: hn, value: hv, }, ) = (&r.lo, &r.hi)",
        "only literal bounds give a range coverage can count; any other bound gives the type's",
    ),
    (
        "mente-sema/src/check.rs",
        "int_read",
        "if let",
        "if let TypeKind::Int { refinement: Some(r), .. } = &tree.ty(t).kind",
        "only a refined integer type narrows the range coverage counts",
    ),
    (
        "mente-sema/src/check.rs",
        "match_expr",
        "matches!",
        "matches!(read, Read::Other)",
        "a scrutinee whose type was reported already is not checked again; every other is",
    ),
    (
        "mente-sema/src/check.rs",
        "pattern",
        "let-else",
        "let Read::Enum(def) = read",
        "a variant pattern on what is no enum is `E0203`, reported in its `else`",
    ),
    (
        "mente-sema/src/check.rs",
        "pattern",
        "matches!",
        "matches!(read, Read::Other)",
        "a scrutinee whose type was reported already is not reported again",
    ),
    (
        "mente-sema/src/check.rs",
        "transition",
        "if let",
        "if let ExprKind::Index { index, .. } = &self.tree.expr(step).kind",
        "only an index step has an index the fix must keep fixed",
    ),
    (
        "mente-sema/src/check.rs",
        "transition",
        "let-else",
        "let ArmBody::Block(b) = arm.body",
        "the transition fix is offered for a block arm only; an expression arm has no assignment to turn into a value",
    ),
    (
        "mente-sema/src/check.rs",
        "transition",
        "let-else",
        "let StmtKind::Assign { place, op: AssignOp::Set, value, } = &self.tree.stmt(last).kind",
        "the transition fix is offered only for an arm that ends by assigning the place; any other has no fix",
    ),
    (
        "mente-sema/src/check.rs",
        "transition",
        "while let",
        "while let ExprKind::Index { base, .. } | ExprKind::Field { base, .. } = &self.tree.expr(step).kind",
        "a place's steps are indexes and fields down to its root, which is a name",
    ),
    (
        "mente-sema/src/check.rs",
        "variant",
        "matches!",
        "matches!(payload, Payload::Positional(args) if args.is_empty())",
        "only an empty list of positions is offered the fix that removes the parentheses",
    ),
    (
        "mente-sema/src/check.rs",
        "wildcard",
        "let-else",
        "let ExprKind::Match { arms, .. } = &self.tree.expr(m).kind",
        "`E0231` is reported for an arm of a `match`, which is the expression it names",
    ),
    (
        "mente-sema/src/check.rs",
        "wildcard",
        "matches!",
        "matches!(p, PatternKind::Variant { name, .. } if *name == v.name)",
        "a variant is covered by a variant pattern of its name; no other pattern names it",
    ),
    (
        "mente-sema/src/incremental.rs",
        "parse_item",
        "matches!",
        "matches!(t.kind, TokenKind::DocComment(_))",
        "doc comments are skipped to find a `module` line; every other token ends the search",
    ),
    (
        "mente-sema/src/incremental.rs",
        "parse_item",
        "wildcard arm",
        "match module.items.as_slice() { _ }",
        "an item that is not one function is checked with the whole module",
    ),
    (
        "mente-sema/src/names.rs",
        "pattern",
        "let-else",
        "let PatternKind::Variant { enum_name, bindings, .. } = &pattern.kind",
        "only a variant pattern binds names",
    ),
    (
        "mente-sema/src/names.rs",
        "ty",
        "matches!",
        "matches!( kind, BindingKind::Var | BindingKind::Param { is_mut: true } )",
        "only a named type is resolved to a struct or an enum; every other type kind has nothing to resolve",
    ),
    (
        "mente-sema/src/structs.rs",
        "check_type",
        "let-else",
        "let (Bound::Name(n) | Bound::Len(n)) = b",
        "a literal bound names no field",
    ),
    (
        "mente-sema/src/structs.rs",
        "field_sizes",
        "let-else",
        "let TypeKind::Array { elem, len } = &node.kind",
        "only an array type has an element count to bound",
    ),
    (
        "mente-sema/src/structs.rs",
        "qualify",
        "if let",
        "if let TypeKind::Named(n) = &t.kind",
        "only a named type can name an enum; every other type kind is left as it is",
    ),
    (
        "mente-sema/src/structs.rs",
        "qualify",
        "let-else",
        "let ExprKind::Field { base, name } = &tree.exprs[i].kind",
        "a variant is named as a field of its enum's name; every other expression is left as it is",
    ),
    (
        "mente-sema/src/structs.rs",
        "qualify",
        "let-else",
        "let ExprKind::Name(n) = &base.kind",
        "a variant is named as a field of its enum's name; every other expression is left as it is",
    ),
    (
        "mente-sema/src/structs.rs",
        "via_variant",
        "wildcard arm",
        "match (n, via.variant) { _ }",
        "a struct, or an enum reached by no variant, has no variant to name",
    ),
    (
        "mente-sema/src/tier1.rs",
        "add_vals",
        "wildcard arm",
        "match (l, r) { _ }",
        "a sum Tier 1 does not keep as a linear form or a quotient becomes an interval below, which is sound for every kind of value",
    ),
    (
        "mente-sema/src/tier1.rs",
        "anchor",
        "wildcard arm",
        "match (self.stmt, self.syntax.body) { _ }",
        "an obligation outside a statement, an arm and an expression body has nowhere a fix could go",
    ),
    (
        "mente-sema/src/tier1.rs",
        "assign_stmt",
        "if let",
        "if let Shape::Array(n, _) = self.place_shape(root, &steps[..k])",
        "only an index into an array of known length has a bound to check here; the checker gives every index an array",
    ),
    (
        "mente-sema/src/tier1.rs",
        "assign_stmt",
        "if let",
        "if let Step::Index(i) = step",
        "only an index step is checked against a length; a field step has nothing to check",
    ),
    (
        "mente-sema/src/tier1.rs",
        "assume",
        "matches!",
        "matches!(c, Cond::And(..))",
        "`and` and `or` are the two conditions this arm takes; the test tells them apart",
    ),
    (
        "mente-sema/src/tier1.rs",
        "bit_not",
        "wildcard arm",
        "match (v, t) { _ }",
        "Tier 1 knows nothing of `~x` for any other value or an unknown type, which is sound",
    ),
    (
        "mente-sema/src/tier1.rs",
        "callee_path_var",
        "let-else",
        "let ExprKind::Name(n) = &tree.expr(at).kind",
        "a clause's field path starts at a parameter's name; any other start has no variable",
    ),
    (
        "mente-sema/src/tier1.rs",
        "callee_path_var",
        "while let",
        "while let ExprKind::Field { base, name } = &tree.expr(at).kind",
        "a field path is fields down to a name; anything else ends it",
    ),
    (
        "mente-sema/src/tier1.rs",
        "clause_paths",
        "matches!",
        "matches!(&tree.expr(at).kind, ExprKind::Name(n) if n == param)",
        "a field path of the parameter starts at its name; any other start is another's",
    ),
    (
        "mente-sema/src/tier1.rs",
        "clause_paths",
        "while let",
        "while let ExprKind::Field { base, name } = &tree.expr(at).kind",
        "a field path is fields down to a name; anything else ends it",
    ),
    (
        "mente-sema/src/tier1.rs",
        "clobber",
        "if let",
        "if let ExprKind::Name(_) = self.tree.expr(place).kind",
        "a whole binding assigned loses its facts here; an element or a field keeps the binding's other facts",
    ),
    (
        "mente-sema/src/tier1.rs",
        "compared_with",
        "matches!",
        "matches!( e.kind, ExprKind::Binary { op: BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge, .. } )",
        "only a comparison compares; every other expression is no candidate",
    ),
    (
        "mente-sema/src/tier1.rs",
        "div_vals",
        "if let",
        "if let Val::Lin(t, k) = l",
        "only a linear dividend divides into a quotient Tier 1 keeps; any other gives an interval, which is sound",
    ),
    (
        "mente-sema/src/tier1.rs",
        "emit",
        "wildcard arm",
        "match (guard, anchor) { (g, _) }",
        "a guard is withdrawn only before a loop that changes what it reads; every other place keeps it",
    ),
    (
        "mente-sema/src/tier1.rs",
        "eval",
        "if let",
        "if let Ty::Array(n, _) = &typing.exprs[base.0 as usize]",
        "only an array's `.len` is a constant; every other field read is evaluated",
    ),
    (
        "mente-sema/src/tier1.rs",
        "eval",
        "let-else",
        "let Site::Value(var) = self.sites[e.0 as usize]",
        "a value without a variable of its own is its summary, which is sound for every site",
    ),
    (
        "mente-sema/src/tier1.rs",
        "eval",
        "let-else",
        "let Site::Value(var) = self.sites[e.0 as usize]",
        "a value without a variable of its own is its summary, which is sound for every site",
    ),
    (
        "mente-sema/src/tier1.rs",
        "eval_into",
        "wildcard arm",
        "match (&self.tree.expr(e).kind, shape) { _ }",
        "every value but an array literal is evaluated, then narrowed into its shape, which `narrow` does for each kind",
    ),
    (
        "mente-sema/src/tier1.rs",
        "facts",
        "if let",
        "if let Shape::Int { int, refine } = shape",
        "only an integer has facts of its type and refinement; Tier 1 follows nothing of the other shapes, which is sound",
    ),
    (
        "mente-sema/src/tier1.rs",
        "followed",
        "matches!",
        "matches!(self.typing.exprs[base.0 as usize], Ty::Array(..))",
        "`.len` is followed as a constant on an array only, its one field",
    ),
    (
        "mente-sema/src/tier1.rs",
        "form",
        "let-else",
        "let ExprKind::Name(root) = &tree.expr(at).kind",
        "a field path in the fragment starts at a binding; any other start is outside it, as the error says",
    ),
    (
        "mente-sema/src/tier1.rs",
        "form",
        "while let",
        "while let ExprKind::Field { base, name } = &tree.expr(at).kind",
        "a field path is fields down to a name; anything else ends it",
    ),
    (
        "mente-sema/src/tier1.rs",
        "form",
        "wildcard arm",
        "match (names.path_ty)(root, &path) { _ }",
        "a path that is no integer is outside the fragment, as the error says",
    ),
    (
        "mente-sema/src/tier1.rs",
        "form",
        "wildcard arm",
        "match (names.path_ty)(root, prefix) { _ }",
        "a path that is no array has no length",
    ),
    (
        "mente-sema/src/tier1.rs",
        "if_ends",
        "if let",
        "if let ExprKind::Match { scrutinee, arms } = &self.tree.expr(e).kind",
        "a `match`'s arms end apart, as an `if`'s branches do; every other value ends where it is evaluated",
    ),
    (
        "mente-sema/src/tier1.rs",
        "if_ends",
        "let-else",
        "let ExprKind::If { cond, then, otherwise, } = &self.tree.expr(e).kind",
        "only an `if` has branches whose ends are kept apart; any other value ends where it is evaluated",
    ),
    (
        "mente-sema/src/tier1.rs",
        "is_int_type",
        "matches!",
        "matches!(tree.ty(t).kind, TypeKind::Int { .. })",
        "only an integer type is an integer type",
    ),
    (
        "mente-sema/src/tier1.rs",
        "join_summary",
        "wildcard arm",
        "match (a, b) { _ }",
        "the join of summaries of different kinds is a value Tier 1 knows nothing of, which is sound",
    ),
    (
        "mente-sema/src/tier1.rs",
        "literal_refinement",
        "matches!",
        "matches!(b, Bound::Int { .. })",
        "only a literal bound is a literal",
    ),
    (
        "mente-sema/src/tier1.rs",
        "match_arms",
        "matches!",
        "matches!(self.typing.exprs[scrutinee.0 as usize], Ty::Bool)",
        "a `bool` scrutinee is read as a condition; every other is read as a value",
    ),
    (
        "mente-sema/src/tier1.rs",
        "names_changed",
        "matches!",
        "matches!(&t.kind, mente_syntax::TokenKind::Ident(n) if self.named(n).is_some_and(|b| changed.contains(&b)))",
        "only a name token can name a binding",
    ),
    (
        "mente-sema/src/tier1.rs",
        "new",
        "let-else",
        "let Ty::Struct(s) = t",
        "only a struct has field paths to follow",
    ),
    (
        "mente-sema/src/tier1.rs",
        "new",
        "matches!",
        "matches!(t, Ty::Int(_))",
        "only an integer binding gets an octagon variable",
    ),
    (
        "mente-sema/src/tier1.rs",
        "own_bound",
        "wildcard arm",
        "match self.named(n).map(|i| &self.typing.bindings[i]) { _ }",
        "a length of what is no array is a value Tier 1 knows nothing of, which is sound",
    ),
    (
        "mente-sema/src/tier1.rs",
        "own_int",
        "matches!",
        "matches!(t, Ty::Int(_))",
        "only an integer binding has a variable",
    ),
    (
        "mente-sema/src/tier1.rs",
        "path_shape",
        "wildcard arm",
        "match (Fx::declared_shape(def.tree(), f.decl), rest) { _ }",
        "a path that ends before an integer or runs through what is no struct has no scalar shape",
    ),
    (
        "mente-sema/src/tier1.rs",
        "path_ty",
        "let-else",
        "let Ty::Struct(s) = t",
        "only a struct has fields to follow",
    ),
    (
        "mente-sema/src/tier1.rs",
        "pattern_shapes",
        "let-else",
        "let ExprKind::Match { arms, .. } = &e.kind",
        "only a variant pattern of a `match` binds a payload's fields",
    ),
    (
        "mente-sema/src/tier1.rs",
        "pattern_shapes",
        "let-else",
        "let PatternKind::Variant { enum_name, name, bindings: Some(bs), .. } = &arm.pattern.kind",
        "only a variant pattern of a `match` binds a payload's fields",
    ),
    (
        "mente-sema/src/tier1.rs",
        "reading",
        "if let",
        "if let ExprKind::Construct { fields, .. } = &self.tree.expr(at).kind",
        "only a struct literal has fields to read without a place; every other value is read where it lies",
    ),
    (
        "mente-sema/src/tier1.rs",
        "reading",
        "matches!",
        "matches!(self.typing.exprs[at.0 as usize], Ty::Array(..))",
        "`.len` is read as a constant on an array only, its one field",
    ),
    (
        "mente-sema/src/tier1.rs",
        "refined_var_fix",
        "let-else",
        "let TypeKind::Int { name: declared, refinement: None, } = &tree.ty(t).kind",
        "only a binding declared with an integer type has a refinement a fix could widen",
    ),
    (
        "mente-sema/src/tier1.rs",
        "rem",
        "let-else",
        "let Site::Value(var) = self.sites[e.0 as usize]",
        "a remainder without a variable of its own is its interval, which is sound",
    ),
    (
        "mente-sema/src/tier1.rs",
        "rest",
        "let-else",
        "let Ty::Struct(s) = Ty::of(ct, p.ty)",
        "only a parameter of struct type has field paths a clause can read",
    ),
    (
        "mente-sema/src/tier1.rs",
        "rest",
        "matches!",
        "matches!(f.typing.exprs[i], Ty::Int(_))",
        "only an integer value needs a variable at its site",
    ),
    (
        "mente-sema/src/tier1.rs",
        "route",
        "if let",
        "if let StmtKind::While { invariants, .. } = &s.kind",
        "only a loop has invariants to route",
    ),
    (
        "mente-sema/src/tier1.rs",
        "shift_vals",
        "wildcard arm",
        "match (op, k) { _ }",
        "Tier 1 knows nothing of a shift by an unknown amount to the left, which is sound",
    ),
    (
        "mente-sema/src/tier1.rs",
        "struct_vals",
        "if let",
        "if let (ExprKind::Construct { .. }, Some(built)) = (&self.tree.expr(e).kind, &self.built[e.0 as usize])",
        "only a struct literal has the values it was built of; every other struct value is read field by field",
    ),
    (
        "mente-sema/src/tier1/smt.rs",
        "names",
        "let-else",
        "let StmtKind::While { decreases, .. } = &tree.stmts[s].kind",
        "only a loop has `decreases` measures to name",
    ),
    (
        "mente-sema/src/tier1/smt.rs",
        "text",
        "if let",
        "if let Tag::Clause(s) = o.tag",
        "only a clause's obligation names a clause; every other names its operation",
    ),
    (
        "mente-sema/src/types.rs",
        "accepts",
        "wildcard arm",
        "match (self, value) { (a, b) }",
        "after the arms that relate integers, arrays and any type, two types accept each other when they are one",
    ),
    (
        "mente-sema/src/types.rs",
        "join",
        "wildcard arm",
        "match (self, other) { (a, b) if a == b }",
        "two types join when they are one",
    ),
    (
        "mente-sema/src/types.rs",
        "join",
        "wildcard arm",
        "match (self, other) { (a, b) }",
        "integers of two types join in a mix, whatever the types",
    ),
    (
        "mente-sema/src/types.rs",
        "join",
        "wildcard arm",
        "match (self, other) { _ }",
        "two different types have no join, and a type added later fails loudly with `E0203` until it is joined",
    ),
    (
        "mente-sema/src/types.rs",
        "same_as",
        "wildcard arm",
        "match (self, place) { (a, b) }",
        "after the arms for integers, arrays and any type, two types are the same when they are equal",
    ),
    (
        "mente-sema/src/wide.rs",
        "array",
        "let-else",
        "let Ty::Array(_, elem) = &self.typing.exprs[e.0 as usize]",
        "only an array literal has elements to bound",
    ),
    (
        "mente-sema/src/wide.rs",
        "array",
        "matches!",
        "matches!(leaf, Ty::Int(Int::Mixed(None)))",
        "only elements of no one type need a bound on their mix",
    ),
    (
        "mente-sema/src/wide.rs",
        "array",
        "while let",
        "while let Ty::Array(_, inner) = leaf",
        "an array's leaves are what lies below every array level",
    ),
    (
        "mente-sema/src/wide.rs",
        "check",
        "if let",
        "if let StmtKind::Assign { place, op: AssignOp::Compound(op), value, } = &s.kind",
        "only a compound assignment computes an operation of its own; every other statement's are its expressions'",
    ),
    (
        "mente-sema/src/wide.rs",
        "check",
        "wildcard arm",
        "match f.body { _ }",
        "a block body or none has no expression body",
    ),
    (
        "mente-sema/src/wide.rs",
        "declared",
        "let-else",
        "let TypeKind::Int { refinement: Some(r), .. } = &self.tree.ty(t).kind",
        "only a refined integer type declares a range",
    ),
    (
        "mente-sema/src/wide.rs",
        "insertion",
        "matches!",
        "matches!(s.kind, StmtKind::While { .. })",
        "a binding cannot go before a loop's condition, which runs every round; any other statement takes it",
    ),
    (
        "mente-sema/src/wide.rs",
        "leaves",
        "matches!",
        "matches!(self.typing.exprs[i.0 as usize], Ty::Array(..))",
        "an element that is itself an array is bounded by its leaves' type; any other element by its own range",
    ),
    (
        "mente-sema/src/wide.rs",
        "leaves",
        "while let",
        "while let Ty::Array(_, inner) = t",
        "an array's leaves are what lies below every array level",
    ),
    (
        "mente-sema/src/wide.rs",
        "report_match",
        "if let",
        "if let ArmBody::Block(_) = body",
        "a block arm takes the binding before its value; an expression arm is made a block first",
    ),
];

#[test]
fn test_what_the_lint_does_not_see_is_listed_with_its_reason() {
    // Each site by its key, with the lines it stands on; each entry by its key, with how many
    // times it is listed, so that two sites of one text need two entries.
    let mut sites: BTreeMap<(String, String, String, String), Vec<usize>> = BTreeMap::new();
    for s in all_blind_spots() {
        sites
            .entry((s.file, s.function, s.kind.to_string(), s.text))
            .or_default()
            .push(s.line);
    }
    let mut allowed: BTreeMap<(String, String, String, String), usize> = BTreeMap::new();
    for (file, function, kind, text, reason) in ALLOWED {
        assert!(
            !reason.is_empty(),
            "{} `{}` {}: no reason",
            file,
            function,
            text
        );
        *allowed
            .entry((
                file.to_string(),
                function.to_string(),
                kind.to_string(),
                text.to_string(),
            ))
            .or_default() += 1;
    }
    let mut problems = Vec::new();
    for (key, lines) in &sites {
        let (file, function, kind, text) = key;
        match allowed.get(key) {
            Some(count) if *count == lines.len() => {}
            Some(count) => problems.push(format!(
                "{} `{}`: {} {} sites `{}` at lines {:?}, {} listed",
                file,
                function,
                lines.len(),
                kind,
                text,
                lines,
                count
            )),
            None => problems.push(format!(
                "{} `{}`: {} `{}` at line {:?}: name the variants, or list it with its reason",
                file, function, kind, text, lines
            )),
        }
    }
    for (file, function, kind, text) in allowed.keys() {
        let key = (file.clone(), function.clone(), kind.clone(), text.clone());
        if !sites.contains_key(&key) {
            problems.push(format!(
                "{} `{}`: no {} `{}` any more; remove its entry",
                file, function, kind, text
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// `if let` or `while let`, when the `let` at `i` follows `if` or `while`, a condition rather
/// than a statement.
fn condition_keyword(code: &[char], i: usize) -> Option<&'static str> {
    let before: String = code[..i]
        .iter()
        .rev()
        .skip_while(|c| c.is_whitespace())
        .take(5)
        .collect();
    let word: String = before.chars().rev().collect();
    let is = |w: &str| {
        word.ends_with(w)
            && !word[..word.len() - w.len()].ends_with(|c: char| c.is_alphanumeric() || c == '_')
    };
    if is("while") {
        Some("while let")
    } else if is("if") {
        Some("if let")
    } else {
        None
    }
}

/// The scan itself: each kind of site is found, in the function it lies in and by its text, an
/// enum's name a grouped `use ... as` or a `type` gives included, and `Self` in an `impl` of an
/// enum, a trait's included; and what is not one, a partial tuple wildcard, `Self` in an `impl`
/// of a struct, a comment, a string and a test, is not.
#[test]
fn test_the_scan_finds_each_place_the_lint_does_not_see() {
    let src = r#"
use crate::types::{
    Int as SemaInt,
    Ty as SemaTy,
};
enum Ty { Int, Bool }
enum V { Int(u8), Never }
fn classify(t: &SemaTy) -> bool {
    matches!(t, SemaTy::Int)
}
fn unwrap(v: V) -> u8 {
    let V::Int(n) = v else {
        return 0;
    };
    n
}
fn pair(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Int, Ty::Int) => true,
        (Ty::Bool, _) => false,
        (x, _) => x == b,
    }
}
fn conditions(v: V) -> u8 {
    while let V::Never = v {}
    if let V::Int(n) = v {
        return n;
    }
    0
}
fn quiet(v: V) -> u8 {
    // matches!(v, V::Int(_)) in a comment
    let text = "let V::Int(n) = v else";
    match v {
        V::Int(n) => n,
        V::Never => 0,
    }
}
type Kind = crate::k::V;
fn aliased(k: Kind) -> bool {
    matches!(k, Kind::Never)
}
impl V {
    fn is_never(&self) -> bool {
        matches!(self, Self::Never)
    }
    fn number(&self) -> u8 {
        if let Self::Int(n) = self {
            return *n;
        }
        0
    }
}
impl std::fmt::Display for V {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(n) => write!(f, "{}", n),
            _ => write!(f, "never"),
        }
    }
}
struct W(u8);
impl W {
    fn new() -> Self {
        let Self(n) = Self(0) else { unreachable!() };
        Self(n)
    }
}
#[cfg(test)]
mod tests {
    fn t(v: V) -> bool {
        matches!(v, V::Never)
    }
}
"#;
    let files = vec![("k/src/a.rs".to_string(), src.to_string())];
    let names = enum_names(&files);
    assert!(names.contains("SemaTy") && names.contains("Ty") && !names.contains("SemaInt"));
    assert!(names.contains("Kind") && !names.contains("W"));
    let found: Vec<(String, &str, String)> = blind_spots("k/src/a.rs", src, &names)
        .into_iter()
        .map(|s| (s.function, s.kind, s.text))
        .collect();
    let want = [
        ("classify", "matches!", "matches!(t, SemaTy::Int)"),
        ("unwrap", "let-else", "let V::Int(n) = v"),
        ("pair", "wildcard arm", "match (a, b) { (x, _) }"),
        ("conditions", "while let", "while let V::Never = v"),
        ("conditions", "if let", "if let V::Int(n) = v"),
        ("aliased", "matches!", "matches!(k, Kind::Never)"),
        ("is_never", "matches!", "matches!(self, Self::Never)"),
        ("number", "if let", "if let Self::Int(n) = self"),
        ("fmt", "wildcard arm", "match self { _ }"),
    ];
    assert_eq!(
        found,
        want.map(|(f, k, t)| (f.to_string(), k, t.to_string())),
        "{:?}",
        found
    );
}
