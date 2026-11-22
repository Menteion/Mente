// The workflow policy's reading of the repository (README.md in this directory): its root, its
// files as git tracks them and the commit checked out, from the collector's facts.

import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { BUILD_TIME_CODE, CRATES_IO, approvedCrates, listedMembers, lockPackages, lockProblems, main as buildTimeCode, metadataProblems, toolchainProblems } from "../lib/build-time-code.mjs";
/// .github/scripts, the directory every path the policy names from here is relative to: this module
/// lies in its policy/ directory.
export const HERE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const ROOT = path.resolve(HERE, "../..");
export const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");
/// The commit checked out at `root`, read from its .git as git keeps it: a directory or, in a
/// worktree, a file naming one, whose HEAD is a commit or a ref, found loose or packed, in it or in
/// the common directory its `commondir` names. The policy starts no process (policy-runtime-judge),
/// so it reads this from the files.
export function gitHead(root) {
  let gitDir = path.join(root, ".git");
  if (fs.statSync(gitDir).isFile()) gitDir = path.resolve(root, fs.readFileSync(gitDir, "utf8").replace(/^gitdir:\s*/, "").trim());
  const commonFile = path.join(gitDir, "commondir");
  const common = fs.existsSync(commonFile) ? path.resolve(gitDir, fs.readFileSync(commonFile, "utf8").trim()) : gitDir;
  const head = fs.readFileSync(path.join(gitDir, "HEAD"), "utf8").trim();
  const ref = /^ref:\s*(\S+)$/.exec(head)?.[1];
  if (ref === undefined) return head;
  for (const dir of [gitDir, common]) {
    const loose = path.join(dir, ref);
    if (fs.existsSync(loose)) return fs.readFileSync(loose, "utf8").trim();
  }
  const packed = path.join(common, "packed-refs");
  const line = fs.existsSync(packed) ? fs.readFileSync(packed, "utf8").split(/\r?\n/).find((l) => l.endsWith(` ${ref}`)) : undefined;
  if (line === undefined) throw new Error(`${root}: HEAD names ${ref}, which no ref file holds`);
  return line.split(" ")[0];
}

/// What .github/scripts/policy-collect.mjs gave the policy, run just before it (the Documentation
/// job's step, and npm run test:ci): the files git tracks and the milestones' facts. Refused when
/// they are of another commit than the one checked out, so that no stale facts pass.
export const FACTS_FILE = path.join(ROOT, "target", "policy-facts.json");
/// The collector's facts from their text, refused unless they are of `head` and whole: an object
/// whose files and tags are names, each a string that is not empty, and whose base is a full commit
/// id, so that nothing the policy reads from them is of another type than it expects (the review of
/// 100e1e5).
export function readFacts(text, head) {
  if (text === null) throw new Error("no facts from the collector: run node .github/scripts/policy-collect.mjs first, as npm run test:ci does");
  const facts = JSON.parse(text);
  if (facts === null || typeof facts !== "object" || Array.isArray(facts)) throw new Error("the collector's facts are not an object: run node .github/scripts/policy-collect.mjs again");
  if (facts.head !== head) throw new Error(`the collector's facts are of ${facts.head}, not of ${head}, the commit checked out: run node .github/scripts/policy-collect.mjs again, as npm run test:ci does`);
  const names = (list) => Array.isArray(list) && list.every((name) => typeof name === "string" && name !== "");
  const whole =
    names(facts.tracked) &&
    facts.tracked.length > 0 &&
    typeof facts.base === "string" &&
    /^[0-9a-f]{40}$/.test(facts.base) &&
    names(facts.tags) &&
    typeof facts.baseRoadmap === "string";
  if (!whole) throw new Error("the collector's facts are not whole: run node .github/scripts/policy-collect.mjs again");
  return facts;
}
export const FACTS = readFacts(fs.existsSync(FACTS_FILE) ? fs.readFileSync(FACTS_FILE, "utf8") : null, gitHead(ROOT));

