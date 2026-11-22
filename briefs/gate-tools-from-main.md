---
id: gate-tools-from-main
status: deferred
trigger: when #203 merges
type: chore
affectedFiles: [tools/local-limits.mjs, tools/local-limits.test.mjs, tools/capped.mjs, tools/cargo.mjs, tools/mn.mjs, tools/quick.mjs, tools/quick.test.mjs, tools/push.mjs, tools/push.test.mjs, tools/git-hooks/**, .github/scripts/policy/local-tools.test.mjs, CLAUDE.md, briefs/gate-tools-from-main.md, briefs/archive/gate-tools-from-main.md]
protectedFiles: [spec/**, compiler/**, .github/workflows/**]
---

# Run the local gate tools from a verified copy of main

## Intent

After #203, two local gates run from the main checkout's working tree: cargo's runner, the main checkout's `tools/capped.mjs` (`runnerLine`), and git's pre-push hook, whose `core.hooksPath` is the main checkout's `tools/git-hooks` (`hooksDir`). The main checkout is then a single point of failure:
- switched off main, every session's quick, cargo and mn are locked out, and pushes go unguarded;
- fallen behind main, they run old gates;
- with an uncommitted edit to a gate file, every session runs the edit.

CLAUDE.md can only ask that the main checkout stay on main, which is a written rule, the weakest guard.

When this round is done, the gate tools come from a verified copy of main, not from whatever a working tree holds. For example, a tool materializes the gate files of `tools/` from `origin/main`'s tree into a directory under the common git directory (`git rev-parse --git-common-dir`), and records the tree id it copied. Cargo's runner and `core.hooksPath` point there. quick.mjs, cargo.mjs and mn.mjs refuse, naming the one command to run, when that copy does not match `origin/main`'s tree for those paths.

The brief settles the design before any code:
- **What refreshes the copy.** Say which of these does it: an explicit command, every tool's start when `origin/main` has moved, or a `git fetch`. A refresh never runs code of the copy it replaces.
- **How a refresh is verified.** For example, the copy's files are compared with `git ls-tree` and `git cat-file` of `origin/main`'s tree, by object id, and the recorded tree id is checked on every run, not trusted.
- **What happens offline**, when `origin/main` cannot be fetched: the copy stays as verified against the last fetched `origin/main`, and the tools say how old that is.
- **How CLAUDE.md's line "the main checkout must stay on main" goes away**, and what replaces it in the Verification section.

## Negative Scope

- No change to what the gates check: the job object's limits, the record of the tree quick passed, the pre-push hook's rule, and the settings' rule against bypass stay as #203 left them.
- No change to CI, which builds and tests from its own checkout.
- No hook of another kind, and no new external dependency.

## Not Empowered

- CLAUDE.md is Norman's file: its change needs his approval in the pull request.
- `~/.cargo/config.toml`'s runner line is also Norman's to change, outside the repository; the tools print the line, and he runs it.
- `core.hooksPath` is written by Norman in his own terminal, since the settings' rule refuses a session's write.

## Invariants

- [ ] With the main checkout switched to a branch from before #203, quick, cargo, mn and the pre-push hook still run main's gates, or refuse with the one command that restores them; none of them runs unguarded.
- [ ] An uncommitted edit to a gate file in any checkout, the main one included, changes nothing the gates run.
- [ ] A copy that differs from `origin/main`'s tree for the gate paths, by one byte or one file, refuses, naming the command to refresh it; a planted difference is seen to fail.
- [ ] Offline, the tools run the last verified copy and say since when.
- [ ] `node tools/quick.mjs` passes before the push.
