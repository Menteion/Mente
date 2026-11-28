//! The tests that hold the repository's gates: the dependency policy and the crates `Cargo.lock`
//! may hold, English-only text, and the workflow policy's place in the required Documentation job.
//! They are an integration test of their own, apart from main.rs, which changes with every feature:
//! nothing there can unhook them, and `.github/CODEOWNERS` gives them to their owner
//! (`docs/design-review.md`, `guard-per-problem`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// JSON as npm reads it, defined once in the library.
use mente_cli::json::{parse as parse_json, Json};

/// Where a crate of the lock comes from: the workspace, a member the root manifest names; crates.io;
/// or anywhere else, another registry, a git repository or a path outside the workspace.
#[derive(Clone, Debug, PartialEq)]
enum Origin {
    Workspace,
    CratesIo,
    Elsewhere(String),
}

impl Origin {
    /// Where the crate `name` with `source` (none for a path) comes from.
    fn of(name: &str, source: Option<&str>, members: &BTreeSet<String>) -> Origin {
        match source {
            None if members.contains(name) => Origin::Workspace,
            None => Origin::Elsewhere("a path outside the workspace".to_string()),
            Some(CRATES_IO) => Origin::CratesIo,
            Some(other) => Origin::Elsewhere(other.to_string()),
        }
    }
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Workspace => write!(f, "the workspace"),
            Origin::CratesIo => write!(f, "crates.io"),
            Origin::Elsewhere(place) => write!(f, "{place}"),
        }
    }
}

/// The dependency policy of CLAUDE.md, applied to one crate of the workspace and its dependencies,
/// each with where it comes from: a compiler crate depends only on the workspace's own crates;
/// `mente-mir` depends on nothing; the reference interpreter never on `mente-mir` or
/// `mente-codegen`; only `mente-codegen` may add Cranelift, and `gimli` and `object` for its object
/// files and debug information (Norman, 2026-11-10), from crates.io; and nothing comes from
/// anywhere else. Returns each dependency outside the policy.
fn dependency_policy_violations(krate: &str, deps: &[(String, Origin)]) -> Vec<String> {
    deps.iter()
        .filter(|(name, origin)| {
            let allowed = if name.starts_with("mente-") {
                *origin == Origin::Workspace
                    && krate != "mente-mir"
                    && !(krate == "mente-interp"
                        && matches!(name.as_str(), "mente-mir" | "mente-codegen"))
            } else {
                (name.starts_with("cranelift") || matches!(name.as_str(), "gimli" | "object"))
                    && krate == "mente-codegen"
                    && *origin == Origin::CratesIo
            };
            !allowed
        })
        .map(|(name, origin)| format!("{krate}: {name}, from {origin}"))
        .collect()
}

/// The members of the workspace, by package name: each path the root manifest's `[workspace]`
/// lists in `members`, read by `member`, which gives that path's manifest, as the name its
/// `[package]` declares. A path matching no manifest is no member.
fn workspace_members(root: &str, member: &dyn Fn(&str) -> Option<String>) -> BTreeSet<String> {
    // The value of `key` in `section` of a manifest, from its `=` to the end of the value: one line,
    // or an array to its `]`.
    fn value<'a>(manifest: &'a str, section: &str, key: &str) -> Option<&'a str> {
        let start = manifest.find(&format!("\n[{section}]")).map(|at| at + 1)?;
        let body = &manifest[start + section.len() + 2..];
        let body = &body[..body.find("\n[").unwrap_or(body.len())];
        let at = body.lines().find(|l| {
            l.trim_start()
                .strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        })?;
        let from = body.find(at)? + at.find('=')? + 1;
        let rest = &body[from..];
        let end = if rest.trim_start().starts_with('[') {
            rest.find(']')? + 1
        } else {
            rest.find('\n').unwrap_or(rest.len())
        };
        Some(&rest[..end])
    }
    // The strings of a value, in either quote, comments left out.
    fn strings(value: &str) -> Vec<String> {
        let mut out = Vec::new();
        for line in value.lines() {
            let line = line.split('#').next().unwrap_or("");
            let mut rest = line;
            while let Some(at) = rest.find(['"', '\'']) {
                let quote = rest[at..].chars().next().unwrap_or('"');
                let inner = &rest[at + 1..];
                let Some(end) = inner.find(quote) else {
                    break;
                };
                out.push(inner[..end].to_string());
                rest = &inner[end + 1..];
            }
        }
        out
    }
    let root = format!("\n{root}");
    let paths = value(&root, "workspace", "members")
        .map(strings)
        .unwrap_or_default();
    paths
        .iter()
        .filter_map(|p| {
            let manifest = format!("\n{}", member(p)?);
            strings(value(&manifest, "package", "name")?)
                .into_iter()
                .next()
        })
        .collect()
}

/// The members of this repository's workspace.
fn members_here(root: &Path) -> BTreeSet<String> {
    workspace_members(
        &fs::read_to_string(root.join("Cargo.toml"))
            .unwrap_or_else(|e| panic!("the root manifest: {e}")),
        &|p| fs::read_to_string(root.join(p).join("Cargo.toml")).ok(),
    )
}

