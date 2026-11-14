//! The tests that hold the repository's gates: the dependency policy and the crates `Cargo.lock`
//! may hold, English-only text, and the workflow policy's place in the required Documentation job.
//! They are an integration test of their own, apart from main.rs, which changes with every feature:
//! nothing there can unhook them, and `.github/CODEOWNERS` gives them to their owner
//! (`docs/design-review.md`, `guard-per-problem`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// The dependency policy of CLAUDE.md, applied to one manifest: a compiler crate depends
/// only on the workspace's own crates, by path; `mente-mir` depends on nothing; only
/// `mente-codegen` may add Cranelift, and `gimli` and `object` for its object files and
/// debug information (Norman, 2026-11-10). Returns each dependency line outside the policy.
fn dependency_policy_violations(krate: &str, manifest: &str) -> Vec<String> {
    let mut section = String::new();
    let mut found = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_string();
            continue;
        }
        if !section.ends_with("dependencies") || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name = line.split(['=', '.']).next().unwrap_or("").trim();
        let allowed = if name.starts_with("mente-") {
            krate != "mente-mir"
                && !(krate == "mente-interp" && matches!(name, "mente-mir" | "mente-codegen"))
                && line.contains("path =")
        } else {
            (name.starts_with("cranelift") || matches!(name, "gimli" | "object"))
                && krate == "mente-codegen"
        };
        if !allowed {
            found.push(format!("{}: [{}] {}", krate, section, line));
        }
    }
    found
}

/// A new external dependency needs Norman's approval and a change to this policy.
#[test]
fn test_crates_depend_only_on_allowed_crates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = dependency_policy_violations(
        "workspace",
        &fs::read_to_string(root.join("Cargo.toml")).unwrap(),
    );
    let mut crates = 0;
    for entry in fs::read_dir(root.join("compiler")).unwrap().flatten() {
        if let Ok(manifest) = fs::read_to_string(entry.path().join("Cargo.toml")) {
            crates += 1;
            found.extend(dependency_policy_violations(
                &entry.file_name().to_string_lossy(),
                &manifest,
            ));
        }
    }
    assert!(
        crates >= 7,
        "expected the seven compiler crates, read {}",
        crates
    );
    assert!(
        found.is_empty(),
        "dependencies outside the policy:\n{}",
        found.join("\n")
    );

    // The policy itself: what it lets through and what it stops.
    let manifest = "[package]\nname = \"x\"\n\n[dependencies]\nmente-ast = { path = \"../mente-ast\" }\ncranelift-codegen = \"0.135\"\n\n[dev-dependencies]\nproptest = \"1\"\n";
    assert_eq!(
        dependency_policy_violations("mente-sema", manifest),
        vec![
            "mente-sema: [dependencies] cranelift-codegen = \"0.135\"".to_string(),
            "mente-sema: [dev-dependencies] proptest = \"1\"".to_string(),
        ]
    );
    assert_eq!(
        dependency_policy_violations("mente-codegen", manifest).len(),
        1
    );
    assert_eq!(dependency_policy_violations("mente-mir", manifest).len(), 3);
    // The reference interpreter shares no code with the paths it checks.
    let interp = "[dependencies]\nmente-sema = { path = \"../mente-sema\" }\nmente-mir = { path = \"../mente-mir\" }\nmente-codegen = { path = \"../mente-codegen\" }\n";
    assert_eq!(
        dependency_policy_violations("mente-interp", interp),
        vec![
            "mente-interp: [dependencies] mente-mir = { path = \"../mente-mir\" }".to_string(),
            "mente-interp: [dependencies] mente-codegen = { path = \"../mente-codegen\" }"
                .to_string(),
        ]
    );
    assert!(dependency_policy_violations("mente-cli", interp).is_empty());
    assert_eq!(
        dependency_policy_violations("mente-cli", "[dependencies]\nmente-ast = \"0.1\"\n").len(),
        1
    );
    // The object and DWARF writers are mente-codegen's alone, and only those two.
    let objects =
        "[dependencies]\ngimli = { version = \"0.33\" }\nobject = \"0.40\"\nobject-pool = \"1\"\n";
    assert_eq!(
        dependency_policy_violations("mente-codegen", objects),
        vec!["mente-codegen: [dependencies] object-pool = \"1\"".to_string()]
    );
    assert_eq!(
        dependency_policy_violations("mente-sema", objects),
        vec![
            "mente-sema: [dependencies] gimli = { version = \"0.33\" }".to_string(),
            "mente-sema: [dependencies] object = \"0.40\"".to_string(),
            "mente-sema: [dependencies] object-pool = \"1\"".to_string(),
        ]
    );
}

