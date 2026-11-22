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
//
// The checks read the working tree, and a push sends HEAD, so a run ends by saying which of the two
// it read. On 2026-11-22 a merge of the evidence stack (#174, c94b58e) left two resolved files
// unstaged; these checks passed on the working tree, and CI then ran HEAD without them. The run
// takes the working tree's tree, every tracked and unignored file as it stands, before its checks
// and after them, and says the checks read HEAD only when both are HEAD's tree and HEAD did not
// move. A run that passes on a tree that stood still writes that tree's id into the git directory
// (local-limits.mjs, PASSED), and any other run takes it away: git's pre-push hook of this
// repository (tools/git-hooks/pre-push, tools/push.mjs) refuses a push of a commit whose tree is not
// that one, and quick runs nothing until core.hooksPath names that hook's directory.

import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { BUILD, acquireSlot, requireRunner, runCapped } from "./local-limits.mjs";
import { PASSED, requireHooks } from "./local-limits.mjs";

/**
 * git's output, line by line, with `env` added to its environment and `input` on its standard
 * input; or, with `nul`, its entries ended by NUL, as `-z` writes them, names beyond ASCII as they
 * are and never quoted.
 */
function git(args, env = {}, { input, nul = false } = {}) {
  const out = execFileSync("git", args, { encoding: "utf8", env: { ...process.env, ...env }, input });
  return out.split(nul ? "\0" : /\r?\n/).filter(Boolean);
}

/** The files the index of `env` marks assume-unchanged (a small letter) or skip-worktree (S): `ls-files -v`'s entries. */
function marked(env = {}) {
  return git(["ls-files", "-v", "-z"], env, { nul: true }).filter((entry) => /^([a-z]|S) /.test(entry));
}

/**
 * The id of the tree the working tree would commit: every tracked and unignored file as it stands,
 * however the repository's index marks it, through an index of its own, which leaves the
 * repository's index as it was. It starts from a copy of the repository's index, whose modes a
 * commit takes (a file staged executable where core.fileMode is false, as on Windows, which an
 * index read from HEAD would take as not: the push of 203's third round met it), with the marks
 * that keep git from comparing a file, assume-unchanged and skip-worktree, taken off.
 */
function workingTree() {
  const index = git(["rev-parse", "--git-path", `quick-index-${process.pid}`])[0];
  const own = git(["rev-parse", "--git-path", "index"])[0];
  try {
    if (existsSync(own)) copyFileSync(own, index);
    else git(["read-tree", "HEAD"], { GIT_INDEX_FILE: index });
    // Read and written NUL-ended: a name beyond ASCII, which `ls-files -v` alone quotes, would
    // reach update-index quoted and keep its mark (the third review of #203); and no command line
    // grows with the names.
    const names = marked({ GIT_INDEX_FILE: index }).map((entry) => entry.slice(2));
    if (names.length) git(["update-index", "--no-assume-unchanged", "--no-skip-worktree", "-z", "--stdin"], { GIT_INDEX_FILE: index }, { input: `${names.join("\0")}\0` });
    git(["add", "--all"], { GIT_INDEX_FILE: index });
    return git(["write-tree"], { GIT_INDEX_FILE: index })[0];
  } finally {
    rmSync(index, { force: true });
  }
}

/** HEAD's commit and the working tree's tree, as they stand now. */
function snapshot() {
  return { head: git(["rev-parse", "HEAD"])[0], tree: workingTree() };
}

/**
 * Says whether the checks read what a push sends: HEAD's tree, before the checks and after them, or
 * a tree that differs from it, or one that changed while they ran; and then git's short status of
 * what differs, with the files git does not compare, marked assume-unchanged or skip-worktree.
 * Returns whether the tree stood still.
 */
