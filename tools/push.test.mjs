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
import { execFileSync, spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { PASSED, RUNNER_TARGET, RUNNER_VARIABLE, capped, runnerLine, workingTreeRunnerLine } from "./local-limits.mjs";
import { ENTRIES, INTERFACE, NODE_ENGINES, blobId, bootstrapLine, filesOf, gateEnv, gitLocations, localMain, manifestText, mismatch, nodeMeetsFloor, nothingToRefresh, pendingCopy, readManifest, systemGit, verifyCopy } from "./gate-entries/verify.mjs";
import { LOCK_FD_VARIABLE, TREE_RUNNER, lockHolder, takeLock } from "./gate-processes.mjs";
import { GET_URL_MS, REFRESH_MS, REMEMBER_MS, REMOTE_MS, RETRY_MS, WAIT_MS, WRITE_MS, refresh } from "./gates.mjs";
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
    // Read what the transport sent: a peer's orderly close is reported only once its data is read,
    // and on Linux a killed client closes so (the ninth review: Windows resets, Linux does not).
    socket.resume();
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
    const url = `http://127.0.0.1:${server.address().port}/stalled.git`;
    const settle = () => new Promise((done) => setTimeout(done, 500));
    // A positive control, on every platform this runs on: git alone killed at its time, as round 8
    // did, leaves its transport holding a connection, which the check below must see. Without it,
    // a check that sees nothing would pass whatever survived.
    spawnSync("git", ["ls-remote", "--", url, "refs/heads/main"], { cwd: work, env: { ...env, GIT_TERMINAL_PROMPT: "0" }, timeout: 3000 });
    await settle();
    assert.ok(sockets.size > 0, "killing git alone left no connection open, so the check below could not see a survivor");
    for (const socket of sockets) socket.destroy();
    await settle();
    connected = 0;
    const started = Date.now();
    const answer = prePush(`refs/heads/stalled ${sha} refs/heads/stalled ${"0".repeat(40)}\n`, work, url, { timeoutMs: 3000 });
    assert.equal(answer.status, 1, answer.words);
    assert.match(answer.words, /did not say which commit is its main within 3 s/);
    assert.ok(Date.now() - started < 30000, "the hook waited past its time");
    // The server learns of a connection only once this process's loop turns, after the hook has
    // returned: a check at once would see none and pass whatever survived.
    await settle();
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

test("on posix the runner kills the command's group at its time even after the command has exited, a child of it holding the pipe", { skip: process.platform === "win32" && "on Windows the job object, not a process group, ends the tree" }, () => {
  // A stand-in for git that starts a sleeper holding its output, as a transport would, and exits.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "orphan-"));
  const sleeper = path.join(dir, "sleeper.pid");
  // The sleeper carries a marker of this run in its command line, so that it is told apart from any
  // process that later takes its id (the eleventh review).
  const marker = `mente-orphan-${process.pid}-${Date.now()}`;
  const sleep = `setTimeout(() => {}, 30000); // ${marker}`;
  const standIn = `const { spawn } = require("node:child_process"); const s = spawn(process.execPath, ["-e", ${JSON.stringify(sleep)}], { stdio: ["ignore", "inherit", "inherit"] }); require("node:fs").writeFileSync(${JSON.stringify(sleeper)}, String(s.pid)); process.exit(0);`;
  const env = { ...process.env, TREE_ARGS: JSON.stringify([process.execPath, "-e", standIn]), TREE_TIMEOUT: "1500" };
  // Ends the sleeper if it still lives: only the process whose command line holds this run's
  // marker, never another that has taken its id since.
  const end = () => {
    try {
      const pid = Number(fs.readFileSync(sleeper, "utf8"));
      if (fs.readFileSync(`/proc/${pid}/cmdline`, "utf8").includes(marker)) process.kill(pid, "SIGKILL");
    } catch {
      // Gone already.
    }
  };
  try {
    // A positive control: round 10's runner killed only while the command lived, so it waits on the
    // pipe the sleeper holds, past any time.
    const fixed = `  if (windows) {
    if (child.exitCode === null && child.signalCode === null) child.kill();
  } else {
    try { process.kill(-child.pid, "SIGKILL"); } catch {}
  }`;
    const guarded = `  if (child.exitCode !== null || child.signalCode !== null) return;
  if (windows) child.kill();
  else { try { process.kill(-child.pid, "SIGKILL"); } catch {} }`;
    assert.equal(TREE_RUNNER.split(fixed).length, 2, "the runner's kill is as this test knows it");
    const old = spawnSync(process.execPath, ["-e", TREE_RUNNER.split(fixed).join(guarded)], { env, encoding: "utf8", timeout: 6000 });
    assert.equal(old.error?.code, "ETIMEDOUT", "round 10's runner returned, so this case does not reproduce the orphan");
    end();
    // The runner: its time passes, the whole group dies, the pipe closes, and it answers at once.
    const started = Date.now();
    const r = spawnSync(process.execPath, ["-e", TREE_RUNNER], { env, encoding: "utf8", timeout: 20000 });
    assert.equal(r.error, undefined, "the runner waited on the pipe past its time");
    const answer = JSON.parse(r.stdout);
    assert.equal(answer.timedOut, true);
    assert.ok(Date.now() - started < 10000, "the runner answered long after its time");
  } finally {
    end();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("the runner reports once, as one JSON, even when its command cannot start", () => {
  // A command that cannot start makes Node report both an error and a close; the runner answers the
  // first and only that one (the ninth review), so the hook reads its answer, not a parse error.
  const r = spawnSync(process.execPath, ["-e", TREE_RUNNER], { encoding: "utf8", env: { ...process.env, TREE_ARGS: JSON.stringify(["no-such-program-of-mente"]), TREE_TIMEOUT: "5000" } });
  assert.equal(r.status, 0, r.stderr);
  const answer = JSON.parse(r.stdout);
  assert.equal(answer.code, -1);
  assert.match(answer.err, /ENOENT|not recognized|cannot find/i);
  assert.equal(answer.timedOut, false);
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


// The gates' processes (briefs/gate-tools-from-main.md, "How the refresher's processes are run").

/// The module of the lock, which the lock's programs below import as `L`.
const LIMITS = new URL("./gate-processes.mjs", import.meta.url).href;

test("the gates' environment drops git's, node's and the certificate variables, and closes the system and user configuration", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "gate-env-"));
  try {
    const rewrite = '[url "https://elsewhere.invalid/"]\n\tinsteadOf = https://example.invalid/\n';
    fs.writeFileSync(path.join(dir, "global"), rewrite);
    fs.mkdirSync(path.join(dir, "home"));
    fs.writeFileSync(path.join(dir, "home", ".gitconfig"), rewrite);
    const empty = path.join(dir, "empty");
    fs.mkdirSync(empty);
    const home = path.join(dir, "home");
    const given = { ...process.env, GIT_CONFIG_GLOBAL: path.join(dir, "global"), git_dir: "x", NODE_OPTIONS: "--require=x", node_path: "x", SSL_CERT_FILE: "x", ssl_cert_dir: "x", CURL_CA_BUNDLE: "x", HOME: home, USERPROFILE: home, XDG_CONFIG_HOME: home, KEPT: "1" };
    const env = gateEnv(given);
    for (const key of Object.keys(env)) assert.doesNotMatch(key, /^(NODE_|SSL_CERT_FILE$|SSL_CERT_DIR$|CURL_CA_BUNDLE$|GIT_(?!CONFIG_NOSYSTEM$|CONFIG_GLOBAL$|TERMINAL_PROMPT$))/i, key);
    assert.equal(env.KEPT, "1");
    assert.deepEqual([env.GIT_CONFIG_NOSYSTEM, env.GIT_CONFIG_GLOBAL, env.GIT_TERMINAL_PROMPT], ["1", "/dev/null", "0"]);
    const git = systemGit();
    assert.ok(git && path.isAbsolute(git), "no git of the list is found on this machine");
    const url = "https://example.invalid/a.git";
    const ask = (e) => spawnSync(git, ["ls-remote", "--get-url", "--", url], { cwd: empty, env: { ...e, GIT_CEILING_DIRECTORIES: dir }, encoding: "utf8" });
    // A positive control: the same configuration, open, rewrites the URL.
    assert.equal(ask(given).stdout.trim(), "https://elsewhere.invalid/a.git", "the control did not rewrite, so this case proves nothing");
    const closed = ask(env);
    assert.equal(closed.status, 0, closed.stderr);
    assert.equal(closed.stdout.trim(), url);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("git is looked for by absolute path in a fixed list, never on PATH", () => {
  const local = "C:/Users/someone/AppData/Local";
  const list = gitLocations({ PATH: "/nowhere", LOCALAPPDATA: local });
  assert.ok(list.length > 0 && list.every((at) => path.isAbsolute(at)));
  assert.deepEqual(gitLocations({ PATH: "/elsewhere", LOCALAPPDATA: local }), list, "the list depends on PATH");
  if (process.platform === "win32") assert.deepEqual(list, ["C:/Program Files/Git/cmd/git.exe", `${local}/Programs/Git/cmd/git.exe`]);
  else assert.deepEqual(list, ["/usr/bin/git"]);
});

test("a blob's id is git's, for any bytes, line ends kept as they are", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "blob-"));
  try {
    for (const bytes of [Buffer.from(""), Buffer.from("a\r\nb\r\n"), Buffer.from([0, 255, 13, 10, 13])]) {
      const file = path.join(dir, "f");
      fs.writeFileSync(file, bytes);
      assert.equal(blobId(bytes), execFileSync(systemGit(), ["hash-object", "--no-filters", file], { env: gateEnv(), encoding: "utf8" }).trim());
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

/** The source of a node program that imports gate-processes.mjs as `L`, then runs `code`. */
const source = (code) => `import * as L from ${JSON.stringify(LIMITS)};\n${code}`;

/** A node program of `code` (source), started with `options`. */
const program = (code, options = {}) => spawn(process.execPath, ["--input-type=module", "-e", source(code)], { stdio: ["ignore", "pipe", "pipe"], ...options });

/** The first line `child` writes, or a rejection with what it said before it ended. */
const firstLine = (child) =>
  new Promise((resolve, reject) => {
    let out = "";
    let err = "";
    child.stdout.setEncoding("utf8").on("data", (d) => {
      out += d;
      if (out.includes("\n")) resolve(out.split("\n")[0]);
    });
    child.stderr.setEncoding("utf8").on("data", (d) => (err += d));
    child.on("exit", (code) => reject(new Error(`ended with ${code} before a line: ${out}${err}`)));
  });

/** Resolves once `child` has ended. */
const ended = (child) => new Promise((resolve) => (child.exitCode !== null || child.signalCode !== null ? resolve() : child.on("exit", resolve)));

test("the lock is released when its holder is killed, and waits only its bound while held", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "lock-"));
  const file = path.join(dir, "lock");
  const holder = program(`await L.takeLock(${JSON.stringify(file)}, { waitMs: 5000, label: "holder" }); console.log("held"); setInterval(() => {}, 1000);`);
  try {
    assert.equal(await firstLine(holder), "held");
    const started = Date.now();
    assert.equal(await takeLock(file, { waitMs: 400, label: "second" }), null, "a second holder took a held lock");
    const waited = Date.now() - started;
    assert.ok(waited >= 400 && waited < 3000, `the wait, ${waited} ms, was not held to its bound`);
    holder.kill("SIGKILL");
    await ended(holder);
    const lock = await takeLock(file, { waitMs: 3000, label: "after" });
    assert.ok(lock, "a killed holder left the lock taken");
    lock.release();
  } finally {
    holder.kill("SIGKILL");
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
  }
});

test("a lock handed down stays held after the hander ends, and a claim without it is not the lock", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "lock-"));
  const file = path.join(dir, "lock");
  const other = path.join(dir, "other");
  fs.writeFileSync(other, "");
  // Each child says whether it holds the lock, and its pid, and stays.
  const child = source(`process.stdout.write(String(L.inheritedLock(${JSON.stringify(file)})) + " " + process.pid + "\\n"); setInterval(() => {}, 1000);`);
  // The hander takes the lock, starts one child with it handed down and another with only the
  // variable and another file on the descriptor, says both answers, and ends. The children are
  // detached, so that they outlive it: on Windows, libuv ends a child that is not when its parent
  // ends, as an old refresher and the new one it starts end together.
  const hander = program(`
    const { spawn } = await import("node:child_process");
    const fs = await import("node:fs");
    const lock = await L.takeLock(${JSON.stringify(file)}, { waitMs: 5000, label: "hander" });
    const start = (options) => new Promise((resolve) => {
      const c = spawn(process.execPath, ["--input-type=module", "-e", ${JSON.stringify(child)}], options);
      c.stdout.setEncoding("utf8").on("data", (d) => resolve(d.trim()));
      c.unref();
    });
    const real = await start({ ...L.handDown(lock, ["ignore", "pipe", "ignore"], process.env), detached: true });
    const fd = fs.openSync(${JSON.stringify(other)}, "r");
    const claim = await start({ stdio: ["ignore", "pipe", "ignore", fd], env: { ...process.env, ${LOCK_FD_VARIABLE}: "3" }, detached: true });
    console.log(real + " " + claim);
    process.exit(0);
  `);
  let pids = [];
  try {
    const [realHeld, realPid, claimHeld, claimPid] = (await firstLine(hander)).split(" ");
    pids = [Number(realPid), Number(claimPid)];
    await ended(hander);
    assert.equal(realHeld, "true", "the child the lock was handed to does not see it as held");
    assert.equal(claimHeld, "false", "a claim with another file on the descriptor passed as the lock");
    assert.equal(await takeLock(file, { waitMs: 400, label: "third" }), null, "the lock was free once its hander ended, though handed down");
    process.kill(pids[0], "SIGKILL");
    let lock = null;
    for (let i = 0; i < 20 && !lock; i++) lock = await takeLock(file, { waitMs: 250, label: "after" });
    assert.ok(lock, "the lock stayed taken after the child it was handed to was killed");
    lock.release();
  } finally {
    for (const pid of pids) {
      try {
        process.kill(pid, "SIGKILL");
      } catch {
        // Gone already.
      }
    }
    hander.kill("SIGKILL");
    // Windows frees a killed process's files a moment after it ends.
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
  }
});

// The local gates run from a verified copy of main (briefs/gate-tools-from-main.md). Each case
// makes a fixture: a bare repository standing for the remote, and a checkout of it whose main
// holds this checkout's set, its tools/gate-set.json naming the bare repository's path as the
// remote, which only the seam of tools/gates.mjs admits; the command admits only https.

const TREE = fileURLToPath(new URL("..", import.meta.url));
const SET = JSON.parse(fs.readFileSync(new URL("./gate-set.json", import.meta.url), "utf8"));
const GATES_MODULE = new URL("./gates.mjs", import.meta.url).href;
const EXECUTABLE = ["tools/git-hooks/pre-push", "tools/gate-entries/hooks/pre-push"];
const posixOnly = process.platform === "win32" && "Windows keeps no executable bit";
const windowsOnly = process.platform !== "win32" && "only Windows refuses to replace a file in use: a local proof the quick map runs";

/** The environment of a fixture's own git commands: git's configuration closed, a fixed author. */
function fixtureEnv(dir) {
  const empty = path.join(dir, "no-config");
  fs.writeFileSync(empty, "");
  return {
    ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^(GIT_|NODE_OPTIONS$)/i.test(k))),
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: empty,
    GIT_AUTHOR_NAME: "fixture",
    GIT_AUTHOR_EMAIL: "fixture@example.invalid",
    GIT_COMMITTER_NAME: "fixture",
    GIT_COMMITTER_EMAIL: "fixture@example.invalid",
  };
}

