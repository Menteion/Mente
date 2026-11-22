// The push guard (tools/push-guard.mjs, tools/push.mjs): which commands it reads as a push, and
// what it answers for each state of a checkout. The answers run the real hook, an event on its
// standard input, against repositories this test makes, with git's own configuration and the
// caller's GIT_ variables left out. A pass of the quick checks is written as tools/quick.mjs writes
// it (local-limits.mjs, PASSED), so that each case sets the record it needs.
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
import { QUICK, pushes } from "./push.mjs";

const GUARD = fileURLToPath(new URL("./push-guard.mjs", import.meta.url));

test("the guard reads a push in each form it names, from the directory it runs in, and nothing else as one", () => {
  const cwd = path.resolve("/w/repo");
  const at = (p) => path.resolve(cwd, p);
  const cases = [
    ["git push", [[cwd, []]]],
    ["git push -u origin b", [[cwd, ["-u", "origin", "b"]]]],
    ["git -C ../other push origin b", [[at("../other"), ["origin", "b"]]]],
    ["git -C a -C b push", [[at("a/b"), []]]],
    ["git -c user.name=x --no-pager push", [[cwd, []]]],
    ["cd ../other && git push", [[at("../other"), []]]],
    ["cd /c/Repos/x && git push", [[process.platform === "win32" ? path.resolve("C:/Repos/x") : "/c/Repos/x", []]]],
    ["Set-Location ..\\other; git push origin b", [[at("../other"), ["origin", "b"]]]],
    ["echo a; git status && git push origin b | tail -1", [[cwd, ["origin", "b"]]]],
    ['bash -c "cd x && git push"', [[at("x"), []]]],
    ['pwsh -Command "git push"', [[cwd, []]]],
    ["FOO=1 timeout 60 git push", [[cwd, []]]],
    ["& git push", [[cwd, []]]],
    ['git commit -m "git push later"', []],
    ["echo git push", []],
    ["git log --grep push", []],
    ["git stash push -m x", []],
    ["node tools/push.mjs", []],
  ];
  for (const [command, want] of cases) {
    assert.deepEqual(
      pushes(command, cwd).map((p) => [p.dir, p.args]),
      want,
      command,
    );
  }
  assert.equal(pushes("cd $D && git push", cwd)[0].dir, null, "a directory named by a variable");
  assert.equal(pushes("cd && git push", cwd)[0].dir, null, "a cd with no directory");
  assert.equal(pushes("git -C $D push", cwd)[0].dir, null, "git -C with a variable");
  assert.equal(pushes("git --git-dir=x/.git push", cwd)[0].elsewhere, true);
  assert.equal(pushes("git --work-tree x push", cwd)[0].elsewhere, true);
});

