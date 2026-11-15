#!/usr/bin/env node
// The quick checks before a push (CLAUDE.md, "Verification"): from the files the branch changes
// against origin/main, with those changed but not yet committed, this runs the fast local checks
// those files call for, at half the laptop's cores, and reports each; CI runs everything else, and
// its required checks are the gate. A session does not choose the checks by hand, so none that a
// change calls for is forgotten: on 2026-11-16 a pull request that added conformance programs went
// out without its golden files and failed CI for it.
//
//   node tools/quick.mjs                       the checks the changed files call for
//   node tools/quick.mjs --list                which checks those are, without running them
//   node tools/quick.mjs -- -p mente-sema frames
//                                              and `cargo test` with these arguments: the tests the
//                                              change adds or changes, each to be seen failing
//                                              without the fix and passing with it
//
// Which files call for which checks is written once, in CHECKS below.

import { execFileSync, spawnSync } from "node:child_process";

const env = { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "8", RUST_TEST_THREADS: process.env.RUST_TEST_THREADS ?? "8" };
const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).split(/\r?\n/).filter(Boolean);

/** Every file the branch changes against origin/main, committed or not, and every new file. */
function changedFiles() {
  const base = git("merge-base", "origin/main", "HEAD")[0];
  return new Set([
    ...git("diff", "--name-only", `${base}...HEAD`),
    ...git("diff", "--name-only", "HEAD"),
    ...git("ls-files", "--others", "--exclude-standard"),
  ]);
}

const RUST_INPUTS = /\.rs$|(^|\/)Cargo\.(toml|lock)$|^clippy\.toml$|^rust-toolchain(\.toml)?$|^\.config\/|^\.cargo\//;
const PROGRAMS = /^compiler\/|^conformance\/|^examples\/|^docs\/language-card\.md$|^docs\/platform-layer\.md$/;
const DOCUMENTS = /\.md$|^spec\/|^docs\//;
const GATES = /^\.github\/|^\.config\/|^\.cargo\/|(^|\/)Cargo\.(toml|lock)$|^clippy\.toml$|^package(-lock)?\.json$|^\.gitattributes$|^rust-toolchain(\.toml)?$/;
const SITE = /^site\/|^README\.md$|^spec\/|^docs\//;

/** Each check: the files that call for it, and the command. */
const CHECKS = [
  { name: "format", when: RUST_INPUTS, run: ["cargo", "fmt", "--all", "--check"] },
  { name: "lints", when: RUST_INPUTS, run: ["cargo", "clippy", "--all-targets", "--workspace", "--", "-D", "warnings"] },
  // The repository's gates: English text, the dependency policy, the lock's crates.
  { name: "gates", when: /./, run: ["cargo", "test", "-p", "mente-cli", "--test", "gates"] },
  // What a change to the compiler or to its programs must keep: the lint's list of what it does not
  // see, the golden files every conformance program writes, and each program's expected report.
  { name: "lint list, golden files and conformance", when: PROGRAMS, run: ["cargo", "test", "-p", "mente-cli", "--test", "lints", "--test", "reproducible", "--test", "conformance"] },
  { name: "spec directives", when: DOCUMENTS, run: ["cargo", "run", "--quiet", "--bin", "mn", "--", "spec"] },
  { name: "documentation tools", when: DOCUMENTS, run: ["npm", "run", "verify:docs"] },
  { name: "workflow policy", when: GATES, run: ["npm", "run", "test:ci"] },
  { name: "benchmark corpus", when: /^benchmark\//, run: ["npm", "run", "check:benchmark"] },
  { name: "benchmark checker", when: /^benchmark\//, run: ["npm", "run", "test:benchmark"] },
  { name: "benchmark harness", when: /^benchmark\/harness\//, run: ["npm", "run", "test:harness"] },
  { name: "site", when: SITE, run: ["npm", "run", "test:site"] },
];

const dash = process.argv.indexOf("--");
const ownTests = dash >= 0 ? process.argv.slice(dash + 1) : [];
const changed = changedFiles();
const due = CHECKS.filter((c) => [...changed].some((f) => c.when.test(f)));
if (ownTests.length) due.push({ name: `the change's own tests (cargo test ${ownTests.join(" ")})`, run: ["cargo", "test", ...ownTests] });
if (due.length === 0) {
  console.log("quick: nothing changed against origin/main, so nothing to check");
  process.exit(0);
}

console.log(`quick: ${changed.size} changed file(s); ${due.length} check(s)\n`);
if (process.argv.includes("--list")) {
  for (const check of due) console.log(`  ${check.name}: ${check.run.join(" ")}`);
  process.exit(0);
}
const results = [];
for (const check of due) {
  console.log(`== ${check.name}: ${check.run.join(" ")}`);
  const [cmd, ...args] = check.run;
  // npm is a .cmd on Windows, which only a shell starts.
  const r = spawnSync(cmd, args, { stdio: "inherit", env, shell: process.platform === "win32" && cmd === "npm" });
  results.push([check.name, r.status === 0]);
}
console.log("\nquick:");
for (const [name, ok] of results) console.log(`  ${ok ? "pass" : "FAIL"}  ${name}`);
const failed = results.filter(([, ok]) => !ok).length;
if (!ownTests.length) console.log("\nAlso run the tests the change adds or changes: node tools/quick.mjs -- -p <crate> <filter>");
process.exit(failed ? 1 : 0);