/**
 * Runs `body` with a fixture of the gates, and removes it after. `commit(message, changes)` writes
 * the set as this checkout holds it, then `changes` ({ path: text, or a function of the old bytes,
 * or null to remove }), commits it, and unless `push` is false pushes main and fetches it.
 */
async function withGates(body, { patched = false } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "gates-"));
  try {
    const env = fixtureEnv(dir);
    const run = (cwd, args, extra = {}) => spawnSync("git", args, { cwd, env: { ...env, ...extra }, encoding: "utf8" });
    const git = (cwd, ...args) => {
      const r = run(cwd, args);
      if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr}`);
      return r.stdout.trim();
    };
    const remote = path.join(dir, "remote.git").replace(/\\/g, "/");
    const work = path.join(dir, "work");
    git(dir, "init", "-q", "--bare", "-b", "main", remote);
    git(dir, "init", "-q", "-b", "main", work);
    git(work, "remote", "add", "origin", remote);
    const commit = (message, changes = {}, { push = true } = {}) => {
      for (const rel of SET.set) {
        const at = path.join(work, rel);
        fs.mkdirSync(path.dirname(at), { recursive: true });
        fs.copyFileSync(path.join(TREE, rel), at);
      }
      fs.writeFileSync(path.join(work, "tools", "gate-set.json"), `${JSON.stringify({ ...SET, url: remote }, null, 2)}\n`);
      if (patched) {
        const at = path.join(work, "tools", "gates.mjs");
        const text = fs.readFileSync(at, "utf8");
        assert.equal(text.split(COMMAND_POLICY).length, 2, "the command's policy is not as a patched fixture knows it");
        fs.writeFileSync(at, text.replace(COMMAND_POLICY, COMMAND_POLICY.replace("allowLocal: false", "allowLocal: true")));
      }
      for (const [rel, content] of Object.entries(changes)) {
        const at = path.join(work, rel);
        if (content === null) fs.rmSync(at, { force: true });
        else {
          fs.mkdirSync(path.dirname(at), { recursive: true });
          fs.writeFileSync(at, typeof content === "function" ? content(fs.readFileSync(at)) : content);
        }
      }
      git(work, "add", "-A");
      git(work, "update-index", "--chmod=+x", ...EXECUTABLE);
      git(work, "commit", "-q", "--allow-empty", "-m", message);
      const sha = git(work, "rev-parse", "HEAD");
      if (push) {
        git(work, "push", "-q", "origin", "main");
        git(work, "fetch", "-q", "origin");
      }
      return sha;
    };
    const repo = path.join(work, ".git");
    const gates = path.join(repo, "mente-gates");
    const said = [];
    const seam = (options = {}) => refresh({ repo, policy: { allowLocal: true }, say: (line) => said.push(line), ...options });
    /** The seam in a process of its own, which a kill at `stopAt` ends: its status and words. */
    const seamChild = (options = {}, extra = {}) =>
      spawnSync(process.execPath, ["--input-type=module", "-e", `import { refresh } from ${JSON.stringify(GATES_MODULE)}; process.exit(await refresh({ repo: ${JSON.stringify(repo)}, policy: { allowLocal: true }, ...${JSON.stringify(options)} }));`], { encoding: "utf8", env: { ...env, ...extra }, cwd: extra.CWD ?? work });
    const entry = (name, args = [], extra = {}) => spawnSync(process.execPath, [path.join(gates, "entry", name), ...args], { encoding: "utf8", cwd: work, env: { ...env, ...extra } });
    const current = () => fs.readFileSync(path.join(gates, "current"), "utf8").trim();
    const blob = (sha, rel) => execFileSync(systemGit(), ["--git-dir", repo, "--no-replace-objects", "cat-file", "blob", `${sha}:${rel}`], { env: gateEnv() });
    const copies = () => fs.readdirSync(gates).filter((name) => /^[0-9a-f]{40}$|^staging-|^aside-/.test(name)).sort();
    const base = commit("the set");
    await body({ dir, env, run, git, remote, work, repo, gates, said, seam, seamChild, entry, current, blob, copies, commit, base });
  } finally {
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
  }
}

/**
 * Writes `commit`'s set into `dir` by hand, as a refresher stages one: each file with its blob's
 * bytes and the mode the tree records, so that a check of the set refuses only what a case plants.
 * Returns the files, as their manifest names them.
 */
function stageByHand(gitDir, commit, dir) {
  const files = filesOf(execFileSync(systemGit(), ["--git-dir", gitDir, "ls-tree", "-r", "-z", "--full-tree", commit, "--", ...SET.set], { env: gateEnv(), encoding: "utf8" }), SET.set);
  for (const f of files) {
    const at = path.join(dir, ...f.path.split("/"));
    fs.mkdirSync(path.dirname(at), { recursive: true });
    fs.writeFileSync(at, execFileSync(systemGit(), ["--git-dir", gitDir, "--no-replace-objects", "cat-file", "blob", f.id], { env: gateEnv() }));
    fs.chmodSync(at, f.mode === "100755" ? 0o755 : 0o644);
  }
  return files;
}

/** Writes `rel` of the copy in use with one byte changed. */
function drift(gates, rel) {
  const at = path.join(gates, fs.readFileSync(path.join(gates, "current"), "utf8").trim(), ...rel.split("/"));
  const bytes = fs.readFileSync(at);
  bytes[0] ^= 1;
  fs.writeFileSync(at, bytes);
}

test("a copy holds the confirmed commit's bytes and modes, from its objects, whatever the working tree holds", async () => {
  await withGates(async ({ work, gates, seam, current, blob, commit, entry }) => {
    // A file committed with CRLF line ends before an eol attribute that would convert them.
    commit("CRLF", { "tools/capped.ps1": (old) => Buffer.from(old.toString("utf8").replace(/\r?\n/g, "\r\n")) });
    const head = commit("the attribute", { ".gitattributes": "* text=auto eol=lf\n", "tools/capped.ps1": (old) => Buffer.from(old.toString("utf8").replace(/\r?\n/g, "\r\n")) });
    // Uncommitted edits to the set, to the refresher and to the entries, in the checkout.
    for (const rel of ["tools/capped.mjs", "tools/gates.mjs", "tools/gate-entries/verify.mjs"]) fs.appendFileSync(path.join(work, rel), "\n// an uncommitted edit\n");
    assert.equal(await seam(), 0);
    assert.equal(current(), head);
    for (const rel of SET.set) assert.ok(fs.readFileSync(path.join(gates, head, ...rel.split("/"))).equals(blob(head, rel)), `${rel} of the copy is not the commit's blob`);
    assert.ok(fs.readFileSync(path.join(gates, head, "tools", "capped.ps1")).includes("\r\n"), "the CRLF file lost its line ends");
    for (const name of ENTRIES) assert.ok(fs.readFileSync(path.join(gates, "entry", ...name.split("/"))).equals(blob(head, `tools/gate-entries/${name}`)), `the entry ${name} is not the commit's`);
    const checked = verifyCopy(gates);
    assert.equal(checked.refusal, undefined, checked.refusal);
    if (process.platform !== "win32") {
      for (const at of [path.join(gates, head, "tools", "git-hooks", "pre-push"), path.join(gates, "entry", "hooks", "pre-push")]) assert.ok(fs.statSync(at).mode & 0o100, `${at} is not executable`);
    }
    const r = entry("capped.mjs", ["--", process.execPath, "-e", "console.log('ran')"]);
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stdout, /ran/);
  });
});

test("the entries run the copy whatever the checkout holds, and a push runs the copy's hook", async () => {
  await withGates(async ({ work, gates, seam, git, run, entry }) => {
    assert.equal(await seam(), 0);
    git(work, "config", "core.hooksPath", path.join(gates, "entry", "hooks").replace(/\\/g, "/"));
    // The checkout switched to a branch that holds no tools at all, as one from before #203 holds none of these.
    git(work, "checkout", "-q", "--orphan", "before");
    git(work, "rm", "-q", "-rf", ".");
    fs.writeFileSync(path.join(work, "old.md"), "old\n");
    git(work, "add", "old.md");
    git(work, "commit", "-q", "-m", "old");
    const r = entry("capped.mjs", ["--", process.execPath, "-e", "console.log('ran')"]);
    assert.equal(r.status, 0, r.stderr);
    const refused = run(work, ["push", "origin", "HEAD:refs/heads/before"]);
    assert.notEqual(refused.status, 0);
    assert.match(refused.stderr, /the pre-push hook refused/);
    fs.writeFileSync(path.join(repo(work), "quick-passed"), `${git(work, "rev-parse", "HEAD^{tree}")}\n`);
    const sent = run(work, ["push", "origin", "HEAD:refs/heads/before"]);
    assert.equal(sent.status, 0, sent.stderr);
  });
});

/** The git directory of the checkout `work`. */
const repo = (work) => path.join(work, ".git");

