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

When this round is done, the gates run from a verified copy of `origin/main`'s files, never from a working tree. The design, settled here before any code:

**The set.** The copy holds exactly the load closure of the two gates, at their paths in the repository, so that relative imports and loads resolve inside the copy:
- cargo's runner: `tools/capped.mjs` → `tools/local-limits.mjs` → `tools/store-apps.mjs`; `local-limits.mjs` starts `tools/capped.ps1` by its path, which loads `.github/scripts/lib/job-object.cs`;
- the hook: `tools/git-hooks/pre-push` → `tools/git-hooks/pre-push.mjs` → `tools/push.mjs` → `tools/local-limits.mjs` and `tools/raw-build.mjs`.

The set is declared once, in `tools/gate-set.json`. A test reads every module of the set, its imports and the files it loads by path, and fails when one lies outside the set.

**Where it lives.** Under the common git directory (`git rev-parse --git-common-dir`), in `mente-gates/`:
- `mente-gates/<id>/`: one copy, `<id>` naming the set's content as `origin/main` holds it, written from git's objects (`git ls-tree` and `git cat-file` of `origin/main`), never from a working tree;
- `mente-gates/<id>/manifest`: each file's path and blob id, as `origin/main`'s tree gives them;
- `mente-gates/current`: the id in use, switched atomically, a new file renamed over the old;
- `mente-gates/entry/`: the stable entries, `capped.mjs` and `hooks/pre-push`. They are small files, written from `origin/main`'s `tools/gate-entries/`. Each reads `current`, checks every file of that copy against its manifest by blob id (`git hash-object --stdin-paths`, one process), and only then imports or runs it. A mismatch refuses, naming the command that refreshes the copy.

The runner line of `~/.cargo/config.toml` names `mente-gates/entry/capped.mjs`, and `core.hooksPath` names `mente-gates/entry/hooks`. Both paths are stable, so Norman writes his two lines once.

**What refreshes it.** quick.mjs, cargo.mjs and mn.mjs, when they start, whenever `origin/main`'s set differs from the copy `current` names. The refresh:
- takes `mente-gates/lock` by an exclusive create, and waits while another refresh holds it;
- writes the new id's directory in a temporary name, renames it whole, then switches `current`;
- never replaces a file in place, so a copy in use, a capped test binary's runner on Windows among them, is never touched;
- leaves a copy no longer current for a later sweep to remove, skipping one still in use;
- never imports or runs any file of the copy it replaces, reading the new one only from git's objects.

The stable entries never refresh. A raw `git push`, and a test binary cargo starts, run the verified current copy even when `origin/main` has moved since. They say so and name the refresh, so that a fetch that moves a gate file locks no one out.

**The code of the copy finds the checkout from where it runs, not from its own file.** `hooksDir`, `requireHooks`, `mainCheckout` and the like take the checkout from the current directory or `GIT_DIR`, never from `path.resolve(HERE, "..")`, which inside the copy would name the copy.

**Offline.** No tool fetches. Every comparison reads the local `refs/remotes/origin/main`, which a `git fetch` the session runs moves. When the copy is older than that ref's last move, the tools say since when: the date of `refs/remotes/origin/main`'s last reflog entry.

**What stays the branch's, by design.** The entry points quick.mjs, cargo.mjs, mn.mjs and `tools/quick-checks.json`, and `$CLAUDE_PROJECT_DIR/tools/push-guard.mjs`, the settings' rule, stay in the working tree. They are the branch's own tools, which its own changes test. The copy holds the gates that judge every branch alike.

**CLAUDE.md.** The line that the main checkout stays on main, from #203's body and the manager's planned proposal, goes away. The Verification section says where the gates run from and the one refresh command.

## Negative Scope

- No change to what the gates check: the job object's limits, the record of the tree quick passed, the pre-push hook's rule, and the settings' rule against bypass stay as #203 left them.
- No change to CI, which builds and tests from its own checkout.
- No hook of another kind, and no new external dependency.

## Not Empowered

- CLAUDE.md is Norman's file: its change needs his approval in the pull request.
- The runner line of `~/.cargo/config.toml`, outside the repository, is Norman's to write; the tools print it, once, for the stable entry.
- `core.hooksPath` is Norman's to write in his own terminal, once, for the stable entry, since the settings' rule refuses a session's write.
- `.claude/settings.json` is Norman's: this round does not change it.

## Invariants

- [ ] With the main checkout switched to a branch from before #203, quick, cargo, mn, a raw `git push` and a test binary cargo starts all run `origin/main`'s gates, or refuse with the one command that restores them.
- [ ] An uncommitted edit to a file of the copy's set, in any checkout, the main one included, changes nothing the gates run. The entry points named above stay the branch's, by design.
- [ ] Drift of the copy, one byte of one file, is refused on each path: quick's refresh, the hook's entry and the runner's entry. Each is planted.
- [ ] A module of the set that imports or loads a file outside the set fails the closure test, planted.
- [ ] Two refreshes at once leave one valid copy. A refresh while a file of the old copy is in use succeeds, and leaves that file alone. Both are planted on Windows.
- [ ] A refresh never runs code of the copy it replaces: a marker module planted in the old copy, which writes a file when imported, writes none.
- [ ] Running the command a refusal names clears the refusal.
- [ ] Offline, with the remote's URL unreachable, the tools run the last verified copy and print since when, from the reflog of `refs/remotes/origin/main`.
- [ ] New tests go into `tools/quick.test.mjs` and `tools/push.test.mjs`, which CI's Documentation job runs. The quick map covers every new file, and each new gate file is in `GATE_FILES` and CODEOWNERS.
- [ ] CLAUDE.md's line that the main checkout stays on main is removed, and its Verification section updated, with Norman's approval.
- [ ] `node tools/quick.mjs` passes before the push.
