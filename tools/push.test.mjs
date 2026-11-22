// The push guard (tools/push.mjs): git's own pre-push hook of this repository
// (tools/git-hooks/pre-push), run by real pushes to a bare remote this test makes, and the narrow
// rule of .claude/settings.json against bypassing it (tools/push-guard.mjs). Each repository is
// made with git's own configuration and the caller's GIT_ variables left out, its core.hooksPath
// naming this checkout's hooks. A pass of the quick checks is written as tools/quick.mjs writes it
// (local-limits.mjs, PASSED), so that each case sets the record it needs.
//
//   node --test tools/push.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { PASSED } from "./local-limits.mjs";
import { QUICK } from "./push.mjs";

const HOOKS = fileURLToPath(new URL("./git-hooks", import.meta.url));
const GUARD = fileURLToPath(new URL("./push-guard.mjs", import.meta.url));

/** A shell that reads `$(...)`: sh, or on Windows Git for Windows' own. */
function shell() {
  if (process.platform !== "win32") return "sh";
  const sh = path.resolve(execFileSync("git", ["--exec-path"], { encoding: "utf8" }).trim(), "..", "..", "..", "usr", "bin", "sh.exe");
  return fs.existsSync(sh) ? sh : null;
}

/**
 * Runs `body` with a checkout whose `origin` is a bare repository, both made for it, `main` pushed
 * to it while the hooks were off, and removes them after.
 */