test("a refresher run by hand installs nothing", async () => {
  await withGates(async ({ dir, work, repo: gitDir, gates, seam, current, commit, blob, git, base }) => {
    assert.equal(await seam(), 0);
    // From a working tree.
    let r = spawnSync(process.execPath, [path.join(work, "tools", "gates.mjs")], { encoding: "utf8" });
    assert.equal(r.status, 2);
    assert.match(r.stderr, /starts only from a copy under/);
    // An edited refresher in a staging directory, for main's next commit.
    const next = commit("next");
    const staging = path.join(gates, `staging-${next}-0a`);
    stageByHand(gitDir, next, staging);
    fs.appendFileSync(path.join(staging, "tools", "gates.mjs"), "\n// edited\n");
    r = spawnSync(process.execPath, [path.join(staging, "tools", "gates.mjs")], { encoding: "utf8" });
    assert.equal(r.status, 1, r.stderr);
    assert.match(r.stderr, /writes nothing: its directory does not hold exactly the commit's set: tools\/gates\.mjs/);
    assert.equal(current(), base);
    fs.rmSync(staging, { recursive: true });
    // The installed refresher, imported by a script with a local-path policy.
    const installed = await import(pathToFileURL(path.join(gates, base, "tools", "gates.mjs")).href);
    await assert.rejects(installed.refresh({ repo: gitDir, policy: { allowLocal: true } }), /under mente-gates\/ honours no seam argument/);
    // This working tree's refresher, against its own repository.
    const own = execFileSync(systemGit(), ["rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd: TREE, env: gateEnv(), encoding: "utf8" }).trim();
    await assert.rejects(refresh({ repo: own, policy: { allowLocal: true } }), /other than the module's own/);
    // A worktree's refresher, against its main checkout.
    git(work, "worktree", "add", "-q", path.join(dir, "second"), "-b", "second");
    const second = await import(pathToFileURL(path.join(dir, "second", "tools", "gates.mjs")).href);
    await assert.rejects(second.refresh({ repo: gitDir, policy: { allowLocal: true } }), /every worktree of it counts as its own/);
    // A refresher copied outside any repository.
    const outside = fs.mkdtempSync(path.join(os.tmpdir(), "outside-"));
    try {
      for (const rel of ["tools/gates.mjs", "tools/gate-processes.mjs", "tools/local-limits.mjs", "tools/store-apps.mjs", "tools/capped.ps1", "tools/gate-entries/verify.mjs"]) {
        fs.mkdirSync(path.dirname(path.join(outside, rel)), { recursive: true });
        fs.copyFileSync(path.join(TREE, rel), path.join(outside, rel));
      }
      const copied = await import(pathToFileURL(path.join(outside, "tools", "gates.mjs")).href);
      await assert.rejects(copied.refresh({ repo: gitDir, policy: { allowLocal: true } }), /outside any repository honours no seam argument/);
    } finally {
      fs.rmSync(outside, { recursive: true, force: true });
    }
    assert.equal(current(), base);
  });
});

test("a refresher refuses any argument, and reads only its own repository whatever GIT_ variables it starts with", async () => {
  await withGates(async ({ dir, gates, seam, seamChild, current, commit, blob, entry, base }) => {
    assert.equal(await seam(), 0);
    for (const extra of [base, "https://github.com/Menteion/Mente.git", dir]) {
      const r = spawnSync(process.execPath, [path.join(gates, base, "tools", "gates.mjs"), extra], { encoding: "utf8" });
      assert.equal(r.status, 2);
      assert.match(r.stderr, /takes no argument/);
    }
    // Another repository, whose objects and configuration the variables name.
    const other = path.join(dir, "other.git");
    execFileSync("git", ["init", "-q", "--bare", other], { env: fixtureEnv(dir) });
    const variables = { GIT_DIR: other, GIT_OBJECT_DIRECTORY: path.join(other, "objects"), GIT_ALTERNATE_OBJECT_DIRECTORIES: path.join(other, "objects"), GIT_CONFIG_COUNT: "1", GIT_CONFIG_KEY_0: "core.bare", GIT_CONFIG_VALUE_0: "true" };
    const next = commit("next");
    const r = seamChild({}, variables);
    assert.equal(r.status, 0, r.stderr);
    assert.equal(current(), next);
    assert.ok(fs.readFileSync(path.join(gates, next, "tools", "gates.mjs")).equals(blob(next, "tools/gates.mjs")));
    const ran = entry("capped.mjs", ["--", process.execPath, "-e", "0"], variables);
    assert.equal(ran.status, 0, ran.stderr);
  });
});

test("a locally moved origin/main, an insteadOf, a URL not https, and a remote that cannot be reached or does not answer install nothing", async () => {
  await withGates(async ({ dir, work, git, gates, seam, seamChild, current, commit, said, remote, base }) => {
    assert.equal(await seam(), 0);
    // origin/main moved by update-ref to a commit whose capped.mjs differs, and a second remote whose main it is.
    const moved = commit("moved", { "tools/capped.mjs": (old) => `${old}\n// moved\n` }, { push: false });
    const second = path.join(dir, "second.git").replace(/\\/g, "/");
    git(dir, "init", "-q", "--bare", "-b", "main", second);
    git(work, "push", "-q", second, "HEAD:main");
    git(work, "update-ref", "refs/remotes/origin/main", moved);
    assert.equal(await seam(), 0);
    assert.equal(current(), base);
    assert.match(said.join("\n"), /the remote's main is/);
    // An insteadOf to that second remote, in the repository's and the user's configuration; a
    // positive control shows git following it.
    const rewrite = `[url "${second}"]\n\tinsteadOf = ${remote}\n`;
    git(work, "config", `url.${second}.insteadOf`, remote);
    const user = path.join(dir, "user-config");
    fs.writeFileSync(user, rewrite);
    assert.match(git(work, "ls-remote", remote, "refs/heads/main"), new RegExp(moved), "the control did not follow the insteadOf, so this case proves nothing");
    for (const cwd of [work, gates]) {
      fs.rmSync(path.join(gates, "kept"), { force: true });
      const r = seamChild({}, { GIT_CONFIG_GLOBAL: user, HOME: dir, USERPROFILE: dir, CWD: cwd });
      assert.equal(r.status, 0, r.stderr);
      assert.equal(current(), base, `started from ${cwd}, an insteadOf installed the moved commit`);
    }
    git(work, "config", "--unset", `url.${second}.insteadOf`);
    // The command admits only https: the fixture's URL is a path.
    git(work, "update-ref", "refs/remotes/origin/main", commit("next"));
    const r = spawnSync(process.execPath, [path.join(gates, "entry", "refresh.mjs")], { encoding: "utf8", cwd: work });
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stderr, /is not https/);
    assert.equal(current(), base);
    // A remote that cannot be reached.
    fs.rmSync(path.join(gates, "kept"), { force: true });
    fs.renameSync(remote, `${remote}.away`);
    said.length = 0;
    assert.equal(await seam(), 0);
    assert.match(said.join("\n"), /could not be asked/);
    assert.equal(current(), base);
    fs.renameSync(`${remote}.away`, remote);
  });
});

test("a remote that does not answer in time installs nothing, its tree killed at the time", async () => {
  const sockets = [];
  const server = net.createServer((socket) => sockets.push(socket)).listen(0, "127.0.0.1");
  await new Promise((resolve) => server.once("listening", resolve));
  try {
    await withGates(async ({ seam, said, commit, current }) => {
      const silent = `http://127.0.0.1:${server.address().port}/silent.git`;
      // A commit that declares the silent remote, installed by the seam's bootstrap: it must confirm there.
      commit("silent", { "tools/gate-set.json": () => `${JSON.stringify({ ...SET, url: silent }, null, 2)}\n` });
      const started = Date.now();
      assert.equal(await seam({ bootstrap: true, remoteMs: 2000 }), 1);
      assert.ok(Date.now() - started < 20000, "the ask was not held to its time");
      assert.match(said.join("\n"), /did not say which commit is its main within 2 s/);
      assert.throws(current);
    });
  } finally {
    for (const socket of sockets) socket.destroy();
    server.close();
  }
});

test("a moved origin/main whose gate-set.json names a local-path remote installs nothing, at a refresh and at the bootstrap", async () => {
  await withGates(async ({ dir, work, git, gates, seam, current, commit, blob, base }) => {
    assert.equal(await seam(), 0);
    const second = path.join(dir, "second.git").replace(/\\/g, "/");
    git(dir, "init", "-q", "--bare", "-b", "main", second);
    const moved = commit("moved", { "tools/gate-set.json": () => `${JSON.stringify({ ...SET, url: second }, null, 2)}\n`, "tools/capped.mjs": (old) => `${old}\n// moved\n` }, { push: false });
    git(work, "push", "-q", second, "HEAD:main");
    git(work, "update-ref", "refs/remotes/origin/main", moved);
    assert.equal(await seam(), 0);
    assert.equal(current(), base);
    // A staged refresher of that commit, started by hand: the command admits only https. The answer
    // the refresh above remembered for that commit is cleared, so that the refresher reaches the
    // remote rather than stop at it.
    fs.rmSync(path.join(gates, "kept"), { force: true });
    const staging = path.join(gates, `staging-${moved}-0b`);
    stageByHand(repo(work), moved, staging);
    const r = spawnSync(process.execPath, [path.join(staging, "tools", "gates.mjs")], { encoding: "utf8" });
    assert.equal(r.status, 1, r.stderr);
    assert.match(r.stderr, /writes nothing: the remote's URL .* is not https/);
    assert.equal(current(), base);
  });
});

test("git replace of the confirmed commit changes nothing the gates install or check", async () => {
  await withGates(async ({ work, git, gates, seam, current, commit, blob, entry }) => {
    const main = commit("main");
    const other = commit("other", { "tools/capped.mjs": (old) => `${old}\n// replaced\n` }, { push: false });
    git(work, "reset", "-q", "--hard", main);
    git(work, "replace", main, other);
    assert.notEqual(git(work, "show", `${main}:tools/capped.mjs`), blob(main, "tools/capped.mjs").toString().trim(), "the replace did not take, so this case proves nothing");
    assert.equal(await seam(), 0);
    assert.equal(current(), main);
    assert.ok(fs.readFileSync(path.join(gates, main, "tools", "capped.mjs")).equals(blob(main, "tools/capped.mjs")), "the copy holds the replacement's capped.mjs");
    const r = entry("capped.mjs", ["--", process.execPath, "-e", "0"]);
    assert.equal(r.status, 0, r.stderr);
  });
});

test("a copy of a commit that is not origin/main or one of its ancestors is refused by every entry", async () => {
  await withGates(async ({ work, git, gates, seam, commit, blob, entry, base }) => {
    assert.equal(await seam(), 0);
    const branch = commit("a branch", { "tools/capped.mjs": (old) => `${old}\n// branch\n` }, { push: false });
    git(work, "reset", "-q", "--hard", base);
    const copy = path.join(gates, branch);
    const files = stageByHand(repo(work), branch, copy);
    fs.writeFileSync(path.join(copy, "manifest"), manifestText(files));
    fs.writeFileSync(path.join(gates, "current"), `${branch}\n`);
    for (const [name, status] of [["capped.mjs", 2], ["hook.mjs", 1], ["refresh.mjs", 2]]) {
      const r = entry(name, name === "capped.mjs" ? ["--", process.execPath, "-e", "0"] : []);
      assert.equal(r.status, status, `${name}: ${r.stderr}`);
      assert.match(r.stderr, /is not refs\/remotes\/origin\/main or one of its ancestors; Norman runs the bootstrap/);
    }
  });
});

test("drift of the copy is refused on every path, a manifest edited to match too, and the bootstrap clears it", async () => {
  await withGates(async ({ gates, seam, current, entry, base }) => {
    assert.equal(await seam(), 0);
    drift(gates, "tools/push.mjs");
    const paths = [["capped.mjs", 2], ["hook.mjs", 1], ["refresh.mjs", 2]];
    for (const [name, status] of paths) {
      const r = entry(name, name === "capped.mjs" ? ["--", process.execPath, "-e", "0"] : []);
      assert.equal(r.status, status, `${name}: ${r.stderr}`);
      assert.match(r.stderr, /has drifted from its manifest: tools\/push\.mjs .*Norman runs the bootstrap/s);
    }
    const manifest = path.join(gates, base, "manifest");
    const files = readManifest(fs.readFileSync(manifest, "utf8")).map((f) => (f.path === "tools/push.mjs" ? { ...f, id: blobId(fs.readFileSync(path.join(gates, base, "tools", "push.mjs"))) } : f));
    fs.writeFileSync(manifest, manifestText(files));
    for (const [name, status] of paths) {
      const r = entry(name, name === "capped.mjs" ? ["--", process.execPath, "-e", "0"] : []);
      assert.equal(r.status, status, `${name}: ${r.stderr}`);
      assert.match(r.stderr, /disagrees with that commit's tree/);
    }
    assert.equal(await seam({ bootstrap: true }), 0);
    assert.equal(current(), base);
    for (const [name] of paths) assert.equal(entry(name, name === "capped.mjs" ? ["--", process.execPath, "-e", "0"] : []).status, 0, name);
  });
});

/**
 * Why the set under `root` is not its own load closure: each file it imports or loads by path,
 * in the forms its files use (static and dynamic imports of a literal path, path.join or
 * path.resolve of HERE and a file name, new URL of a literal path and import.meta.url, PowerShell's
 * Join-Path $PSScriptRoot, and sh's "$(dirname "$0")/<file>"), that lies outside it.
 */
function closureProblems(root, set) {
  const inSet = new Set(set);
  const problems = [];
  for (const rel of set) {
    const text = fs.readFileSync(path.join(root, ...rel.split("/")), "utf8");
    const loads = [];
    const forms = [
      /\bimport\s[^;]*?\bfrom\s+["']([^"']+)["']/g,
      /\bimport\s+["']([^"']+)["']/g,
      /\bimport\(\s*["']([^"']+)["']\s*\)/g,
      /path\.(?:join|resolve)\(\s*HERE\s*,\s*["']([^"']+\.[A-Za-z0-9]+)["']\s*\)/g,
      /new URL\(\s*["']([^"']+)["']\s*,\s*import\.meta\.url\s*\)/g,
      /Join-Path \$PSScriptRoot '([^']+)'/g,
      /"\$\(dirname "\$0"\)\/([^"]+)"/g,
    ];
    for (const form of forms) for (const m of text.matchAll(form)) loads.push(m[1]);
    for (const load of loads) {
      if (load.startsWith("node:")) continue;
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(rel), load));
      if (!inSet.has(target)) problems.push(`${rel} loads ${load}, which is not in the set`);
    }
  }
  return problems;
}