/// Each member of the workspace `lock` holds, with its dependencies, each with where it comes from.
/// A lock is cargo's own reading of every manifest, and CI builds with `--locked`, which refuses a
/// lock that does not match them: a dependency stands in it under the crate's own name, however
/// its manifest spells it, renamed with `package` in either quote, in a `[dependencies.<name>]`
/// table, under a target or as a dev-dependency. The tests cannot run `cargo metadata` itself,
/// since their sandbox holds no toolchain (.github/scripts/test-sandbox.sh). A crate with no
/// source is the workspace's only if the root manifest names it a member.
fn workspace_dependencies(
    lock: &str,
    members: &BTreeSet<String>,
) -> BTreeMap<String, Vec<(String, Origin)>> {
    let entries = lock_entries(lock);
    // A dependency as the lock writes it, `name`, or `name version` beside a second version.
    let origin = |dep: &str| {
        let mut words = dep.split(' ');
        let name = words.next().unwrap_or("");
        let version = words.next();
        let same: Vec<&LockEntry> = entries
            .iter()
            .filter(|e| e.name == name && version.is_none_or(|v| e.version == v))
            .collect();
        match same.as_slice() {
            [one] => Origin::of(name, one.source.as_deref(), members),
            _ => Origin::Elsewhere("no one crate of the lock".to_string()),
        }
    };
    entries
        .iter()
        .filter(|e| Origin::of(&e.name, e.source.as_deref(), members) == Origin::Workspace)
        .map(|e| {
            let deps = e
                .dependencies
                .iter()
                .map(|d| (d.split(' ').next().unwrap_or(d).to_string(), origin(d)))
                .collect();
            (e.name.clone(), deps)
        })
        .collect()
}

/// A new external dependency needs Norman's approval and a change to this policy.
#[test]
fn test_crates_depend_only_on_allowed_crates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let members = members_here(&root);
    let crates = workspace_dependencies(
        &fs::read_to_string(root.join("Cargo.lock")).unwrap(),
        &members,
    );
    // Every crate of compiler/ is in the lock, read by its name.
    let mut compiler = 0;
    for entry in fs::read_dir(root.join("compiler")).unwrap().flatten() {
        if entry.path().join("Cargo.toml").exists() {
            compiler += 1;
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(
                crates.contains_key(&name),
                "Cargo.lock holds no member {name}"
            );
        }
    }
    assert!(
        compiler >= 8,
        "expected the compiler's crates, read {compiler}"
    );
    let found: Vec<String> = crates
        .iter()
        .filter(|(name, _)| name.starts_with("mente-"))
        .flat_map(|(name, deps)| dependency_policy_violations(name, deps))
        .collect();
    assert!(
        found.is_empty(),
        "dependencies outside the policy:\n{}",
        found.join("\n")
    );

    // The policy on a lock, which writes a dependency's version where it holds two of the crate.
    // Whatever form a manifest gives a dependency, the lock names the crate
    // itself: `evil` here stands for `x = { package = "evil" }`, `x = { package = 'evil' }`,
    // `[dependencies.evil]`, `[target.'cfg(windows)'.dependencies] evil = "1"` and
    // `[dev-dependencies] evil = "1"` alike, so that no spelling reaches the policy.
    let package = |name: &str, version: &str, source: bool, deps: &[&str]| {
        let source = if source {
            format!("source = \"{CRATES_IO}\"\n")
        } else {
            String::new()
        };
        let deps = if deps.is_empty() {
            String::new()
        } else {
            let list: Vec<String> = deps.iter().map(|d| format!(" \"{d}\",\n")).collect();
            format!("dependencies = [\n{}]\n", list.concat())
        };
        format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n{source}{deps}\n")
    };
    let lock = [
        "version = 4\n\n".to_string(),
        package(
            "mente-sema",
            "0.1.0",
            false,
            &["mente-ast 0.1.0", "cranelift-codegen 0.137.1", "evil"],
        ),
        package("mente-ast", "0.1.0", false, &[]),
        package(
            "mente-codegen",
            "0.1.0",
            false,
            &[
                "cranelift-codegen 0.137.1",
                "cranelift-vendored",
                "evil",
                "gimli",
                "object",
            ],
        ),
        package(
            "mente-interp",
            "0.1.0",
            false,
            &["mente-sema", "mente-mir", "mente-codegen"],
        ),
        package("mente-mir", "0.1.0", false, &["mente-ast 0.1.0"]),
        package("mente-cli", "0.1.0", false, &["mente-ast 0.1.1"]),
        package("mente-ast", "0.1.1", true, &[]),
        package("cranelift-codegen", "0.137.1", true, &[]),
        package("cranelift-codegen", "0.136.0", true, &[]),
        package("evil", "1.0.0", true, &[]),
        package("gimli", "0.33.0", true, &[]),
        package("object", "0.40.0", true, &[]),
        // A crate at a path outside the workspace, as `cranelift-vendored = { path = ".." }`
        // gives, with no source in the lock, and what it brings.
        package("cranelift-vendored", "0.1.0", false, &["serde"]),
        package("serde", "1.0.0", true, &[]),
    ]
    .concat();
    let members: BTreeSet<String> = [
        "mente-sema",
        "mente-ast",
        "mente-codegen",
        "mente-interp",
        "mente-mir",
        "mente-cli",
    ]
    .map(String::from)
    .into();
    let crates = workspace_dependencies(&lock, &members);
    // A crate with no source is no member for that: the policy reads the members alone.
    assert!(!crates.contains_key("cranelift-vendored"));
    let refused = |name: &str| dependency_policy_violations(name, &crates[name]);
    assert_eq!(
        refused("mente-sema"),
        [
            "mente-sema: cranelift-codegen, from crates.io",
            "mente-sema: evil, from crates.io"
        ]
    );
    // Cranelift, gimli and object are mente-codegen's alone, from crates.io, and a second version
    // of a crate is read by the one the dependency names; a crate at a path outside the workspace
    // is refused whatever its name.
    assert_eq!(
        refused("mente-codegen"),
        [
            "mente-codegen: cranelift-vendored, from a path outside the workspace",
            "mente-codegen: evil, from crates.io"
        ]
    );
    // The reference interpreter shares no code with the paths it checks, and MIR takes nothing.
    assert_eq!(
        refused("mente-interp"),
        [
            "mente-interp: mente-mir, from the workspace",
            "mente-interp: mente-codegen, from the workspace"
        ]
    );
    assert_eq!(
        refused("mente-mir"),
        ["mente-mir: mente-ast, from the workspace"]
    );
    // A crate named as one of the workspace's but taken from outside it is refused.
    assert_eq!(
        refused("mente-cli"),
        ["mente-cli: mente-ast, from crates.io"]
    );
}

