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
/// and the names `use ... as` gives one of them.
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
    for (_, text) in files {
        for line in code_only(text).lines() {
            let line = line.trim();
            if let Some((path, alias)) = line
                .strip_prefix("use ")
                .and_then(|l| l.trim_end_matches(';').split_once(" as "))
            {
                let original = path.rsplit("::").next().unwrap_or("");
                if names.contains(original) {
                    names.insert(alias.trim().to_string());
                }
            }
        }
    }
    names
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
/// left unnamed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    file: String,
    function: String,
    kind: &'static str,
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

/// The sites of one file: `matches!` and `let ... else` over a variant of `names`, and
/// wildcard arms in a `match` whose arms name one.
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
    let site = |kind: &'static str, at: usize| Site {
        file: file.to_string(),
        function: function_at(&code, at),
        kind,
        line: line_of(at),
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < code.len() {
        if word_at(&code, i, "matches") && code.get(i + 7) == Some(&'!') {
            let open = i + 8;
            let end = close(&code, open);
            let inner: String = code[open + 1..end.saturating_sub(1)].iter().collect();
            let parts = split_top(&inner, ',');
            if parts.len() > 1 && names_a_variant(&parts[1..].join(","), names) {
                out.push(site("matches!", i));
            }
            i = open;
        } else if word_at(&code, i, "let") && !after_if_or_while(&code, i) {
            // `let PATTERN = VALUE else { ... };`: the value ends where a top-level `else` or `;`
            // comes, and the `else` of an `if` follows a `}`.
            let mut j = i + 3;
            let mut depth = 0;
            let mut eq = None;
            while j < code.len() {
                match code[j] {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    '=' if depth == 0
                        && eq.is_none()
                        && !matches!(code.get(j + 1), Some('=') | Some('>'))
                        && !matches!(
                            code.get(j - 1),
                            Some('<') | Some('>') | Some('!') | Some('=')
                        ) =>
                    {
                        eq = Some(j)
                    }
                    ';' if depth == 0 => break,
                    _ => {}
                }
                if depth < 0 {
                    break;
                }
                if depth == 0 && eq.is_some() && word_at(&code, j, "else") {
                    let before = code[..j].iter().rev().find(|c| !c.is_whitespace());
                    if before != Some(&'}') {
                        let pattern: String = code[i + 3..eq.unwrap_or(i + 3)].iter().collect();
                        if names_a_variant(&pattern, names) {
                            out.push(site("let-else", i));
                        }
                    }
                    break;
                }
                j += 1;
            }
            i += 3;
        } else if word_at(&code, i, "match") {
            // The scrutinee, then the arms: each a pattern up to its `=>`, and a body that is a
            // block or runs to a top-level `,`.
            let Some(open) = (i..code.len()).find(|&k| code[k] == '{') else {
                break;
            };
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
                let pattern: String = code[start..k].iter().collect();
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
                arms.push((start, pattern.trim().to_string()));
            }
            if arms.iter().any(|(_, p)| names_a_variant(p, names)) {
                for (at, p) in &arms {
                    if wildcard_pattern(p) {
                        out.push(site("wildcard arm", *at));
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

/// Every site of the four crates, by file, function and kind, with how many there are.
fn all_blind_spots() -> BTreeMap<(String, String, &'static str), Vec<usize>> {
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
    let mut sites: BTreeMap<(String, String, &'static str), Vec<usize>> = BTreeMap::new();
    for (file, text) in &files {
        for s in blind_spots(file, text, &names) {
            sites
                .entry((s.file, s.function, s.kind))
                .or_default()
                .push(s.line);
        }
    }
    sites
}

/// Each place the lint does not see, by file, function and kind, with how many there are and
/// why every variant it leaves unnamed belongs where it is.
const ALLOWED: &[(&str, &str, &str, usize, &str)] = &[
    // mente-codegen: the lowering.
    ("mente-codegen/src/lower.rs", "address", "wildcard arm", 1, "a step and a shape that do not go together is a lowering bug, reported as an error"),
    ("mente-codegen/src/lower.rs", "aliases", "wildcard arm", 2, "a block whose last statement is no expression gives no value, so it aliases nothing"),
    ("mente-codegen/src/lower.rs", "binary", "matches!", 1, "only a quotient and a remainder have a divisor to check; every other arithmetic operator has none"),
    ("mente-codegen/src/lower.rs", "bound", "wildcard arm", 2, "a bound names an integer value or an array place, which the checker requires; anything else is reported as an error"),
    ("mente-codegen/src/lower.rs", "expr", "wildcard arm", 1, "operands of different kinds in a comparison are a lowering bug, reported as an error"),
    ("mente-codegen/src/lower.rs", "function", "wildcard arm", 2, "a `mut` parameter or one in memory is a place, whatever its shape; a block that ends with no expression gives `()`"),
    ("mente-codegen/src/lower.rs", "if_expr", "wildcard arm", 1, "an `if` of `()` or of no type has no value to meet in its join"),
    ("mente-codegen/src/lower.rs", "in_memory", "matches!", 1, "a value lies in memory when it is an aggregate; an integer, a `bool` and `()` are SSA values"),
    ("mente-codegen/src/lower.rs", "literal_element", "matches!", 1, "only an element that is itself an array is built in place; any other element is evaluated"),
    ("mente-codegen/src/lower.rs", "literal_element", "wildcard arm", 1, "every element but an exact integer is narrowed into the element's shape, which `narrow` does for each kind"),
    ("mente-codegen/src/lower.rs", "match_expr", "wildcard arm", 4, "a scrutinee other than an enum in memory is tested as it is; an arm after which no other comes needs no test; a `match` of `()` or of no type has no value to join; a pattern of another kind than its scrutinee is reported as an error"),
    ("mente-codegen/src/lower.rs", "meet", "wildcard arm", 1, "a branch of `()`, or one whose `if` has no value, passes nothing to the join"),
    ("mente-codegen/src/lower.rs", "narrow", "wildcard arm", 1, "narrowing changes integers only, at every depth of arrays; a `bool`, a struct, an enum, `()` and no value land as they are"),
    ("mente-codegen/src/lower.rs", "reaches_never", "matches!", 1, "only an index of no value ends a place's steps; every other step continues them"),
    ("mente-codegen/src/lower.rs", "stmt", "matches!", 1, "`break` leaves the loop, and `continue`, the other statement that reaches here, goes to its head"),
    // mente-codegen: native code.
    ("mente-codegen/src/native.rs", "constant", "let-else", 1, "only an integer MIR value can be a constant integer"),
    ("mente-codegen/src/native.rs", "define_print", "matches!", 1, "Windows differs from the other targets in its system calls only"),
    ("mente-codegen/src/native.rs", "emit_text", "matches!", 1, "Windows differs from the other targets in its system calls only"),
    ("mente-codegen/src/native.rs", "first", "wildcard arm", 1, "an argument of another shape than its parameter has no first value out of range; `refusal` reports it"),
    ("mente-codegen/src/native.rs", "inst", "matches!", 1, "an order comparison is signed or unsigned by its operands' type; a `bool` or an address compares unsigned"),
    ("mente-codegen/src/native.rs", "inst", "wildcard arm", 1, "a call of a result type but `()` without a result or its slot is a code-generation bug, reported as an error"),
    ("mente-codegen/src/native.rs", "make_call", "wildcard arm", 2, "an argument that is not a scalar value lies in memory, whatever its shape; a result that is no integer or `bool` prints as `()`, which is all of `()`'s"),
    ("mente-codegen/src/native.rs", "program", "matches!", 1, "Windows differs from the other targets in its imports only"),
    ("mente-codegen/src/native.rs", "store_arg", "wildcard arm", 1, "an argument of another shape than its parameter is reported as an error"),
    // mente-interp: the reference interpreter.
    ("mente-interp/src/eval.rs", "arith", "let-else", 1, "arithmetic on a value that is no integer is a broken state, which stops the run"),
    ("mente-interp/src/eval.rs", "eval", "matches!", 1, "a shift's amount takes no context type; every other operator's right operand does"),
    ("mente-interp/src/eval.rs", "matches", "wildcard arm", 1, "a pattern that does not fit the value does not match it; `match` checking makes each pattern fit its scrutinee"),
    ("mente-interp/src/eval.rs", "shaped", "wildcard arm", 1, "a value from outside the program of another shape than its type is refused as a bad argument"),
    // mente-mir.
    ("mente-mir/src/cfg.rs", "admits", "matches!", 2, "what an operand admits is a list of MIR types; every type outside it is refused by the verifier"),
    ("mente-mir/src/cfg.rs", "inst", "let-else", 1, "an integer constant of another type is broken MIR, which stops the run"),
    ("mente-mir/src/cfg.rs", "inst", "wildcard arm", 7, "an operand of another type than its operation takes is broken MIR, which stops the run"),
    ("mente-mir/src/cfg.rs", "store", "wildcard arm", 1, "a store of a value of another type than the slot's is broken MIR, which stops the run"),
    ("mente-mir/src/cfg_mutate.rs", "apply", "let-else", 1, "the branch mutants change only a block that ends in a branch, the one `sites` found"),
    ("mente-mir/src/cfg_mutate.rs", "inst_changes", "let-else", 1, "the arithmetic mutants change integer values only"),
    ("mente-mir/src/cfg_mutate.rs", "loc", "wildcard arm", 1, "a terminator other than a branch has no condition to point at"),
    // mente-sema: the checker.
    ("mente-sema/src/check.rs", "arrays_in_one_piece", "let-else", 1, "only an array type has a size to bound"),
    ("mente-sema/src/check.rs", "binary", "matches!", 2, "structs and enums compare later in M2 (`E0109`); integers compare with integers and `bool`s with `bool`s, and any other pair is `E0203`"),
    ("mente-sema/src/check.rs", "discarded", "matches!", 1, "only `()` and a value of no type are no value to discard; every other type is one"),
    ("mente-sema/src/check.rs", "freeze", "matches!", 1, "only a `var` or a `mut` parameter can change; every other binding is reported where it would be changed"),
    ("mente-sema/src/check.rs", "match_expr", "matches!", 1, "a scrutinee whose type was reported already is not checked again; every other is"),
    ("mente-sema/src/check.rs", "pattern", "let-else", 1, "a variant pattern on what is no enum is `E0203`, reported in its `else`"),
    ("mente-sema/src/check.rs", "pattern", "matches!", 1, "a scrutinee whose type was reported already is not reported again"),
    ("mente-sema/src/check.rs", "transition", "let-else", 2, "the transition fix is offered for one shape only, a block arm that ends by assigning the place, and none for any other"),
    ("mente-sema/src/check.rs", "variant", "matches!", 1, "only an empty list of positions is offered the fix that removes the parentheses"),
    ("mente-sema/src/check.rs", "wildcard", "let-else", 1, "`E0231` is reported for an arm of a `match`, which is the expression it names"),
    ("mente-sema/src/check.rs", "wildcard", "matches!", 1, "a variant is covered by a variant pattern of its name; no other pattern names it"),
    ("mente-sema/src/incremental.rs", "parse_item", "matches!", 1, "doc comments are skipped to find a `module` line; every other token ends the search"),
    ("mente-sema/src/incremental.rs", "parse_item", "wildcard arm", 1, "an item that is not one function is checked with the whole module"),
    ("mente-sema/src/names.rs", "pattern", "let-else", 1, "only a variant pattern binds names"),
    ("mente-sema/src/names.rs", "ty", "matches!", 1, "only a named type is resolved to a struct or an enum; every other type kind has nothing to resolve"),
    ("mente-sema/src/structs.rs", "check_type", "let-else", 1, "a literal bound names no field"),
    ("mente-sema/src/structs.rs", "field_sizes", "let-else", 1, "only an array type has an element count to bound"),
    ("mente-sema/src/structs.rs", "qualify", "let-else", 2, "a variant is named as a field of its enum's name; every other expression is left as it is"),
    ("mente-sema/src/structs.rs", "via_variant", "wildcard arm", 1, "a struct, or an enum reached by no variant, has no variant to name"),
    // mente-sema: Tier 1.
    ("mente-sema/src/tier1.rs", "add_vals", "wildcard arm", 1, "a sum Tier 1 does not keep as a linear form or a quotient becomes an interval below, which is sound for every kind of value"),
    ("mente-sema/src/tier1.rs", "anchor", "wildcard arm", 1, "an obligation outside a statement, an arm and an expression body has nowhere a fix could go"),
    ("mente-sema/src/tier1.rs", "assume", "matches!", 1, "`and` and `or` are the two conditions this arm takes; the test tells them apart"),
    ("mente-sema/src/tier1.rs", "bit_not", "wildcard arm", 1, "Tier 1 knows nothing of `~x` for any other value or an unknown type, which is sound"),
    ("mente-sema/src/tier1.rs", "callee_path_var", "let-else", 1, "a clause's field path starts at a parameter's name; any other start has no variable"),
    ("mente-sema/src/tier1.rs", "clause_paths", "matches!", 1, "a field path of the parameter starts at its name; any other start is another's"),
    ("mente-sema/src/tier1.rs", "compared_with", "matches!", 1, "only a comparison compares; every other expression is no candidate"),
    ("mente-sema/src/tier1.rs", "emit", "wildcard arm", 1, "a guard is withdrawn only before a loop that changes what it reads; every other place keeps it"),
    ("mente-sema/src/tier1.rs", "eval", "let-else", 2, "a value without a variable of its own is its summary, which is sound for every site"),
    ("mente-sema/src/tier1.rs", "eval_into", "wildcard arm", 1, "every value but an array literal is evaluated, then narrowed into its shape, which `narrow` does for each kind"),
    ("mente-sema/src/tier1.rs", "followed", "matches!", 1, "`.len` is followed as a constant on an array only, its one field"),
    ("mente-sema/src/tier1.rs", "form", "let-else", 1, "a field path in the fragment starts at a binding; any other start is outside it, as the error says"),
    ("mente-sema/src/tier1.rs", "form", "wildcard arm", 2, "a path that is no array has no length, and one that is no integer is outside the fragment, as the error says"),
    ("mente-sema/src/tier1.rs", "if_ends", "let-else", 1, "only an `if` has branches whose ends are kept apart; any other value ends where it is evaluated"),
    ("mente-sema/src/tier1.rs", "is_int_type", "matches!", 1, "only an integer type is an integer type"),
    ("mente-sema/src/tier1.rs", "join_summary", "wildcard arm", 1, "the join of summaries of different kinds is a value Tier 1 knows nothing of, which is sound"),
    ("mente-sema/src/tier1.rs", "literal_refinement", "matches!", 1, "only a literal bound is a literal"),
    ("mente-sema/src/tier1.rs", "match_arms", "matches!", 1, "a `bool` scrutinee is read as a condition; every other is read as a value"),
    ("mente-sema/src/tier1.rs", "names_changed", "matches!", 1, "only a name token can name a binding"),
    ("mente-sema/src/tier1.rs", "new", "let-else", 1, "only a struct has field paths to follow"),
    ("mente-sema/src/tier1.rs", "new", "matches!", 1, "only an integer binding gets an octagon variable"),
    ("mente-sema/src/tier1.rs", "own_bound", "wildcard arm", 1, "a length of what is no array is a value Tier 1 knows nothing of, which is sound"),
    ("mente-sema/src/tier1.rs", "own_int", "matches!", 1, "only an integer binding has a variable"),
    ("mente-sema/src/tier1.rs", "path_shape", "wildcard arm", 1, "a path that ends before an integer or runs through what is no struct has no scalar shape"),
    ("mente-sema/src/tier1.rs", "path_ty", "let-else", 1, "only a struct has fields to follow"),
    ("mente-sema/src/tier1.rs", "pattern_shapes", "let-else", 2, "only a variant pattern of a `match` binds a payload's fields"),
    ("mente-sema/src/tier1.rs", "reading", "matches!", 1, "`.len` is read as a constant on an array only, its one field"),
    ("mente-sema/src/tier1.rs", "refined_var_fix", "let-else", 1, "only a binding declared with an integer type has a refinement a fix could widen"),
    ("mente-sema/src/tier1.rs", "rem", "let-else", 1, "a remainder without a variable of its own is its interval, which is sound"),
    ("mente-sema/src/tier1.rs", "rest", "let-else", 1, "only a parameter of struct type has field paths a clause can read"),
    ("mente-sema/src/tier1.rs", "rest", "matches!", 1, "only an integer value needs a variable at its site"),
    ("mente-sema/src/tier1.rs", "shift_vals", "wildcard arm", 1, "Tier 1 knows nothing of a shift by an unknown amount to the left, which is sound"),
    ("mente-sema/src/tier1/smt.rs", "names", "let-else", 1, "only a loop has `decreases` measures to name"),
    // mente-sema: types and intermediates.
    ("mente-sema/src/types.rs", "accepts", "wildcard arm", 1, "after the arms that relate integers, arrays and any type, two types accept each other when they are one"),
    ("mente-sema/src/types.rs", "join", "wildcard arm", 3, "integers of two types join in a mix; any other two types join when they are one, and a type added later fails loudly with E0203 until it is joined"),
    ("mente-sema/src/types.rs", "same_as", "wildcard arm", 1, "after the arms for integers, arrays and any type, two types are the same when they are equal"),
    ("mente-sema/src/wide.rs", "array", "let-else", 1, "only an array literal has elements to bound"),
    ("mente-sema/src/wide.rs", "array", "matches!", 1, "only elements of no one type need a bound on their mix"),
    ("mente-sema/src/wide.rs", "check", "wildcard arm", 1, "a block body or none has no expression body"),
    ("mente-sema/src/wide.rs", "declared", "let-else", 1, "only a refined integer type declares a range"),
    ("mente-sema/src/wide.rs", "insertion", "matches!", 1, "a binding cannot go before a loop's condition, which runs every round; any other statement takes it"),
    ("mente-sema/src/wide.rs", "leaves", "matches!", 1, "an element that is itself an array is bounded by its leaves' type; any other element by its own range"),
];

#[test]
fn test_what_the_lint_does_not_see_is_listed_with_its_reason() {
    let sites = all_blind_spots();
    let mut allowed: BTreeMap<(String, String, String), (usize, &str)> = BTreeMap::new();
    for (file, function, kind, count, reason) in ALLOWED {
        assert!(
            !reason.is_empty(),
            "{} {} {}: no reason",
            file,
            function,
            kind
        );
        allowed.insert(
            (file.to_string(), function.to_string(), kind.to_string()),
            (*count, reason),
        );
    }
    let mut problems = Vec::new();
    for ((file, function, kind), lines) in &sites {
        let key = (file.clone(), function.clone(), kind.to_string());
        match allowed.get(&key) {
            Some((count, _)) if *count == lines.len() => {}
            Some((count, _)) => problems.push(format!(
                "{} `{}`: {} {} sites at lines {:?}, {} listed",
                file,
                function,
                lines.len(),
                kind,
                lines,
                count
            )),
            None => problems.push(format!(
                "{} `{}`: {} at lines {:?}: name the variants, or list it with its reason",
                file, function, kind, lines
            )),
        }
    }
    for (file, function, kind) in allowed.keys() {
        if !sites.contains_key(&(file.clone(), function.clone(), kind_static(kind))) {
            problems.push(format!(
                "{} `{}`: no {} site any more; remove its entry",
                file, function, kind
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn kind_static(kind: &str) -> &'static str {
    match kind {
        "matches!" => "matches!",
        "let-else" => "let-else",
        _ => "wildcard arm",
    }
}

/// Whether the `let` at `i` follows `if` or `while`, a condition rather than a statement.
fn after_if_or_while(code: &[char], i: usize) -> bool {
    let before: String = code[..i]
        .iter()
        .rev()
        .skip_while(|c| c.is_whitespace())
        .take(5)
        .collect();
    let word: String = before.chars().rev().collect();
    word.ends_with("if") || word.ends_with("while")
}

/// The scan itself: each kind of site is found, in the function it lies in, and what is not
/// one, a partial tuple wildcard, an `if let`, a comment, a string and a test, is not.
#[test]
fn test_the_scan_finds_each_place_the_lint_does_not_see() {
    let names: BTreeSet<String> = ["Ty", "V"].iter().map(|s| s.to_string()).collect();
    let src = r#"
fn classify(t: &Ty) -> bool {
    matches!(t, Ty::Int(_))
}
fn unwrap(v: V) -> u8 {
    let V::Int(n) = v else {
        return 0;
    };
    n
}
fn pair(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Int(_), Ty::Int(_)) => true,
        (Ty::Bool, _) => false,
        (x, _) => x == b,
    }
}
fn quiet(v: V) -> u8 {
    // matches!(v, V::Int(_)) in a comment
    let text = "let V::Int(n) = v else";
    if let V::Int(n) = v {
        return n;
    }
    match v {
        V::Int(n) => n,
        V::Never => 0,
    }
}
#[cfg(test)]
mod tests {
    fn t(v: V) -> bool {
        matches!(v, V::Never)
    }
}
"#;
    let found: Vec<(String, &str)> = blind_spots("k/src/a.rs", src, &names)
        .into_iter()
        .map(|s| (s.function, s.kind))
        .collect();
    assert_eq!(
        found,
        [
            ("classify".to_string(), "matches!"),
            ("unwrap".to_string(), "let-else"),
            ("pair".to_string(), "wildcard arm"),
        ],
        "{:?}",
        found
    );
}