test("the set is its own load closure, of files of mode 100644 or 100755, and a refresh refuses a commit whose set holds anything else", async () => {
  assert.deepEqual(closureProblems(TREE, SET.set), []);
  const listed = execFileSync("git", ["ls-files", "-s", "--", ...SET.set], { cwd: TREE, encoding: "utf8" }).trim().split("\n");
  assert.equal(listed.length, SET.set.length, "a file of the set is not tracked");
  for (const line of listed) assert.match(line, /^100(644|755) /, line);
  for (const rel of fs.readdirSync(path.join(TREE, "tools", "gate-entries"), { recursive: true })) {
    const file = `tools/gate-entries/${String(rel).replace(/\\/g, "/")}`;
    if (fs.statSync(path.join(TREE, file)).isFile()) assert.ok(SET.set.includes(file), `${file} is not in the set`);
  }
  // Plants: a module that imports a file outside the set, and one that loads one by path.
  const planted = fs.mkdtempSync(path.join(os.tmpdir(), "closure-"));
  try {
    for (const rel of SET.set) {
      fs.mkdirSync(path.dirname(path.join(planted, rel)), { recursive: true });
      fs.copyFileSync(path.join(TREE, rel), path.join(planted, rel));
    }
    fs.appendFileSync(path.join(planted, "tools", "capped.mjs"), '\nimport "./outside.mjs";\n');
    fs.appendFileSync(path.join(planted, "tools", "store-apps.mjs"), '\nconst X = path.join(HERE, "outside.ps1");\n');
    assert.deepEqual(closureProblems(planted, SET.set), ["tools/capped.mjs loads ./outside.mjs, which is not in the set", "tools/store-apps.mjs loads outside.ps1, which is not in the set"]);
  } finally {
    fs.rmSync(planted, { recursive: true, force: true });
  }
  // A commit whose set holds a symlink, and one whose set holds a submodule.
  await withGates(async ({ work, git, seam, said, current, base }) => {
    assert.equal(await seam(), 0);
    for (const [mode, type] of [["120000", "a symlink"], ["160000", "a submodule"]]) {
      const id = mode === "160000" ? base : git(work, "hash-object", "-w", "--stdin");
      git(work, "update-index", "--cacheinfo", `${mode},${id},tools/raw-build.mjs`);
      git(work, "commit", "-q", "-m", type);
      git(work, "push", "-q", "origin", "main");
      git(work, "fetch", "-q", "origin");
      said.length = 0;
      assert.equal(await seam(), 0, type);
      assert.match(said.join("\n"), /tools\/raw-build\.mjs is a \w+ of mode \d+, not a file of mode 100644 or 100755/, type);
      assert.equal(current(), base);
      git(work, "reset", "-q", "--hard", base);
      git(work, "push", "-q", "-f", "origin", "main");
      git(work, "fetch", "-q", "origin");
    }
  });
});

test("a copy file whose executable bit was dropped is refused", { skip: posixOnly }, async () => {
  await withGates(async ({ gates, seam, entry, base }) => {
    assert.equal(await seam(), 0);
    fs.chmodSync(path.join(gates, base, "tools", "git-hooks", "pre-push"), 0o644);
    const r = entry("capped.mjs", ["--", process.execPath, "-e", "0"]);
    assert.equal(r.status, 2);
    assert.match(r.stderr, /tools\/git-hooks\/pre-push is not executable, though its mode is 100755/);
  });
});

/** The text of every call of `name` in `text`, its parentheses balanced. */
function calls(text, name) {
  const out = [];
  for (const m of text.matchAll(new RegExp(`\\b${name}\\(`, "g"))) {
    let depth = 0;
    for (let i = m.index + name.length; i < text.length; i++) {
      if (text[i] === "(") depth += 1;
      else if (text[i] === ")" && --depth === 0) {
        out.push(text.slice(m.index, i + 1));
        break;
      }
    }
  }
  return out;
}