export let trackedCache;
export function tracked() {
  trackedCache ??= new Set(FACTS.tracked);
  return trackedCache;
}
export const filesUnder = (dir) => [...tracked()].filter((f) => f.startsWith(`${dir}/`));

/// A line of the TOML subset the policy reads, without its trailing comment and the whitespace
/// around it: everything from the first `#` outside a string on goes.
export function uncomment(line) {
  let quote = null;
  for (let k = 0; k < line.length; k++) {
    const c = line[k];
    if (quote !== null) {
      if (c === "\\" && quote === '"') k++;
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'") quote = c;
    else if (c === "#") return line.slice(0, k).trim();
  }
  return line.trim();
}

/// A file of the TOML subset the policy reads: its tables in order, each with its keys and their
/// values as written. nextest and rustup at most warn about a key they do not know, and TOML has
/// spellings a narrower reader would pass over (quoted and dotted keys, a key before the first
/// table), so every line must be a blank, a comment, a table's header, a bare `key = value`, or a
/// line of a `"""` string, each with an optional trailing comment. Any other line is an error
/// rather than a line skipped, and so is a table or a key twice.
export function tomlSubset(text, file) {
  const tables = [];
  const lines = text.split(/\r?\n/);
  let current = null;
  const fail = (k, why) => {
    throw new Error(`${file}:${k + 1}: ${why}: ${lines[k]}`);
  };
  for (let k = 0; k < lines.length; k++) {
    const line = uncomment(lines[k]);
    if (line === "") continue;
    const header = /^(\[\[?)([a-z][a-z0-9-]*(?:\.[a-z][a-z0-9-]*)*)(\]\]?)$/.exec(line);
    if (header) {
      const array = header[1] === "[[";
      if (array !== (header[3] === "]]")) fail(k, "a header whose brackets do not match");
      if (tables.some((t) => t.name === header[2] && (!array || !t.array))) fail(k, "a table twice");
      current = { name: header[2], array, keys: new Map() };
      tables.push(current);
      continue;
    }
    const entry = /^([a-z][a-z0-9-]*) = (.+)$/.exec(line);
    if (!entry) fail(k, "not a line the policy reads");
    if (current === null) fail(k, "a key outside any table");
    if (current.keys.has(entry[1])) fail(k, "a key twice");
    let value = entry[2];
    if (value.startsWith('"""')) {
      const start = k;
      value = lines[k].slice(lines[k].indexOf('"""'));
      while (value.indexOf('"""', 3) < 0) {
        if (++k >= lines.length) fail(start, "a string never closed");
        value += `\n${lines[k]}`;
      }
      const end = value.indexOf('"""', 3);
      if (uncomment(value.slice(end + 3)) !== "") fail(k, "more after the string's end");
      value = value.slice(0, end + 3);
    }
    current.keys.set(entry[1], value);
  }
  return tables;
}

export const strings = (v) => Array.isArray(v) && v.every((s) => typeof s === "string");

/// The digest a pin holds: SHA-256 of the text, Windows line ends read as Unix ones.
export const digest = (text) => crypto.createHash("sha256").update(String(text).replace(/\r\n/g, "\n")).digest("hex");

/// The workspace's members, each with the members it depends on by path.
export function workspace() {
  // Every member the root manifest lists, in either quote, comments aside; a glob is refused,
  // since cargo would expand it to members this list never names (lib/build-time-code.mjs, which
  // also holds the list against the packages cargo itself reads).
  const members = listedMembers(read("Cargo.toml"));
  assert.ok(members.length > 0, "Cargo.toml lists no workspace members");
  const crates = new Map();
  for (const dir of members) {
    const manifest = read(`${dir}/Cargo.toml`);
    const name = /^\s*name\s*=\s*"([^"]+)"/m.exec(manifest)?.[1];
    const deps = [...manifest.matchAll(/^\s*([\w-]+)\s*=\s*\{[^}]*\bpath\s*=\s*"([^"]+)"/gm)].map((m) => path.posix.normalize(path.posix.join(dir, m[2])));
    crates.set(name, { dir, deps });
  }
  return crates;
}