/// The members are the packages the root manifest's `members` names, in either quote, an array
/// over lines with comments, and a path with no manifest is none.
#[test]
fn test_the_members_are_those_the_root_manifest_names() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let here = members_here(&root);
    for name in ["mente-cli", "mente-codegen", "mente-mir", "mente-threads"] {
        assert!(here.contains(name), "{name} is no member: {here:?}");
    }
    let manifest = "[package]\nname = \"x\"\n\n[workspace]\nresolver = \"2\"\nmembers = [\n    \"a\", # the first\n    'b',\n    \"gone\",\n]\nexclude = [\"c\"]\n\n[workspace.package]\nname = \"no\"\n";
    let member = |p: &str| match p {
        "a" => Some("[package]\nname = \"alpha\"\nversion = \"0.1.0\"\n".to_string()),
        "b" => Some("[lib]\nname = \"lib-b\"\n\n[package]\nname = 'beta'\n".to_string()),
        "c" => Some("[package]\nname = \"excluded\"\n".to_string()),
        _ => None,
    };
    let read = workspace_members(manifest, &member);
    assert_eq!(read, ["alpha", "beta"].map(String::from).into());
}

/// The one source a crate of the lock may come from outside the workspace.
const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// One `[[package]]` of a `Cargo.lock`: its name, its version, its source (none for the
/// workspace's own crates), and its dependencies as the lock writes them, `gimli` or, where the
/// lock holds two versions of it, `gimli 0.33.0`.
struct LockEntry {
    name: String,
    version: String,
    source: Option<String>,
    dependencies: Vec<String>,
}

/// The `[[package]]` sections of a `Cargo.lock`, in its order. Only those are read, so that a
/// `[[patch.unused]]` or `[metadata]` section never adds to the package before it.
fn lock_entries(lock: &str) -> Vec<LockEntry> {
    let mut sections: Vec<(bool, Vec<&str>)> = Vec::new();
    for line in lock.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
            sections.push((line == "[[package]]", Vec::new()));
        } else if let Some((_, lines)) = sections.last_mut() {
            lines.push(line);
        }
    }
    let mut entries = Vec::new();
    for (_, lines) in sections.iter().filter(|(package, _)| *package) {
        let mut entry = LockEntry {
            name: String::new(),
            version: String::new(),
            source: None,
            dependencies: Vec::new(),
        };
        let mut in_deps = false;
        for line in lines {
            if in_deps {
                if *line == "]" {
                    in_deps = false;
                } else {
                    let dep = line.trim_matches(|c| c == '"' || c == ',');
                    entry.dependencies.push(dep.to_string());
                }
            } else if let Some(value) = line.strip_prefix("name = ") {
                entry.name = value.trim_matches('"').to_string();
            } else if let Some(value) = line.strip_prefix("version = ") {
                entry.version = value.trim_matches('"').to_string();
            } else if let Some(value) = line.strip_prefix("source = ") {
                entry.source = Some(value.trim_matches('"').to_string());
            } else if *line == "dependencies = [" {
                in_deps = true;
            }
        }
        entries.push(entry);
    }
    entries
}

/// The packages of a `Cargo.lock`: each name, the sources its versions come from (none for
/// the workspace's own crates), and the names it depends on. Versions are left out: a name
/// the lock holds twice is one crate here.
fn lock_packages(lock: &str) -> BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> {
    let mut packages: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for entry in lock_entries(lock) {
        let package = packages.entry(entry.name).or_default();
        package.0.extend(entry.source);
        package.1.extend(
            entry
                .dependencies
                .iter()
                .map(|d| d.split(' ').next().unwrap_or(d).to_string()),
        );
    }
    packages
}

/// The crates cranelift uses in its own signatures, `gimli::write::Address`,
/// `FrameDescriptionEntry`, `object::write::SymbolId`, each with the crate of cranelift whose
/// version it must be.
const WITH_CRANELIFT: [(&str, &str); 2] = [
    ("gimli", "cranelift-codegen"),
    ("object", "cranelift-object"),
];

