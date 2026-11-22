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

When this round is done, the gates run from a copy of the files of a commit the remote confirms is main, never from a working tree or from a ref any local command can move. The design, settled here before any code:

**What the local gates hold against.** Slips: accidental states, such as these:
- a main checkout switched off main or fallen behind it;
- an uncommitted edit;
- a buggy refresher;
- an environment a process inherits, since git runs hooks with `GIT_DIR` set;
- a fetch that moves refs.

Every path a slip can reach is closed. So is every deliberate path that costs only a flag or a check:
- a local ref moved by `update-ref`;
- a URL rewritten by `insteadOf`;
- a replace ref;
- a `GIT_` variable;
- an argument handed to a refresher;
- a refresher run by hand.

A process with full shell access can still rewrite anything local, and no local check stops it. CI holds that line, since it builds and tests from its own checkout. What is left is listed under **Limits** below.

**The set.** The copy holds exactly the load closure of the two gates and the refresher, at their paths in the repository, so that relative imports and loads resolve inside the copy:
- cargo's runner: `tools/capped.mjs` → `tools/local-limits.mjs` → `tools/store-apps.mjs`; `local-limits.mjs` starts `tools/capped.ps1` by its path, which loads `.github/scripts/lib/job-object.cs`;
- the hook: `tools/git-hooks/pre-push` → `tools/git-hooks/pre-push.mjs` → `tools/push.mjs` → `tools/local-limits.mjs` and `tools/raw-build.mjs`;
- the refresher, `tools/gates.mjs`, its declaration, `tools/gate-set.json`, and the entries' sources, `tools/gate-entries/`.

`tools/gate-set.json` declares, once, three things:
- the set;
- the remote's URL;
- the version of the interface between the entries and a copy.

A test reads every module of the set, its imports and the files it loads by path. It fails when one lies outside the set, and it lists the refresher's own closure.

