---
id: cargo-mutants-release
status: deferred
type: chore
affectedFiles: [.github/workflows/mutants.yml, .github/workflows/mutants-full.yml, .github/scripts/policy/build-and-toolchain.test.mjs, .github/scripts/policy/mutation.test.mjs, .github/scripts/policy/sandbox.test.mjs, .github/scripts/policy/pins.mjs, briefs/cargo-mutants-release.md, briefs/archive/cargo-mutants-release.md]
protectedFiles: [CLAUDE.md, spec/**, .cargo/mutants.toml]
---

# Move cargo-mutants back to a release once one carries the struct-field fix

## Intent

Every run of cargo-mutants, a pull request's plan and the full run's build job, installs upstream's commit `b00aff0c6ca0670bccdf4eb2ece9e15c4c35c8b9`, "Fix name filters for struct-field deletion mutants" ([sourcefrog/cargo-mutants#634](https://github.com/sourcefrog/cargo-mutants/pull/634), 2026-10-08), from git with its lock (`CARGO_MUTANTS_REV`).

27.1.0, the latest release when PR 202 took the commit (Norman, 2026-11-22), applies neither `--re` nor `--exclude-re`, nor `.cargo/mutants.toml`'s `exclude_re`, to its struct-field mutants. So a shard of "Mutants in the diff" listed every such mutant of the tree beside its share (PR 202's probe, run 45006783404).

Dependabot does not see a git install, so this brief holds the way back. When this round is done:
- the jobs install the first release that holds that fix, `cargo install cargo-mutants --locked --version "<release>"`;
- the policy holds that release where it holds the commit now;
- the release's changes since `b00aff0` have been read for anything that alters the outcome files, the lists, the logs or the mutants' names and order that the floor reads.

The round starts when a release's notes (https://mutants.rs/changelog.html) list "Apply `--re` and `--exclude-re` name filters to struct-field deletion mutants" outside "Unreleased".

## Negative Scope

- No release before the one that holds the fix: 27.1.0 and earlier list a shard's mutants wrong.
- No change to the plan, the shards' dealing or the floor beyond what the release's own changes call for.
- No change to `.cargo/mutants.toml`'s exclusions.

## Invariants

- [ ] The release's notes list the struct-field filter fix.
- [ ] `CARGO_MUTANTS_REV` is gone. Every install line installs the release with `--locked --version`, and the policy holds that version (`TOOL_ENV_ALLOWED`, and the install lines in `mutation.test.mjs`).
- [ ] Built from the release, one exact `--re` of a struct-field-free name lists that one mutant.
- [ ] Over the whole tree, the release lists the same names, in the same order, as `b00aff0`, or the pull request names each difference and what it changes for the floor.
- [ ] The release's changes since `b00aff0` are summarised in the pull request, each one that touches outcomes, lists, logs, names or order named.
- [ ] Any new source attribute of cargo-mutants that removes mutants from a run is refused in sources, as `mutants::skip` and `mutants::exclude_re` are.