/// The rule a second version of gimli or object breaks, with its fix.
const MOVE_WITH_CRANELIFT: &str = "cranelift-codegen and cranelift-object use gimli and object in their own signatures, so a version of either apart from cranelift's is a second crate whose types are not cranelift's, and every Rust job fails with E0308, as Dependabot's pull request 164 did: move gimli and object only with cranelift, in the Dependabot group `cranelift` of .github/dependabot.yml, and reach them in mente-codegen only through `cranelift_codegen::gimli` and `cranelift_object::object`";

/// What keeps gimli and object from being cranelift's: a crate the lock holds in more versions
/// than one, or in none, or whose crate of cranelift no longer takes it.
fn cranelift_splits(lock: &str) -> Vec<String> {
    let entries = lock_entries(lock);
    let mut problems = Vec::new();
    for (krate, user) in WITH_CRANELIFT {
        let versions: Vec<&str> = entries
            .iter()
            .filter(|e| e.name == krate)
            .map(|e| e.version.as_str())
            .collect();
        let taken: Vec<&str> = entries
            .iter()
            .filter(|e| e.name == user)
            .flat_map(|e| &e.dependencies)
            .filter(|d| d.split(' ').next() == Some(krate))
            .map(|d| {
                d.split(' ')
                    .nth(1)
                    .unwrap_or(versions.first().copied().unwrap_or(""))
            })
            .collect();
        match (versions.as_slice(), taken.as_slice()) {
            ([_], [_]) => {}
            (_, []) => problems.push(format!("{user} takes no {krate}")),
            (held, [version]) => problems.push(format!(
                "Cargo.lock holds {krate} {}, where {user} takes {version}",
                held.join(" and ")
            )),
            (_, versions) => {
                problems.push(format!("{user} takes {krate} {}", versions.join(" and ")))
            }
        }
    }
    problems
}

