// Whether code of a change runs where cargo builds, outside every sandbox and before any test, and
// whether every crate cargo would build from outside the workspace is an approved one
// (docs/design-review.md, untrusted-test-isolation and dependency-approval). The tests of a change
// run in a sandbox on Linux; a build script, a procedural macro and the build script of a new
// dependency run where cargo or rustc does, with the job's rights.
//
// The manifests are read as cargo reads them, by cargo itself: `cargo metadata` names every
// package it would build, with its source and its targets, and builds nothing. A reader of the
// manifests' text sees no key written with a TOML escape, and no package that only a path names.
//
//   node .github/scripts/build-time-code.mjs --before-toolchain   before rustup installs anything
//   node .github/scripts/build-time-code.mjs                      once the toolchain is installed
//
// - rust-toolchain.toml names a release, its two components and its profile, and nothing else, and
//   no cargo configuration file is in the tree, above it or in CARGO_HOME: rustup installs what the
//   first names, and cargo starts what the second names, before any manifest is read. This runs
//   first in both forms, and alone in the first.
// - A package cargo takes from a path is a member the root manifest lists, and has no build script
//   and no procedural macro, but for those BUILD_TIME_CODE names.
// - Every other package comes from crates.io and is in .config/approved-crates.txt.
// - Cargo.lock holds only lines cargo writes, every source in it is crates.io, and every crate
//   with a source is approved. This is read before cargo runs, so that cargo fetches from nowhere
//   else.
//
// It runs in every job that builds, as its first step that reads the workspace after the watch, so
// that a change whose build-time code would run fails there before any of it runs; and in the
// Documentation job, which builds nothing, after the workflow policy.

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

export const CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index";

/// The packages of the workspace that may run code at build time, a build script or a procedural
/// macro, each with its reason. That code runs before the tests' limits and outside their sandbox,
/// with the job's rights.
export const BUILD_TIME_CODE = new Map([]);

/// The members the root manifest lists, as written: every string of `members = [...]`, in either
/// quote, comments aside. A glob is refused, since cargo would expand it to members no one named.
export function listedMembers(manifest) {
  const list = (/members\s*=\s*\[([\s\S]*?)\]/.exec(manifest)?.[1] ?? "").replace(/#.*$/gm, "");
  const members = [...list.matchAll(/"([^"]+)"|'([^']+)'/g)].map((m) => m[1] ?? m[2]);
  for (const m of members) {
    if (/[*?[]/.test(m)) throw new Error(`Cargo.toml's workspace member ${m} is a glob`);
  }
  return members;
}

/// The crates .config/approved-crates.txt names, from its `<crate>: <direct dependency>` lines.
export function approvedCrates(text) {
  const names = new Set();
  for (const [n, raw] of text.split(/\r?\n/).entries()) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const m = /^([A-Za-z0-9_-]+): ([A-Za-z0-9_-]+)$/.exec(line);
    if (!m) throw new Error(`.config/approved-crates.txt:${n + 1}: not "<crate>: <dependency>": ${line}`);
    names.add(m[1]);
  }
  return names;
}

/// Cargo.lock, read as the lines cargo writes and no others: its packages, each with its source
/// (null for a package of the workspace), and every line that is not one of cargo's, which could
/// read otherwise to cargo than to this.
export function lockPackages(text) {
  const packages = [];
  const problems = [];
  let current = null;
  let inDependencies = false;
  for (const [n, line] of text.split(/\r?\n/).entries()) {
    const bad = () => problems.push(`Cargo.lock:${n + 1}: not a line cargo writes: ${line}`);
    if (inDependencies) {
      if (line === "]") inDependencies = false;
      else if (!/^ "[A-Za-z0-9_-]+( [0-9A-Za-z.+-]+)?( \([^"\\()]+\))?",$/.test(line)) bad();
      continue;
    }
    if (line === "" || line.startsWith("#")) continue;
    if (line === "[[package]]") {
      current = { name: null, version: null, source: null };
      packages.push(current);
      continue;
    }
    if (current === null) {
      if (!/^version = \d+$/.test(line)) bad();
      continue;
    }
    let m;
    if ((m = /^name = "([A-Za-z0-9_-]+)"$/.exec(line)) && current.name === null) current.name = m[1];
    else if ((m = /^version = "([0-9A-Za-z.+-]+)"$/.exec(line)) && current.version === null) current.version = m[1];
    else if ((m = /^source = "([^"\\]+)"$/.exec(line)) && current.source === null) current.source = m[1];
    else if (/^checksum = "[0-9a-f]{64}"$/.test(line)) continue;
    else if (line === "dependencies = [") inDependencies = true;
    else bad();
  }
  if (inDependencies) problems.push("Cargo.lock: a list of dependencies never closes");
  for (const p of packages) {
    if (p.name === null || p.version === null) problems.push(`Cargo.lock: a package without its name and version: ${JSON.stringify(p)}`);
  }
  return { packages, problems };
}

