// tools/quick.mjs says, at the end of a listing and of a run, whether its checks read what a push
// sends: HEAD's tree, before the checks and after them; or a working tree that differs from it,
// with each entry of git's short status and each file git does not compare; or a tree that changed
// while the checks ran. A run that passes on a tree that stood still writes that tree's id into the
// git directory, and any other run takes it away. Each case runs the real script in a repository
// this test makes, its `origin/main` one commit back, with git's own configuration and the caller's
// GIT_ variables left out, and slots of its own, never the machine's.
//
// The checks' commands, cargo and npm, are shims put first on PATH, which pass, fail, change a file
// or commit as a case asks: on Windows cargo.cmd and npm.cmd, which capped.ps1 finds as
// applications, and elsewhere executable scripts.
//
//   node --test tools/quick.test.mjs
//
// CI runs it beside the workflow policy (ci.yml, Documentation). Where cargo's runner is not the
// capped one (tools/local-limits.mjs), quick.mjs runs no check, and the cases of a run are skipped.

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const QUICK = fileURLToPath(new URL("./quick.mjs", import.meta.url));
const WINDOWS = process.platform === "win32";

/**
 * The shims of cargo and npm in `dir`, which do what QUICK_SHIM says, and fail with 3 outside the
 * repository's root, where quick.mjs runs its checks.
 */
function shims(dir) {
  fs.mkdirSync(dir);
  for (const name of ["cargo", "npm"]) {
    if (WINDOWS) {
      fs.writeFileSync(
        path.join(dir, `${name}.cmd`),
        [
          "@echo off",
          "if not exist a.md exit /b 3",
          'if "%QUICK_SHIM%"=="fail" exit /b 1',
          'if "%QUICK_SHIM%"=="touch" echo changed>>a.md',
          'if "%QUICK_SHIM%"=="commit" (echo changed>>a.md& git commit -q -am during)',
          'if "%QUICK_SHIM%"=="empty" git commit -q --allow-empty -m during',
          "exit /b 0",
          "",
        ].join("\r\n"),
      );
    } else {
      const file = path.join(dir, name);
      fs.writeFileSync(
        file,
        '#!/bin/sh\n[ -f a.md ] || exit 3\ncase "$QUICK_SHIM" in\n  fail) exit 1 ;;\n  touch) echo changed >> a.md ;;\n  commit) echo changed >> a.md; git commit -q -am during ;;\n  empty) git commit -q --allow-empty -m during ;;\nesac\nexit 0\n',
      );
      fs.chmodSync(file, 0o755);
    }
  }
}

/** Runs `body` with a repository made for it, `origin/main` one commit back, and removes it after. */
function withRepository(body) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "quick-"));
  try {
    const empty = path.join(dir, "no-config");
    fs.writeFileSync(empty, "");
    fs.mkdirSync(path.join(dir, "slots"));
    shims(path.join(dir, "shims"));
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^GIT_/i.test(k)));
    const pathKey = Object.keys(env).find((k) => k.toUpperCase() === "PATH") ?? "PATH";
    Object.assign(env, {
      [pathKey]: `${path.join(dir, "shims")}${path.delimiter}${env[pathKey] ?? ""}`,
      MENTE_SLOTS_DIR: path.join(dir, "slots"),
      GIT_CONFIG_NOSYSTEM: "1",
      GIT_CONFIG_GLOBAL: empty,
      GIT_AUTHOR_NAME: "fixture",
      GIT_AUTHOR_EMAIL: "fixture@example.invalid",
      GIT_COMMITTER_NAME: "fixture",
      GIT_COMMITTER_EMAIL: "fixture@example.invalid",
      GIT_AUTHOR_DATE: "2026-11-22T00:00:00Z",
      GIT_COMMITTER_DATE: "2026-11-22T00:00:00Z",
    });
    const work = path.join(dir, "work");
    fs.mkdirSync(work);
    const git = (...args) => execFileSync("git", args, { cwd: work, env, encoding: "utf8" }).trim();
    git("init", "-q", "-b", "main");
    git("config", "core.hooksPath", "tools/git-hooks");
    fs.writeFileSync(path.join(work, "a.md"), "a\n");
    fs.writeFileSync(path.join(work, ".gitignore"), "out/\n");
    fs.mkdirSync(path.join(work, "sub"));
    fs.writeFileSync(path.join(work, "sub", "s.md"), "s\n");
    git("add", ".");
    git("commit", "-q", "-m", "base");
    git("update-ref", "refs/remotes/origin/main", "HEAD");
    fs.writeFileSync(path.join(work, "b.md"), "b\n");
    fs.writeFileSync(path.join(work, "hidden.md"), "h\n");
    git("add", "b.md", "hidden.md");
    git("commit", "-q", "-m", "committed");
    const quick = (args = [], shim = "pass", cwd = work) =>
      spawnSync(process.execPath, [QUICK, ...args], { cwd, env: { ...env, QUICK_SHIM: shim }, encoding: "utf8" });
    const passed = () => {
      const file = path.join(work, git("rev-parse", "--git-path", "quick-passed"));
      return fs.existsSync(file) ? fs.readFileSync(file, "utf8").trim() : null;
    };
    body({ work, git, quick, passed });
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