/// What keeps a lock from matching the approved list (`<crate>: <direct dependency>` lines):
/// a crate outside the workspace that the list lacks, a listed crate the lock does not hold,
/// and a listed dependency that is not a direct dependency of a workspace crate or does not
/// bring the crate.
fn lock_approval_problems(lock: &str, approved: &str, members: &BTreeSet<String>) -> Vec<String> {
    let packages = lock_packages(lock);
    let local = |name: &str| {
        members.contains(name)
            && packages
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
        if sources.is_empty() && !local(name) {
            problems.push(format!(
                "{name} comes from a path outside the workspace, not crates.io: only a member the \
                 root manifest names, or a crate of crates.io Norman approved, may be in the lock"
            ));
        }
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
            Some(_) if local(krate) => problems.push(format!("{} is a workspace crate", krate)),
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

/// gimli and object each resolve to one version, the one cranelift-codegen and cranelift-object
/// take, so that no update moves them apart from cranelift (Dependabot's pull request 164 bumped
/// them alone, and every Rust job failed).
#[test]
fn test_gimli_and_object_are_cranelifts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let problems = cranelift_splits(&fs::read_to_string(root.join("Cargo.lock")).unwrap());
    assert!(
        problems.is_empty(),
        "{}:\n{}",
        MOVE_WITH_CRANELIFT,
        problems.join("\n")
    );

    // The check itself, on a lock of cranelift and the two crates, then with gimli split.
    let lock = "version = 4\n\n[[package]]\nname = \"cranelift-codegen\"\nversion = \"0.137.1\"\ndependencies = [\n \"gimli\",\n]\n\n[[package]]\nname = \"cranelift-object\"\nversion = \"0.137.1\"\ndependencies = [\n \"gimli\",\n \"object\",\n]\n\n[[package]]\nname = \"gimli\"\nversion = \"0.33.0\"\n\n[[package]]\nname = \"object\"\nversion = \"0.40.0\"\n\n[[package]]\nname = \"mente-codegen\"\nversion = \"0.1.0\"\ndependencies = [\n \"cranelift-codegen\",\n \"object\",\n]\n";
    assert_eq!(cranelift_splits(lock), Vec::<String>::new());
    let split = lock
        .replace(
            "dependencies = [\n \"gimli\",\n]",
            "dependencies = [\n \"gimli 0.33.0\",\n]",
        )
        .replace(
            "dependencies = [\n \"gimli\",\n \"object\",",
            "dependencies = [\n \"gimli 0.33.0\",\n \"object\",",
        )
        .replace(
            "[[package]]\nname = \"object\"",
            "[[package]]\nname = \"gimli\"\nversion = \"0.34.0\"\n\n[[package]]\nname = \"object\"",
        );
    assert_eq!(
        cranelift_splits(&split),
        vec!["Cargo.lock holds gimli 0.33.0 and 0.34.0, where cranelift-codegen takes 0.33.0"]
    );
    let dropped = lock.replace(" \"gimli\",\n \"object\",\n", " \"gimli\",\n");
    assert_eq!(
        cranelift_splits(&dropped),
        vec!["cranelift-object takes no object"]
    );
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
        &members_here(&root),
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    // The check itself, on a lock of two workspace crates and three outside.
    let lock = "version = 4\n\n[[package]]\nname = \"mente-a\"\nversion = \"0.1.0\"\ndependencies = [\n \"mente-b\",\n \"top\",\n]\n\n[[package]]\nname = \"mente-b\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"top\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\ndependencies = [\n \"leaf 2.0.0\",\n]\n\n[[package]]\nname = \"leaf\"\nversion = \"2.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n[[package]]\nname = \"other\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n";
    let ab: BTreeSet<String> = ["mente-a", "mente-b"].map(String::from).into();
    let wrong_dependency = "# comment\ntop: top\nleaf: top\nother: top\n";
    assert_eq!(
        lock_approval_problems(lock, wrong_dependency, &ab),
        vec!["other: top does not bring it".to_string()]
    );
    let gained = lock_approval_problems(lock, "top: top\nother: top\n", &ab);
    assert_eq!(gained.len(), 2, "{:?}", gained);
    assert!(gained[0].starts_with("Cargo.lock holds leaf, which"));
    let stale = lock_approval_problems(lock, "top: top\nleaf: top\ngone: top\n", &ab);
    assert!(stale.contains(&"gone is listed, but Cargo.lock no longer holds it".to_string()));
    assert!(stale
        .iter()
        .any(|p| p.starts_with("Cargo.lock holds other")));
    let indirect = lock_approval_problems(lock, "top: top\nleaf: leaf\nother: top\n", &ab);
    assert!(indirect
        .contains(&"leaf: leaf is not a direct dependency of a workspace crate".to_string()));
    let shapes = lock_approval_problems(lock, "top: top\ntop: top\nleaf top\nmente-b: top\n", &ab);
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
        lock_approval_problems(&git, "top: top\nleaf: top\nother: other\n", &ab),
        vec![
            "leaf comes from git+https://example.invalid/leaf#0123, not crates.io: only a crate of crates.io can be approved".to_string(),
            "other: other is not a direct dependency of a workspace crate".to_string(),
        ]
    );
    // A crate with no source that the root manifest names no member, at a path outside the
    // workspace, is refused, whatever it is named and whatever it brings.
    let vendored = format!(
        "{}\n[[package]]\nname = \"vendored\"\nversion = \"0.1.0\"\n",
        lock.replace(
            "\"mente-b\",\n \"top\",",
            "\"mente-b\",\n \"top\",\n \"vendored\","
        )
    );
    assert_ne!(vendored, lock);
    assert!(lock_approval_problems(&vendored, "top: top\nleaf: top\nother: top\n", &ab).contains(
        &"vendored comes from a path outside the workspace, not crates.io: only a member the root manifest names, or a crate of crates.io Norman approved, may be in the lock".to_string()
    ));
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

/// The lines under `root` that hold a CJK character, as `path:line`.
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
    lines_where(root, |line| line.chars().any(is_cjk))
}

/// The lines under `root` that name a checkout's place by a literal path, as `path:line`: a
/// drive's `Repos` directory, in each spelling the tree's text would give it, Windows' with
/// slashes or backslashes, single or escaped, and Git Bash's `/d/`, ending a token or followed by
/// a separator.
fn checkout_path_lines(root: &Path) -> Vec<String> {
    fn names_a_checkout_path(line: &str) -> bool {
        let b = line.as_bytes();
        // `repos` at `j`, ending its token: no letter or digit follows.
        let repos_at = |j: usize| {
            b.len() >= j + 5
                && b[j..j + 5].eq_ignore_ascii_case(b"repos")
                && !b.get(j + 5).is_some_and(u8::is_ascii_alphanumeric)
        };
        (0..b.len()).any(|i| {
            let after_word = i > 0 && b[i - 1].is_ascii_alphanumeric();
            // A drive, a colon and one or more separators, as Windows writes it.
            if b[i].is_ascii_alphabetic() && b.get(i + 1) == Some(&b':') && !after_word {
                let mut j = i + 2;
                while matches!(b.get(j), Some(b'/' | b'\\')) {
                    j += 1;
                }
                return j > i + 2 && repos_at(j);
            }
            // `/d/`, as Git Bash writes a drive, not inside a word or a path's name.
            b[i] == b'/'
                && !(after_word || (i > 0 && matches!(b[i - 1], b'_' | b'.' | b'~' | b'-')))
                && b.get(i + 1).is_some_and(u8::is_ascii_alphabetic)
                && b.get(i + 2) == Some(&b'/')
                && repos_at(i + 3)
        })
    }
    lines_where(root, names_a_checkout_path)
}

/// The lines of the repository's files under `root` (`repository_files`) that `holds`, as
/// `path:line`. A file that is not UTF-8 text is not read.
fn lines_where(root: &Path, holds: fn(&str) -> bool) -> Vec<String> {
    let mut found = Vec::new();
    for path in repository_files(root) {
        if let Ok(text) = fs::read_to_string(&path) {
            for (i, line) in text.lines().enumerate() {
                if holds(line) {
                    found.push(format!("{}:{}", path.display(), i + 1));
                }
            }
        }
    }
    found
}

/// The repository's files under `root`, the text the rules of this file judge: the files a commit
/// would hold, tracked or untracked but not ignored, as git names them, so that an ignored local
/// file, a tool's settings written with this machine's paths say, is no repository text. Where git
/// cannot name them, as in CI's sandbox, which empties `.git`, every file the walk reaches: a
/// superset of git's list, so the rules can refuse more there, never less, on a disk that CI checks
/// out fresh from the commit (and `briefs/clean-tree-around-every-test-run.md` is to hold equal to it).
fn repository_files(root: &Path) -> Vec<PathBuf> {
    git_files(root).unwrap_or_else(|| {
        let mut files = Vec::new();
        walk_files(root, &mut files);
        files
    })
}

/// The files git would commit under `root`, with the user's and the system's configuration left
/// out, so that only the repository's own ignore rules apply; None where git cannot name them.
fn git_files(root: &Path) -> Option<Vec<PathBuf>> {
    let empty = std::env::temp_dir().join(format!("gates-git-config-{}", std::process::id()));
    fs::write(&empty, "").ok()?;
    let out = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output();
    let _ = fs::remove_file(&empty);
    let out = out.ok().filter(|o| o.status.success())?;
    Some(
        out.stdout
            .split(|&c| c == 0)
            .filter(|name| !name.is_empty())
            .map(|name| root.join(String::from_utf8_lossy(name).as_ref()))
            .collect(),
    )
}

/// Every file under `dir` the walk reaches, where git cannot list them. It leaves out what no
/// commit holds: git's own `.git`, a directory or, in a worktree, the file that names its git
/// directory; build output and dependencies (`target`, `node_modules`); and a nested checkout,
/// a directory holding `.git`, such as a worktree under `.wt` or `.claude/worktrees`, which is
/// another branch's files. `briefs/clean-tree-around-every-test-run.md` is to make these one list
/// with its clean-tree check's, git's own `.git` among them, so that no second rule says it.
fn walk_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        if path.is_dir() {
            if !matches!(name.as_str(), "target" | "node_modules") && !path.join(".git").exists() {
                walk_files(&path, files);
            }
        } else {
            files.push(path);
        }
    }
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
const POLICY_STEP: [&str; 4] = [
    "      - name: The workflow policy, its required checks, path lists, pins and milestone tags",
    "        run: |",
    "          node .github/scripts/policy-collect.mjs",
    "          node --test \".github/scripts/policy/*.test.mjs\" .github/scripts/policy-run.test.mjs .github/scripts/yaml.test.mjs",
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
        "node .github/scripts/policy-collect.mjs && node --test \".github/scripts/policy/*.test.mjs\" .github/scripts/policy-run.test.mjs .github/scripts/yaml.test.mjs",
    ),
    (
        "verify:docs",
        "npm run spec-guard && npm run spec-guard:prove && npm run spec-guard:claims && npm run spec-graph && npm run spec-graph:whole && npm run spec-brief",
    ),
];

