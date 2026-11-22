// tools/quick.mjs says, at the end of a listing and of a run, whether its checks read what a push
// sends: HEAD, or a working tree that differs from it, with each file that differs, staged or not,
// and each new one, however deep. A file the branch has committed is no such file. Each case runs
// the real script in a repository this test makes, its `origin/main` one commit back, with git's
// own configuration left out.
//
//   node --test tools/quick.test.mjs
//
// The run stops at its first check where cargo's runner is not the capped one (tools/local-limits.mjs),
// and that case is then skipped; in the repository made here, every check fails at once.

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const QUICK = fileURLToPath(new URL("./quick.mjs", import.meta.url));

/** A repository with `origin/main` at its first commit and one commit beyond it. */
function repository() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "quick-"));
  const empty = path.join(dir, ".no-config");
  fs.writeFileSync(empty, "");
  const env = {
    ...process.env,
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: empty,
    GIT_AUTHOR_NAME: "fixture",
    GIT_AUTHOR_EMAIL: "fixture@example.invalid",
    GIT_COMMITTER_NAME: "fixture",
    GIT_COMMITTER_EMAIL: "fixture@example.invalid",
    GIT_AUTHOR_DATE: "2026-11-22T00:00:00Z",
    GIT_COMMITTER_DATE: "2026-11-22T00:00:00Z",
  };
  const work = path.join(dir, "work");
  fs.mkdirSync(work);
  const git = (...args) => execFileSync("git", args, { cwd: work, env, encoding: "utf8" });
  git("init", "-q", "-b", "main");
  fs.writeFileSync(path.join(work, "a.md"), "a\n");
  fs.writeFileSync(path.join(work, ".gitignore"), "out/\n");
  git("add", ".");
  git("commit", "-q", "-m", "base");
  git("update-ref", "refs/remotes/origin/main", "HEAD");
  fs.writeFileSync(path.join(work, "b.md"), "b\n");
  git("add", "b.md");
  git("commit", "-q", "-m", "committed");
  const quick = (...args) => spawnSync(process.execPath, [QUICK, ...args], { cwd: work, env, encoding: "utf8" });
  return { dir, work, git, quick };
}

/** What `quick` said of the working tree: its report's lines, from its first to the end. */
function report(out) {
  const at = out.lastIndexOf("\nquick: the working tree ");
  assert.ok(at >= 0, `no report of the working tree in:\n${out}`);
  return out.slice(at + 1).trimEnd().split(/\r?\n/);
}

/** Changes the working tree: one file changed, one staged, one new in a new directory, one ignored. */
function change({ work, git }) {
  fs.writeFileSync(path.join(work, "a.md"), "a, changed\n");
  fs.writeFileSync(path.join(work, "c.md"), "c\n");
  git("add", "c.md");
  fs.mkdirSync(path.join(work, "notes"));
  fs.writeFileSync(path.join(work, "notes", "d.md"), "d\n");
  fs.mkdirSync(path.join(work, "out"));
  fs.writeFileSync(path.join(work, "out", "e.md"), "e\n");
}

const DIFFERS = [
  "quick: the working tree differs from HEAD in 3 file(s), which these checks read and a push does not send; this is no pass of HEAD until they are committed:",
  "   M a.md",
  "  A  c.md",
  "  ?? notes/d.md",
];

test("a listing says the checks read HEAD when the working tree is HEAD, and each file that differs when it is not", () => {
  const r = repository();
  try {
    const clean = r.quick("--list");
    assert.equal(clean.status, 0, clean.stderr);
    assert.deepEqual(report(clean.stdout), ["quick: the working tree is HEAD, so these checks read what a push sends."]);
    change(r);
    const dirty = r.quick("--list");
    assert.equal(dirty.status, 0, dirty.stderr);
    assert.deepEqual(report(dirty.stdout), DIFFERS);
  } finally {
    fs.rmSync(r.dir, { recursive: true, force: true });
  }
});

test("a run ends with the same report, after its verdict, whatever the checks gave", (t) => {
  const r = repository();
  try {
    change(r);
    const run = r.quick();
    if (run.status === 2 && /cargo's runner|CARGO_TARGET_.*_RUNNER/.test(run.stderr)) {
      t.skip("cargo's runner is not the capped one here, so quick.mjs runs no check");
      return;
    }
    assert.equal(run.status, 1, `every check fails in the repository made here:\n${run.stdout}\n${run.stderr}`);
    const out = run.stdout;
    assert.ok(out.indexOf("\nquick:\n") < out.lastIndexOf("\nquick: the working tree "), "the report comes after the verdict");
    assert.deepEqual(report(out), DIFFERS);
  } finally {
    fs.rmSync(r.dir, { recursive: true, force: true });
  }
});
