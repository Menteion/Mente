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
import { fileURLToPath } from "node:url";
import { LOCK_FD_VARIABLE, PASSED, TREE_RUNNER, blobId, gateEnv, gitLocations, systemGit, takeLock } from "./local-limits.mjs";
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

const LIMITS = new URL("./local-limits.mjs", import.meta.url).href;

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

/** The source of a node program that imports local-limits.mjs as `L`, then runs `code`. */
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