/// The directory of the workflow policy's files.
const POLICY_DIR: &str = ".github/scripts/policy/";

/// The workflow policy's list of its own files, `modules.json` in its directory, which its pins
/// hold: the test modules node runs, the libraries they share, and the checks the modules hold in
/// all.
struct PolicyList {
    modules: Vec<String>,
    libraries: Vec<String>,
    checks: usize,
}

/// Reads the policy's list from its text: `modules` and `libraries` each one array of plain
/// strings, `checks` one number. A key written twice, which a JSON reader would take the last of,
/// or an entry of any other kind is refused, so that the list is never read as fewer files.
fn policy_list(text: &str) -> Result<PolicyList, String> {
    let after_key = |key: &str| -> Result<&str, String> {
        let quoted = format!("\"{key}\"");
        let rest = match text.split(&quoted).collect::<Vec<_>>()[..] {
            [_, rest] => rest.trim_start(),
            _ => return Err(format!("modules.json names {quoted} other than once")),
        };
        rest.strip_prefix(':')
            .map(str::trim_start)
            .ok_or(format!("{quoted} is no key of modules.json"))
    };
    let strings = |key: &str| -> Result<Vec<String>, String> {
        let body = after_key(key)?
            .strip_prefix('[')
            .and_then(|r| r.split_once(']'))
            .ok_or(format!("{key} is no array"))?
            .0;
        body.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .filter(|s| !s.contains(['"', '\\']))
                    .map(str::to_string)
                    .ok_or(format!("{key} holds {s}, no plain string"))
            })
            .collect()
    };
    let checks = after_key("checks")?;
    let digits: String = checks.chars().take_while(char::is_ascii_digit).collect();
    if !checks[digits.len()..].trim_start().starts_with(['}', ',']) {
        return Err(format!("checks is {checks}, no number"));
    }
    Ok(PolicyList {
        modules: strings("modules")?,
        libraries: strings("libraries")?,
        checks: digits.parse().map_err(|e| format!("checks: {e}"))?,
    })
}

/// What keeps the policy's directory from holding exactly its listed files, given the names it
/// holds (`entries`), the list, and a reader of a file's text: no dotfile, which node's glob does not
/// run; nothing but the listed modules and libraries, README.md and the list; every listed file
/// there; every module a `.test.mjs` and no library a test file; and the modules' checks adding up to
/// the list's total (the review of f217d1e: a module renamed out of the glob left every count
/// passing).
fn policy_dir_problems(
    entries: &[String],
    list: &PolicyList,
    read: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut problems = Vec::new();
    let mut listed: BTreeSet<String> = BTreeSet::from(["README.md".into(), "modules.json".into()]);
    for f in list.modules.iter().chain(&list.libraries) {
        match f.strip_prefix(POLICY_DIR).filter(|n| !n.contains('/')) {
            Some(name) => {
                listed.insert(name.to_string());
            }
            None => problems.push(format!("{f}, listed, lies outside {POLICY_DIR}")),
        }
    }
    for e in entries {
        if e.starts_with('.') {
            problems.push(format!(
                "{POLICY_DIR}{e} is a dotfile, which node's glob does not run"
            ));
        } else if !listed.contains(e) {
            problems.push(format!(
                "{POLICY_DIR}{e} is none of the files modules.json lists"
            ));
        }
    }
    for name in &listed {
        if !entries.contains(name) {
            problems.push(format!("{POLICY_DIR}{name}, listed, is missing"));
        }
    }
    for f in &list.modules {
        if !f.ends_with(".test.mjs") {
            problems.push(format!(
                "{f}, listed as a module, is no .test.mjs node runs"
            ));
        }
    }
    for f in &list.libraries {
        if f.contains(".test.") {
            problems.push(format!("{f}, listed as a library, is a test file"));
        }
    }
    let total: usize = list
        .modules
        .iter()
        .filter_map(|f| read(f))
        .map(|text| text.lines().filter(|l| l.starts_with("check(\"")).count())
        .sum();
    if total != list.checks {
        problems.push(format!(
            "the policy's modules hold {total} checks, not the {} modules.json pins",
            list.checks
        ));
    }
    problems
}

