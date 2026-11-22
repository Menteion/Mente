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
// CI runs it beside the workflow policy (ci.yml, Documentation). Each repository stands in for an
// installed copy of main's gates (briefs/gate-tools-from-main.md) with a refresh entry that passes,
// a hook entry that core.hooksPath names, and a runner entry that CARGO_TARGET_<target>_RUNNER
// names, so that quick.mjs runs its checks on every platform.

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
    // The gates' stable entries, as an installed copy leaves them: the refresh entry, the hook entry
    // core.hooksPath names by its absolute path, and the runner entry cargo's runner names.
    const gates = path.join(work, ".git", "mente-gates");
    const hooks = path.join(gates, "entry", "hooks").replace(/\\/g, "/");
    fs.mkdirSync(hooks, { recursive: true });
    fs.writeFileSync(path.join(hooks, "pre-push"), "#!/bin/sh\nexit 0\n");
    fs.writeFileSync(path.join(gates, "entry", "refresh.mjs"), "process.exit(0);\n");
    fs.writeFileSync(path.join(gates, "entry", "capped.mjs"), "process.exit(0);\n");
    env.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER = `node ${path.join(gates, "entry", "capped.mjs").replace(/\\/g, "/")} --mente-only --`;
    git("config", "core.hooksPath", hooks);
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
    const quick = (args = [], shim = "pass", cwd = work, extra = {}) =>
      spawnSync(process.execPath, [QUICK, ...args], { cwd, env: { ...env, QUICK_SHIM: shim, ...extra }, encoding: "utf8" });
    const passed = () => {
      const file = path.join(work, git("rev-parse", "--git-path", "quick-passed"));
      return fs.existsSync(file) ? fs.readFileSync(file, "utf8").trim() : null;
    };
    body({ dir, work, git, quick, passed, hooks, gates });
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

test("a new file staged executable is recorded as a commit takes it, so the record is HEAD's tree after the commit", (t) => {
  withRepository((r) => {
    const file = path.join(r.work, "run.sh");
    fs.writeFileSync(file, "#!/bin/sh\nexit 0\n");
    fs.chmodSync(file, 0o755);
    r.git("add", "--chmod=+x", "run.sh");
    const run = r.quick();
    if (noRunner(run)) return t.skip("cargo's runner is not the capped one here, so quick.mjs runs no check");
    assert.equal(run.status, 0, `${run.stdout}\n${run.stderr}`);
    r.git("commit", "-q", "-m", "an executable file");
    assert.equal(r.git("ls-files", "-s", "run.sh").split(" ")[0], "100755");
    assert.equal(r.passed(), r.git("rev-parse", "HEAD^{tree}"));
  });
});