function withRemote(body) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "push-"));
  try {
    const empty = path.join(dir, "no-config");
    fs.writeFileSync(empty, "");
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^GIT_/i.test(k)));
    Object.assign(env, {
      GIT_CONFIG_NOSYSTEM: "1",
      GIT_CONFIG_GLOBAL: empty,
      GIT_AUTHOR_NAME: "fixture",
      GIT_AUTHOR_EMAIL: "fixture@example.invalid",
      GIT_COMMITTER_NAME: "fixture",
      GIT_COMMITTER_EMAIL: "fixture@example.invalid",
    });
    const remote = path.join(dir, "remote.git");
    const work = path.join(dir, "work");
    const run = (cwd, args, file = "git") => spawnSync(file, args, { cwd, env, encoding: "utf8" });
    const git = (cwd, ...args) => {
      const r = run(cwd, args);
      if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr}`);
      return r.stdout.trim();
    };
    git(dir, "init", "-q", "--bare", remote);
    fs.mkdirSync(work);
    git(work, "init", "-q", "-b", "main");
    fs.writeFileSync(path.join(work, "a.md"), "a\n");
    git(work, "add", ".");
    git(work, "commit", "-q", "-m", "base");
    git(work, "remote", "add", "origin", remote);
    git(work, "push", "-q", "origin", "main");
    git(work, "config", "core.hooksPath", HOOKS);
    const tree = (at, ref = "HEAD") => git(at, "rev-parse", `${ref}^{tree}`);
    /** Writes the record of a pass of `id` in the checkout `at`, as tools/quick.mjs does. */
    const pass = (at, id) => fs.writeFileSync(path.resolve(at, git(at, "rev-parse", "--git-path", PASSED)), `${id}\n`);
    /** A push in `at` with these words: its status and git's words, the hook's among them. */
    const push = (at, ...args) => {
      const r = run(at, ["push", "-q", ...args]);
      return { status: r.status, words: `${r.stdout}${r.stderr}` };
    };
    const commit = (at, file, text) => {
      fs.writeFileSync(path.join(at, file), text);
      git(at, "add", file);
      git(at, "commit", "-q", "-m", file);
    };
    const remoteHas = (ref) => run(remote, ["rev-parse", "--verify", "--quiet", ref]).stdout.trim();
    body({ dir, work, remote, run, git, tree, pass, push, commit, remoteHas });
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const refused = (r, words) => {
  assert.notEqual(r.status, 0, r.words);
  assert.match(r.words, /the pre-push hook refused/);
  if (words) assert.match(r.words, words);
};
const sent = (r) => assert.equal(r.status, 0, r.words);

test("a push sends only commits whose tree quick last passed in the checkout", () => {
  withRemote(({ work, git, tree, pass, push, commit, remoteHas }) => {
    commit(work, "b.md", "b\n");
    refused(push(work, "origin", "main"), new RegExp(`no tree has passed the quick checks.*run ${QUICK}`));
    assert.notEqual(remoteHas("main"), git(work, "rev-parse", "HEAD"), "nothing reached the remote");
    pass(work, tree(work));
    sent(push(work, "origin", "main"));
    assert.equal(remoteHas("main"), git(work, "rev-parse", "HEAD"));
    // A commit after the pass makes the record stale.
    commit(work, "c.md", "c\n");
    refused(push(work, "origin", "main"), /not the tree the quick checks last passed/);
    // A record that holds no tree refuses.
    fs.writeFileSync(path.resolve(work, git(work, "rev-parse", "--git-path", PASSED)), "not a tree\n");
    refused(push(work, "origin", "main"), /holds no tree id/);
    // A deletion sends nothing.
    git(work, "branch", "gone", "main~1");
    pass(work, tree(work, "gone"));
    sent(push(work, "origin", "gone"));
    fs.rmSync(path.resolve(work, git(work, "rev-parse", "--git-path", PASSED)));
    sent(push(work, "origin", "--delete", "gone"));
    assert.equal(remoteHas("gone"), "");
  });
});

test("every push git makes is held, however its refs are named: configuration, --repo beside the repository, a refspec a shell computes", () => {
  withRemote(({ work, run, git, tree, pass, push, commit }) => {
    // \`other\` on the remote, then a commit on it that no pass read; main passed and pushed.
    git(work, "branch", "other");
    pass(work, tree(work, "other"));
    sent(push(work, "origin", "other"));
    git(work, "checkout", "-q", "other");
    commit(work, "o.md", "o\n");
    git(work, "checkout", "-q", "main");
    pass(work, tree(work));
    const cases = [
      ["-c push.default=matching, pushing every branch both sides have", ["-c", "push.default=matching", "push", "-q", "origin"]],
      ["remote.origin.push naming the refspec", ["-c", "remote.origin.push=refs/heads/other:refs/heads/other", "push", "-q", "origin"]],
      ["remote.origin.mirror, pushing every ref", ["-c", "remote.origin.mirror=true", "push", "-q", "origin"]],
    ];
    for (const [what, args] of cases) {
      const r = run(work, args);
      refused({ status: r.status, words: r.stderr }, /refs\/heads\/other/);
      assert.ok(r.status !== 0, what);
    }
    // --repo= beside the repository word, from a branch with an upstream: git pushes that branch.
    git(work, "checkout", "-q", "other");
    git(work, "branch", "-q", "-u", "origin/other");
    refused(push(work, "--repo=origin", "origin"), /refs\/heads\/other/);
    git(work, "checkout", "-q", "main");
    git(work, "config", "remote.origin.push", "refs/heads/other:refs/heads/other");
    refused(push(work, "origin"), /refs\/heads\/other/);
    git(work, "config", "--unset", "remote.origin.push");
    // One push of two refs, the first passed and the second not: every ref git sends is held.
    commit(work, "n.md", "n\n");
    pass(work, tree(work));
    refused(push(work, "origin", "main", "other"), /refs\/heads\/other/);
    const sh = shell();
    if (sh) {
      const r = run(work, ["-c", 'git push -q origin "$(echo other)"'], sh);
      refused({ status: r.status, words: r.stderr }, /refs\/heads\/other/);
    }
  });
});

test("a merge in progress refuses, and each worktree is held to its own record", () => {
  withRemote(({ dir, work, git, tree, pass, push, commit }) => {
    git(work, "checkout", "-q", "-b", "side");
    commit(work, "a.md", "side\n");
    git(work, "checkout", "-q", "main");
    commit(work, "a.md", "main\n");
    pass(work, tree(work));
    assert.throws(() => git(work, "merge", "side"), "the merge stops on its conflict");
    refused(push(work, "origin", "main"), /a merge is in progress/);
    git(work, "merge", "--abort");
    sent(push(work, "origin", "main"));
    // A second worktree, whose record is its own.
    const other = path.join(dir, "other");
    git(work, "worktree", "add", "-q", other, "-b", "two");
    commit(other, "t.md", "t\n");
    refused(push(other, "origin", "two"), /no tree has passed the quick checks/);
    pass(other, tree(work));
    refused(push(other, "origin", "two"), /not the tree the quick checks last passed/);
    pass(other, tree(other));
    sent(push(other, "origin", "two"));
    commit(work, "m.md", "m\n");
    refused(push(work, "origin", "main"), /not the tree the quick checks last passed/);
  });
});

/** The settings' hook's answer to `command`, given as `tool` gives it. */
function guard(command, tool = "Bash") {
  const r = spawnSync(process.execPath, [GUARD], { input: JSON.stringify({ hook_event_name: "PreToolUse", tool_name: tool, tool_input: { command, cwd: os.tmpdir() } }), encoding: "utf8" });
  return { status: r.status, words: r.stderr.trim() };
}

test("the settings refuse the ways around the hook, and only those", () => {
  for (const command of [
    "git push --no-verify",
    "git push --no-veri origin main",
    "timeout 60 git push --no-verify",
    'bash -c "git push --no-verify"',
    "git -c core.hooksPath=/dev/null push",
    "git -ccore.HooksPath=x commit -m y",
    "git --config-env=core.hooksPath=HOOKS push",
    "git --config-env core.hooksPath=HOOKS status",
    "GIT_DIR=elsewhere git push",
    "env GIT_CONFIG_COUNT=1 git push",
    "$env:GIT_DIR='x'; git push",
  ]) {
    for (const tool of ["Bash", "PowerShell", "Monitor", "mcp__terminal__run_in_terminal"]) {
      const r = guard(command, tool);
      assert.equal(r.status, 2, `${tool}: ${command}: ${r.words}`);
      assert.match(r.words, /^the push guard refused it: /);
    }
  }
  for (const command of ["git push", "git push -u origin main", "git push -n origin main", 'git commit -m "--no-verify later"', "git status", "echo push", "git -c user.name=x push"]) {
    assert.deepEqual(guard(command), { status: 0, words: "" }, command);
  }
  const ws = spawnSync(process.execPath, [GUARD], { input: JSON.stringify({ tool_name: "Monitor", tool_input: { ws: { url: "wss://example.invalid" } } }), encoding: "utf8" });
  assert.equal(ws.status, 0, "a Monitor of a WebSocket runs no command");
  for (const input of ["{", JSON.stringify({ tool_input: {} })]) {
    const r = spawnSync(process.execPath, [GUARD], { input, encoding: "utf8" });
    assert.equal(r.status, 2, input);
    assert.match(r.stderr, /the push guard (could not read|found no command)/);
  }
});