function reportTree(before, listing = false) {
  const after = snapshot();
  const headTree = git(["rev-parse", "HEAD^{tree}"])[0];
  const still = before.head === after.head && before.tree === after.tree;
  if (still && after.tree === headTree) {
    console.log(listing
      ? "\nquick: the working tree is HEAD's tree: no tracked or unignored file differs from HEAD, so checks run now would read what a push sends."
      : "\nquick: the working tree is HEAD's tree, before the checks and after them: no tracked or unignored file differs from HEAD, so these checks read what a push sends.");
    return true;
  }
  const status = git(["status", "--porcelain", "--untracked-files=all"]);
  const hidden = marked();
  if (!still) {
    console.log("\nquick: the working tree changed while the checks ran, or HEAD moved, so this run passed no one tree; run it again on the tree to push. What differs from HEAD now, as git's short status writes it:");
  } else if (status.length === 0) {
    console.log("\nquick: the working tree differs from HEAD only in files git does not compare, which these checks read and a push does not send; this is no pass of HEAD until they are committed:");
  } else {
    console.log(`\nquick: the working tree differs from HEAD, in ${status.length} entr${status.length === 1 ? "y" : "ies"} of git's short status, which these checks read and a push does not send; this is no pass of HEAD until they are committed:`);
  }
  for (const line of status) console.log(`  ${line}`);
  if (hidden.length) {
    console.log("and these files, which git does not compare, marked assume-unchanged (a small letter) or skip-worktree (S):");
    for (const line of hidden) console.log(`  ${line}`);
  }
  return still;
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

// From the repository's root, whatever directory it is run from: the map's paths, the checks'
// commands and git's list of new files are the root's.
process.chdir(git(["rev-parse", "--show-toplevel"])[0]);

/** Every file the branch changes against origin/main, committed or not, and every new file. */
function changedFiles() {
  const base = git(["merge-base", "origin/main", "HEAD"])[0];
  return new Set([
    ...git(["diff", "--name-only", `${base}...HEAD`]),
    ...git(["diff", "--name-only", "HEAD"]),
    ...git(["ls-files", "--others", "--exclude-standard"]),
  ]);
}

const dash = process.argv.indexOf("--");
const ownTests = dash >= 0 ? process.argv.slice(dash + 1) : [];
const changed = changedFiles();
const due = CHECKS.filter((c) => [...changed].some((f) => c.when.test(f)));
if (ownTests.length) due.push({ name: `the change's own tests (cargo test ${ownTests.join(" ")})`, run: ["cargo", "test", ...ownTests] });
if (due.length === 0) {
  // HEAD is a commit of main and nothing differs from it, so there is nothing to check; the record
  // is HEAD's tree, which the merge queue tested, so that a push of it is not sent back here (the
  // fourth review of #203).
  console.log("quick: nothing changed against origin/main, so nothing to check");
  if (!process.argv.includes("--list")) {
    const tree = workingTree();
    const passed = git(["rev-parse", "--git-path", PASSED])[0];
    if (tree === git(["rev-parse", "HEAD^{tree}"])[0]) writeFileSync(passed, `${tree}\n`);
    else rmSync(passed, { force: true });
  }
  process.exit(0);
}

console.log(`quick: ${changed.size} changed file(s); ${due.length} check(s)\n`);
const before = snapshot();
if (process.argv.includes("--list")) {
  for (const check of due) console.log(`  ${check.name}: ${check.run.join(" ")}`);
  reportTree(before, true);
  process.exit(0);
}
if (!requireRunner()) process.exit(2);
if (!requireHooks({ cwd: process.cwd() })) process.exit(2);
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
let still = false;
try {
  still = reportTree(before);
} catch (e) {
  console.log(`\nquick: could not tell whether the checks read HEAD's tree: ${e.message.split(/\r?\n/)[0]}`);
}
const passed = git(["rev-parse", "--git-path", PASSED])[0];
if (!failed && still) writeFileSync(passed, `${before.tree}\n`);
else rmSync(passed, { force: true });
process.exit(failed ? 1 : 0);