test("a run takes its gates from what core.hooksPath names, and refuses where that holds no hook, saying what to do", (t) => {
  withRepository((r) => {
    const first = r.quick();
    if (noRunner(first)) return t.skip("cargo's runner is not the capped one here, so quick.mjs runs no check");
    assert.equal(first.status, 0, `${first.stdout}\n${first.stderr}`);
    // A worktree runs the same entry, in the common git directory, not one of its own.
    const other = path.join(r.dir, "other");
    r.git("worktree", "add", "-q", other, "-b", "other");
    const inOther = r.quick([], "pass", other);
    assert.equal(inOther.status, 0, `a worktree runs the gates' hook entry: ${inOther.stdout}
${inOther.stderr}`);
    // Until the round's second pull request, a core.hooksPath that names no stable entry means
    // today's working-tree gates (FALLBACK): the main checkout's hooks, with its runner, which a
    // stand-in here is, since on Windows the runner is read first.
    fs.mkdirSync(path.join(r.work, "tools"), { recursive: true });
    fs.appendFileSync(path.join(r.work, ".git", "info", "exclude"), "tools/\n");
    fs.writeFileSync(path.join(r.work, "tools", "capped.mjs"), "process.exit(0);\n");
    const today = { CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER: `node ${path.join(r.work, "tools", "capped.mjs").replace(/\\/g, "/")} --mente-only --` };
    const mainHooks = path.join(r.work, "tools", "git-hooks").replace(/\\/g, "/");
    for (const [value, words] of [
      [null, /core\.hooksPath is not set, not the main checkout's hooks/],
      ["tools/git-hooks", /core\.hooksPath is "tools\/git-hooks", not the main checkout's hooks/],
      [path.join(other, "tools", "git-hooks").replace(/\\/g, "/"), /not the main checkout's hooks/],
    ]) {
      if (value === null) r.git("config", "--unset", "core.hooksPath");
      else r.git("config", "core.hooksPath", value);
      const run = r.quick([], "pass", r.work, today);
      assert.equal(run.status, 2, `${value}: ${run.stdout}\n${run.stderr}`);
      assert.match(run.stderr, /WARNING: a copy of main's gates is installed/);
      assert.match(run.stderr, words);
      assert.ok(run.stderr.includes(`git config core.hooksPath ${mainHooks}`), run.stderr);
      assert.doesNotMatch(run.stdout, /\n== /, "no check ran");
    }
    r.git("config", "core.hooksPath", r.hooks);
    fs.rmSync(path.join(r.hooks, "pre-push"));
    const empty = r.quick();
    assert.equal(empty.status, 2);
    assert.match(empty.stderr, /holds no pre-push hook/);
  });
});

test("a file git does not compare is held to the tree whatever its name, one beyond ASCII among them", () => {
  withRepository((r) => {
    fs.writeFileSync(path.join(r.work, "caf\u00e9.md"), "c\n");
    r.git("add", "caf\u00e9.md");
    r.git("commit", "-q", "-m", "a name beyond ASCII");
    r.git("update-index", "--assume-unchanged", "caf\u00e9.md");
    fs.writeFileSync(path.join(r.work, "caf\u00e9.md"), "c, changed\n");
    const listing = r.quick(["--list"]);
    assert.equal(listing.status, 0, listing.stderr);
    const lines = report(listing.stdout);
    assert.match(lines[0], /^quick: the working tree differs from HEAD only in files git does not compare/);
    assert.ok(lines.includes("  h caf\u00e9.md"), lines.join("\n"));
  });
});

test("a run on a commit of main with nothing changed checks nothing and records HEAD's tree; a listing records none", () => {
  withRepository((r) => {
    r.git("update-ref", "refs/remotes/origin/main", "HEAD");
    const listing = r.quick(["--list"]);
    assert.equal(listing.status, 0, listing.stderr);
    assert.match(listing.stdout, /nothing changed against origin\/main/);
    assert.equal(r.passed(), null, "a listing records no tree");
    const run = r.quick();
    assert.equal(run.status, 0, `${run.stdout}\n${run.stderr}`);
    assert.match(run.stdout, /nothing changed against origin\/main/);
    assert.equal(r.passed(), r.git("rev-parse", "HEAD^{tree}"));
  });
});

test("the tools refresh and require the gates of their own checkout, wherever they are run from", () => {
  const ROOT = path.resolve(path.dirname(QUICK), "..");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "elsewhere-"));
  try {
    const other = path.join(dir, "other");
    fs.mkdirSync(other);
    execFileSync("git", ["init", "-q", other]);
    for (const cwd of [dir, other]) {
      const cargo = spawnSync(process.execPath, [path.join(ROOT, "tools", "cargo.mjs"), "--version"], { cwd, encoding: "utf8" });
      assert.doesNotMatch(cargo.stderr, /could not name the repository/, cwd);
      assert.ok(!cargo.stderr.includes(other.replace(/\\/g, "/")) && !cargo.stderr.includes(other), `cargo.mjs from ${cwd} named that repository: ${cargo.stderr}`);
    }
    for (const tool of ["cargo.mjs", "mn.mjs"]) {
      const text = fs.readFileSync(path.join(ROOT, "tools", tool), "utf8");
      assert.match(text, /refreshGates\(\{ cwd: ROOT \}\)/, `${tool} refreshes its own checkout's gates`);
      assert.match(text, /requireHooks\(\{ cwd: ROOT(, fallback)? \}\)/, `${tool} asks for its own checkout's hooks`);
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a run refreshes the gates first, and a refusing refresh entry stops it", () => {
  withRepository((r) => {
    fs.writeFileSync(path.join(r.gates, "entry", "refresh.mjs"), 'console.error("the copy has drifted"); process.exit(2);\n');
    const refused = r.quick();
    assert.equal(refused.status, 2, `${refused.stdout}\n${refused.stderr}`);
    assert.match(refused.stderr, /the copy has drifted/);
    assert.doesNotMatch(refused.stdout, /\n== /, "a check ran past a refusing refresh entry");
  });
});

test("with no copy installed, a run warns with the bootstrap and Norman's two lines and keeps today's working-tree gates, until the round's second pull request", () => {
  withRepository((r) => {
    fs.rmSync(path.join(r.gates, "entry"), { recursive: true });
    // Today's working-tree gates: the main checkout's tools/capped.mjs, which cargo's runner names,
    // and its tools/git-hooks, which core.hooksPath names; ignored, so that no check runs for them.
    fs.appendFileSync(path.join(r.work, ".git", "info", "exclude"), "tools/\n");
    const hooks = path.join(r.work, "tools", "git-hooks").replace(/\\/g, "/");
    fs.mkdirSync(hooks, { recursive: true });
    fs.writeFileSync(path.join(hooks, "pre-push"), "#!/bin/sh\nexit 0\n");
    fs.writeFileSync(path.join(r.work, "tools", "capped.mjs"), "process.exit(0);\n");
    const runner = `node ${path.join(r.work, "tools", "capped.mjs").replace(/\\/g, "/")} --mente-only --`;
    r.git("config", "core.hooksPath", hooks);
    const kept = r.quick([], "pass", r.work, { CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER: runner });
    assert.equal(kept.status, 0, `${kept.stdout}\n${kept.stderr}`);
    assert.match(kept.stderr, /WARNING: no copy of main's gates is installed/);
    assert.match(kept.stderr, /\$c = \(git ls-remote https:\/\/\S+ refs\/heads\/main\)/);
    assert.match(kept.stderr, /runner = \["node", ".*\/mente-gates\/entry\/capped\.mjs", "--mente-only", "--"\]/);
    assert.match(kept.stderr, /git config core\.hooksPath .*\/mente-gates\/entry\/hooks/);
    // core.hooksPath naming the stable entry asks for a copy: with none installed, it names the
    // bootstrap.
    r.git("config", "core.hooksPath", r.hooks);
    const early = r.quick([], "pass", r.work, { CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER: runner });
    assert.equal(early.status, 2);
    assert.match(early.stderr, /core\.hooksPath names the gates' stable hook entry, but no copy of main's gates is installed .*the bootstrap/s);
  });
});

test("a copy installed while Norman's lines name today's gates is refreshed, and changes nothing until they name it", () => {
  withRepository((r) => {
    // The stand-in refresh entry says it ran.
    const ran = path.join(r.dir, "refreshed");
    fs.writeFileSync(path.join(r.gates, "entry", "refresh.mjs"), `require("node:fs").writeFileSync(${JSON.stringify(ran)}, ""); process.exit(2);\n`.replace('require("node:fs")', '(await import("node:fs"))'));
    fs.appendFileSync(path.join(r.work, ".git", "info", "exclude"), "tools/\n");
    const hooks = path.join(r.work, "tools", "git-hooks").replace(/\\/g, "/");
    fs.mkdirSync(hooks, { recursive: true });
    fs.writeFileSync(path.join(hooks, "pre-push"), "#!/bin/sh\nexit 0\n");
    fs.writeFileSync(path.join(r.work, "tools", "capped.mjs"), "process.exit(0);\n");
    r.git("config", "core.hooksPath", hooks);
    const run = r.quick([], "pass", r.work, { CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUNNER: `node ${path.join(r.work, "tools", "capped.mjs").replace(/\\/g, "/")} --mente-only --` });
    assert.equal(run.status, 0, `${run.stdout}\n${run.stderr}`);
    assert.ok(fs.existsSync(ran), "the installed copy was not refreshed");
    assert.match(run.stderr, /WARNING: a copy of main's gates is installed in .*, but Norman's lines still name today's working-tree gates/s);
    assert.match(run.stderr, /every open branch merges main/);
    assert.match(run.stderr, /was not refreshed, which changes nothing until Norman's lines name it/);
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
