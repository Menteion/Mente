//! Each Rust item the documents name by a path rooted at one of the workspace's crates or source
//! modules, such as `mir_run::HOST_ALLOWANCE`, is defined in the compiler's source: a name that a
//! change removes or renames fails here, where it would otherwise mislead the next reader
//! ([`guard-per-problem`](../../../docs/design-review.md#guard-per-problem)).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The repository's root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The files under `dir` whose names end in `ext`, past build output, installed packages and
/// hidden directories.
fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "target" || name == "node_modules" || name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            files(&path, ext, out);
        } else if name.ends_with(ext) {
            out.push(path);
        }
    }
}

/// The identifiers of a line, in order.
fn identifiers(line: &str) -> Vec<&str> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| w.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
        .collect()
}

/// What the compiler's source defines, by name: each function, constant, static, type, module and
/// trait, each enum's variants, read as a line that begins with a capitalised name, and each
/// source file's module; and the roots a document's path may start from: each crate, and each
/// module of a crate's source.
fn defined(compiler: &Path) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut rust = Vec::new();
    files(compiler, ".rs", &mut rust);
    let mut names = BTreeSet::new();
    let mut roots: BTreeSet<String> = ["crate", "super"].map(String::from).into();
    if let Ok(crates) = fs::read_dir(compiler) {
        for c in crates.flatten() {
            roots.insert(c.file_name().to_string_lossy().replace('-', "_"));
        }
    }
    for path in &rust {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let in_src = path.components().any(|c| c.as_os_str() == "src");
        if in_src && !["lib", "main", "mod"].contains(&stem.as_str()) {
            roots.insert(stem.clone());
        }
        names.insert(stem);
        let text = fs::read_to_string(path).unwrap_or_default();
        for line in text.lines().map(str::trim).filter(|l| !l.starts_with("//")) {
            let ids = identifiers(line);
            for pair in ids.windows(2) {
                if [
                    "fn", "const", "static", "struct", "enum", "type", "mod", "trait",
                ]
                .contains(&pair[0])
                {
                    names.insert(pair[1].to_string());
                }
            }
            if let Some(first) = ids
                .first()
                .filter(|w| w.starts_with(|c: char| c.is_ascii_uppercase()))
            {
                let after = line[first.len()..].trim_start();
                if line.starts_with(first)
                    && (after.is_empty() || after.starts_with(['(', '{', ',']))
                {
                    names.insert(first.to_string());
                }
            }
        }
    }
    names.extend(roots.iter().cloned());
    (names, roots)
}

/// The paths a text names in backticks, `a::b` or `a::b()`, each with its line.
fn paths(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (k, quoted) in line.split('`').enumerate() {
            let quoted = quoted.strip_suffix("()").unwrap_or(quoted);
            let segments: Vec<&str> = quoted.split("::").collect();
            let path =
                k % 2 == 1 && segments.len() > 1 && segments.iter().all(|s| identifiers(s) == [*s]);
            if path {
                out.push((i + 1, quoted.to_string()));
            }
        }
    }
    out
}

/// Every path the documents name from a crate or a module of the workspace names items the
/// source defines; a path to an item since removed, as `mir_run::MAX_NESTING` once was, fails.
#[test]
fn test_documents_name_only_items_the_source_defines() {
    let root = root();
    let (names, roots) = defined(&root.join("compiler"));
    let mut docs = Vec::new();
    for dir in ["docs", "spec"] {
        files(&root.join(dir), ".md", &mut docs);
    }
    for file in ["ROADMAP.md", "CLAUDE.md", "README.md"] {
        docs.push(root.join(file));
    }
    let mut checked = 0;
    let mut stale = Vec::new();
    for doc in &docs {
        let text = fs::read_to_string(doc).unwrap_or_default();
        for (line, path) in paths(&text) {
            let segments: Vec<&str> = path.split("::").collect();
            if !roots.contains(segments[0]) {
                continue;
            }
            checked += 1;
            if segments.iter().any(|s| !names.contains(*s)) {
                stale.push(format!("{}:{}: `{}`", doc.display(), line, path));
            }
        }
    }
    assert!(checked > 0, "no document names a path of the workspace");
    assert!(
        stale.is_empty(),
        "the documents name items the source does not define:\n{}",
        stale.join("\n")
    );
    // The reading itself: a path of the workspace is read and checked, and one to no item fails.
    let (_, sample) =
        paths("the allowance, `mir_run::HOST_ALLOWANCE`, and `mir_run::MAX_NESTING()`")
            .into_iter()
            .unzip::<_, _, Vec<_>, Vec<_>>();
    assert_eq!(sample, ["mir_run::HOST_ALLOWANCE", "mir_run::MAX_NESTING"]);
    assert!(names.contains("HOST_ALLOWANCE") && !names.contains("MAX_NESTING"));
}