/** Runs `body` with a checkout of this repository's shape made for it, and removes it after. */
function withCheckout(body) {
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
    const work = path.join(dir, "work");
    fs.mkdirSync(path.join(work, "tools"), { recursive: true });
    const git = (cwd, ...args) => execFileSync("git", args, { cwd, env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
    git(work, "init", "-q", "-b", "main");
    fs.writeFileSync(path.join(work, "tools", "quick.mjs"), "// this repository's shape\n");
    fs.writeFileSync(path.join(work, "a.md"), "a\n");
    git(work, "add", ".");
    git(work, "commit", "-q", "-m", "base");
    /** Writes the record of a pass of `tree` in the checkout `at`, as tools/quick.mjs does. */
    const pass = (at, tree) => fs.writeFileSync(path.resolve(at, git(at, "rev-parse", "--git-path", PASSED)), `${tree}\n`);
    /** The guard's answer to `command` run from `cwd`: its status and its words. */
    const guard = (command, cwd = work) => {
      const r = spawnSync(process.execPath, [GUARD], {
        input: JSON.stringify({ hook_event_name: "PreToolUse", tool_name: "Bash", tool_input: { command }, cwd }),
        env,
        encoding: "utf8",
      });
      return { status: r.status, words: r.stderr.trim() };
    };
    body({ dir, work, git, pass, guard, tree: (at, ref = "HEAD") => git(at, "rev-parse", `${ref}^{tree}`) });
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const refused = (answer, words) => {
  assert.equal(answer.status, 2, answer.words);
  assert.match(answer.words, /^the push guard refused /);
  assert.match(answer.words, words);
};
const allowed = (answer) => assert.deepEqual(answer, { status: 0, words: "" });

test("a push runs only when the tree it sends is the tree quick last passed in its checkout", () => {
  withCheckout(({ work, git, pass, guard, tree }) => {
    // No record: nothing has passed.
    refused(guard("git push origin main"), new RegExp(`no tree has passed the quick checks.*run ${QUICK}`));
    // A pass of HEAD's tree: the push runs, by any form that names the checkout.
    pass(work, tree(work));
    allowed(guard("git push origin main"));
    allowed(guard(`git -C ${JSON.stringify(work)} push`, os.tmpdir()));
    allowed(guard(`cd ${JSON.stringify(work)} && git push`, os.tmpdir()));
    // A file changed but not committed after the pass: HEAD's tree is still the one that passed.
    fs.writeFileSync(path.join(work, "a.md"), "a, changed after\n");
    allowed(guard("git push"));
    // A pass of a working tree with a file not yet committed: HEAD's tree is not the one that
    // passed, the slip of #174.
    fs.writeFileSync(path.join(work, "b.md"), "b\n");
    git(work, "add", "b.md");
    const read = git(work, "write-tree");
    pass(work, read);
    refused(guard("git push"), /would push HEAD, whose tree .* is not the tree the quick checks last passed/);
    // Committed, the tree that passed is HEAD's.
    git(work, "commit", "-q", "-m", "b");
    assert.equal(tree(work), read);
    allowed(guard("git push"));
    // A commit after the pass makes the record stale.
    fs.writeFileSync(path.join(work, "c.md"), "c\n");
    git(work, "add", "c.md");
    git(work, "commit", "-q", "-m", "c");
    refused(guard("git push"), /is not the tree the quick checks last passed/);
    // A refspec is held as HEAD is: each source's tree, a deletion none.
    pass(work, tree(work));
    allowed(guard("git push origin HEAD:main"));
    refused(guard("git push origin HEAD~1:old"), /would push HEAD~1/);
    allowed(guard("git push origin --delete old"));
    allowed(guard("git push origin :old"));
    refused(guard("git push --all origin"), /`--all` pushes refs beyond those it names/);
  });
});

test("a merge in progress, a record the guard cannot read, git's failure and a directory it cannot tell each refuse", () => {
  withCheckout(({ work, git, pass, guard, tree }) => {
    pass(work, tree(work));
    git(work, "checkout", "-q", "-b", "side");
    fs.writeFileSync(path.join(work, "a.md"), "side\n");
    git(work, "commit", "-q", "-am", "side");
    git(work, "checkout", "-q", "main");
    fs.writeFileSync(path.join(work, "a.md"), "main\n");
    git(work, "commit", "-q", "-am", "main");
    pass(work, tree(work));
    allowed(guard("git push"));
    assert.throws(() => git(work, "merge", "side"), "the merge stops on its conflict");
    assert.ok(fs.existsSync(path.join(work, ".git", "MERGE_HEAD")), "the merge stopped on its conflict");
    refused(guard("git push"), /a merge is in progress/);
    git(work, "merge", "--abort");
    allowed(guard("git push"));
    fs.writeFileSync(path.resolve(work, git(work, "rev-parse", "--git-path", PASSED)), "not a tree\n");
    refused(guard("git push"), /holds no tree id/);
    pass(work, tree(work));
    refused(guard("git push", path.join(work, "..", "no-such-directory")), /git failed/);
    refused(guard("cd $D && git push"), /cannot tell which directory it runs in/);
    refused(guard("git --git-dir=.git push"), /--git-dir or --work-tree/);
    const unreadable = spawnSync(process.execPath, [GUARD], { input: "{", encoding: "utf8" });
    assert.equal(unreadable.status, 2);
    assert.match(unreadable.stderr, /the push guard could not read its event/);
  });
});

test("each worktree is held to its own record, by the directory a push runs in", () => {
  withCheckout(({ dir, work, git, pass, guard, tree }) => {
    const other = path.join(dir, "other");
    git(work, "worktree", "add", "-q", other, "-b", "other");
    pass(work, tree(work));
    allowed(guard("git push"));
    refused(guard("git push", other), /no tree has passed the quick checks/);
    refused(guard(`git -C ${JSON.stringify(other)} push`), /no tree has passed the quick checks/);
    fs.writeFileSync(path.join(other, "o.md"), "o\n");
    git(other, "add", "o.md");
    git(other, "commit", "-q", "-m", "o");
    pass(other, tree(work));
    refused(guard("git push", other), /is not the tree the quick checks last passed/);
    pass(other, tree(other));
    allowed(guard("git push", other));
  });
});

test("a push from a repository of another shape, and any command with no push, run untouched", () => {
  withCheckout(({ dir, git, guard }) => {
    const elsewhere = path.join(dir, "elsewhere");
    fs.mkdirSync(elsewhere);
    git(elsewhere, "init", "-q");
    allowed(guard("git push", elsewhere));
    allowed(guard("git status"));
    allowed(guard("echo push"));
  });
});