/// The one source a crate of the lock may come from outside the workspace.
const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// The packages of a `Cargo.lock`: each name, the sources its versions come from (none for
/// the workspace's own crates), and the names it depends on. Versions are left out: a name
/// the lock holds twice is one crate here. Only `[[package]]` sections are read, so that a
/// `[[patch.unused]]` or `[metadata]` section never adds to the package before it.
fn lock_packages(lock: &str) -> BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> {
    let mut sections: Vec<(bool, Vec<&str>)> = Vec::new();
    for line in lock.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
            sections.push((line == "[[package]]", Vec::new()));
        } else if let Some((_, lines)) = sections.last_mut() {
            lines.push(line);
        }
    }
    let mut packages: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for (_, lines) in sections.iter().filter(|(package, _)| *package) {
        let mut name = String::new();
        let mut sources = BTreeSet::new();
        let mut deps = BTreeSet::new();
        let mut in_deps = false;
        for line in lines {
            if in_deps {
                if *line == "]" {
                    in_deps = false;
                } else {
                    let dep = line.trim_matches(|c| c == '"' || c == ',');
                    deps.insert(dep.split(' ').next().unwrap_or(dep).to_string());
                }
            } else if let Some(value) = line.strip_prefix("name = ") {
                name = value.trim_matches('"').to_string();
            } else if let Some(value) = line.strip_prefix("source = ") {
                sources.insert(value.trim_matches('"').to_string());
            } else if *line == "dependencies = [" {
                in_deps = true;
            }
        }
        let entry = packages.entry(name).or_default();
        entry.0.extend(sources);
        entry.1.extend(deps);
    }
    packages
}

/// What keeps a lock from matching the approved list (`<crate>: <direct dependency>` lines):
/// a crate outside the workspace that the list lacks, a listed crate the lock does not hold,
/// and a listed dependency that is not a direct dependency of a workspace crate or does not
/// bring the crate.
fn lock_approval_problems(lock: &str, approved: &str) -> Vec<String> {
    let packages = lock_packages(lock);
    let local = |name: &str| {
        packages
            .get(name)
            .is_some_and(|(sources, _)| sources.is_empty())
    };
    let direct: BTreeSet<&str> = packages
        .iter()
        .filter(|(name, _)| local(name))
        .flat_map(|(_, (_, deps))| deps)
        .filter(|d| packages.contains_key(*d) && !local(d))
        .map(String::as_str)
        .collect();
    let brings = |from: &str, to: &str| {
        let mut seen = BTreeSet::new();
        let mut stack = vec![from.to_string()];
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if seen.insert(n.clone()) {
                stack.extend(packages.get(&n).map(|p| p.1.clone()).unwrap_or_default());
            }
        }
        false
    };
    let mut listed = BTreeMap::new();
    let mut problems = Vec::new();
    for line in approved.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.split_once(':') {
            Some((krate, via)) => {
                if listed.insert(krate.trim(), via.trim()).is_some() {
                    problems.push(format!("{} is listed twice", krate.trim()));
                }
            }
            None => problems.push(format!("\"{}\" is not \"<crate>: <dependency>\"", line)),
        }
    }
    for (name, (sources, _)) in &packages {
        for source in sources.iter().filter(|s| s.as_str() != CRATES_IO) {
            problems.push(format!(
                "{} comes from {}, not crates.io: only a crate of crates.io can be approved",
                name, source
            ));
        }
        if !sources.is_empty() && !listed.contains_key(name.as_str()) {
            problems.push(format!(
                "Cargo.lock holds {}, which .config/approved-crates.txt does not list: Norman \
                 approves every crate a dependency brings (dependency-approval)",
                name
            ));
        }
    }
    for (krate, via) in &listed {
        match packages.get(*krate) {
            None => problems.push(format!(
                "{} is listed, but Cargo.lock no longer holds it",
                krate
            )),
            Some((sources, _)) if sources.is_empty() => {
                problems.push(format!("{} is a workspace crate", krate))
            }
            Some(_) if !direct.contains(via) => problems.push(format!(
                "{}: {} is not a direct dependency of a workspace crate",
                krate, via
            )),
            Some(_) if !brings(via, krate) => {
                problems.push(format!("{}: {} does not bring it", krate, via))
            }
            Some(_) => {}
        }
    }
    problems
}

