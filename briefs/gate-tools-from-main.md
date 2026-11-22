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
- a fetch that moves refs;
- a refresh killed or refused partway, by the job's time limit, by Ctrl-C, by a reset of the machine, or by Windows refusing to replace a file in use;
- a refresh that hangs while it holds the lock.

Every path a slip can reach is closed. So is every deliberate path that costs only a flag or a check:
- a local ref moved by `update-ref`;
- a URL rewritten by `insteadOf`, or any other setting of the system or user git configuration;
- a replace ref;
- a `GIT_` variable, or a variable that picks the git binary, preloads node code or names certificate files;
- an argument handed to a refresher;
- a refresher run by hand, or imported by a script.

A process with full shell access can still rewrite anything local, and no local check stops it. CI holds that line, since it builds and tests from its own checkout. What is left is listed under **Limits** below.

**The set.** The copy holds exactly the load closure of the two gates and the refresher, at their paths in the repository, so that relative imports and loads resolve inside the copy:
- cargo's runner: `tools/capped.mjs` → `tools/local-limits.mjs` → `tools/store-apps.mjs`; `local-limits.mjs` starts `tools/capped.ps1` by its path, which loads `.github/scripts/lib/job-object.cs`;
- the hook: `tools/git-hooks/pre-push` → `tools/git-hooks/pre-push.mjs` → `tools/push.mjs` → `tools/local-limits.mjs` and `tools/raw-build.mjs`;
- the refresher, `tools/gates.mjs`, its declaration, `tools/gate-set.json`, and the entries' sources, `tools/gate-entries/`.

`tools/gate-set.json` declares, once, three things:
- the set;
- the remote's URL;
- the version of the interface between the stable entries and a copy.

A test reads every module of the set, its imports and the files it loads by path. It fails when one lies outside the set, and it lists the refresher's own closure.

