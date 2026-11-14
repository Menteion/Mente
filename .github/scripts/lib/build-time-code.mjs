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
//   node .github/scripts/build-time-code.mjs
//
// - A package cargo takes from a path is a member the root manifest lists, and has no build script
//   and no procedural macro, but for those BUILD_TIME_CODE names.
// - Every other package comes from crates.io and is in .config/approved-crates.txt.
// - Cargo.lock holds only lines cargo writes, every source in it is crates.io, and every crate
//   with a source is approved. This is read before cargo runs, so that cargo fetches from nowhere
//   else.
//
// It runs in the Documentation job, a required check that builds nothing, after the workflow
// policy, which has refused every cargo configuration file and read rust-toolchain.toml: there no
// code of the change has run when it reads, as it has where a job builds before it tests.

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

/// What cargo reports of the workspace, with every feature on, so that no optional dependency is
/// left out, and with the lock as it stands. cargo reads the manifests and fetches the crates the
/// lock names; it compiles nothing and runs no build script.
function cargoMetadata(root) {
  return execFileSync("cargo", ["metadata", "--format-version", "1", "--locked", "--all-features"], { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] });
}

/// The CLI: the problems found, and what was read. `metadata` stands in for cargo in the policy's
/// tests.
export function main(root = process.cwd(), metadata = cargoMetadata) {
  const read = (f) => fs.readFileSync(path.join(root, f), "utf8");
  const approved = approvedCrates(read(".config/approved-crates.txt"));
  const lockText = read("Cargo.lock");
  // The lock first: cargo fetches what it names.
  const { unreadable, unapproved } = lockProblems(lockText, approved);
  if (unreadable.length > 0) return { problems: unreadable, summary: "" };
  const report = JSON.parse(metadata(root));
  const lock = lockPackages(lockText).packages;
  const problems = [...unapproved, ...metadataProblems(report, { listed: listedMembers(read("Cargo.toml")), lock })];
  const own = (report.packages ?? []).filter((p) => p.source === null || p.source === undefined);
  const summary = `cargo reads ${own.length} packages of the workspace, none with a build script or a procedural macro, and ${report.packages.length - own.length} crates of crates.io, each approved.`;
  return { problems, summary };
}