/// The workflow policy runs where it blocks a merge, as written. ci.yml's Documentation job, a
/// required check, runs it first, after only the checkout and node, through node itself, in a step
/// of a name and a command alone; `npm ci` runs after it with no install script; the job sets
/// nothing but its name, runner and steps, and none of its scripts writes the environment or the
/// path of a later step; the workflow sets no shell and no environment but cargo's colours. The npm
/// scripts are exactly SCRIPTS, with no hook, and no `.npmrc` changes npm. The policy's suites mark
/// no test skipped, todo or only, and each module declares a floor equal to its number of checks,
/// which a run that skips any fails; the policy's directory holds exactly the files its list
/// names, and the modules hold the checks the list pins. Without this, a step deleted, a hook or an install script, a
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
    for (k, expected) in POLICY_STEP.iter().enumerate().skip(1) {
        assert_eq!(job.get(line + k), Some(expected), "the policy step changed");
    }
    assert!(
        job.get(line + POLICY_STEP.len())
            .is_some_and(|l| l.starts_with("      - ") || l.trim().starts_with('#')),
        "the policy step carries more than its name and commands"
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
    // The policy's files, exactly those its list names: the test modules, which the step runs by
    // their glob, and the libraries they share; the directory holds nothing else, and the modules
    // hold the checks the list pins.
    let entries: Vec<String> = fs::read_dir(root.join(POLICY_DIR))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let list =
        policy_list(&fs::read_to_string(root.join(POLICY_DIR).join("modules.json")).unwrap())
            .unwrap_or_else(|e| panic!("{e}"));
    let read = |f: &str| fs::read_to_string(root.join(f)).ok();
    assert_eq!(
        policy_dir_problems(&entries, &list, read),
        Vec::<String>::new()
    );
    // Planted, as the review of f217d1e found them: a module renamed out of the glob, to plain
    // `.mjs` or to a dotfile, which node's glob does not match; a test module the list does not
    // name; a module dropped from the list and the tree; a module listed that is not there; and the
    // total lowered.
    let first = list.modules[0]
        .strip_prefix(POLICY_DIR)
        .unwrap()
        .to_string();
    let renamed = |to: &str| -> Vec<String> {
        entries
            .iter()
            .map(|e| {
                if *e == first {
                    to.to_string()
                } else {
                    e.clone()
                }
            })
            .collect()
    };
    let read_as = |to: String| {
        let first = first.clone();
        let root = &root;
        move |f: &str| {
            let f = if f == format!("{POLICY_DIR}{to}") {
                format!("{POLICY_DIR}{first}")
            } else {
                f.to_string()
            };
            fs::read_to_string(root.join(f)).ok()
        }
    };
    let plain = first.replace(".test.mjs", ".mjs");
    let dotted = format!(".{first}");
    let dropped = PolicyList {
        modules: list.modules[1..].to_vec(),
        libraries: list.libraries.clone(),
        checks: list.checks,
    };
    let lowered = PolicyList {
        modules: list.modules.clone(),
        libraries: list.libraries.clone(),
        checks: list.checks - 1,
    };
    let ghost = PolicyList {
        modules: [&list.modules[..], &[format!("{POLICY_DIR}ghost.test.mjs")]].concat(),
        libraries: list.libraries.clone(),
        checks: list.checks,
    };
    let mut extra = entries.clone();
    extra.push("extra.test.mjs".to_string());
    let without: Vec<String> = entries.iter().filter(|e| **e != first).cloned().collect();
    for (problems, what) in [
        (
            policy_dir_problems(&renamed(&plain), &list, read_as(plain.clone())),
            "a module renamed to plain .mjs",
        ),
        (
            policy_dir_problems(&renamed(&dotted), &list, read_as(dotted.clone())),
            "a module renamed to a dotfile",
        ),
        (
            policy_dir_problems(&extra, &list, read),
            "a test module no list names",
        ),
        (
            policy_dir_problems(&without, &dropped, read),
            "a module dropped from the list and the tree",
        ),
        (
            policy_dir_problems(&entries, &ghost, read),
            "a module listed that is not there",
        ),
        (
            policy_dir_problems(&entries, &lowered, read),
            "the total lowered",
        ),
    ] {
        assert!(!problems.is_empty(), "{what} passed");
    }
    // The list is read as written or refused: a second key, or an entry that is no plain string.
    let text = fs::read_to_string(root.join(POLICY_DIR).join("modules.json")).unwrap();
    assert!(policy_list(&text.replacen("\"checks\"", "\"checks\": 1, \"checks\"", 1)).is_err());
    assert!(policy_list(&text.replacen("\"modules\": [", "\"modules\": [1, ", 1)).is_err());
    let policy: Vec<String> = list
        .modules
        .iter()
        .chain(&list.libraries)
        .cloned()
        .collect();
    let policy_tests: Vec<&String> = list.modules.iter().collect();
    // No test of the policy's suites is skipped, left to do or singled out.
    for suite in policy.iter().map(String::as_str).chain([
        ".github/scripts/policy-run.test.mjs",
        ".github/scripts/yaml.test.mjs",
    ]) {
        let text = fs::read_to_string(root.join(suite)).unwrap();
        for marker in [".skip(", ".todo(", ".only(", "skip:", "todo:", "only:"] {
            assert!(
                !text.contains(marker),
                "{suite} holds `{marker}`: a test it would not run"
            );
        }
    }
    // The floor of checks of each module of the policy and of its run tests equals the checks each
    // declares.
    for suite in policy_tests
        .iter()
        .map(|f| f.as_str())
        .chain([".github/scripts/policy-run.test.mjs"])
    {
        let text = fs::read_to_string(root.join(suite)).unwrap();
        let checks = text.lines().filter(|l| l.starts_with("check(\"")).count();
        let floor: usize = text
            .lines()
            .find_map(|l| l.strip_prefix("const FLOOR = "))
            .and_then(|v| v.trim_end_matches(';').parse().ok())
            .unwrap_or_else(|| panic!("{suite} declares no FLOOR"));
        assert!(checks > 0, "{suite} declares no check");
        assert_eq!(
            floor, checks,
            "{suite}'s FLOOR is not the number of its checks"
        );
        assert!(
            text.contains("after(() => {\n  assert.ok(ran >= FLOOR"),
            "{suite} does not fail when fewer checks ran than its FLOOR"
        );
    }
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

/// Where a checkout lies is read from the checkout at run time, as the tools read it, and never
/// restated as a literal path: CLAUDE.md's `core.hooksPath` went stale when the repositories
/// moved from one drive to another. No file of the repository names a drive's `Repos` directory.
#[test]
fn test_repository_text_names_no_checkout_path() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let found = checkout_path_lines(&root);
    assert!(
        found.is_empty(),
        "a checkout's place written as a literal path at:\n{}",
        found.join("\n")
    );
}

