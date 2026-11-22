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
import { PASSED, capped } from "./local-limits.mjs";
import { INTERFACE, blobId, bootstrapLine, filesOf, gateEnv, gitLocations, localMain, manifestText, mismatch, readManifest, systemGit, verifyCopy } from "./gate-entries/verify.mjs";
import { ENTRIES, GET_URL_MS, LOCK_FD_VARIABLE, NODE_ENGINES, REFRESH_MS, REMOTE_MS, TREE_RUNNER, WAIT_MS, WRITE_MS, lockHolder, nodeMeetsFloor, refresh, takeLock } from "./gates.mjs";
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
const LIMITS = new URL("./gates.mjs", import.meta.url).href;

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

/** The source of a node program that imports gates.mjs as `L`, then runs `code`. */
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
async function withGates(body) {
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
    for (const rel of SET.set) {
      fs.mkdirSync(path.dirname(path.join(staging, rel)), { recursive: true });
      fs.writeFileSync(path.join(staging, rel), blob(next, rel));
    }
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
      for (const rel of ["tools/gates.mjs", "tools/local-limits.mjs", "tools/store-apps.mjs", "tools/capped.ps1", "tools/gate-entries/verify.mjs"]) {
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
    // The bootstrap's staged refresher of that commit: the command admits only https.
    const staging = path.join(gates, `staging-${moved}-0b`);
    for (const rel of SET.set) {
      fs.mkdirSync(path.dirname(path.join(staging, rel)), { recursive: true });
      fs.writeFileSync(path.join(staging, rel), blob(moved, rel));
    }
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
    const listing = execFileSync(systemGit(), ["--git-dir", repo(work), "ls-tree", "-r", "-z", "--full-tree", branch, "--", ...SET.set], { env: gateEnv(), encoding: "utf8" });
    const files = filesOf(listing, SET.set);
    for (const f of files) {
      fs.mkdirSync(path.dirname(path.join(copy, f.path)), { recursive: true });
      fs.writeFileSync(path.join(copy, f.path), blob(branch, f.path), { mode: f.mode === "100755" ? 0o755 : 0o644 });
    }
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
  await withGates(async ({ work, git, seam, current, base }) => {
    assert.equal(await seam(), 0);
    for (const [mode, type] of [["120000", "a symlink"], ["160000", "a submodule"]]) {
      const id = mode === "160000" ? base : git(work, "hash-object", "-w", "--stdin");
      git(work, "update-index", "--cacheinfo", `${mode},${id},tools/raw-build.mjs`);
      git(work, "commit", "-q", "-m", type);
      git(work, "push", "-q", "origin", "main");
      git(work, "fetch", "-q", "origin");
      await assert.rejects(seam(), /tools\/raw-build\.mjs is a \w+ of mode \d+, not a file of mode 100644 or 100755/, type);
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
  for (const rel of ["tools/gates.mjs", "tools/gate-entries/verify.mjs", "tools/gate-entries/refresh.mjs"]) {
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

test("a refresh's time limit covers both asks of the remote, the wait and the write, and a waiter's bound is shorter", () => {
  assert.ok(REFRESH_MS >= 2 * (GET_URL_MS + REMOTE_MS) + WAIT_MS + WRITE_MS);
  assert.ok(REFRESH_MS > 2 * REMOTE_MS + WRITE_MS);
  assert.ok(WAIT_MS < REFRESH_MS);
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

test("the bootstrap line stages exactly main's set from its objects and starts the staged refresher, which confirms it itself", async () => {
  await withGates(async ({ work, git, gates, remote, blob, base, env }) => {
    // The line's body, read from the commit's objects and run from standard input with the commit,
    // as the line runs it. In the fixture the staged refresher then refuses, since the remote's URL
    // is a path and the command admits only https: it writes nothing, and says why.
    const body = blob(base, "tools/gate-entries/bootstrap.mjs");
    const ran = spawnSync(process.execPath, ["--input-type=module", "-", base], { cwd: work, env, input: body, encoding: "utf8" });
    assert.equal(ran.status, 1, ran.stderr);
    assert.match(ran.stderr, /the refresher of [0-9a-f]{12} writes nothing: the remote's URL .* is not https/);
    const staged = fs.readdirSync(gates).filter((name) => name.startsWith(`staging-${base}-`));
    assert.equal(staged.length, 1, "the line staged no set, or more than one");
    const files = filesOf(execFileSync(systemGit(), ["--git-dir", repo(work), "ls-tree", "-r", "-z", "--full-tree", base, "--", ...SET.set], { env: gateEnv(), encoding: "utf8" }), SET.set);
    assert.equal(mismatch(path.join(gates, staged[0]), files, []), null, "the staged set is not the commit's");
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
      assert.equal(fs.readdirSync(gates).filter((name) => name.startsWith(`staging-${base}-`)).length, 1);
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
