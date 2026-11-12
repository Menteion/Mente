//! The tests that hold the repository's gates: the dependency policy and the crates `Cargo.lock`
//! may hold, English-only text, and the workflow policy's place in the required Documentation job.
//! They live apart from main.rs, which changes with every feature, so that `.github/CODEOWNERS`
//! can give them to their owner (`docs/design-review.md`, `guard-per-problem`).

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
        for entry in fs::read_dir(dir).unwrap().flatten() {
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

/// The step of ci.yml's Documentation job that runs the workflow policy, exactly: a name and a
/// command, with no shell, environment, directory or condition of its own.
const POLICY_STEP: [&str; 2] = [
    "      - name: The workflow policy, its required checks, path lists, pins and milestone tags",
    "        run: npm run test:ci",
];

/// The command `npm run test:ci` runs, exactly: both suites of the policy, with no option that
/// could skip a test.
const TEST_CI: &str =
    "\"test:ci\": \"node --test .github/scripts/policy.test.mjs .github/scripts/yaml.test.mjs\",";

/// The workflow policy runs where it blocks a merge, as written. ci.yml's Documentation job, a
/// required check, runs `npm run test:ci` in a step of a name and a command alone; the job sets
/// nothing but its name, runner and steps, and none of its scripts writes the environment or the
/// path of a later step; the workflow sets no shell and no environment but cargo's colours.
/// `test:ci` is exactly the command that runs the policy's suites, and the policy declares a floor
/// equal to its number of checks, which a run that skips any fails. Without this, a step deleted,
/// a skip pattern, an option in `NODE_OPTIONS` or a shell that runs nothing would turn every check
/// of the policy off unseen.
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
        vec!["name", "runs-on", "steps"],
        "the Documentation job sets more than its name, runner and steps"
    );
    assert!(job.contains(&"    name: Documentation"));
    let at = job
        .iter()
        .position(|l| *l == POLICY_STEP[0])
        .expect("the Documentation job has no workflow policy step");
    assert_eq!(job[at + 1], POLICY_STEP[1], "the policy step changed");
    assert!(
        job.get(at + 2).is_some_and(|l| l.starts_with("      - ")),
        "the policy step carries more than its name and command"
    );
    for word in [
        "GITHUB_ENV",
        "GITHUB_PATH",
        "NODE_OPTIONS",
        "continue-on-error",
        "shell:",
    ] {
        assert!(
            !job.iter().any(|l| l.contains(word)),
            "the Documentation job names {word}"
        );
    }
    assert!(
        !job.iter().any(|l| l.trim().starts_with("if:")),
        "a step of the Documentation job is conditional"
    );
    let required = fs::read_to_string(root.join(".github/required-checks.txt")).unwrap();
    assert!(required.lines().any(|l| l == "Documentation"));
    let package = fs::read_to_string(root.join("package.json")).unwrap();
    assert!(
        package.lines().any(|l| l.trim() == TEST_CI),
        "test:ci is not exactly {TEST_CI}"
    );
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