/// What the checkout path rule reads, and what it leaves: each spelling of a drive's `Repos`
/// directory, and none of the paths that only resemble one; outside any repository, every file but
/// git's own `.git` file; in a repository, the files a commit would hold, an ignored one aside.
/// The probes are built at run time, so that this file names no such path itself, and the probe
/// directory is removed before any assertion.
#[test]
fn test_the_checkout_path_rule_reads_the_commits_text_alone() {
    let r = "Repos";
    let spellings = [
        (format!("hooks at C:/{r}/Mente/tools/git-hooks"), true),
        (format!("cd D:\\{r}\\Menteion\\Mente"), true),
        (format!("the json \"C:\\\\{r}\\\\Mente\""), true),
        (format!("under `D:\\{r}`."), true),
        (format!("cd /d/{r}/Menteion/Mente"), true),
        (format!("the drive /c/{r}"), true),
        ("X:\\checkout\\target\\release\\mn.exe".to_string(), false),
        (format!("a repository of C:/Users/x/{r}itories"), false),
        (format!("ABC:/{r}/x"), false),
        (
            format!("api.github.com/{}/Menteion/Mente", r.to_lowercase()),
            false,
        ),
        (format!("https://example.com/c/{r}/x"), false),
        (
            format!("C:/Program Files/Git/{}/x", r.to_lowercase()),
            false,
        ),
    ];
    let dir = std::env::temp_dir().join(format!("checkout-paths-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let read: Vec<(String, bool, usize)> = spellings
        .into_iter()
        .map(|(text, named)| {
            fs::write(dir.join("probe.md"), &text).unwrap();
            let found = checkout_path_lines(&dir).len();
            (text, named, found)
        })
        .collect();
    // A worktree's `.git` file names its git directory by its path, and is no repository text.
    fs::write(dir.join("probe.md"), "nothing").unwrap();
    fs::write(
        dir.join(".git"),
        format!("gitdir: C:/{r}/Mente/.git/worktrees/w\n"),
    )
    .unwrap();
    let dot_git = checkout_path_lines(&dir);
    fs::remove_file(dir.join(".git")).unwrap();
    // In a repository, an ignored file, a tool's settings say, is no text of a commit; an
    // untracked one that is not ignored is.
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .map(|o| o.status.success());
    fs::write(repo.join(".gitignore"), "local.json\n").unwrap();
    fs::write(
        repo.join("local.json"),
        format!("\"Bash(node D:/{r}/Mente/tools/quick.mjs:*)\""),
    )
    .unwrap();
    fs::write(repo.join("new.md"), format!("cd D:\\{r}\\Mente")).unwrap();
    let in_repo = checkout_path_lines(&repo);
    let _ = fs::remove_dir_all(&dir);
    for (text, named, found) in read {
        assert_eq!(found, usize::from(named), "{text}");
    }
    assert!(dot_git.is_empty(), "{dot_git:?}");
    assert!(matches!(init, Ok(true)), "git init failed: {init:?}");
    assert_eq!(in_repo.len(), 1, "{in_repo:?}");
    assert!(in_repo[0].contains("new.md"), "{in_repo:?}");
}