/** What quick said of the working tree: its report's lines, from its first to the end. */
function report(out) {
  const at = out.lastIndexOf("\nquick: the working tree ");
  assert.ok(at >= 0, `no report of the working tree in:\n${out}`);
  return out.slice(at + 1).trimEnd().split(/\r?\n/);
}

/**
 * Changes the working tree: one file changed, one staged, one new in a new directory, one ignored,
 * and one changed that the index marks assume-unchanged.
 */
function change({ work, git }) {
  fs.writeFileSync(path.join(work, "a.md"), "a, changed\n");
  fs.writeFileSync(path.join(work, "c.md"), "c\n");
  git("add", "c.md");
  fs.mkdirSync(path.join(work, "notes"));
  fs.writeFileSync(path.join(work, "notes", "d.md"), "d\n");
  fs.mkdirSync(path.join(work, "out"));
  fs.writeFileSync(path.join(work, "out", "e.md"), "e\n");
  git("update-index", "--assume-unchanged", "hidden.md");
  fs.writeFileSync(path.join(work, "hidden.md"), "h, changed\n");
}

const CLEAN = [
  "quick: the working tree is HEAD's tree, before the checks and after them: no tracked or unignored file differs from HEAD, so these checks read what a push sends.",
];
const CLEAN_LISTING = [
  "quick: the working tree is HEAD's tree: no tracked or unignored file differs from HEAD, so checks run now would read what a push sends.",
];
const DIFFERS = [
  "quick: the working tree differs from HEAD, in 3 entries of git's short status, which these checks read and a push does not send; this is no pass of HEAD until they are committed:",
  "   M a.md",
  "  A  c.md",
  "  ?? notes/d.md",
  "and these files, which git does not compare, marked assume-unchanged (a small letter) or skip-worktree (S):",
  "  h hidden.md",
];

/** Whether a run ran no check, since cargo's runner is not the capped one here. */
const noRunner = (run) => run.status === 2 && /cargo's runner|CARGO_TARGET_.*_RUNNER/.test(run.stderr);

/** The report of a run comes after its verdict. */
function afterVerdict(out) {
  const verdict = out.indexOf("\nquick:\n");
  assert.ok(verdict >= 0, `no verdict in:\n${out}`);
  assert.ok(verdict < out.lastIndexOf("\nquick: the working tree "), "the report comes after the verdict");
}

test("a listing says the checks read HEAD's tree when nothing differs from it, and what differs when something does, a file git does not compare among it", () => {
  withRepository((r) => {
    const clean = r.quick(["--list"]);
    assert.equal(clean.status, 0, clean.stderr);
    assert.deepEqual(report(clean.stdout), CLEAN_LISTING);
    change(r);
    const dirty = r.quick(["--list"]);
    assert.equal(dirty.status, 0, dirty.stderr);
    assert.deepEqual(report(dirty.stdout), DIFFERS);
    assert.equal(r.passed(), null, "a listing passes no tree");
  });
});