/// Every crate the lock holds was approved: a dependency that brings a crate the list lacks,
/// as a Dependabot update may, fails by the crate's name until the pull request lists it for
/// Norman (`docs/design-review.md`, `dependency-approval`).
#[test]
fn test_the_lock_holds_only_approved_crates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let problems = lock_approval_problems(
        &fs::read_to_string(root.join("Cargo.lock")).unwrap(),
        &fs::read_to_string(root.join(".config/approved-crates.txt")).unwrap(),
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    // The check itself, on a lock of two workspace crates and three outside.
    let lock = "version = 4\n\n[[package]]\nname = \"mente-a\"\nversion = \"0.1.0\"\ndependencies = [\n \"mente-b\",\n \"top\",\n]\n\n[[package]]\nname = \"mente-b\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"top\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\ndependencies = [\n \"leaf 2.0.0\",\n]\n\n[[package]]\nname = \"leaf\"\nversion = \"2.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n[[package]]\nname = \"other\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n";
    let wrong_dependency = "# comment\ntop: top\nleaf: top\nother: top\n";
    assert_eq!(
        lock_approval_problems(lock, wrong_dependency),
        vec!["other: top does not bring it".to_string()]
    );
    let gained = lock_approval_problems(lock, "top: top\nother: top\n");
    assert_eq!(gained.len(), 2, "{:?}", gained);
    assert!(gained[0].starts_with("Cargo.lock holds leaf, which"));
    let stale = lock_approval_problems(lock, "top: top\nleaf: top\ngone: top\n");
    assert!(stale.contains(&"gone is listed, but Cargo.lock no longer holds it".to_string()));
    assert!(stale
        .iter()
        .any(|p| p.starts_with("Cargo.lock holds other")));
    let indirect = lock_approval_problems(lock, "top: top\nleaf: leaf\nother: top\n");
    assert!(indirect
        .contains(&"leaf: leaf is not a direct dependency of a workspace crate".to_string()));
    let shapes = lock_approval_problems(lock, "top: top\ntop: top\nleaf top\nmente-b: top\n");
    assert!(shapes.contains(&"top is listed twice".to_string()));
    assert!(shapes.contains(&"\"leaf top\" is not \"<crate>: <dependency>\"".to_string()));
    assert!(shapes.contains(&"mente-b is a workspace crate".to_string()));
    // A crate from anywhere but crates.io is refused, listed or not.
    let git = lock.replace(
        "name = \"leaf\"\nversion = \"2.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"",
        "name = \"leaf\"\nversion = \"2.0.0\"\nsource = \"git+https://example.invalid/leaf#0123\"",
    );
    assert_ne!(git, lock);
    assert_eq!(
        lock_approval_problems(&git, "top: top\nleaf: top\nother: other\n"),
        vec![
            "leaf comes from git+https://example.invalid/leaf#0123, not crates.io: only a crate of crates.io can be approved".to_string(),
            "other: other is not a direct dependency of a workspace crate".to_string(),
        ]
    );
    // A [[patch.unused]] section adds nothing to the package before it.
    let patched = format!(
        "{lock}\n[[patch.unused]]\nname = \"evil\"\nversion = \"1.0.0\"\nsource = \"git+https://example.invalid/evil\"\n"
    );
    assert_eq!(
        lock_packages(&patched).keys().collect::<Vec<_>>(),
        vec!["leaf", "mente-a", "mente-b", "other", "top"]
    );
    assert_eq!(
        lock_packages(&patched)["other"].0,
        BTreeSet::from([CRATES_IO.to_string()])
    );
}

/// The lines under `root` that hold a CJK character, as `path:line`. Build output, dependencies
/// and nested checkouts are skipped: a directory holding `.git`, such as a worktree under `.wt`
/// or `.claude/worktrees`, is another branch's files, which this tree does not own.
fn non_english_lines(root: &Path) -> Vec<String> {
    // The same ranges as CJK_RANGES in .github/scripts/lib/commits.mjs, which checks commit
    // messages and pull requests; a test of the workflow policy keeps the two equal.
    fn is_cjk(c: char) -> bool {
        matches!(c as u32,
            0x1100..=0x11FF     // Hangul Jamo
            | 0x2E80..=0x2FDF   // CJK and Kangxi radicals
            | 0x2FF0..=0x9FFF   // description characters, punctuation, kana, Bopomofo, Hangul
                                // compatibility Jamo, Kanbun, strokes, enclosed and
                                // compatibility forms, and the unified ideographs
            | 0xA960..=0xA97F   // Hangul Jamo extended A
            | 0xAC00..=0xD7FF   // Hangul syllables, Hangul Jamo extended B
            | 0xF900..=0xFAFF   // CJK compatibility ideographs
            | 0xFE30..=0xFE4F   // CJK compatibility forms
            | 0xFF00..=0xFFEF   // full-width forms
            | 0x20000..=0x3FFFF) // the supplementary ideographic planes
    }
    fn walk(dir: &Path, found: &mut Vec<String>) {
        let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                let skipped = matches!(name.as_str(), ".git" | "target" | "node_modules");
                if !skipped && !path.join(".git").exists() {
                    walk(&path, found);
                }
            } else if let Ok(text) = fs::read_to_string(&path) {
                for (i, line) in text.lines().enumerate() {
                    if line.chars().any(is_cjk) {
                        found.push(format!("{}:{}", path.display(), i + 1));
                    }
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(root, &mut found);
    found
}

/// The check reads every directory of the tree but a nested checkout, whose files belong to
/// another branch (`guard-audit` in `docs/design-review.md`).
#[test]
fn test_the_english_check_skips_nested_checkouts_only() {
    let root = std::env::temp_dir().join(format!("mente-english-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    // Written at run time, so this file itself stays English.
    let cjk: String = [char::from_u32(0x4E2D).unwrap()].iter().collect();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join(".wt/other")).unwrap();
    fs::write(root.join("docs/a.md"), format!("text {cjk}\n")).unwrap();
    fs::write(root.join(".wt/other/.git"), "gitdir: elsewhere\n").unwrap();
    fs::write(root.join(".wt/other/b.md"), format!("text {cjk}\n")).unwrap();
    let found = non_english_lines(&root);
    let _ = fs::remove_dir_all(&root);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].ends_with(":1") && found[0].contains("a.md"),
        "{found:?}"
    );
}

/// The check finds a character of every CJK block, at either end of each range, and passes
/// Latin text with accents, dashes and emoji.
#[test]
fn test_the_english_check_finds_every_cjk_block() {
    let root = std::env::temp_dir().join(format!("mente-cjk-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    // Written at run time, so this file itself stays English.
    let blocks = [
        0x1100, 0x11FF, 0x2E80, 0x2FDF, 0x2FF0, 0x3000, 0x3105, 0x3131, 0x31A0, 0x3200, 0x33FF,
        0x4E2D, 0x9FFF, 0xA960, 0xAC00, 0xD7FF, 0xF900, 0xFE30, 0xFF01, 0x20000, 0x3FFFF,
    ];
    let mut text = String::from("caf\u{e9}, \u{2014} \u{1F916}\n");
    for code in blocks {
        text.push(char::from_u32(code).unwrap());
        text.push('\n');
    }
    fs::write(root.join("a.md"), text).unwrap();
    let found = non_english_lines(&root);
    let _ = fs::remove_dir_all(&root);
    let lines: Vec<String> = (2..=blocks.len() + 1).map(|l| format!(":{l}")).collect();
    assert_eq!(found.len(), blocks.len(), "{found:?}");
    assert!(
        found.iter().zip(&lines).all(|(f, l)| f.ends_with(l)),
        "{found:?}"
    );
}

/// A JSON value as the guard reads package.json: every key of an object in order, duplicates kept,
/// so that a second `"scripts"` key cannot hide behind the first (npm's `JSON.parse` keeps the
/// last), and every escape decoded as `JSON.parse` decodes it, so an escaped key still names `scripts`.
#[derive(Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// Reads a whole JSON text, refusing anything after its value.
fn parse_json(text: &str) -> Result<Json, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    let value = json_value(&chars, &mut at)?;
    json_space(&chars, &mut at);
    if at != chars.len() {
        return Err(format!("text after the value at {at}"));
    }
    Ok(value)
}

fn json_space(chars: &[char], at: &mut usize) {
    while chars
        .get(*at)
        .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
    {
        *at += 1;
    }
}

fn json_value(chars: &[char], at: &mut usize) -> Result<Json, String> {
    json_space(chars, at);
    match chars.get(*at) {
        Some('{') => {
            *at += 1;
            let mut entries = Vec::new();
            json_space(chars, at);
            if chars.get(*at) == Some(&'}') {
                *at += 1;
                return Ok(Json::Object(entries));
            }
            loop {
                json_space(chars, at);
                let key = json_string(chars, at)?;
                json_space(chars, at);
                if chars.get(*at) != Some(&':') {
                    return Err(format!("no `:` after a key at {at}"));
                }
                *at += 1;
                entries.push((key, json_value(chars, at)?));
                json_space(chars, at);
                match chars.get(*at) {
                    Some(',') => *at += 1,
                    Some('}') => {
                        *at += 1;
                        return Ok(Json::Object(entries));
                    }
                    _ => return Err(format!("no `,` or `}}` at {at}")),
                }
            }
        }
        Some('[') => {
            *at += 1;
            let mut items = Vec::new();
            json_space(chars, at);
            if chars.get(*at) == Some(&']') {
                *at += 1;
                return Ok(Json::Array(items));
            }
            loop {
                items.push(json_value(chars, at)?);
                json_space(chars, at);
                match chars.get(*at) {
                    Some(',') => *at += 1,
                    Some(']') => {
                        *at += 1;
                        return Ok(Json::Array(items));
                    }
                    _ => return Err(format!("no `,` or `]` at {at}")),
                }
            }
        }
        Some('"') => Ok(Json::Str(json_string(chars, at)?)),
        Some(_) => {
            let start = *at;
            while chars
                .get(*at)
                .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.'))
            {
                *at += 1;
            }
            let word: String = chars[start..*at].iter().collect();
            match word.as_str() {
                "null" => Ok(Json::Null),
                "true" => Ok(Json::Bool(true)),
                "false" => Ok(Json::Bool(false)),
                _ if word.parse::<f64>().is_ok() => Ok(Json::Number(word)),
                _ => Err(format!("not a JSON value at {start}: {word:?}")),
            }
        }
        None => Err("the text ends before a value".to_string()),
    }
}

fn json_string(chars: &[char], at: &mut usize) -> Result<String, String> {
    if chars.get(*at) != Some(&'"') {
        return Err(format!("no string at {at}"));
    }
    *at += 1;
    let mut out = String::new();
    let hex = |at: usize| -> Result<u32, String> {
        let digits: String = chars
            .get(at..at + 4)
            .ok_or("a short \\u escape")?
            .iter()
            .collect();
        u32::from_str_radix(&digits, 16).map_err(|_| format!("a bad \\u escape at {at}"))
    };
    loop {
        match chars.get(*at) {
            None => return Err("a string that does not end".to_string()),
            Some('"') => {
                *at += 1;
                return Ok(out);
            }
            Some('\\') => {
                let escaped = chars.get(*at + 1).ok_or("an escape at the end")?;
                *at += 2;
                match escaped {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let mut code = hex(*at)?;
                        *at += 4;
                        // A high surrogate and its low one make one character.
                        if (0xD800..0xDC00).contains(&code)
                            && chars.get(*at) == Some(&'\\')
                            && chars.get(*at + 1) == Some(&'u')
                        {
                            let low = hex(*at + 2)?;
                            if (0xDC00..0xE000).contains(&low) {
                                code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                *at += 6;
                            }
                        }
                        out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    c => return Err(format!("an unknown escape \\{c}")),
                }
            }
            Some(c) => {
                out.push(*c);
                *at += 1;
            }
        }
    }
}

#[test]
fn test_the_guard_reads_json_as_npm_does() {
    let text = "{\"scr\\u0069pts\": {\"a\": \"x \\\"y\\\"\"}, \"n\": [1, -2.5e3, true, null], \"scripts\": {}}";
    let Ok(Json::Object(top)) = parse_json(text) else {
        panic!("not an object")
    };
    let keys: Vec<&str> = top.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec!["scripts", "n", "scripts"],
        "an escaped key reads as npm reads it, and a duplicate stays"
    );
    assert_eq!(
        top[0].1,
        Json::Object(vec![("a".to_string(), Json::Str("x \"y\"".to_string()))])
    );
    assert_eq!(
        top[1].1,
        Json::Array(vec![
            Json::Number("1".to_string()),
            Json::Number("-2.5e3".to_string()),
            Json::Bool(true),
            Json::Null
        ])
    );
    assert_eq!(
        parse_json("\"\\ud83e\\udd16\""),
        Ok(Json::Str("\u{1F916}".to_string()))
    );
    for bad in ["{", "{\"a\" 1}", "[1,]", "\"open", "{} x", "nope"] {
        assert!(parse_json(bad).is_err(), "{bad}");
    }
}

/// The scripts of package.json the guard reads: its one `scripts` object, as (name, command)
/// pairs, or why there is not exactly one.
fn package_scripts(text: &str) -> Result<Vec<(String, String)>, String> {
    let Json::Object(top) = parse_json(text)? else {
        return Err("package.json is not an object".to_string());
    };
    let found: Vec<&Json> = top
        .iter()
        .filter(|(k, _)| k == "scripts")
        .map(|(_, v)| v)
        .collect();
    let [Json::Object(entries)] = found.as_slice() else {
        return Err(format!(
            "package.json holds {} scripts objects, not one",
            found.len()
        ));
    };
    entries
        .iter()
        .map(|(k, v)| match v {
            Json::Str(command) => Ok((k.clone(), command.clone())),
            _ => Err(format!("the script {k} is not a string")),
        })
        .collect()
}

/// The step of ci.yml's Documentation job that runs the workflow policy, exactly: a name and a
/// command, with no shell, environment, directory or condition of its own, run through node itself.
const POLICY_STEP: [&str; 2] = [
    "      - name: The workflow policy, its required checks, path lists, pins and milestone tags",
    "        run: node --test .github/scripts/policy.test.mjs .github/scripts/yaml.test.mjs",
];

/// The steps before it, exactly: the checkout and node. Nothing has run when the policy does.
const BEFORE_POLICY: [&str; 2] = [
    "      - uses: actions/checkout@",
    "      - uses: actions/setup-node@",
];

/// The npm scripts, exactly: no `pre`, `post` or lifecycle script can run before or around one of
/// them, and `test:ci` runs the policy's suites with no option that could skip a test.
const SCRIPTS: [(&str, &str); 13] = [
    (
        "spec-guard",
        r#"spec-guard "spec/*.md" "docs/*.md" README.md ROADMAP.md CLAUDE.md"#,
    ),
    (
        "spec-guard:prove",
        r#"spec-guard prove "spec/*.md" "docs/*.md" README.md ROADMAP.md CLAUDE.md"#,
    ),
    ("spec-guard:claims", "spec-guard claims"),
    ("spec-graph", "spec-graph check"),
    (
        "spec-graph:whole",
        "spec-graph check --format json | node .github/scripts/graph-whole.mjs",
    ),
    ("spec-brief", "spec-brief lint --strict"),
    ("check:benchmark", "node benchmark/check.mjs"),
    ("test:benchmark", "node --test benchmark/check.test.mjs"),
    ("test:harness", "node --test benchmark/harness/harness.test.ts"),
    ("site", "node site/build.mjs"),
    ("test:site", "node --test site/site.test.mjs"),
    (
        "test:ci",
        "node --test .github/scripts/policy.test.mjs .github/scripts/yaml.test.mjs",
    ),
    (
        "verify:docs",
        "npm run spec-guard && npm run spec-guard:prove && npm run spec-guard:claims && npm run spec-graph && npm run spec-graph:whole && npm run spec-brief",
    ),
];

/// The workflow policy runs where it blocks a merge, as written. ci.yml's Documentation job, a
/// required check, runs it first, after only the checkout and node, through node itself, in a step
/// of a name and a command alone; `npm ci` runs after it with no install script; the job sets
/// nothing but its name, runner and steps, and none of its scripts writes the environment or the
/// path of a later step; the workflow sets no shell and no environment but cargo's colours. The npm
/// scripts are exactly SCRIPTS, with no hook, and no `.npmrc` changes npm. The policy's suites mark
/// no test skipped, todo or only, and the policy declares a floor equal to its number of checks,
/// which a run that skips any fails. Without this, a step deleted, a hook or an install script, a
/// skip pattern, an option in `NODE_OPTIONS` or a shell that runs nothing would turn every check of
/// the policy off unseen.
#[test]
fn test_the_documentation_job_runs_the_workflow_policy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let top: Vec<&str> = ci.lines().take_while(|l| *l != "jobs:").collect();
    assert!(
        !top.iter().any(|l| l.starts_with("defaults:")),
        "ci.yml sets a default shell"
    );
    let env: Vec<&str> = top
        .iter()
        .skip_while(|l| **l != "env:")
        .skip(1)
        .take_while(|l| l.starts_with(' ') || l.is_empty())
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .copied()
        .collect();
    assert_eq!(
        env,
        vec!["  CARGO_TERM_COLOR: always"],
        "ci.yml's environment changed"
    );
    let job: Vec<&str> = ci
        .split("\n  docs:\n")
        .nth(1)
        .expect("ci.yml has a docs job")
        .lines()
        .take_while(|l| l.is_empty() || l.starts_with("    "))
        .collect();
    let keys: Vec<&str> = job
        .iter()
        .filter(|l| l.starts_with("    ") && !l.starts_with("     "))
        .filter_map(|l| l.trim().split(':').next())
        .filter(|k| !k.starts_with('#'))
        .collect();
    assert_eq!(
        keys,
        vec!["name", "runs-on", "timeout-minutes", "steps"],
        "the Documentation job sets more than its name, runner, time limit and steps"
    );
    assert!(job.contains(&"    name: Documentation"));
    // The steps, by the lines that open them.
    let steps: Vec<&str> = job
        .iter()
        .filter(|l| l.starts_with("      - "))
        .copied()
        .collect();
    let at = steps
        .iter()
        .position(|l| *l == POLICY_STEP[0])
        .expect("the Documentation job has no workflow policy step");
    assert_eq!(at, BEFORE_POLICY.len(), "the policy step is not third");
    for (step, start) in steps.iter().zip(BEFORE_POLICY) {
        assert!(step.starts_with(start), "a step before the policy: {step}");
    }
    let line = job.iter().position(|l| *l == POLICY_STEP[0]).unwrap();
    assert_eq!(job[line + 1], POLICY_STEP[1], "the policy step changed");
    assert!(
        job.get(line + 2)
            .is_some_and(|l| l.starts_with("      - ") || l.trim().starts_with('#')),
        "the policy step carries more than its name and command"
    );
    assert!(
        job[..line].iter().all(|l| !l.contains("run:")),
        "something runs before the policy"
    );
    assert!(
        steps.contains(&"      - run: npm ci --ignore-scripts"),
        "npm ci runs install scripts"
    );
    for word in [
        "GITHUB_ENV",
        "GITHUB_PATH",
        "NODE_OPTIONS",
        "continue-on-error",
        "shell:",
        "npm install",
    ] {
        assert!(
            !job.iter().any(|l| l.contains(word)),
            "the Documentation job names {word}"
        );
    }
    assert!(
        job.iter()
            .filter(|l| !l.trim().starts_with('#') && l.contains("npm ci"))
            .all(|l| *l == "      - run: npm ci --ignore-scripts"),
        "npm ci runs in another form"
    );
    assert!(
        !job.iter().any(|l| l.trim().starts_with("if:")),
        "a step of the Documentation job is conditional"
    );
    let required = fs::read_to_string(root.join(".github/required-checks.txt")).unwrap();
    assert!(required.lines().any(|l| l == "Documentation"));
    // The npm scripts, exactly, read as npm reads them: one scripts object, these pairs in this
    // order; and no project .npmrc.
    let package = fs::read_to_string(root.join("package.json")).unwrap();
    let scripts = package_scripts(&package).unwrap_or_else(|e| panic!("{e}"));
    let expected: Vec<(String, String)> = SCRIPTS
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert_eq!(scripts, expected, "package.json's scripts changed");
    // A second scripts object, which npm would read instead, is refused.
    let doubled = package.replacen(
        "\"devDependencies\"",
        "\"scripts\": {}, \"devDependencies\"",
        1,
    );
    assert_ne!(doubled, package);
    assert!(package_scripts(&doubled).is_err());
    assert!(
        !root.join(".npmrc").exists(),
        "a .npmrc changes what npm runs"
    );
    // No test of the policy's suites is skipped, left to do or singled out.
    for suite in [
        ".github/scripts/policy.test.mjs",
        ".github/scripts/yaml.test.mjs",
    ] {
        let text = fs::read_to_string(root.join(suite)).unwrap();
        for marker in [".skip(", ".todo(", ".only(", "skip:", "todo:", "only:"] {
            assert!(
                !text.contains(marker),
                "{suite} holds `{marker}`: a test it would not run"
            );
        }
    }
    // The policy's floor of checks equals the checks it declares.
    let policy = fs::read_to_string(root.join(".github/scripts/policy.test.mjs")).unwrap();
    let checks = policy.lines().filter(|l| l.starts_with("check(\"")).count();
    let floor: usize = policy
        .lines()
        .find_map(|l| l.strip_prefix("const FLOOR = "))
        .and_then(|v| v.trim_end_matches(';').parse().ok())
        .expect("the policy declares no FLOOR");
    assert!(checks > 0);
    assert_eq!(
        floor, checks,
        "the policy's FLOOR is not the number of its checks"
    );
    assert!(policy.contains("after(() => {\n  assert.ok(ran >= FLOOR"));
}

/// Every text file in the repository is English: no CJK characters in code, comments, docs or
/// examples (CLAUDE.md). Build output, dependencies and nested checkouts are skipped.
#[test]
fn test_repository_text_is_english() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let found = non_english_lines(&root);
    assert!(
        found.is_empty(),
        "non-English text at:\n{}",
        found.join("\n")
    );
}