**How git is run.** Every git command of the refresher and the entries runs:
- with every inherited `GIT_` variable removed (`GIT_DIR`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES` and `GIT_CONFIG_*` among them);
- naming its repository by `--git-dir`: the common git directory that holds `mente-gates/`, found from the running file's own path, never from the current directory or the caller;
- with `--no-replace-objects`, so that `git replace` changes nothing it reads.

**Which commit is main.** The remote says, as #203's pre-push hook asks it. The URL is the one the running copy's `gate-set.json` declares, so a refresher takes it from its own verified copy, never from the working tree, from `git remote get-url`, or from its caller. A refresh:
- asks only when the local `refs/remotes/origin/main` names a commit other than `current`; otherwise it uses no network, and an offline start does not wait;
- refuses a URL that is not `https://`;
- runs `git ls-remote --get-url -- <url>` and goes on only if it gives back `<url>` itself, which no `insteadOf` rewrote;
- asks `git ls-remote -- <url> refs/heads/main`, within a time, with no prompt for credentials, its whole process tree killed when the time passes, as #203's hook does;
- installs that commit's set only when the local `refs/remotes/origin/main` names that same commit.

When the remote names a commit that is not in the repository, the refresh says "fetch first". When the remote cannot be asked, does not answer in time, or names another commit, it keeps the current copy and says why. Such an answer is remembered for that local commit for ten minutes, so an offline machine waits out the time once, not at every start. What is remembered can only keep the current copy, never install one.

No tool fetches. Nothing is installed from a commit the remote has not confirmed.

**Where it lives.** Under the common git directory, in `mente-gates/`:
- `mente-gates/<commit>/`: one copy, named by the confirmed commit, written from git's objects of that commit (`git ls-tree` and `git cat-file`), never from a working tree;
- `mente-gates/<commit>/manifest`: each file's path and blob id, as that commit's tree gives them;
- `mente-gates/current`: the commit in use, switched atomically, a new file renamed over the old;
- `mente-gates/staging-<name>/`: a set being installed, which no entry runs;
- `mente-gates/entry/`: the stable entries `capped.mjs` and `hooks/pre-push`, and the refresher's entry `refresh.mjs`, written from the installed commit's `tools/gate-entries/`.

**What each entry checks.** Before importing or running anything:
- its own interface version equals the version the copy's `gate-set.json` declares, so an entry left behind by a refused rename reports itself as stale and names the refresh;
- `current` names a directory whose name is that commit;
- that commit is the local `refs/remotes/origin/main` or one of its ancestors (`git merge-base --is-ancestor`, local and so offline), so a copy of a branch's commit is refused;
- the manifest agrees with `git ls-tree <commit> -- <set>`;
- every file of the copy matches the manifest by blob id (`git hash-object --stdin-paths`, one process).

A mismatch refuses and names the command that refreshes the copy.

The runner line of `~/.cargo/config.toml` names `mente-gates/entry/capped.mjs`, and `core.hooksPath` names `mente-gates/entry/hooks`. Both paths are stable, so Norman writes his two lines once.

**Who writes it.** Only the refresher of the commit being installed, never the branch's own `tools/gates.mjs`.
- The refresher starts only from a copy under `mente-gates/`, and refuses when run from a working tree. It takes no argument and refuses any.
- quick.mjs, cargo.mjs and mn.mjs, when they start, run `mente-gates/entry/refresh.mjs`. This happens before `requireRunner` and `requireHooks`, since the runner line and the hooks name entries a first refresh writes. The entry checks the current copy as every entry does, then runs that copy's refresher.
- When the remote confirms a commit other than `current`, the refresher writes that commit's set, read from its objects, into a staging directory. It checks the staging directory against `git ls-tree` of that commit and starts the staged refresher. It writes nothing else.
- The staged refresher confirms the commit with the remote itself. It writes only when the blob of every file of its own closure equals that file of the commit it just confirmed. It then writes the manifest, renames the staging directory to the commit's name, switches `current`, and replaces the entries.

So, of the copy a refresh replaces, only the refresher and its closure run, and they do only two things: confirm the commit, and start the new refresher. A broken installed refresher is recovered by running the bootstrap again.

**The bootstrap.** With nothing installed, no copy holds a trusted URL, so the first install is Norman's one command.
- The command is written in CLAUDE.md's Verification section, Norman's file, with the URL in it literally. The tools print the same line. A test holds the printed line equal to CLAUDE.md's, and its URL equal to the URL in `gate-set.json`.
- The command runs only `git ls-remote` at that URL. It writes the set of the commit the remote names, read from that commit's objects with `--no-replace-objects`, into a staging directory, and starts the staged refresher with no argument.
- That refresher then confirms the commit itself, at the URL its own `gate-set.json` declares, which must be https and which the test on main holds equal to CLAUDE.md's.

**How it is replaced.** Under `mente-gates/lock`, taken by an exclusive create; a second refresh waits while another holds it.
- A staged set is renamed whole to its commit's name, and then `current` is switched.
- An entry is replaced by a new file renamed over it, only by the staged refresher, from its own commit's `gate-entries/`. A shell already reading the old hook keeps its open file. Where Windows refuses to rename over a file in use, the old entry stays, the refresh says it will try again, and the old entry reports itself as stale while its interface version differs from the copy's.
- No file of a copy is ever replaced in place, so a copy in use, a capped test binary's runner on Windows among them, is never touched.
- After a switch, the refresher removes every copy and staging directory but `current` and the copy before it, and skips any it cannot remove because a file of it is in use.

The stable entries never install. A raw `git push`, and a test binary cargo starts, run the verified current copy even when main has moved since; they say so and name the refresh. So a fetch that moves a gate file locks no one out.

**The tests' remotes.** The fixtures' remotes are local paths. The tests reach them through the refresher's exported functions, which take the URL policy as an argument. The command admits only https however it is started, and a test holds that no argument or variable changes that.

**The code of the copy finds the checkout from where it runs, not from its own file.** `hooksDir`, `requireHooks`, `mainCheckout` and the like take the checkout from the current directory or `GIT_DIR`, never from `path.resolve(HERE, "..")`, which inside the copy would name the copy. The refresher and the entries alone name the repository by the common git directory that holds them, as said above.

**Offline.** No tool fetches. While the local `origin/main` names `current`, nothing asks the remote. Otherwise, when the remote cannot be asked, the tools run the current verified copy and say since when: the date of `refs/remotes/origin/main`'s last reflog entry.

**What stays the branch's, by design.** These stay in the working tree:
- the entry points quick.mjs, cargo.mjs and mn.mjs;
- `tools/quick-checks.json`;
- the settings' rule, `$CLAUDE_PROJECT_DIR/tools/push-guard.mjs`.

They are the branch's own tools, which its own changes test. What judges every branch alike, the gates and their refresher, comes from main.

**Limits.** These the local gates do not hold; CI does:
- A process with full shell access can rewrite together everything the entries trust, among them:
  - the copy, its manifest, and a commit object written to match;
  - `current` and the local `refs/remotes/origin/main` at once;
  - the entries;
  - `objects/info/alternates` or `info/grafts`;
  - the user's git configuration, such as a proxy or `http.sslVerify`;
  - the runner line of `~/.cargo/config.toml`, or `core.hooksPath`.
- Norman's bootstrap runs in his own terminal, with that terminal's environment.
- A verified copy may lag main: until a fetch moves the local `origin/main`, the gates stay at the commit last installed.
- The remote is trusted through https and its certificate.

**CLAUDE.md.** The line that the main checkout stays on main, from #203's body and the manager's planned proposal, goes away. The Verification section says:
- where the gates run from;
- the refresh;
- these limits;
- the bootstrap line.

## Negative Scope

- No change to what the gates check: the job object's limits, the record of the tree quick passed, the pre-push hook's rule, and the settings' rule against bypass stay as #203 left them.
- No change to CI, which builds and tests from its own checkout.
- No hook of another kind, and no new external dependency.

## Not Empowered

- CLAUDE.md is Norman's file: its change, the bootstrap line among it, needs his approval in the pull request.
- The runner line of `~/.cargo/config.toml`, outside the repository, is Norman's to write, once, for the stable entry; the tools print it.
- `core.hooksPath` is Norman's to write in his own terminal, once, for the stable entry, since the settings' rule refuses a session's write.
- The bootstrap, the first install, is Norman's to run, once, from CLAUDE.md's line; the tools print it.
- `.claude/settings.json` is Norman's: this round does not change it.

## Invariants

- [ ] With the main checkout switched to a branch from before #203, quick, cargo, mn, a raw `git push` and a test binary cargo starts all run main's gates, or refuse with the one command that restores them.
- [ ] An uncommitted edit, in any checkout, the main one included, to a file of the copy's set, to `tools/gates.mjs` or to `tools/gate-entries/`, changes nothing the gates run or install; each kind is planted. The entry points named above stay the branch's, by design.
- [ ] A `gates.mjs` run from a working tree refuses to start, planted. An edited `gates.mjs` placed in a staging directory under `mente-gates/` confirms the commit and writes nothing, since its blob differs from that commit's, planted.
- [ ] A refresher started with an argument (a commit, a URL or a path) refuses it, planted. One started with `GIT_DIR`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES` or `GIT_CONFIG_*` naming another repository or configuration reads only the repository that holds its copy, planted.
- [ ] `refs/remotes/origin/main` moved locally by `update-ref` to a commit whose `capped.mjs` differs installs nothing, planted. So do these, each planted:
  - a remote's URL rewritten by `insteadOf`;
  - a URL that is not https;
  - a remote that cannot be reached;
  - a remote that does not answer in time.
- [ ] A local `origin/main` moved to a commit whose `gate-set.json` names a local-path remote installs nothing, at the bootstrap and at a refresh, planted.
- [ ] `git replace` of the confirmed commit by another commit, whose `capped.mjs` differs, changes nothing: the refresh installs the true commit's set, and every entry checks against the true commit, planted.
- [ ] A copy whose commit is not the local `origin/main` or an ancestor of it, written by hand from a branch's commit, is refused by every entry, planted.
- [ ] Drift of the copy, one byte of one file, is refused on each path: quick's refresh, the hook's entry and the runner's entry. A manifest edited to match the drifted file is refused too, by `git ls-tree` of the commit. Each is planted.
- [ ] A module of the set that imports or loads a file outside the set fails the closure test, planted.
- [ ] Two refreshes at once leave one valid copy, planted.
- [ ] A refresh while a file of the old copy is in use succeeds and leaves that file alone. Windows alone refuses to replace a file in use, so this is a local proof that the quick map runs, skipped on Linux.
- [ ] Of the copy a refresh replaces, only the refresher and its closure run: a module committed in the old set outside that closure, which writes a file when imported, writes none during a refresh to a newer commit, planted.
- [ ] An installed refresher with a committed bug that keeps it from confirming leaves the current copy in place, and the bootstrap recovers it, planted.
- [ ] An entry of an older interface version, left by a refused rename, reports itself as stale against a newer copy and names the refresh, planted.
- [ ] The bootstrap line the tools print equals CLAUDE.md's, and its URL equals the URL in `gate-set.json`. The command admits only https, and no argument or variable changes that.
- [ ] Running the command a refusal names clears the refusal.
- [ ] Offline, with the remote's URL unreachable, these hold, each planted:
  - with the local `origin/main` naming `current`, a start asks no remote;
  - with it naming another commit, the first start waits out the time, and the next ones within ten minutes do not;
  - all run the last verified copy and print since when, from the reflog of `refs/remotes/origin/main`.
- [ ] New tests go into `tools/quick.test.mjs` and `tools/push.test.mjs`, which CI's Documentation job runs. The quick map covers every new file, and each new gate file is in `GATE_FILES` and CODEOWNERS.
- [ ] CLAUDE.md's line that the main checkout stays on main is removed, and its Verification section updated with the bootstrap line and the limits, with Norman's approval.
- [ ] `node tools/quick.mjs` passes before the push.