test("a passing run on HEAD's tree says so and writes that tree's id; a failing run says the same and takes the id away", (t) => {
  withRepository((r) => {
    const pass = r.quick();
    if (noRunner(pass)) return t.skip("cargo's runner is not the capped one here, so quick.mjs runs no check");
    assert.equal(pass.status, 0, `${pass.stdout}\n${pass.stderr}`);
    afterVerdict(pass.stdout);
    assert.deepEqual(report(pass.stdout), CLEAN);
    assert.equal(r.passed(), r.git("rev-parse", "HEAD^{tree}"));
    const fail = r.quick([], "fail");
    assert.equal(fail.status, 1, `${fail.stdout}\n${fail.stderr}`);
    afterVerdict(fail.stdout);
    assert.deepEqual(report(fail.stdout), CLEAN);
    assert.equal(r.passed(), null, "a failing run takes the passing tree away");
  });
});

test("a passing run on a working tree that differs from HEAD exits 0, says what differs, and writes the id of the tree it read, which is not HEAD's", (t) => {
  withRepository((r) => {
    change(r);
    const pass = r.quick();
    if (noRunner(pass)) return t.skip("cargo's runner is not the capped one here, so quick.mjs runs no check");
    assert.equal(pass.status, 0, `${pass.stdout}\n${pass.stderr}`);
    afterVerdict(pass.stdout);
    assert.deepEqual(report(pass.stdout), DIFFERS);
    const tree = r.passed();
    assert.match(tree ?? "", /^[0-9a-f]{40}$/);
    assert.notEqual(tree, r.git("rev-parse", "HEAD^{tree}"));
    // The record is the whole working tree: committed whole, new files and the file git was told
    // not to compare among it, HEAD's tree is the tree the run read.
    r.git("update-index", "--no-assume-unchanged", "hidden.md");
    r.git("add", "-A");
    r.git("commit", "-q", "-m", "everything the run read");
    assert.equal(tree, r.git("rev-parse", "HEAD^{tree}"));
  });
});

test("a run refuses to start until core.hooksPath names the repository's hooks, and says the command to run", (t) => {
  withRepository((r) => {
    r.git("config", "--unset", "core.hooksPath");
    const run = r.quick();
    if (noRunner(run)) return t.skip("cargo's runner is not the capped one here, so quick.mjs runs no check");
    assert.equal(run.status, 2, `${run.stdout}\n${run.stderr}`);
    assert.match(run.stderr, /core\.hooksPath is not set/);
    assert.match(run.stderr, /git config core\.hooksPath tools\/git-hooks/);
    assert.doesNotMatch(run.stdout, /\n== /, "no check ran");
    r.git("config", "core.hooksPath", "elsewhere");
    const other = r.quick();
    assert.equal(other.status, 2);
    assert.match(other.stderr, /core\.hooksPath is "elsewhere"/);
  });
});

test("a run from a subdirectory runs its checks from the repository's root", (t) => {
  withRepository((r) => {
    const run = r.quick([], "pass", path.join(r.work, "sub"));
    if (noRunner(run)) return t.skip("cargo's runner is not the capped one here, so quick.mjs runs no check");
    assert.equal(run.status, 0, `${run.stdout}\n${run.stderr}`);
    assert.deepEqual(report(run.stdout), CLEAN);
  });
});

test("a run during which a file changes, a commit lands, or HEAD moves alone, says the tree changed while the checks ran, and passes no tree", (t) => {
  for (const shim of ["touch", "commit", "empty"]) {
    withRepository((r) => {
      const run = r.quick([], shim);
      if (noRunner(run)) return t.skip("cargo's runner is not the capped one here, so quick.mjs runs no check");
      assert.equal(run.status, 0, `${shim}: ${run.stdout}\n${run.stderr}`);
      afterVerdict(run.stdout);
      assert.equal(
        report(run.stdout)[0],
        "quick: the working tree changed while the checks ran, or HEAD moved, so this run passed no one tree; run it again on the tree to push. What differs from HEAD now, as git's short status writes it:",
        shim,
      );
      assert.equal(r.passed(), null, `${shim}: no tree passed`);
    });
  }
});