**How the refresher's processes are run.** Every process the refresher starts, and every git command of the entries' checks, runs with an environment of their own making:
- every inherited `GIT_` and `NODE_` variable removed (`GIT_DIR`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_CONFIG_*` and `NODE_OPTIONS` among them), and so are `SSL_CERT_FILE`, `SSL_CERT_DIR` and `CURL_CA_BUNDLE`;
- `GIT_CONFIG_NOSYSTEM=1` and `GIT_CONFIG_GLOBAL=/dev/null`, which Git for Windows accepts when node starts it. So no system or user configuration reaches git, wherever `HOME` or `XDG_CONFIG_HOME` points, and no file is left for a command to fill.

The binaries are not found on `PATH`:
- git runs by its absolute path, from a constant list of the system's locations (`C:/Program Files/Git/cmd/git.exe` on Windows, `/usr/bin/git` on Linux). When none is found, the refresher refuses and says where it looked.
- node runs by `process.execPath`.

Every git command runs with `--no-replace-objects`, so that `git replace` changes nothing it reads. Further:
- `git ls-remote` runs in an empty directory the refresher creates for it, by an exclusive create, outside the common git directory. The refresher sets `GIT_CEILING_DIRECTORIES` to that directory's parent itself. So git finds no repository, and no repository's configuration reaches it, wherever the refresher was started from.
- Every other command names its repository by `--git-dir`: the common git directory that holds `mente-gates/`, found from the running file's own path, never from the current directory or the caller.
- `git hash-object` runs with `--no-filters`, so no attribute or filter changes what it hashes.
- On Windows, `ls-remote` runs with `-c http.sslBackend=schannel`, a constant, so the system's certificate store decides what https trusts. A local proof on Windows shows `ls-remote` over https working with the system and user configuration closed.

**Which commit is main.** The remote says, as #203's pre-push hook asks it. The URL is the one the running copy's `gate-set.json` declares, so a refresher takes it from its own verified copy, never from the working tree, from `git remote get-url`, or from its caller. A refresh:
- asks only when the local `refs/remotes/origin/main` names a commit other than `current`; otherwise it uses no network, and an offline start does not wait;
- refuses a URL that is not `https://`;
- runs `git ls-remote --get-url -- <url>` and goes on only if it gives back `<url>` itself. This one local process holds against an `insteadOf` that the closed configuration should already exclude;
- asks `git ls-remote -- <url> refs/heads/main`, within a time, with no prompt for credentials, its whole process tree killed when the time passes, as #203's hook does;
- installs that commit's set only when the local `refs/remotes/origin/main` names that same commit.

When the remote names a commit that is not in the repository, the refresh says "fetch first". When the remote cannot be asked, does not answer in time, or names another commit, it keeps the current copy and says why. Such an answer is remembered for that local commit for ten minutes, so an offline machine waits out the time once, not at every start. What is remembered can only keep the current copy, never install one.

No tool fetches. Nothing is installed from a commit the remote has not confirmed.

**Where it lives.** Under the common git directory, in `mente-gates/`:
- `mente-gates/<commit>/`: one copy, named by the confirmed commit, written from git's objects of that commit (`git ls-tree` and `git cat-file`), never from a working tree;
- `mente-gates/<commit>/manifest`: each file's path and blob id, as that commit's tree gives them;
- `mente-gates/current`: the commit in use, switched atomically, a new file renamed over the old;
- `mente-gates/staging-<commit>-<unique>/`: a set being installed, named by the commit it holds, which no entry runs;
- `mente-gates/entry/`: the stable entries `capped.mjs` and `hooks/pre-push`, and the refresher's entry `refresh.mjs`, written from the installed commit's `tools/gate-entries/`;
- `mente-gates/lock`: the lock every refresher takes before it writes.

**What each entry checks.** Before importing or running anything:
- the hook's and the runner's entries check that their own interface version equals the version the copy's `gate-set.json` declares. So an entry left behind by a refused rename reports itself as stale and names the refresh.
- The refresh entry is exempt from that check. Its interface with a copy is fixed: the manifest's format, and running the copy's `tools/gates.mjs` with no argument. A change to that interface needs a new bootstrap.

Every entry also checks these:
- `current` names a directory whose name is that commit;
- that commit is the local `refs/remotes/origin/main` or one of its ancestors (`git merge-base --is-ancestor`, local and so offline), so a copy of a branch's commit is refused;
- the manifest agrees with `git ls-tree <commit> -- <set>`;
- every file of the copy matches the manifest by blob id (`git hash-object --no-filters --stdin-paths`, one process).

A refusal names the one command that clears it:
- a stale entry, or a copy behind the local `origin/main`, names the refresh;
- a bad current copy names the bootstrap, since the refresh entry refuses it too. A bad copy is a drifted file, from antivirus or a stray edit, a manifest that disagrees with the commit, or a commit that is not `origin/main` or its ancestor.

The runner line of `~/.cargo/config.toml` names `mente-gates/entry/capped.mjs`, and `core.hooksPath` names `mente-gates/entry/hooks`. Both paths are stable, so Norman writes his two lines once. The hook's sh entry removes `NODE_OPTIONS` before it starts node.

**Who writes it.** Only the refresher of the commit being installed, never the branch's own `tools/gates.mjs`.
- The refresher starts only from a copy under `mente-gates/`, and refuses when run from a working tree. It takes no argument and refuses any.
- quick.mjs, cargo.mjs and mn.mjs, when they start, run `mente-gates/entry/refresh.mjs`, by `process.execPath` and with the environment cleaned as above. This happens before `requireRunner` and `requireHooks`, since the runner line and the hooks name entries a first refresh writes. The entry checks the current copy as every entry does, then runs that copy's refresher.
- When the remote confirms a commit other than `current`, the refresher takes the lock and readies that commit's set:
  - an existing `<commit>/` that matches `git ls-tree` of the commit, manifest included, stands as the new set;
  - otherwise, it moves any `<commit>/` aside and writes the set, read from the commit's objects, into a staging directory.
  
  It then starts the new set's refresher and writes nothing else.
- The new set's refresher learns its commit from its own directory's name. It confirms the commit with the remote itself, and goes on only when the remote names that same commit, which also covers main moving between the two asks.
- It writes only when two things hold: the blob of every file of its own closure equals that file of the commit, and its whole directory matches `git ls-tree` of the commit. It then writes the manifest into its directory, renames a staging directory to the commit's name, replaces the entries, and only then switches `current`.

So, of the copy a refresh replaces, only the refresher and its closure run, and they do only two things: confirm the commit, and start the new refresher. A broken installed refresher is recovered by running the bootstrap again.

**The seam, for the tests alone.** The fixtures' remotes are local paths. So the refresher's exported functions take a URL policy and a repository, and only these use them:
- A module whose file lies under a `mente-gates/` directory, or outside any repository, honours no policy, URL or repository argument at all: it refuses them.
- A working-tree module honours them only against a repository whose common git directory differs from its own, which is where the fixtures live. Every worktree of the same repository, the main checkout included, counts as its own.

The command admits only https however it is started, and a test holds that no argument or variable changes that.

**The bootstrap.** With nothing installed, or with a bad current copy, no copy holds a trusted URL, so the install is Norman's one command.
- The command is written in CLAUDE.md's Verification section, Norman's file, with the URL in it literally. The tools print the same line. A test holds the printed line equal to CLAUDE.md's, and its URL equal to the URL in `gate-set.json`.
- The command runs only `git ls-remote` at that URL. It writes the set of the commit the remote names, read from that commit's objects with `--no-replace-objects`, into a staging directory, and starts the staged refresher with no argument.
- That refresher takes the lock and confirms the commit itself, at the URL its own `gate-set.json` declares, which must be https and which the test on main holds equal to CLAUDE.md's. It then installs as above, moving a bad `<commit>/` aside.

**How it is replaced.**
- **The lock.** It is an open handle that the operating system releases when its holder ends, however it ends, so a kill, Ctrl-C or a reset leaves it free.
  - On Windows it is a lock file opened with no sharing, as local-limits' slots are (`UV_FS_O_EXLOCK`). On Linux, where the gates' tests run in CI, it is a socket in the abstract namespace named from the common git directory. Both are defined once, in `local-limits.mjs`.
  - The lock spans both refreshers. The old refresher takes it before it readies the new set, and hands its open handle to the new set's refresher, so it stays held, with no gap, until both have ended.
  - The bootstrap's staged refresher inherits no handle, so it takes the lock itself.
- **The time limit.** A refresh runs under a time limit with a tree kill, as `ls-remote` does.
  - `tools/gates.mjs` does its work in a child process of its own, started under the tree runner with fixed code and no argument, and that child holds the lock.
  - The new set's refresher is a descendant of that child, so the time covers both refreshers.
  - When the time passes, the tree dies, and the operating system releases the lock.
- **Waiting.** A start of quick, cargo or mn that finds the lock held waits at most a short bound. It then runs the current verified copy and says which process has held the lock, and since when. The bootstrap waits at most the refresh's time limit, by which any holder has been killed.
- **The order.** A staged set is renamed whole to its commit's name. Then the entries are replaced, each by a new file renamed over it, from the new set's `gate-entries/`. `current` is switched last, and only once every entry has been replaced.
  - A shell already reading the old hook keeps its open file.
  - Where Windows refuses to rename over a file in use, the refresh stops before switching `current` and says it will try again. The next start of quick, cargo or mn retries through the refresh entry, which the version check exempts. Until then, an entry already replaced reports itself as stale against the old copy.
- **A kill at any step** leaves a state the next refresh, or the bootstrap, completes:
  - an abandoned staging directory is swept;
  - a renamed `<commit>/` is verified and reused, or moved aside;
  - entries replaced before the kill are replaced again;
  - `current` still names a whole, verified copy.
- No file of a copy is ever replaced in place, so a copy in use, a capped test binary's runner on Windows among them, is never touched.
- After a switch, the refresher removes every copy, staging directory and copy moved aside but `current` and the copy before it, and skips any it cannot remove because a file of it is in use.

The stable entries never install. A raw `git push`, and a test binary cargo starts, run the verified current copy even when main has moved since; they say so and name the refresh. So a fetch that moves a gate file locks no one out.

**The code of the copy finds the checkout from where it runs, not from its own file.** `hooksDir`, `requireHooks`, `mainCheckout` and the like take the checkout from the current directory or `GIT_DIR`, never from `path.resolve(HERE, "..")`, which inside the copy would name the copy. The refresher and the entries alone name the repository by the common git directory that holds them, as said above.

**Offline.** No tool fetches. While the local `origin/main` names `current`, nothing asks the remote. Otherwise, when the remote cannot be asked, the tools run the current verified copy and say since when: the date of `refs/remotes/origin/main`'s last reflog entry.

**What stays the branch's, by design.** These stay in the working tree:
- the entry points quick.mjs, cargo.mjs and mn.mjs;
- `tools/quick-checks.json`;
- the settings' rule, `$CLAUDE_PROJECT_DIR/tools/push-guard.mjs`.

They are the branch's own tools, which its own changes test. What judges every branch alike, the gates and their refresher, comes from main.

**Limits.** These the local gates do not hold; CI does:
- **Full shell access.** A process with it can rewrite together everything the entries trust:
  - any file of the common git directory: its `config`, `objects/info/alternates`, `info/grafts`, `shallow` or the commit-graph files, any of which can change what `ls-tree` or `merge-base --is-ancestor` answer;
  - the copy, its manifest, and a commit object written to match, with `current` and the local `refs/remotes/origin/main` at once, by a script of its own, whether or not it borrows the refresher's code copied into a repository of its own;
  - the entries;
  - the runner line of `~/.cargo/config.toml`, or `core.hooksPath`.
- **Rollback.** Writing `current` alone to name the copy kept before it passes every entry, since that commit is an ancestor of `origin/main`. The next refresh restores the newest confirmed commit.
- **Lag.** The stable entries never refresh, and quick, cargo and mn install only what the local `origin/main` names. A merge to main after the last fetch holds the gates back until the next fetch, and the next start of one of those tools.
- **Proxies.** Proxy variables such as `HTTPS_PROXY` stay. A proxy cannot change what https verifies; at most it keeps the current copy.
- **node.** The runner line's node, and the node the hook's sh entry starts, are found on `PATH`. Cargo starts the runner line's node with whatever `NODE_OPTIONS` holds, which can preload code into it before the entry runs.
- **The bootstrap** runs in Norman's own terminal, with that terminal's environment.
- **https.** The remote is trusted through https and its certificate, as the system's certificate store (on Windows) or the system's default certificate files (on Linux) decide.

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
- The bootstrap is Norman's to run, from CLAUDE.md's line, at the first install and after a bad current copy; the tools print it.
- `.claude/settings.json` is Norman's: this round does not change it.

## Invariants

- [ ] With the main checkout switched to a branch from before #203, quick, cargo, mn, a raw `git push` and a test binary cargo starts all run main's gates, or refuse with the one command that restores them.
- [ ] An uncommitted edit, in any checkout, the main one included, to a file of the copy's set, to `tools/gates.mjs` or to `tools/gate-entries/`, changes nothing the gates run or install; each kind is planted. The entry points named above stay the branch's, by design.
- [ ] Hand runs of a refresher install nothing, each planted:
  - a `gates.mjs` run from a working tree refuses to start;
  - an edited `gates.mjs` placed in a staging directory under `mente-gates/` confirms the commit and writes nothing, since its blob differs from that commit's;
  - the installed `gates.mjs`, imported by a script and called with a local-path policy and URL, writes nothing;
  - a working-tree `gates.mjs`, called through the seam against its own repository, writes nothing;
  - one worktree's `gates.mjs`, called through the seam against the main checkout, writes nothing, since they share a common git directory;
  - a `gates.mjs` copied outside any repository honours no seam argument.
- [ ] A refresher started with an argument (a commit, a URL or a path) refuses it, planted. One started with `GIT_DIR`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES` or `GIT_CONFIG_*` naming another repository or configuration reads only the repository that holds its copy, planted.
- [ ] `refs/remotes/origin/main` moved locally by `update-ref` to a commit whose `capped.mjs` differs installs nothing, planted. So do these, each planted:
  - an `insteadOf` that rewrites the remote's URL, in the user's configuration and in the repository's, with the refresher started from a checkout and from `mente-gates/`;
  - a URL that is not https;
  - a remote that cannot be reached;
  - a remote that does not answer in time.
- [ ] A local `origin/main` moved to a commit whose `gate-set.json` names a local-path remote installs nothing, at the bootstrap and at a refresh, planted.
- [ ] `git replace` of the confirmed commit by another commit, whose `capped.mjs` differs, changes nothing: the refresh installs the true commit's set, and every entry checks against the true commit, planted.
- [ ] A copy whose commit is not the local `origin/main` or an ancestor of it, written by hand from a branch's commit, is refused by every entry, planted.
- [ ] Drift of the copy, one byte of one file, is refused on each path: quick's refresh, the hook's entry and the runner's entry. The refusal names the bootstrap, and running it clears the refusal. A manifest edited to match the drifted file is refused too, by `git ls-tree` of the commit. Each is planted.
- [ ] A module of the set that imports or loads a file outside the set fails the closure test, planted.
- [ ] A refresher started with `PATH` naming another git first, with `NODE_OPTIONS` set, or with `SSL_CERT_FILE`, `SSL_CERT_DIR` or `CURL_CA_BUNDLE` set, does three things, planted: it runs the system's git, it preloads nothing into the processes it starts, and it trusts only the system's certificates. The hook's entry started with `NODE_OPTIONS` set preloads nothing, planted.
- [ ] Two refreshes at once leave one valid copy, planted.
- [ ] A refresher that hangs while it holds the lock is killed with its tree at its time limit, and the lock is then free. Meanwhile a start of quick, cargo or mn runs the current verified copy after the short bound and says so. Each is planted.
- [ ] A refresh killed at each step leaves the lock free and a state the next refresh completes, each planted. The steps are: while it readies the new set; after the new set's refresher has started; after the rename; after each entry; after `current` switches, before the sweep.
- [ ] A refresh while a file of the old copy is in use succeeds and leaves that file alone. Windows alone refuses to replace a file in use, so this is a local proof that the quick map runs, skipped on Linux. So is the proof that an entry whose rename Windows refuses leaves `current` unswitched, and that the next start's refresh completes it.
- [ ] Of the copy a refresh replaces, only the refresher and its closure run: a module committed in the old set outside that closure, which writes a file when imported, writes none during a refresh to a newer commit, planted.
- [ ] An installed refresher with a committed bug that keeps it from confirming leaves the current copy in place, and the bootstrap recovers it, planted.
- [ ] An entry of an older interface version reports itself as stale against a newer copy and names the refresh. The refresh entry, exempt from that check, still runs, planted.
- [ ] The bootstrap line the tools print equals CLAUDE.md's, and its URL equals the URL in `gate-set.json`. The command admits only https, and no argument or variable changes that.
- [ ] Running the command a refusal names clears the refusal.
- [ ] Offline, with the remote's URL unreachable, these hold, each planted:
  - with the local `origin/main` naming `current`, a start asks no remote;
  - with it naming another commit, the first start waits out the time, and the next ones within ten minutes do not;
  - all run the last verified copy and print since when, from the reflog of `refs/remotes/origin/main`.
- [ ] New tests go into `tools/quick.test.mjs` and `tools/push.test.mjs`, which CI's Documentation job runs. The quick map covers every new file, and each new gate file is in `GATE_FILES` and CODEOWNERS.
- [ ] CLAUDE.md's line that the main checkout stays on main is removed, and its Verification section updated with the bootstrap line and the limits, with Norman's approval.
- [ ] `node tools/quick.mjs` passes before the push.