test("no PATH, NODE_OPTIONS or certificate variable reaches a process the gates start", async () => {
  await withGates(async ({ dir, work, git, run, gates, seam, commit }) => {
    assert.equal(await seam(), 0);
    commit("next");
    // A git on PATH that leaves a trace, and a module NODE_OPTIONS preloads, which says where it ran.
    const bin = path.join(dir, "bin");
    fs.mkdirSync(bin);
    const trace = path.join(dir, "git-ran");
    fs.writeFileSync(path.join(bin, "git"), `#!/bin/sh\necho ran > "${trace.replace(/\\/g, "/")}"\nexit 1\n`, { mode: 0o755 });
    fs.writeFileSync(path.join(bin, "git.cmd"), `@echo ran > "${trace}"\r\n@exit /b 1\r\n`);
    const log = path.join(dir, "preloaded");
    const marker = path.join(dir, "marker.cjs");
    fs.writeFileSync(marker, `require("node:fs").appendFileSync(${JSON.stringify(log)}, JSON.stringify(process.argv.slice(1)) + "\\n");\n`);
    const variables = { PATH: `${bin}${path.delimiter}${process.env.PATH}`, NODE_OPTIONS: `--require ${JSON.stringify(marker)}`, SSL_CERT_FILE: marker, SSL_CERT_DIR: dir, CURL_CA_BUNDLE: marker };
    const r = spawnSync(process.execPath, [path.join(gates, "entry", "refresh.mjs")], { encoding: "utf8", cwd: work, env: { ...process.env, ...variables } });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(!fs.existsSync(trace), "a git found on PATH ran");
    const preloaded = fs.readFileSync(log, "utf8").trim().split("\n").map((line) => JSON.parse(line));
    assert.deepEqual(preloaded, [[path.join(gates, "entry", "refresh.mjs")]], "NODE_OPTIONS reached a process the refresh entry started");
    // The hook's entry removes NODE_OPTIONS before node runs.
    fs.rmSync(log, { force: true });
    git(work, "config", "core.hooksPath", path.join(gates, "entry", "hooks").replace(/\\/g, "/"));
    fs.writeFileSync(path.join(repo(work), "quick-passed"), `${git(work, "rev-parse", "HEAD^{tree}")}\n`);
    const pushed = run(work, ["push", "origin", "HEAD:refs/heads/topic"], { NODE_OPTIONS: variables.NODE_OPTIONS });
    assert.equal(pushed.status, 0, pushed.stderr);
    assert.ok(!fs.existsSync(log), "NODE_OPTIONS reached the hook's node");
  });
  // Every process the refresher and the entries start has the gates' environment, the one place
  // that removes the certificate variables; none hands on this process's own.
  for (const rel of ["tools/gates.mjs", "tools/gate-processes.mjs", "tools/gate-entries/verify.mjs", "tools/gate-entries/refresh.mjs"]) {
    // The tree runner's own program spawns its command with the environment inTime built from
    // its caller's.
    const text = fs.readFileSync(path.join(TREE, rel), "utf8").replace(/export const TREE_RUNNER = `[\s\S]*?`;/, "");
    for (const name of ["spawn", "spawnSync", "inTime"]) {
      for (const call of calls(text, name)) assert.match(call, /gateEnv\(\)|handDown\(|\benv\b/, `${rel}: ${call}`);
    }
    // Allowed: the defaults of gateEnv and of where git is looked for, which are given this
    // process's environment to clean or read LOCALAPPDATA from; the handoff's one variable, which
    // inheritedLock reads and checks against the descriptor itself.
    const allowed = text
      .replace(/export function gateEnv\(env = process\.env\)/, "")
      .replace(/systemGit = \(env = process\.env\)/, "")
      .replace(/gitLocations\(env = process\.env\)/, "")
      .replace(/export function inheritedLock\(file, env = process\.env\)/, "");
    assert.doesNotMatch(allowed, /process\.env/, `${rel} reads process.env`);
  }
});

test("two refreshes at once leave one valid copy", async () => {
  await withGates(async ({ repo: gitDir, gates, seam, current, commit, copies, env, base }) => {
    assert.equal(await seam(), 0);
    const next = commit("next");
    const start = () => spawn(process.execPath, ["--input-type=module", "-e", `import { refresh } from ${JSON.stringify(GATES_MODULE)}; process.exit(await refresh({ repo: ${JSON.stringify(gitDir)}, policy: { allowLocal: true } }));`], { env, stdio: "ignore" });
    const both = [start(), start()];
    const codes = await Promise.all(both.map((child) => new Promise((resolve) => child.on("exit", resolve))));
    assert.deepEqual(codes, [0, 0]);
    assert.equal(current(), next);
    assert.deepEqual(copies(), [base, next].sort());
    assert.equal(verifyCopy(gates).refusal, undefined);
  });
});

test("a refresher that hangs holding the lock is killed at its time, and meanwhile a start runs the current copy and a bootstrap gives up", async () => {
  await withGates(async ({ repo: gitDir, gates, seam, said, current, commit, copies, env, base }) => {
    assert.equal(await seam(), 0);
    const next = commit("next");
    const hanging = [process.execPath, "--input-type=module", "-e", `import { refresh } from ${JSON.stringify(GATES_MODULE)}; await refresh({ repo: ${JSON.stringify(gitDir)}, policy: { allowLocal: true }, hang: "locked" });`];
    const job = process.platform === "win32" ? capped(hanging, { processGB: 4, jobGB: 8, minutes: 1 }) : null;
    const runner = spawn(process.execPath, ["-e", TREE_RUNNER], { env: { ...env, ...(job ? job.env : {}), TREE_ARGS: JSON.stringify(job ? [job.cmd, ...job.args] : hanging), TREE_TIMEOUT: "12000" }, stdio: ["ignore", "pipe", "ignore"] });
    let out = "";
    runner.stdout.setEncoding("utf8").on("data", (d) => (out += d));
    const done = new Promise((resolve) => runner.on("exit", resolve));
    for (let i = 0; i < 100 && !lockHolder(path.join(gates, "lock")).includes("a refresh to"); i++) await new Promise((resolve) => setTimeout(resolve, 100));
    assert.match(lockHolder(path.join(gates, "lock")), /a refresh to/);
    said.length = 0;
    assert.equal(await seam({ waitMs: 800 }), 0);
    assert.match(said.join("\n"), /a refresh holds the lock \(pid \d+ since .*\), so this start writes nothing and runs the current copy/);
    said.length = 0;
    assert.equal(await seam({ bootstrap: true, bootstrapWaitMs: 800 }), 1);
    assert.match(said.join("\n"), /the bootstrap gives up and writes nothing: a refresh holds the lock \(pid \d+/);
    assert.equal(current(), base);
    await done;
    assert.equal(JSON.parse(out).timedOut, true, "the hanging refresher was not killed at its time");
    const lock = await takeLock(path.join(gates, "lock"), { waitMs: 3000, label: "after" });
    assert.ok(lock, "the lock stayed taken after the hanging refresher was killed");
    lock.release();
    assert.equal(await seam(), 0);
    assert.equal(current(), next);
    assert.ok(!copies().some((name) => name.startsWith("staging-")));
  });
});

test("a refresh's time limit covers both asks of the remote, the wait and the write, a waiter's bound is shorter, and so is the memory without a copy", () => {
  assert.ok(REFRESH_MS >= 2 * (GET_URL_MS + REMOTE_MS) + WAIT_MS + WRITE_MS);
  assert.ok(REFRESH_MS > 2 * REMOTE_MS + WRITE_MS);
  assert.ok(WAIT_MS < REFRESH_MS);
  assert.ok(RETRY_MS < REMEMBER_MS);
});

test("with no git of the list found, the current copy stays and says how to fix the list, and the entries hold it to its manifest", async () => {
  await withGates(async ({ work, git, gates, seam, said, current, commit, base }) => {
    assert.equal(await seam(), 0);
    const next = commit("next");
    // The ref packed, its loose file removed: the files still say where main is, and since when.
    git(work, "pack-refs", "--all");
    fs.rmSync(path.join(repo(work), "refs", "remotes", "origin", "main"), { force: true });
    assert.equal(localMain(repo(work)), next);
    assert.equal(await seam({ noGit: true }), 0);
    const words = said.join("\n");
    assert.match(words, /no git is found at .*so the current copy stays; refs\/remotes\/origin\/main has named another commit since \d{4}-/s);
    assert.match(words, /Add the location to gitLocations in tools\/gate-entries\/verify\.mjs, then run the bootstrap/);
    assert.equal(current(), base);
    const checked = verifyCopy(gates, { bin: null });
    assert.equal(checked.refusal, undefined);
    assert.match(checked.notes.join("\n"), /no git is found .* held to its manifest alone/s);
    drift(gates, "tools/capped.mjs");
    assert.match(verifyCopy(gates, { bin: null }).refusal, /has drifted from its manifest: tools\/capped\.mjs/);
  });
});

test("a refresh killed at any step leaves the lock free and a state the next refresh completes", async () => {
  await withGates(async ({ gates, seam, seamChild, current, commit }) => {
    assert.equal(await seam(), 0);
    for (const step of ["halfway", "ready", "manifest", "rename", ...ENTRIES.map((name) => `entry:${name}`), "current"]) {
      const next = commit(step);
      const killed = seamChild({ stopAt: step });
      assert.notEqual(killed.status, 0, `${step}: the refresh was not killed: ${killed.stderr}`);
      const lock = await takeLock(path.join(gates, "lock"), { waitMs: 3000, label: "after a kill" });
      assert.ok(lock, `${step}: the lock stayed taken after the kill`);
      lock.release();
      assert.equal(await seam(), 0, step);
      assert.equal(current(), next, step);
      const checked = verifyCopy(gates);
      assert.equal(checked.refusal, undefined, `${step}: ${checked.refusal}`);
    }
  });
});

test("a refresh while a file of the old copy is in use succeeds and leaves it, and an entry Windows refuses to replace leaves current until the next start", { skip: windowsOnly }, async () => {
  await withGates(async ({ gates, seam, said, current, commit, base }) => {
    assert.equal(await seam(), 0);
    const used = path.join(gates, base, "tools", "capped.ps1");
    const before = fs.readFileSync(used);
    const fd = fs.openSync(used, fs.constants.O_RDONLY | 0x10000000);
    try {
      for (const message of ["second", "third"]) {
        const next = commit(message);
        assert.equal(await seam(), 0);
        assert.equal(current(), next);
      }
    } finally {
      fs.closeSync(fd);
    }
    assert.ok(fs.readFileSync(used).equals(before));
    const hook = path.join(gates, "entry", "hooks", "pre-push");
    fs.appendFileSync(path.join(gates, current(), "manifest"), "");
    const held = current();
    const next = commit("fourth", { "tools/gate-entries/hooks/pre-push": (old) => `${old}# changed\n` });
    const lockFd = fs.openSync(hook, fs.constants.O_RDONLY | 0x10000000);
    try {
      said.length = 0;
      assert.equal(await seam(), 0);
      assert.match(said.join("\n"), /installed but not yet current: Windows refused to replace the entries hooks\/pre-push/);
      assert.equal(current(), held);
    } finally {
      fs.closeSync(lockFd);
    }
    assert.equal(await seam(), 0);
    assert.equal(current(), next);
  });
});

test("of the copy a refresh replaces, only the refresher and its closure run", async () => {
  await withGates(async ({ dir, work, git, run, gates, seam, current, commit }) => {
    const mark = path.join(dir, "imported").replace(/\\/g, "/");
    const marked = commit("marked", { "tools/push.mjs": (old) => `import { writeFileSync as markImported } from "node:fs";\nmarkImported(${JSON.stringify(mark)}, "");\n${old}` });
    assert.equal(await seam(), 0);
    assert.equal(current(), marked);
    commit("next");
    const r = spawnSync(process.execPath, [path.join(gates, "entry", "refresh.mjs")], { encoding: "utf8", cwd: work });
    assert.equal(r.status, 0, r.stderr);
    assert.ok(!fs.existsSync(mark), "a refresh imported the old copy's push.mjs");
    // A positive control: the copy's hook does import it.
    git(work, "config", "core.hooksPath", path.join(gates, "entry", "hooks").replace(/\\/g, "/"));
    run(work, ["push", "origin", "HEAD:refs/heads/topic"]);
    assert.ok(fs.existsSync(mark), "the control did not import push.mjs, so this case proves nothing");
  });
});

test("an installed refresher with a committed bug leaves the current copy, and the bootstrap recovers it", async () => {
  await withGates(async ({ work, gates, seam, current, commit }) => {
    const broken = commit("broken", { "tools/gates.mjs": (old) => String(old).replace("function confirm(ctx, url) {", 'function confirm(ctx, url) {\n  return { refused: "a planted bug" };') });
    assert.equal(await seam(), 0);
    assert.equal(current(), broken);
    const fixed = commit("fixed");
    const r = spawnSync(process.execPath, [path.join(gates, "entry", "refresh.mjs")], { encoding: "utf8", cwd: work });
    assert.equal(r.status, 0, r.stderr);
    assert.equal(current(), broken);
    assert.equal(await seam({ bootstrap: true }), 0);
    assert.equal(current(), fixed);
  });
});

test("an entry of another interface version reports itself as stale, the exempt refresh entry still runs, and the refresh clears it", async () => {
  await withGates(async ({ gates, seam, entry }) => {
    assert.equal(await seam(), 0);
    const verify = path.join(gates, "entry", "verify.mjs");
    fs.writeFileSync(verify, String(fs.readFileSync(verify)).replace(`export const INTERFACE = ${INTERFACE};`, `export const INTERFACE = ${INTERFACE + 1};`));
    for (const [name, status] of [["capped.mjs", 2], ["hook.mjs", 1]]) {
      const r = entry(name, name === "capped.mjs" ? ["--", process.execPath, "-e", "0"] : []);
      assert.equal(r.status, status, `${name}: ${r.stderr}`);
      assert.match(r.stderr, /the entry is stale; run node tools\/quick\.mjs/);
    }
    const refreshed = entry("refresh.mjs");
    assert.equal(refreshed.status, 0, refreshed.stderr);
    assert.match(refreshed.stderr, /restored the entries/);
    assert.equal(entry("capped.mjs", ["--", process.execPath, "-e", "0"]).status, 0);
  });
});

test("offline, a start asks no remote while origin/main names the copy, and asks once while it names another", async () => {
  await withGates(async ({ remote, seam, said, current, commit, base }) => {
    assert.equal(await seam(), 0);
    fs.renameSync(remote, `${remote}.away`);
    said.length = 0;
    assert.equal(await seam(), 0);
    assert.deepEqual(said, [], "a start asked the remote while origin/main named the copy");
    fs.renameSync(`${remote}.away`, remote);
    commit("next");
    fs.renameSync(remote, `${remote}.away`);
    assert.equal(await seam(), 0);
    assert.match(said.join("\n"), /could not be asked .*since \d{4}-/s);
    said.length = 0;
    assert.equal(await seam(), 0);
    assert.match(said.join("\n"), /as it did at the last ask/);
    assert.equal(current(), base);
  });
});

test("the refresher's command reads no variable but the handoff's, so nothing but the seam admits a URL that is not https", () => {
  const text = fs.readFileSync(path.join(TREE, "tools", "gates.mjs"), "utf8").replace(/export function inheritedLock\(file, env = process\.env\)/, "").replace(/export const TREE_RUNNER = `[\s\S]*?`;/, "");
  assert.doesNotMatch(text, /process\.env/);
  assert.match(text, /lockFile: path\.join\(gates, "lock"\), say, allowLocal: false, seam: false/);
});

test("the bootstrap line runs main's bootstrap from its objects, whose staged refresher confirms the commit itself, and a refused install leaves no staged set", async () => {
  await withGates(async ({ work, git, gates, remote, blob, base, env }) => {
    // The line's body, read from the commit's objects and run from standard input with the commit,
    // as the line runs it. In the fixture the staged refresher then refuses, since the remote's URL
    // is a path and the command admits only https: it writes nothing, says why, and the bootstrap
    // removes the staged set. The set it stages is the commit's: the installs of the tests that
    // admit the fixture's remote check it.
    const body = blob(base, "tools/gate-entries/bootstrap.mjs");
    const ran = spawnSync(process.execPath, ["--input-type=module", "-", base], { cwd: work, env, input: body, encoding: "utf8" });
    assert.equal(ran.status, 1, ran.stderr);
    assert.match(ran.stderr, /the refresher of [0-9a-f]{12} writes nothing: the remote's URL .* is not https/);
    assert.match(ran.stderr, /the staged set is removed, since its refresher did not install it/);
    assert.deepEqual(fs.readdirSync(gates).filter((name) => name.startsWith("staging-")), [], "a refused install left its staged set");
    for (const name of ENTRIES) assert.ok(fs.existsSync(path.join(gates, "entry", ...name.split("/"))), `the entry ${name} was not written`);
    assert.ok(!fs.existsSync(path.join(gates, "current")), "a copy was installed from a URL that is not https");
    // A commit the repository does not hold: fetch first.
    const missing = spawnSync(process.execPath, ["--input-type=module", "-", "0".repeat(40)], { cwd: work, env, input: body, encoding: "utf8" });
    assert.equal(missing.status, 1);
    assert.match(missing.stderr, /is not in this repository: fetch first/);
    // The line itself, as CLAUDE.md gives it, in Windows PowerShell, with the fixture's remote.
    if (process.platform === "win32") {
      for (const name of fs.readdirSync(gates)) if (name.startsWith("staging-")) fs.rmSync(path.join(gates, name), { recursive: true });
      const line = bootstrapLine(remote);
      const shell = path.join(process.env.SystemRoot || "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
      const r = spawnSync(shell, ["-NoProfile", "-NonInteractive", "-Command", line], { cwd: work, env, encoding: "utf8" });
      assert.match(`${r.stdout}${r.stderr}`, /writes nothing: the remote's URL .* is not https/, `the line did not reach the staged refresher: ${r.stdout}${r.stderr}`);
      assert.deepEqual(fs.readdirSync(gates).filter((name) => name.startsWith("staging-")), []);
    }
    git(work, "status", "--short");
  });
});

test("the node versions package.json declares are the ones the refresher requires, those with import.meta.main", () => {
  const declared = JSON.parse(fs.readFileSync(path.join(TREE, "package.json"), "utf8")).engines.node;
  assert.equal(declared, NODE_ENGINES, "package.json's engines and the refresher's floor differ");
  const locked = JSON.parse(fs.readFileSync(path.join(TREE, "package-lock.json"), "utf8")).packages[""].engines.node;
  assert.equal(locked, NODE_ENGINES, "package-lock.json records other engines than package.json's");
  const cases = { "22.17.1": false, "22.18.0": true, "22.23.2": true, "23.11.0": false, "24.1.0": false, "24.2.0": true, "24.22.1": true, "26.7.0": true, "20.19.0": false };
  for (const [version, meets] of Object.entries(cases)) assert.equal(nodeMeetsFloor(version), meets, version);
  assert.ok(nodeMeetsFloor(process.versions.node), `this node, ${process.versions.node}, is outside ${NODE_ENGINES}`);
  assert.equal(typeof import.meta.main, "boolean", "this node has no import.meta.main");
});

// The first review of #218.

/// The text of the command's context, which a patched fixture's commits turn to admit the fixture's
/// local remote, so that the command's own path, its two processes and its retry, runs in a test.
const COMMAND_POLICY = 'lockFile: path.join(gates, "lock"), say, allowLocal: false, seam: false';

test("the command hands the lock to the new set's refresher, in a process of its own, which installs", async () => {
  await withGates(
    async ({ work, gates, seam, current, commit, entry }) => {
      assert.equal(await seam(), 0);
      const next = commit("next");
      const r = entry("refresh.mjs");
      assert.equal(r.status, 0, r.stderr);
      assert.match(r.stderr, new RegExp(`installed main's gates at ${next.slice(0, 12)}`));
      assert.equal(current(), next);
      assert.equal(verifyCopy(gates).refusal, undefined);
      assert.ok(fs.readFileSync(path.join(gates, "lock.who"), "utf8").includes(`a refresh to ${next.slice(0, 12)}`), "the lock was not taken for the refresh");
      assert.ok(fs.readFileSync(path.join(work, "tools", "gates.mjs"), "utf8").includes(COMMAND_POLICY.replace("allowLocal: false", "allowLocal: true")));
    },
    { patched: true },
  );
});

/// The kill points of a first install, in order: the bootstrap's lock, its early entries, the hook's
/// first, its staged set, then the install's manifest, rename and entries.
const FIRST_INSTALL = ["locked", ...ENTRIES.map((name) => `early:${name}`), "halfway", "staged", "manifest", "rename", ...ENTRIES.map((name) => `entry:${name}`)];
/// Those after which a whole set stands, staged or renamed, for the next start to finish.
const LEFT_PENDING = new Set(["staged", "manifest", "rename", ...ENTRIES.map((name) => `entry:${name}`)]);

test("a first install killed at any step is finished by the next start's refresh entry, or by the bootstrap", async () => {
  await withGates(
    async ({ gates, seam, seamChild, current, entry, base }) => {
      for (const step of FIRST_INSTALL) {
        fs.rmSync(gates, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
        const killed = seamChild({ bootstrap: true, stopAt: step });
        assert.notEqual(killed.status, 0, `${step}: the install was not killed: ${killed.stderr}`);
        assert.ok(!fs.existsSync(path.join(gates, "current")), `${step}: current was written`);
        const lock = await takeLock(path.join(gates, "lock"), { waitMs: 3000, label: "after a kill" });
        assert.ok(lock, `${step}: the lock stayed taken after the kill`);
        lock.release();
        // The entries written so far are the hook's first, each whole.
        const written = ENTRIES.filter((name) => fs.existsSync(path.join(gates, "entry", ...name.split("/"))));
        assert.deepEqual(written, ENTRIES.slice(0, written.length), `${step}: an entry was written before the hook's`);
        assert.equal(pendingCopy(gates) !== null, LEFT_PENDING.has(step), `${step}: what the next start finishes`);
        if (LEFT_PENDING.has(step)) {
          const r = entry("refresh.mjs");
          assert.equal(r.status, 0, `${step}: ${r.stderr}`);
        } else {
          if (fs.existsSync(path.join(gates, "entry", "refresh.mjs"))) {
            const r = entry("refresh.mjs");
            assert.equal(r.status, 2, `${step}: ${r.stderr}`);
            assert.match(r.stderr, /Norman runs the bootstrap/, step);
          }
          assert.equal(await seam({ bootstrap: true }), 0, step);
        }
        assert.equal(current(), base, step);
        assert.equal(verifyCopy(gates).refusal, undefined, step);
      }
    },
    { patched: true },
  );
});

test("a new refresher writes nothing where the local origin/main names another commit than the remote's", async () => {
  await withGates(
    async ({ work, git, gates, seam, current, commit, base }) => {
      assert.equal(await seam(), 0);
      const next = commit("next");
      git(work, "update-ref", "refs/remotes/origin/main", base);
      const staging = path.join(gates, `staging-${next}-0c`);
      stageByHand(repo(work), next, staging);
      const r = spawnSync(process.execPath, [path.join(staging, "tools", "gates.mjs")], { encoding: "utf8" });
      assert.equal(r.status, 1, r.stderr);
      assert.match(r.stderr, new RegExp(`writes nothing: the remote's main is ${next.slice(0, 12)}, and refs/remotes/origin/main ${base.slice(0, 12)}`));
      assert.equal(current(), base);
    },
    { patched: true },
  );
});

test("a file beyond the set in a copy is refused", async () => {
  await withGates(async ({ gates, seam, entry, base }) => {
    assert.equal(await seam(), 0);
    fs.writeFileSync(path.join(gates, base, "tools", "beyond.mjs"), "export {};\n");
    const r = entry("capped.mjs", ["--", process.execPath, "-e", "0"]);
    assert.equal(r.status, 2);
    assert.match(r.stderr, /tools\/beyond\.mjs is not a file of the set/);
  });
});

test("an answer that keeps the copy is remembered for its time, and asked again after it", async () => {
  await withGates(async ({ remote, gates, seam, said, commit }) => {
    assert.equal(await seam(), 0);
    const next = commit("next");
    fs.renameSync(remote, `${remote}.away`);
    assert.equal(await seam(), 0);
    assert.match(said.join("\n"), /could not be asked/);
    said.length = 0;
    assert.equal(await seam(), 0);
    assert.match(said.join("\n"), /as it did at the last ask/);
    const kept = path.join(gates, "kept");
    const answer = JSON.parse(fs.readFileSync(kept, "utf8"));
    assert.equal(answer.local, next);
    fs.writeFileSync(kept, JSON.stringify({ ...answer, at: Date.now() - REMEMBER_MS - 1000 }));
    said.length = 0;
    assert.equal(await seam(), 0);
    assert.match(said.join("\n"), /could not be asked/);
    assert.doesNotMatch(said.join("\n"), /as it did at the last ask/);
  });
});

test("cargo's runner entry runs a command outside this repository untouched, before it checks any copy", async () => {
  await withGates(async ({ dir, work, gates, seam, entry }) => {
    assert.equal(await seam(), 0);
    drift(gates, "tools/push.mjs");
    const elsewhere = path.join(dir, "elsewhere");
    fs.mkdirSync(elsewhere);
    const outside = spawnSync(process.execPath, [path.join(gates, "entry", "capped.mjs"), "--mente-only", "--", process.execPath, "-e", "process.exit(7)"], { cwd: elsewhere, encoding: "utf8" });
    assert.equal(outside.status, 7, outside.stderr);
    assert.doesNotMatch(outside.stderr, /mente-gates/, "a project outside this repository heard of its gates");
    // Inside a checkout of this repository, the drifted copy is refused.
    fs.writeFileSync(path.join(work, "Cargo.toml"), "[workspace]\n");
    const inside = entry("capped.mjs", ["--mente-only", "--", process.execPath, "-e", "process.exit(7)"]);
    assert.equal(inside.status, 2);
    assert.match(inside.stderr, /has drifted from its manifest/);
  });
});

test("a new main whose install fails is remembered, and its staging is swept by the next refresh", async () => {
  await withGates(async ({ dir, gates, seam, said, current, commit, copies, base }) => {
    assert.equal(await seam(), 0);
    const staged = () => copies().filter((name) => name.startsWith("staging-"));
    // A set that fails before anything is staged.
    commit("no refresher", { "tools/gate-set.json": () => `${JSON.stringify({ ...SET, set: SET.set.filter((p) => p !== "tools/gates.mjs") }, null, 2)}\n` });
    assert.equal(await seam(), 0);
    assert.match(said.join("\n"), /cannot be installed: the tools\/gate-set\.json of [0-9a-f]{12} declares no set/);
    assert.deepEqual(staged(), []);
    // A set whose install fails after it is staged: its own remote cannot be asked.
    const failing = commit("unreachable", { "tools/gate-set.json": () => `${JSON.stringify({ ...SET, url: path.join(dir, "nowhere.git").replace(/\\/g, "/") }, null, 2)}\n` });
    said.length = 0;
    assert.equal(await seam(), 1);
    assert.match(said.join("\n"), new RegExp(`the refresher of ${failing.slice(0, 12)} writes nothing`));
    assert.equal(staged().length, 1);
    // Remembered for that local commit: no ask, no second staging.
    said.length = 0;
    assert.equal(await seam(), 0);
    assert.match(said.join("\n"), /as it did at the last ask/);
    assert.equal(staged().length, 1, "a remembered failure staged its set again");
    // Once the answer expires, the failing install is tried again: its earlier staging goes first,
    // under the lock, so a main that stays broken leaves one staging directory, not one a try.
    const kept = path.join(gates, "kept");
    fs.writeFileSync(kept, JSON.stringify({ ...JSON.parse(fs.readFileSync(kept, "utf8")), at: Date.now() - REMEMBER_MS - 1000 }));
    assert.equal(await seam(), 1);
    assert.equal(staged().length, 1, "a failed install's staging was left beside the next try's");
    // The next refresh sweeps the failed staging under the lock.
    const next = commit("next");
    assert.equal(await seam(), 0);
    assert.equal(current(), next);
    assert.deepEqual(copies(), [base, next].sort(), "the failed staging was left behind");
  });
});

test("the refresh entry's fast path holds only where a refresh would do nothing", async () => {
  await withGates(async ({ work, git, gates, seam, commit, base }) => {
    assert.equal(await seam(), 0);
    assert.equal(nothingToRefresh(gates, verifyCopy(gates)), true);
    const hook = path.join(gates, "entry", "hook.mjs");
    const kept = fs.readFileSync(hook);
    fs.appendFileSync(hook, "// edited\n");
    assert.equal(nothingToRefresh(gates, verifyCopy(gates)), false, "an edited entry took the fast path");
    fs.writeFileSync(hook, kept);
    commit("next");
    assert.equal(nothingToRefresh(gates, verifyCopy(gates)), false, "a moved origin/main took the fast path");
    git(work, "update-ref", "refs/remotes/origin/main", base);
    assert.equal(nothingToRefresh(gates, verifyCopy(gates)), true);
  });
});

test("no module the hooks or the settings' guard import has an effect at import", () => {
  const processes = fs.readFileSync(path.join(TREE, "tools", "gate-processes.mjs"), "utf8");
  assert.doesNotMatch(processes, /process\.exit/);
  // The refresher's command runs only as node's entry point: its one statement beyond declarations.
  const gates = fs.readFileSync(path.join(TREE, "tools", "gates.mjs"), "utf8");
  const top = gates.split("\n").filter((line) => /^[a-z]/.test(line) && !/^(import|export|const|let|function|async function|\}|\/\/)/.test(line));
  assert.deepEqual(top, ["if (import.meta.main) {"]);
  assert.doesNotMatch(fs.readFileSync(path.join(TREE, "tools", "push.mjs"), "utf8"), /from "\.\/gates\.mjs"/, "push.mjs imports the refresher's command");
});

// The rollout's transition matrix (briefs/gate-tools-from-main.md, "The rollout"): each state a
// first install can be left in, under each of Norman's two lines old or new, cargo's runner (R) and
// core.hooksPath (H); and what a start of quick, cargo or mn, a test binary cargo starts through
// the runner line, and a raw push then do, with the remedy each names. The test refuses a matrix
// that misses or doubles a cell, so that a state or an entry added later must be given its row.

/// The states, in the order a first install passes them: nothing, each stable entry written in
/// turn by the bootstrap, a set half staged, staged whole, renamed to its commit; a bootstrap whose
/// install was refused; and current.
const COPY_STATES = ["absent", ...ENTRIES.map((name) => `early:${name}`), "halfway", "staged", "rename", "refused", "current"];

/// What a start of quick, cargo or mn does.
const START = {
  fallback: { status: 0, out: "fallback", words: /WARNING: (no|a) copy of main's gates is installed[\s\S]*in this order/ },
  copy: { status: 0, out: "copy", none: /WARNING/ },
  bootstrap: { status: 2, words: /runs the bootstrap/ },
  unwritten: { status: 2, words: /names the gates' runner entry, .* which no install has written yet[\s\S]*runs the bootstrap/ },
  "runner-line": { status: 2, words: /cargo's runner on this laptop is .*the gates' capped one[\s\S]*mente-gates\/entry\/capped\.mjs/ },
};
/// What a test binary cargo starts through the runner line does: it runs, or is refused, naming the
/// remedy, or dies on node's own error where the line names an entry no install has written, which
/// the tools' start refuses first, naming the bootstrap.
const BINARY = {
  runs: { status: 7 },
  bootstrap: { status: 2, words: /no copy of main's gates is installed[\s\S]*Norman runs the bootstrap/ },
  refresh: { status: 2, words: /was left half done[\s\S]*run node tools\/quick\.mjs/ },
  half: { status: 2, words: /was left half done[\s\S]*Norman runs the bootstrap/ },
  node: { notStatus: [0, 7], words: /Cannot find module/ },
};
/// What a raw push of a commit main does not hold does: refused by today's working-tree hook or by
/// the copy's, both refusing a tree the quick checks have not passed; refused by the entry, naming
/// the remedy; or, with core.hooksPath naming the entry before any install wrote it, sent, since git
/// runs no hook it cannot find. The order Norman follows holds that cell: the bootstrap first, and
/// only once it has succeeded core.hooksPath, which the tools' warning states.
const PUSH = {
  today: { refused: true, words: /matrix: the working tree's hook[\s\S]*the pre-push hook refused/ },
  copy: { refused: true, words: /the pre-push hook refused/, none: /matrix: the working tree's hook/ },
  sh: { refused: true, words: /the hook's entry stands, but (verify|hook)\.mjs beside it is not installed yet/ },
  bootstrap: { refused: true, words: /no copy of main's gates is installed[\s\S]*Norman runs the bootstrap/ },
  refresh: { refused: true, words: /was left half done[\s\S]*run node tools\/quick\.mjs/ },
  unguarded: { refused: false },
};

// copy state, R, H: a start, a test binary, a raw push. Off Windows cargo reads no runner line, so a
// start refused for its runner goes on to the mode it reached (the test says so where it checks).
// An install left half done is finished by a start only where H names the copy, and the entries
// name that start there and the bootstrap elsewhere.
const MATRIX = [
  ["absent", "old", "old", "fallback", "runs", "today"],
  ["absent", "old", "new", "bootstrap", "runs", "unguarded"],
  ["absent", "new", "old", "unwritten", "node", "today"],
  ["absent", "new", "new", "bootstrap", "node", "unguarded"],
  ["early:hooks/pre-push", "old", "old", "fallback", "runs", "today"],
  ["early:hooks/pre-push", "old", "new", "bootstrap", "runs", "sh"],
  ["early:hooks/pre-push", "new", "old", "unwritten", "node", "today"],
  ["early:hooks/pre-push", "new", "new", "bootstrap", "node", "sh"],
  ["early:verify.mjs", "old", "old", "fallback", "runs", "today"],
  ["early:verify.mjs", "old", "new", "bootstrap", "runs", "sh"],
  ["early:verify.mjs", "new", "old", "unwritten", "node", "today"],
  ["early:verify.mjs", "new", "new", "bootstrap", "node", "sh"],
  ["early:hook.mjs", "old", "old", "fallback", "runs", "today"],
  ["early:hook.mjs", "old", "new", "bootstrap", "runs", "bootstrap"],
  ["early:hook.mjs", "new", "old", "unwritten", "node", "today"],
  ["early:hook.mjs", "new", "new", "bootstrap", "node", "bootstrap"],
  ["early:refresh.mjs", "old", "old", "fallback", "runs", "today"],
  ["early:refresh.mjs", "old", "new", "bootstrap", "runs", "bootstrap"],
  ["early:refresh.mjs", "new", "old", "unwritten", "node", "today"],
  ["early:refresh.mjs", "new", "new", "bootstrap", "node", "bootstrap"],
  ["early:capped.mjs", "old", "old", "fallback", "runs", "today"],
  ["early:capped.mjs", "old", "new", "bootstrap", "runs", "bootstrap"],
  ["early:capped.mjs", "new", "old", "fallback", "bootstrap", "today"],
  ["early:capped.mjs", "new", "new", "bootstrap", "bootstrap", "bootstrap"],
  ["halfway", "old", "old", "fallback", "runs", "today"],
  ["halfway", "old", "new", "bootstrap", "runs", "bootstrap"],
  ["halfway", "new", "old", "fallback", "bootstrap", "today"],
  ["halfway", "new", "new", "bootstrap", "bootstrap", "bootstrap"],
  ["staged", "old", "old", "fallback", "runs", "today"],
  ["staged", "old", "new", "runner-line", "runs", "refresh"],
  ["staged", "new", "old", "fallback", "half", "today"],
  ["staged", "new", "new", "copy", "refresh", "refresh"],
  ["rename", "old", "old", "fallback", "runs", "today"],
  ["rename", "old", "new", "runner-line", "runs", "refresh"],
  ["rename", "new", "old", "fallback", "half", "today"],
  ["rename", "new", "new", "copy", "refresh", "refresh"],
  ["refused", "old", "old", "fallback", "runs", "today"],
  ["refused", "old", "new", "bootstrap", "runs", "bootstrap"],
  ["refused", "new", "old", "fallback", "bootstrap", "today"],
  ["refused", "new", "new", "bootstrap", "bootstrap", "bootstrap"],
  ["current", "old", "old", "fallback", "runs", "today"],
  ["current", "old", "new", "runner-line", "runs", "copy"],
  ["current", "new", "old", "fallback", "runs", "today"],
  ["current", "new", "new", "copy", "runs", "copy"],
];

/// The start of quick, cargo and mn, the lines quick.mjs runs, which the policy test holds each of
/// the three tools to.
const START_LINES = [
  "const gates = refreshGates({ cwd: process.cwd() });",
  "if (!gates) process.exit(2);",
  'const fallback = gates === "fallback";',
  "if (!requireRunner({ fallback })) process.exit(2);",
  "if (!requireHooks({ cwd: process.cwd(), fallback })) process.exit(2);",
];

test("the rollout's transition matrix: each copy state under Norman's lines old or new, for a start, a test binary and a raw push, with the remedy each names", async () => {
  const keys = MATRIX.map(([state, r, h]) => `${state} R ${r} H ${h}`).sort();
  const cells = COPY_STATES.flatMap((state) => ["old", "new"].flatMap((r) => ["old", "new"].map((h) => `${state} R ${r} H ${h}`))).sort();
  assert.deepEqual(keys, cells, "the matrix misses or doubles a cell");
  for (const [state, , , start, binary, push] of MATRIX) assert.ok(START[start] && BINARY[binary] && PUSH[push], `${state}: an outcome the matrix names is not defined`);
  const quick = fs.readFileSync(path.join(TREE, "tools", "quick.mjs"), "utf8");
  for (const line of START_LINES) assert.ok(quick.includes(`\n${line}\n`), `quick.mjs's start no longer holds: ${line}`);
  const startCode = [`import { refreshGates, requireHooks, requireRunner } from ${JSON.stringify(new URL("./local-limits.mjs", import.meta.url).href)};`, ...START_LINES, "console.log(gates);"].join("\n");
  await withGates(
    async ({ dir, env, run, git, work, gates, remote, blob, base, seam, seamChild, commit }) => {
      // A commit main does not hold, for the pushes; the checkout taken as this repository's, for
      // the runner; today's working-tree hook marked, so that its refusal is told from the copy's.
      commit("not on main", {}, { push: false });
      fs.writeFileSync(path.join(work, "Cargo.toml"), "[workspace]\n");
      const hookAt = path.join(work, "tools", "git-hooks", "pre-push");
      const hookText = fs.readFileSync(hookAt, "utf8");
      assert.match(hookText, /^#!\/bin\/sh\r?\n/);
      fs.writeFileSync(hookAt, hookText.replace(/^#!\/bin\/sh\r?\n/, "#!/bin/sh\necho \"matrix: the working tree's hook\" >&2\n"));
      const config = path.join(dir, ".cargo", "config.toml");
      fs.mkdirSync(path.dirname(config), { recursive: true });
      const runner = { old: path.join(work, "tools", "capped.mjs"), new: path.join(gates, "entry", "capped.mjs") };
      const line = { old: workingTreeRunnerLine(work), new: runnerLine(gates) };
      const hooks = { old: path.join(work, "tools", "git-hooks"), new: path.join(gates, "entry", "hooks") };
      const startEnv = { ...Object.fromEntries(Object.entries(env).filter(([k]) => k !== RUNNER_VARIABLE)), CARGO_HOME: path.join(dir, "cargo-home") };
      const kept = (state) => path.join(dir, "states", state.replace(/[^A-Za-z0-9.-]/g, "_"));
      const clear = () => fs.rmSync(gates, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
      // Each state made once, by a first install killed where it leaves it, and kept.
      for (const state of COPY_STATES) {
        clear();
        if (state === "current") assert.equal(await seam({ bootstrap: true }), 0);
        else if (state === "refused") {
          // Norman's line, the remote away: its staged refresher refuses.
          fs.renameSync(remote, `${remote}.away`);
          const ran = spawnSync(process.execPath, ["--input-type=module", "-", base], { cwd: work, env, input: blob(base, "tools/gate-entries/bootstrap.mjs"), encoding: "utf8" });
          fs.renameSync(`${remote}.away`, remote);
          assert.equal(ran.status, 1, ran.stderr);
          assert.deepEqual(fs.readdirSync(gates).filter((name) => /^staging-|^[0-9a-f]{40}$/.test(name)), [], `a refused bootstrap left a set: ${ran.stderr}`);
        } else if (state !== "absent") {
          const killed = seamChild({ bootstrap: true, stopAt: state });
          assert.notEqual(killed.status, 0, `${state}: the install was not killed: ${killed.stderr}`);
        }
        if (fs.existsSync(gates)) fs.cpSync(gates, kept(state), { recursive: true });
      }
      const check = (cell, actor, want, status, words, out = "") => {
        const where = `${cell}: ${actor} ended with ${status}: ${words}`;
        if (want.status !== undefined) assert.equal(status, want.status, where);
        if (want.notStatus) assert.ok(!want.notStatus.includes(status), where);
        if (want.refused !== undefined) assert.equal(status !== 0, want.refused, where);
        if (want.words) assert.match(words, want.words, where);
        if (want.none) assert.doesNotMatch(words, want.none, where);
        if (want.out) assert.equal(out.trim(), want.out, where);
      };
      // A push does not depend on R, and neither it nor a test binary writes the gates: a push is
      // run once for its state and H, a test binary once for its state, R and H.
      const listing = (at) => (fs.existsSync(at) ? fs.readdirSync(at, { recursive: true }).map((name) => `${name} ${fs.statSync(path.join(at, name)).isFile() ? fs.statSync(path.join(at, name)).size : "dir"}`).sort() : []);
      const done = new Set();
      let pushes = 0;
      for (const [state, r, h, start, binary, push] of MATRIX) {
        const cell = `${state}, R ${r}, H ${h}`;
        clear();
        if (fs.existsSync(kept(state))) fs.cpSync(kept(state), gates, { recursive: true });
        git(work, "config", "core.hooksPath", hooks[h].replace(/\\/g, "/"));
        fs.writeFileSync(config, `[target.${RUNNER_TARGET}]\n${line[r]}\n`);
        if (!done.has(`binary ${state} ${r} ${h}`)) {
          done.add(`binary ${state} ${r} ${h}`);
          const b = spawnSync(process.execPath, [runner[r], "--mente-only", "--", process.execPath, "-e", "process.exit(7)"], { cwd: work, env, encoding: "utf8" });
          check(cell, "a test binary", BINARY[binary], b.status, `${b.stdout}${b.stderr}`);
        }
        if (!done.has(`push ${state} ${h}`)) {
          done.add(`push ${state} ${h}`);
          pushes += 1;
          const pushed = run(work, ["push", "origin", `HEAD:refs/heads/matrix-${pushes}`]);
          check(cell, "a raw push", PUSH[push], pushed.status, `${pushed.stdout}${pushed.stderr}`);
        }
        const before = listing(gates);
        const s = spawnSync(process.execPath, ["--input-type=module", "-e", startCode], { cwd: work, env: startEnv, encoding: "utf8" });
        const reached = process.platform !== "win32" && (start === "unwritten" || start === "runner-line") ? (h === "old" ? "fallback" : "copy") : start;
        check(cell, "a start", START[reached], s.status, s.stderr, s.stdout);
        if (h === "old") {
          // While the hooks are today's, a start says a copy is installed only where one is
          // current, and writes the gates only to refresh a current copy: an install left half
          // done waits for the copy mode or the bootstrap.
          assert.match(s.stderr, state === "current" ? /WARNING: a copy of main's gates is installed/ : /WARNING: no copy of main's gates is installed/, `${cell}: ${s.stderr}`);
          if (state !== "current") assert.deepEqual(listing(gates), before, `${cell}: a start wrote the gates while no copy is current`);
        }
      }
    },
    { patched: true },
  );
});

test("a staging directory of a commit other than the local origin/main is no install to finish", async () => {
  await withGates(async ({ work, gates, commit, base }) => {
    const branch = commit("a branch's set", {}, { push: false });
    fs.mkdirSync(gates, { recursive: true });
    stageByHand(repo(work), branch, path.join(gates, `staging-${branch}-0a`));
    assert.equal(pendingCopy(gates), null, "a branch's set left staged by a hand run was taken for main's");
    stageByHand(repo(work), base, path.join(gates, `staging-${base}-0b`));
    assert.equal(pendingCopy(gates)?.commit, base);
  });
});

test("restoring the entries reads the copy again under the lock, where another refresh switched current", async () => {
  await withGates(async ({ work, git, gates, seam, current, commit, base, blob }) => {
    assert.equal(await seam(), 0);
    const next = commit("entries of their own", { "tools/gate-entries/capped.mjs": (old) => Buffer.concat([old, Buffer.from("// the next commit's entry\n")]) });
    assert.equal(await seam(), 0);
    assert.equal(current(), next);
    // current set back to base by hand, which origin/main names, so that a refresh only restores
    // the entries, which are next's: it reads base's, then waits for the lock.
    git(work, "update-ref", "refs/remotes/origin/main", base);
    fs.writeFileSync(path.join(gates, "current"), `${base}\n`);
    const lock = await takeLock(path.join(gates, "lock"), { waitMs: 3000, label: "the test" });
    assert.ok(lock);
    const restoring = seam({ waitMs: 20000 });
    // Meanwhile another refresh switches current to next, then lets the lock go.
    fs.writeFileSync(path.join(gates, "current"), `${next}\n`);
    lock.release();
    assert.equal(await restoring, 0);
    assert.ok(fs.readFileSync(path.join(gates, "entry", "capped.mjs")).equals(blob(next, "tools/gate-entries/capped.mjs")), "the entries of the copy read before the lock were written");
  });
});

test("a staged set the refresh entry finishes waits for the lock only a start's bound", async () => {
  await withGates(
    async ({ work, gates, base }) => {
      fs.mkdirSync(gates, { recursive: true });
      stageByHand(repo(work), base, path.join(gates, `staging-${base}-0c`));
      // The refresh entry is written as the bootstrap writes it, before the set is staged.
      const entryAt = path.join(gates, "entry", "refresh.mjs");
      fs.mkdirSync(path.dirname(entryAt), { recursive: true });
      for (const name of ["refresh.mjs", "verify.mjs"]) fs.copyFileSync(path.join(gates, `staging-${base}-0c`, "tools", "gate-entries", name), path.join(gates, "entry", name));
      const lock = await takeLock(path.join(gates, "lock"), { waitMs: 3000, label: "the test" });
      assert.ok(lock);
      // The lock is let go after a start's bound and a margin: a staged set that waited longer would
      // then install.
      let held = true;
      const letGo = () => {
        if (held) lock.release();
        held = false;
      };
      const release = setTimeout(letGo, WAIT_MS + 8000);
      const started = Date.now();
      const child = spawn(process.execPath, [entryAt], { cwd: work, stdio: ["ignore", "pipe", "pipe"] });
      let words = "";
      child.stderr.on("data", (d) => (words += d));
      const status = await new Promise((resolve) => child.on("exit", resolve));
      clearTimeout(release);
      letGo();
      assert.equal(status, 2, words);
      assert.match(words, /the staged set waits for the next start and writes nothing: a refresh holds the lock/);
      assert.ok(Date.now() - started < WAIT_MS + 8000, "the staged set waited past a start's bound");
      assert.ok(!fs.existsSync(path.join(gates, "current")));
    },
    { patched: true },
  );
});

test("the bootstrap takes the lock before it writes anything, writes the entries, the hook's first, and hands the lock to its staged refresher", async () => {
  await withGates(
    async ({ work, gates, blob, base, env, current }) => {
      fs.mkdirSync(gates, { recursive: true });
      const lock = await takeLock(path.join(gates, "lock"), { waitMs: 3000, label: "the test" });
      assert.ok(lock);
      const body = blob(base, "tools/gate-entries/bootstrap.mjs");
      const child = spawn(process.execPath, ["--input-type=module", "-", base], { cwd: work, env, stdio: ["pipe", "pipe", "pipe"] });
      let words = "";
      child.stderr.on("data", (d) => (words += d));
      child.stdout.on("data", (d) => (words += d));
      child.stdin.end(body);
      const ended = new Promise((resolve) => child.on("exit", resolve));
      try {
        // The bootstrap says it waits once it finds the lock held.
        const waiting = await new Promise((resolve) => {
          const until = Date.now() + 60000;
          const poll = setInterval(() => {
            if (/the bootstrap waits for it, at most \d+ s, before it writes anything/.test(words) || Date.now() > until || child.exitCode !== null) {
              clearInterval(poll);
              resolve(/the bootstrap waits for it/.test(words));
            }
          }, 100);
        });
        assert.ok(waiting, `the bootstrap did not wait for the lock: ${words}`);
        // While it waits, nothing of mente-gates/ but the lock is written.
        assert.deepEqual(fs.readdirSync(gates).filter((name) => !/^lock(\.who)?$/.test(name)), [], "the bootstrap wrote under mente-gates/ before it took the lock");
      } catch (e) {
        // The bootstrap is ended before the fixture is removed, so that this error stands.
        child.kill();
        await ended;
        throw e;
      } finally {
        lock.release();
      }
      const status = await ended;
      // The staged refresher installs with the lock handed down: one that took the lock itself
      // would find the bootstrap holding it, wait a start's bound, and give up.
      assert.equal(status, 0, words);
      assert.equal(current(), base);
      assert.doesNotMatch(words, /waits for the next start/);
      for (const name of ENTRIES) assert.ok(fs.readFileSync(path.join(gates, "entry", ...name.split("/"))).equals(blob(base, `tools/gate-entries/${name}`)), `the entry ${name} is not the commit's`);
    },
    { patched: true },
  );
});

test("a failure finishing a half install is remembered only a short time, staged or renamed, so a silent remote costs few starts and the next one after finishes it", async () => {
  await withGates(
    async ({ dir, remote, gates, seamChild, entry, current, base }) => {
      for (const step of ["staged", "rename"]) {
        fs.rmSync(gates, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
        const killed = seamChild({ bootstrap: true, stopAt: step });
        assert.notEqual(killed.status, 0, step);
        // The remote cannot be reached: the install left half done stays as it is.
        const away = path.join(dir, "remote-away.git");
        fs.renameSync(remote, away);
        const failed = entry("refresh.mjs");
        assert.equal(failed.status, 2, `${step}: ${failed.stderr}`);
        const kept = path.join(gates, "kept");
        const answer = JSON.parse(fs.readFileSync(kept, "utf8"));
        assert.equal(answer.local, base, step);
        assert.equal(answer.ms, RETRY_MS, `${step}: a failure with no copy current was remembered for longer than a retry's time`);
        // Within that time a start asks no remote.
        const again = entry("refresh.mjs");
        assert.equal(again.status, 2, step);
        assert.match(again.stderr, /as it did at the last (ask|start)/, step);
        // After it, with the remote back, the next start finishes the install.
        fs.renameSync(away, remote);
        fs.writeFileSync(kept, JSON.stringify({ ...answer, at: Date.now() - RETRY_MS - 1000 }));
        const finished = entry("refresh.mjs");
        assert.equal(finished.status, 0, `${step}: ${finished.stderr}`);
        assert.equal(current(), base, step);
      }
    },
    { patched: true },
  );
});

test("a bootstrap run by hand with a commit main does not hold writes no entry over one that stands, and its entries never outlive a confirmed install", async () => {
  await withGates(
    async ({ work, git, gates, remote, seam, entry, blob, base, env, current, commit }) => {
      const mark = (old) => Buffer.concat([Buffer.from('console.error("mark: the branch\'s entry ran");\n'), old]);
      const branch = commit("a branch's entries", { "tools/gate-entries/hook.mjs": mark, "tools/gate-entries/capped.mjs": mark }, { push: false });
      const bootstrap = (sha) => spawnSync(process.execPath, ["--input-type=module", "-", sha], { cwd: work, env, input: blob(sha, "tools/gate-entries/bootstrap.mjs"), encoding: "utf8" });
      const entriesOf = (sha) => ENTRIES.every((name) => fs.readFileSync(path.join(gates, "entry", ...name.split("/"))).equals(blob(sha, `tools/gate-entries/${name}`)));
      fs.writeFileSync(path.join(work, "Cargo.toml"), "[workspace]\n");
      // A healthy copy of main.
      assert.equal(await seam(), 0);
      assert.ok(entriesOf(base));
      // The branch's commit, which the local origin/main does not name: refused before anything.
      const refused = bootstrap(branch);
      assert.equal(refused.status, 1, refused.stderr);
      assert.match(refused.stderr, /is not what refs\/remotes\/origin\/main names .* so the bootstrap writes nothing: fetch first/);
      // With the local origin/main moved to it by hand: the remote refuses it, and neither its
      // entries nor its staged set stay.
      git(work, "update-ref", "refs/remotes/origin/main", branch);
      const moved = bootstrap(branch);
      assert.equal(moved.status, 1, moved.stderr);
      assert.match(moved.stderr, /the refresher of [0-9a-f]{12} writes nothing: the remote's main is/);
      git(work, "update-ref", "refs/remotes/origin/main", base);
      assert.ok(entriesOf(base), "a bootstrap of a commit the remote refused replaced the entries of the healthy copy");
      assert.equal(current(), base);
      assert.deepEqual(fs.readdirSync(gates).filter((name) => name.startsWith("staging-")), []);
      const ran = entry("capped.mjs", ["--mente-only", "--", process.execPath, "-e", "process.exit(7)"]);
      assert.equal(ran.status, 7, ran.stderr);
      assert.doesNotMatch(ran.stderr, /mark: the branch's entry ran/);
      // An entry missing beside the current copy is not written from that commit either: the next
      // refresh restores it from the copy.
      fs.rmSync(path.join(gates, "entry", "capped.mjs"));
      git(work, "update-ref", "refs/remotes/origin/main", branch);
      assert.equal(bootstrap(branch).status, 1);
      git(work, "update-ref", "refs/remotes/origin/main", base);
      assert.ok(!fs.existsSync(path.join(gates, "entry", "capped.mjs")), "a bootstrap wrote an entry beside a current copy before the remote confirmed its commit");
      assert.equal(await seam(), 0);
      assert.ok(entriesOf(base));
      // With no copy current, an entry that stands, here main's from a bootstrap the remote did not
      // answer, is not replaced either.
      fs.rmSync(gates, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
      fs.renameSync(remote, `${remote}.away`);
      assert.equal(bootstrap(base).status, 1);
      fs.renameSync(`${remote}.away`, remote);
      assert.ok(entriesOf(base));
      git(work, "update-ref", "refs/remotes/origin/main", branch);
      assert.equal(bootstrap(branch).status, 1);
      git(work, "update-ref", "refs/remotes/origin/main", base);
      assert.ok(entriesOf(base), "a bootstrap replaced an entry before the remote confirmed its commit");
      // With nothing installed, the branch's entries stand where none did, inert until a confirmed
      // install replaces them.
      fs.rmSync(gates, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
      git(work, "update-ref", "refs/remotes/origin/main", branch);
      assert.equal(bootstrap(branch).status, 1);
      git(work, "update-ref", "refs/remotes/origin/main", base);
      assert.ok(entriesOf(branch));
      const confirmed = bootstrap(base);
      assert.equal(confirmed.status, 0, confirmed.stderr);
      assert.ok(entriesOf(base), "the entries of a commit the remote refused outlived the confirmed install");
    },
    { patched: true },
  );
});
