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
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { PASSED } from "./local-limits.mjs";
import { QUICK, prePush } from "./push.mjs";

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
    const run = (cwd, args, file = "git", input) => spawnSync(file, args, { cwd, env, encoding: "utf8", input });
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
    // The branch holds the hook from here on, as a branch with main merged in does; origin/main,
    // one commit back, is from before it.
    fs.mkdirSync(path.join(work, "tools", "git-hooks"), { recursive: true });
    fs.writeFileSync(path.join(work, "tools", "git-hooks", "pre-push"), "#!/bin/sh\n");
    git(work, "add", ".");
    git(work, "commit", "-q", "-m", "the hook");
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
    body({ dir, work, remote, env, run, git, tree, pass, push, commit, remoteHas });
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
  withRemote(({ work, run, git, tree, pass, push, commit, remoteHas }) => {
    // A commit already on main passed the merge queue: a tag of it, or a branch at it, needs no
    // record of its own.
    git(work, "tag", "v1", "origin/main");
    sent(push(work, "origin", "v1"));
    git(work, "branch", "at-main", "origin/main");
    sent(push(work, "origin", "at-main"));
    commit(work, "b.md", "b\n");
    refused(push(work, "origin", "main"), new RegExp(`no tree has passed the quick checks.*run ${QUICK}`));
    assert.notEqual(remoteHas("main"), git(work, "rev-parse", "HEAD"), "nothing reached the remote");
    pass(work, tree(work));
    sent(push(work, "origin", "main"));
    assert.equal(remoteHas("main"), git(work, "rev-parse", "HEAD"));
    // A commit after the pass makes the record stale.
    commit(work, "c.md", "c\n");
    refused(push(work, "origin", "main"), /not the tree the quick checks last passed/);
    // A push by --git-dir from a directory with no hooks is held by the same hook and record.
    fs.mkdirSync(path.join(work, "sub"));
    const elsewhere = run(path.join(work, "sub"), ["--git-dir=../.git", "push", "-q", "origin", "main"]);
    refused({ status: elsewhere.status, words: elsewhere.stderr }, /not the tree the quick checks last passed/);
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

test("which commits main holds, the remote says: a locally moved origin/main lets nothing through, and a remote not asked refuses", () => {
  withRemote(({ dir, work, remote, env, run, git, tree, pass, push, commit }) => {
    // An unpassed commit on a branch, and origin/main moved onto it by hand, by update-ref and by
    // its --stdin: the remote's main does not hold it.
    git(work, "checkout", "-q", "-b", "side");
    commit(work, "s.md", "s\n");
    const unpassed = git(work, "rev-parse", "HEAD");
    git(work, "update-ref", "refs/remotes/origin/main", unpassed);
    refused(push(work, "origin", "side"), /no tree has passed the quick checks/);
    git(work, "update-ref", "refs/remotes/origin/main", git(work, "rev-parse", "main~1"));
    const stdin = run(work, ["update-ref", "--stdin"], "git", `update refs/remotes/origin/main ${unpassed}\n`);
    assert.equal(stdin.status, 0, stdin.stderr);
    assert.equal(git(work, "rev-parse", "refs/remotes/origin/main"), unpassed);
    refused(push(work, "origin", "side"), /no tree has passed the quick checks/);
    // A tag of the remote's real main passes with no record of its own.
    const real = git(work, "ls-remote", "origin", "refs/heads/main").split("\t")[0];
    git(work, "tag", "real-main", real);
    sent(push(work, "origin", "real-main"));
    // The remote's main moves on from another clone: until it is fetched, a branch at the old main
    // cannot be told on main, and the hook says to fetch.
    const second = path.join(dir, "second");
    git(dir, "clone", "-q", "-b", "main", remote, second);
    commit(second, "z.md", "z\n");
    git(second, "push", "-q", "origin", "main");
    git(work, "branch", "at-old-main", real);
    refused(push(work, "origin", "at-old-main"), /fetch first/);
    git(work, "fetch", "-q", "origin");
    sent(push(work, "origin", "at-old-main"));
    // A remote the hook cannot ask refuses, whatever the record.
    pass(work, tree(work, "main"));
    const line = `refs/heads/side ${unpassed} refs/heads/side ${"0".repeat(40)}\n`;
    const answer = prePush(line, work, path.join(dir, "no-such-remote.git"));
    assert.equal(answer.status, 1);
    assert.match(answer.words, /could not ask .* for its main/);
    assert.match(prePush(line, work, "").words, /no remote's URL/);
  });
});

test("the hook asks the remote git pushes to, never one an insteadOf rewrites to, and refuses a remote that does not answer in time", async () => {
  // The seventh review's case: url.<fake>.insteadOf points ls-remote at a fake repository whose main
  // holds the unpassed commit, while pushInsteadOf keeps the push itself on the real one.
  await new Promise((done) => {
    withRemote(({ dir, work, run, git, push, commit, remoteHas }) => {
      const real = git(work, "remote", "get-url", "origin");
      const fake = path.join(dir, "fake.git");
      git(dir, "init", "-q", "--bare", fake);
      git(work, "checkout", "-q", "-b", "feature");
      commit(work, "f.md", "f\n");
      git(work, "push", "-q", "--no-verify", fake, "feature:main");
      const r = run(work, ["-c", `url.${fake}.insteadOf=${real}`, "-c", `url.${real}.pushInsteadOf=${real}`, "push", "-q", "origin", "feature"]);
      refused({ status: r.status, words: r.stderr }, /would ask .*fake\.git instead/);
      assert.equal(remoteHas("feature"), "", "nothing reached the real remote");
      done();
    });
  });
  // A remote over HTTP that accepts the connection and never answers: git hands it to its
  // transport, git-remote-http, a process of its own. The hook gives up in time, and kills the
  // whole tree, so no process of it keeps its connection open (the eighth review).
  const sockets = new Set();
  let connected = 0;
  const server = net.createServer((socket) => {
    connected++;
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
    socket.on("error", () => {});
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  // A repository of its own, removed only after the check below, so that a surviving process,
  // which would hold it, fails the check and not the removal.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "stalled-"));
  try {
    const empty = path.join(dir, "no-config");
    fs.writeFileSync(empty, "");
    const env = { ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^GIT_/i.test(k))), GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: empty, GIT_AUTHOR_NAME: "fixture", GIT_AUTHOR_EMAIL: "fixture@example.invalid", GIT_COMMITTER_NAME: "fixture", GIT_COMMITTER_EMAIL: "fixture@example.invalid" };
    const work = path.join(dir, "work");
    fs.mkdirSync(work);
    const git = (...args) => execFileSync("git", args, { cwd: work, env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
    git("init", "-q", "-b", "stalled");
    fs.writeFileSync(path.join(work, "x.md"), "x\n");
    git("add", ".");
    git("commit", "-q", "-m", "x");
    const sha = git("rev-parse", "HEAD");
    const started = Date.now();
    const answer = prePush(`refs/heads/stalled ${sha} refs/heads/stalled ${"0".repeat(40)}\n`, work, `http://127.0.0.1:${server.address().port}/stalled.git`, { timeoutMs: 3000 });
    assert.equal(answer.status, 1, answer.words);
    assert.match(answer.words, /did not say which commit is its main within 3 s/);
    assert.ok(Date.now() - started < 30000, "the hook waited past its time");
    // The server learns of a connection only once this process's loop turns, after the hook has
    // returned: a check at once would see none and pass whatever survived.
    await new Promise((done) => setTimeout(done, 500));
    assert.ok(connected > 0, "the transport never connected, so this case tested nothing");
    // Every connection the transport opened is closed within moments: none of its processes lives.
    const deadline = Date.now() + 5000;
    while (sockets.size > 0 && Date.now() < deadline) await new Promise((done) => setTimeout(done, 100));
    assert.equal(sockets.size, 0, `${sockets.size} connection(s) of the stalled ls-remote outlived its time: a process of its tree survived`);
  } finally {
    for (const socket of sockets) socket.destroy();
    server.close();
    try {
      fs.rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
    } catch {
      // A surviving process holds it; the check above has said so.
    }
  }
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
    // A branch from before the hook is told to merge main, not to run a quick that cannot help.
    git(work, "checkout", "-q", "-b", "old", git(work, "rev-list", "--max-parents=0", "HEAD"));
    commit(work, "old.md", "old\n");
    fs.rmSync(path.resolve(work, git(work, "rev-parse", "--git-path", PASSED)), { force: true });
    refused(push(work, "origin", "old"), /from before the pre-push hook.*merge main into this branch first/);
    // With a record of its tree, which a later quick wrote, the old branch's push holds.
    pass(work, tree(work));
    sent(push(work, "origin", "old"));
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
    "Set-Item env:GIT_CONFIG_COUNT 1; git push",
    // The third review's: a valued option's own value, writes of core.hooksPath, included
    // configuration, an alias that pushes, plumbing, and a quoted program name.
    "git --work-tree . push --no-verify",
    "git --namespace x push --no-verify",
    "git --git-dir .git -c core.hooksPath=/x push",
    "git config core.hooksPath /x",
    "git config --unset core.hooksPath",
    "git config --file .git/config core.hooksPath /x",
    "git config set core.hooksPath /x",
    "git config unset core.hooksPath",
    'git -c alias.p="push --force" p',
    "git -c include.path=/x push",
    "git -c includeIf.gitdir:x.path=/y status",
    "git send-pack origin",
    "g''it push --no-verify",
    "git commit --no-verify -m x",
  ]) {
    for (const tool of ["Bash", "PowerShell", "Monitor", "mcp__terminal__run_in_terminal"]) {
      const r = guard(command, tool);
      assert.equal(r.status, 2, `${tool}: ${command}: ${r.words}`);
      assert.match(r.words, /^the push guard refused it: /);
    }
  }
  for (const command of ["git push", "git push -u origin main", "git push -n origin main", "git status", "echo push", "git -c user.name=x push", "git config --get core.hooksPath", "git config get core.hooksPath", "git config core.hooksPath", "git config --file .git/config core.hooksPath", "git config --show-origin core.hooksPath", "git config user.name x", "git -c alias.st=status st", "git --git-dir .git status", "git fetch origin", "git fetch origin main", "git update-ref refs/heads/x HEAD", "git update-ref refs/remotes/origin/main HEAD", "git fetch . +HEAD:refs/remotes/origin/main"]) {
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