/// Why the lock could bring in a crate no one approved. `unreadable` holds what stops cargo from
/// being run on it at all: a line cargo does not write, which could read otherwise to cargo than
/// to this, and a source other than crates.io, which cargo would fetch from. `unapproved` holds
/// each crate of crates.io that the list lacks.
export function lockProblems(lockText, approved) {
  const { packages, problems: unreadable } = lockPackages(lockText);
  const unapproved = [];
  for (const p of packages) {
    if (p.source === null) continue;
    if (p.source !== CRATES_IO) unreadable.push(`Cargo.lock: ${p.name} comes from ${p.source}, not crates.io: only a crate of crates.io can be approved`);
    else if (!approved.has(p.name)) unapproved.push(`Cargo.lock holds ${p.name}, which .config/approved-crates.txt does not list: Norman approves every crate a dependency brings (dependency-approval)`);
  }
  return { unreadable, unapproved };
}

/// Why code of the change could run where cargo builds, or a crate be built that the lock does not
/// hold, from what `cargo metadata` reports: `listed` holds the members the root manifest lists,
/// `lock` the lock's packages, each of which is approved or reported by `lockProblems`.
export function metadataProblems(metadata, { listed, lock }) {
  const problems = [];
  const root = metadata.workspace_root;
  const inLock = new Set(lock.map((p) => `${p.name} ${p.version}`));
  const seen = new Set();
  for (const p of metadata.packages ?? []) {
    if (!inLock.has(`${p.name} ${p.version}`)) problems.push(`cargo would build ${p.name} ${p.version}, which Cargo.lock does not hold`);
    const kinds = new Set((p.targets ?? []).flatMap((t) => t.kind ?? []));
    if (p.source === null || p.source === undefined) {
      const dir = path.relative(root, path.dirname(p.manifest_path)).split(path.sep).join("/");
      seen.add(dir);
      if (!listed.includes(dir)) problems.push(`cargo builds ${p.name} from ${dir || "the workspace's root"}, which the members of Cargo.toml do not list: a path dependency is a member too, and the policy reads only the members listed`);
      for (const [kind, what] of [["custom-build", "a build script"], ["proc-macro", "a procedural macro"]]) {
        if (kinds.has(kind) && !BUILD_TIME_CODE.has(p.name)) {
          problems.push(`${p.name} (${dir}) has ${what}, code of the change that runs where cargo builds, outside the tests' sandbox and with the job's rights: BUILD_TIME_CODE in .github/scripts/lib/build-time-code.mjs names the packages allowed one`);
        }
      }
    } else if (p.source !== CRATES_IO) {
      problems.push(`${p.name} comes from ${p.source}, not crates.io: only a crate of crates.io can be approved`);
    }
  }
  for (const dir of listed) {
    if (!seen.has(dir)) problems.push(`Cargo.toml lists the member ${dir}, which cargo does not read as a package of the workspace`);
  }
  return problems;
}

/// The lines rust-toolchain.toml may hold: its table, a release, the two components and the minimal
/// profile, each once, with comments and blanks. A `path`, a `targets` or any other key could
/// name a toolchain of the change, which rustup would install and cargo run.
const TOOLCHAIN_LINES = [/^\[toolchain\]$/, /^channel = "\d+\.\d+\.\d+"$/, /^components = \["clippy", "rustfmt"\]$/, /^profile = "minimal"$/];

