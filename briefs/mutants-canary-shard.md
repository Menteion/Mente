---
id: mutants-canary-shard
status: deferred
trigger: when clean-tree-around-every-test-run's round has merged, which changes mutants.yml, the run tests and the gate commands before it
type: chore
affectedFiles: [.github/workflows/mutants.yml, .github/instrument-paths.txt, .github/scripts/mutants-floor.mjs, .github/scripts/lib/mutants-floor.mjs, .github/scripts/policy/mutation.test.mjs, .github/scripts/policy-run.test.mjs, .github/gate-commands.txt, .github/scripts/policy/pins.mjs, .github/scripts/policy/modules.json, briefs/mutants-canary-shard.md, briefs/archive/mutants-canary-shard.md]
protectedFiles: [spec/**, CLAUDE.md, .cargo/mutants.toml]
---

# Run a canary shard whenever a pull request changes how the shards run

## Intent

A pull request that changes only the Mutants workflow, or the scripts its shards run, holds no Rust mutant, so its plan deals no shard and its shards' steps never run before it merges. That let two breaks of every shard reach `main` through pull request 202:
- the review of 560ccd9 found every shard failing on a listing that matched, since a line feed alone counted as a difference;
- the escape `plan-downloaded-into-the-checkout`: the shard downloaded the plan into its checkout, and the plan's `summary.md` failed every shard's tests; 202's own run at its last head dealt no shard.

The run tests run the shards' steps with stand-ins for cargo, so they never run the baseline or a mutant. Only a real shard does.

When this round is done, the plan deals a canary shard whenever the Mutants watch matches the diff and the diff's own mutants deal no shard. **The canary's list and the watch's are one list, read from one place:** the section `[mutants.yml]` of `.github/instrument-paths.txt`, which names what a shard runs (the workflow, the floor, the sandbox's scripts and `test-sandbox.rs`, the limits and `memory-guard.sh`, the tools' record, `build-time-code.mjs` and its library, nextest's settings, `rust-toolchain.toml`, `.cargo/mutants.toml` since pull request 219) and the compiler's sources. So a change that runs the plan always runs a real shard: one to the shards' machinery, and one to the compiler that holds no mutant, as #217's tests did (the review of 62f8265). The canary is a shard like any other, dealt a small fixed set of real mutants of a function the plan names, so that its steps run end to end: the plan's download, cargo-mutants' install by its digest, the listing, the baseline of the whole workspace, the mutants, and the checks of their outcomes. The aggregate requires it as it requires every shard.

## Negative Scope

- No change to how the plan deals the diff's own mutants, or to the floor.
- No change to `.cargo/mutants.toml`'s exclusions.

## Invariants

- [ ] A diff the watch matches whose own mutants deal no shard deals the canary; one whose mutants deal shards, and one the watch does not match, deal none beside the diff's own; each planted.
- [ ] The canary runs the same steps as every shard, and the aggregate fails when it fails.
- [ ] The plan reads the canary's list from the watch's section of `.github/instrument-paths.txt`, written nowhere else, and the workflow policy refuses a file the shards run that the section leaves out.
- [ ] The canary's mutants are caught by the workspace's tests, so a canary that passes has run them.
- [ ] The `.github` changes go to Norman.
