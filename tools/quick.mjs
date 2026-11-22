#!/usr/bin/env node
// The quick checks before a push (CLAUDE.md, "Verification"): from the files the branch changes
// against origin/main, with those changed but not yet committed, this runs the fast local checks
// those files call for, at half the laptop's cores, and reports each; CI runs everything else, and
// its required checks are the gate. A session does not choose the checks by hand, so none that a
// change calls for is forgotten: on 2026-11-16 a pull request that added conformance programs went
// out without its golden files and failed CI for it.
//
// It runs in one of the machine's build slots, and runs each check under the build limits
// (tools/local-limits.mjs): each process at most 8 GB committed, all of a check's at most 16 GB, a
// check at most 30 minutes (the reset of 2026-11-22).
//
//   node tools/quick.mjs                       the checks the changed files call for
//   node tools/quick.mjs --list                which checks those are, without running them
//   node tools/quick.mjs -- -p mente-sema frames
//                                              and `cargo test` with these arguments: the tests the
//                                              change adds or changes, each to be seen failing
//                                              without the fix and passing with it
//
// Which files call for which checks is written once, in tools/quick-checks.json.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { BUILD, acquireSlot, requireRunner, runCapped } from "./local-limits.mjs";

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

/**
 * Each check: the files that call for it, and the command, from tools/quick-checks.json. The map is
 * data, which the workflow policy reads and never runs: a module it imported could turn its
 * assertions off (the quick check of d471f7e).
 */
const MAP = JSON.parse(readFileSync(new URL("./quick-checks.json", import.meta.url), "utf8"));
const CHECKS = MAP.checks.map((c) => {
  const pattern = Object.hasOwn(MAP.patterns, c.when) ? MAP.patterns[c.when] : null;
  if (typeof pattern !== "string") throw new Error(`tools/quick-checks.json: ${c.name} names the pattern ${c.when}, which it does not define`);
  return { name: c.name, when: new RegExp(pattern), run: c.run };
});

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
if (!requireRunner()) process.exit(2);
const slot = acquireSlot(`tools/quick.mjs: ${due.length} check(s)`);
const results = [];
for (const check of due) {
  console.log(`== ${check.name}: ${check.run.join(" ")}`);
  results.push([check.name, runCapped(check.run, BUILD) === 0]);
}
slot.release();
console.log("\nquick:");
for (const [name, ok] of results) console.log(`  ${ok ? "pass" : "FAIL"}  ${name}`);
const failed = results.filter(([, ok]) => !ok).length;
if (!ownTests.length) console.log("\nAlso run the tests the change adds or changes: node tools/quick.mjs -- -p <crate> <filter>");
process.exit(failed ? 1 : 0);