/// Why the toolchain rustup installs, or the cargo that reads the manifests, could be the change's
/// own: a rust-toolchain file beside rust-toolchain.toml, any line of rust-toolchain.toml but
/// those above, or a cargo configuration file anywhere in the tree, above it or in CARGO_HOME,
/// which can name a wrapper around rustc that cargo would start. Read before rustup or cargo runs.
/// `top` is the directory the walk up stops at, the filesystem's root unless a test names another.
export function toolchainProblems(root, { cargoHome = process.env.CARGO_HOME ?? path.join(process.env.HOME ?? process.env.USERPROFILE ?? "", ".cargo"), tracked, top } = {}) {
  const found = [];
  if (fs.existsSync(path.join(root, "rust-toolchain"))) found.push("a rust-toolchain file beside rust-toolchain.toml: rustup reads it instead");
  const lines = fs.readFileSync(path.join(root, "rust-toolchain.toml"), "utf8").split(/\r?\n/);
  const seen = new Array(TOOLCHAIN_LINES.length).fill(0);
  for (const [n, raw] of lines.entries()) {
    const line = raw.replace(/\s+#.*$/, "").trim();
    if (line === "" || line.startsWith("#")) continue;
    const k = TOOLCHAIN_LINES.findIndex((re) => re.test(line));
    if (k < 0) found.push(`rust-toolchain.toml:${n + 1}: not a line a pinned release holds: ${raw}`);
    else seen[k]++;
  }
  if (seen.some((c) => c !== 1)) found.push("rust-toolchain.toml does not hold its table, its release, its two components and its profile once each");
  const configs = new Set();
  for (let dir = path.resolve(root); ; dir = path.dirname(dir)) {
    for (const name of ["config", "config.toml"]) configs.add(path.join(dir, ".cargo", name));
    if (path.dirname(dir) === dir || (top !== undefined && dir === path.resolve(top))) break;
  }
  for (const name of ["config", "config.toml"]) configs.add(path.resolve(cargoHome, name));
  for (const f of configs) if (fs.existsSync(f)) found.push(`${f}: a cargo configuration file could name a wrapper around rustc, which cargo would start`);
  const files = tracked ?? execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).split("\0").filter(Boolean);
  for (const f of files) if (/(^|\/)\.cargo\/config(\.toml)?$/.test(f)) found.push(`${f}: a cargo configuration file in the tree`);
  return found;
}

/// What cargo reports of the workspace, with every feature on, so that no optional dependency is
/// left out, and with the lock as it stands. cargo reads the manifests and fetches the crates the
/// lock names; it compiles nothing and runs no build script.
function cargoMetadata(root) {
  return execFileSync("cargo", ["metadata", "--format-version", "1", "--locked", "--all-features"], { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });
}

/// The CLI: the problems found, and what was read. `metadata` stands in for cargo in the policy's
/// tests.
export function main(root = process.cwd(), metadata = cargoMetadata, { beforeToolchain = false, toolchain = {} } = {}) {
  const read = (f) => fs.readFileSync(path.join(root, f), "utf8");
  // The toolchain and cargo's configuration first: rustup installs what rust-toolchain.toml names,
  // and cargo starts what a configuration file names, before any manifest is read.
  const own = toolchainProblems(root, toolchain);
  if (own.length > 0 || beforeToolchain) return { problems: own, summary: "rust-toolchain.toml names a release alone, and no cargo configuration file is in the tree, above it or in CARGO_HOME." };
  const approved = approvedCrates(read(".config/approved-crates.txt"));
  const lockText = read("Cargo.lock");
  // The lock first: cargo fetches what it names.
  const { unreadable, unapproved } = lockProblems(lockText, approved);
  if (unreadable.length > 0) return { problems: unreadable, summary: "" };
  const report = JSON.parse(metadata(root));
  const lock = lockPackages(lockText).packages;
  const problems = [...unapproved, ...metadataProblems(report, { listed: listedMembers(read("Cargo.toml")), lock })];
  const members = (report.packages ?? []).filter((p) => p.source === null || p.source === undefined);
  const summary = `cargo reads ${members.length} packages of the workspace, none with a build script or a procedural macro, and ${report.packages.length - members.length} crates of crates.io, each approved.`;
  return { problems, summary };
}
