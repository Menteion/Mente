---
id: gate-tools-from-main
status: deferred
trigger: when #203 merges; records-every-file, which shares files with this round, waits for this one
type: chore
affectedFiles: [tools/gates.mjs, tools/gate-set.json, tools/gate-entries/**, tools/local-limits.mjs, tools/capped.mjs, tools/cargo.mjs, tools/mn.mjs, tools/quick.mjs, tools/push.mjs, tools/git-hooks/**, tools/quick.test.mjs, tools/push.test.mjs, tools/local-limits.test.mjs, tools/quick-checks.json, .github/scripts/policy/local-tools.test.mjs, .github/scripts/policy/pins.mjs, .github/CODEOWNERS, CLAUDE.md, briefs/gate-tools-from-main.md, briefs/archive/gate-tools-from-main.md]
protectedFiles: [spec/**, compiler/**, .github/workflows/**, .claude/settings.json]
---

# Run the local gate tools from a verified copy of main

## Intent

After #203, two local gates run from the main checkout's working tree: cargo's runner, the main checkout's `tools/capped.mjs` (`runnerLine`), and git's pre-push hook, whose `core.hooksPath` is the main checkout's `tools/git-hooks` (`hooksDir`). The main checkout is then a single point of failure:
- switched off main, every session's quick, cargo and mn are locked out, and pushes go unguarded;
- fallen behind main, they run old gates;
- with an uncommitted edit to a gate file, every session runs the edit.

#203's body asks that the main checkout stay on main, and the manager plans to propose that line for CLAUDE.md: a written rule, the weakest guard.

When this round is done, the gates run from a copy of the files of a commit the remote confirms is main, never from a working tree or from a ref any local command can move. Nothing the branch holds writes what every checkout runs. The design, settled here before any code:

**The set.** The copy holds exactly the load closure of the two gates and the refresher, at their paths in the repository, so that relative imports and loads resolve inside the copy:
- cargo's runner: `tools/capped.mjs` → `tools/local-limits.mjs` → `tools/store-apps.mjs`; `local-limits.mjs` starts `tools/capped.ps1` by its path, which loads `.github/scripts/lib/job-object.cs`;
- the hook: `tools/git-hooks/pre-push` → `tools/git-hooks/pre-push.mjs` → `tools/push.mjs` → `tools/local-limits.mjs` and `tools/raw-build.mjs`;
- the refresher, `tools/gates.mjs`, and the entries' sources, `tools/gate-entries/`.

The set and the remote's URL are declared once, in `tools/gate-set.json`. A test reads every module of the set, its imports and the files it loads by path, and fails when one lies outside the set.

**Which commit is main.** The remote says, as #203's pre-push hook asks it, by the URL `gate-set.json` declares. The URL is read from the copy already installed, never from the working tree or from `git remote get-url`, which local commands can change. A refresh:
- runs `git ls-remote --get-url -- <url>`, under the same options and environment, and goes on only if it gives back `<url>` itself, which no `insteadOf` rewrote;
- asks `git ls-remote -- <url> refs/heads/main`, within a time, with no prompt for credentials, its whole process tree killed when the time passes, as #203's hook does;
- installs only that commit's set. When the commit is not in the repository, it says "fetch first". When the remote cannot be asked, does not answer in time, or names a commit other than the local `refs/remotes/origin/main`, it keeps the current copy and says why.

No tool fetches. Nothing is installed from a commit the remote has not confirmed.

**Where it lives.** Under the common git directory (`git rev-parse --git-common-dir`), in `mente-gates/`:
- `mente-gates/<commit>/`: one copy, named by the confirmed commit, written from git's objects of that commit (`git ls-tree` and `git cat-file`), never from a working tree;
- `mente-gates/<commit>/manifest`: each file's path and blob id, as that commit's tree gives them;
- `mente-gates/current`: the commit in use, switched atomically, a new file renamed over the old;
- `mente-gates/entry/`: the stable entries `capped.mjs` and `hooks/pre-push`, and the refresher's entry `refresh.mjs`. They are written from the confirmed commit's `tools/gate-entries/`.

Before importing or running anything, each entry checks three things:
- `current` names a directory whose name is that commit;
- the manifest agrees with `git ls-tree <commit> -- <set>`, which reads local objects and so works offline;
- every file of the copy matches the manifest by blob id (`git hash-object --stdin-paths`, one process).

A mismatch refuses and names the command that refreshes the copy.

The runner line of `~/.cargo/config.toml` names `mente-gates/entry/capped.mjs`, and `core.hooksPath` names `mente-gates/entry/hooks`. Both paths are stable, so Norman writes his two lines once.

**Who writes it.** Only the refresher of a confirmed commit, never the branch's own `tools/gates.mjs`.
- quick.mjs, cargo.mjs and mn.mjs, when they start, run `mente-gates/entry/refresh.mjs`. This happens before `requireRunner` and `requireHooks`, since the runner line and the hooks name entries a first refresh writes.
- `refresh.mjs` verifies its own copy as the other entries do. If `origin/main`'s confirmed commit differs from `current`, it reads the new commit's refresher from git's objects of that commit, never from a working tree, and that refresher installs the commit's set.
- The very first install, when `mente-gates/` does not exist yet, is Norman's one command, the bootstrap, printed by the tools beside his two lines. It reads the refresher from git's objects of the commit the remote confirms.

**How it is replaced.** Under `mente-gates/lock`, taken by an exclusive create; a second refresh waits while another holds it.
- A new copy is written in a temporary name, renamed whole to its commit's name, and then `current` is switched.
- An entry is replaced by a new file renamed over it, only from a confirmed commit's `gate-entries/`. A shell already reading the old hook keeps its open file. Where Windows refuses to rename over a file in use, the old entry stays and the refresh says it will try again; the entries are small and rarely change.
- No file of a copy is ever replaced in place, so a copy in use, a capped test binary's runner on Windows among them, is never touched.
- After a switch, the refresher removes every copy but `current` and the one before it, and skips any it cannot remove because a file of it is in use.
- The refresher never imports or runs any file of the copy it replaces.

The stable entries never install. A raw `git push`, and a test binary cargo starts, run the verified current copy even when main has moved since; they say so and name the refresh. So a fetch that moves a gate file locks no one out.

**The code of the copy finds the checkout from where it runs, not from its own file.** `hooksDir`, `requireHooks`, `mainCheckout` and the like take the checkout from the current directory or `GIT_DIR`, never from `path.resolve(HERE, "..")`, which inside the copy would name the copy.

**Offline.** No tool fetches. When the remote cannot be asked, the tools run the current verified copy and say since when: the date of `refs/remotes/origin/main`'s last reflog entry.

**What stays the branch's, by design.** The entry points quick.mjs, cargo.mjs, mn.mjs and `tools/quick-checks.json`, and `$CLAUDE_PROJECT_DIR/tools/push-guard.mjs`, the settings' rule, stay in the working tree. They are the branch's own tools, which its own changes test. What judges every branch alike, the gates and their refresher, comes from main.

**CLAUDE.md.** The line that the main checkout stays on main, from #203's body and the manager's planned proposal, goes away. The Verification section says where the gates run from, the refresh, and the bootstrap.

## Negative Scope

- No change to what the gates check: the job object's limits, the record of the tree quick passed, the pre-push hook's rule, and the settings' rule against bypass stay as #203 left them.
- No change to CI, which builds and tests from its own checkout.
- No hook of another kind, and no new external dependency.

## Not Empowered

- CLAUDE.md is Norman's file: its change needs his approval in the pull request.
- The runner line of `~/.cargo/config.toml`, outside the repository, is Norman's to write, once, for the stable entry; the tools print it.
- `core.hooksPath` is Norman's to write in his own terminal, once, for the stable entry, since the settings' rule refuses a session's write.
- The bootstrap, the first install, is Norman's to run, once; the tools print it.
- `.claude/settings.json` is Norman's: this round does not change it.

## Invariants

- [ ] With the main checkout switched to a branch from before #203, quick, cargo, mn, a raw `git push` and a test binary cargo starts all run main's gates, or refuse with the one command that restores them.
- [ ] An uncommitted edit, in any checkout, the main one included, to a file of the copy's set, to `tools/gates.mjs` or to `tools/gate-entries/`, changes nothing the gates run or install; each kind is planted. The entry points named above stay the branch's, by design.
- [ ] `refs/remotes/origin/main` moved locally by `update-ref` to a commit whose `capped.mjs` differs installs nothing, planted. So do a remote's URL rewritten by `insteadOf`, a remote that cannot be reached, and one that does not answer in time.
- [ ] Drift of the copy, one byte of one file, is refused on each path: quick's refresh, the hook's entry and the runner's entry. A manifest edited to match the drifted file is refused too, by `git ls-tree` of the commit. Each is planted.
- [ ] A module of the set that imports or loads a file outside the set fails the closure test, planted.
- [ ] Two refreshes at once leave one valid copy, planted.
- [ ] A refresh while a file of the old copy is in use succeeds and leaves that file alone. Windows alone refuses to replace a file in use, so this is a local proof that the quick map runs, skipped on Linux.
- [ ] A refresh never runs code of the copy it replaces: a marker module planted in the old copy, which writes a file when imported, writes none.
- [ ] Running the command a refusal names clears the refusal.
- [ ] Offline, with the remote's URL unreachable, the tools run the last verified copy and print since when, from the reflog of `refs/remotes/origin/main`.
- [ ] New tests go into `tools/quick.test.mjs` and `tools/push.test.mjs`, which CI's Documentation job runs. The quick map covers every new file, and each new gate file is in `GATE_FILES` and CODEOWNERS.
- [ ] CLAUDE.md's line that the main checkout stays on main is removed, and its Verification section updated, with Norman's approval.
- [ ] `node tools/quick.mjs` passes before the push.
