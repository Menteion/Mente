---
id: mutants-canary-shard
status: deferred
trigger: when plan-outside-checkout's pull request (219) has merged
type: chore
affectedFiles: [.github/workflows/mutants.yml, .github/scripts/mutants-floor.mjs, .github/scripts/lib/mutants-floor.mjs, .github/scripts/policy/mutation.test.mjs, .github/scripts/policy-run.test.mjs, .github/gate-commands.txt, .github/scripts/policy/pins.mjs, .github/scripts/policy/modules.json, briefs/mutants-canary-shard.md, briefs/archive/mutants-canary-shard.md]
protectedFiles: [spec/**, CLAUDE.md, .cargo/mutants.toml]
---

# Run a canary shard whenever a pull request changes how the shards run

## Intent

A pull request that changes only the Mutants workflow, or the scripts its shards run, holds no Rust mutant, so its plan deals no shard and its shards' steps never run before it merges. That let two breaks of every shard reach `main` through pull request 202:
- the review of 560ccd9 found every shard failing on a listing that matched, since a line feed alone counted as a difference;
- the escape `plan-downloaded-into-the-checkout`: the shard downloaded the plan into its checkout, and the plan's `summary.md` failed every shard's tests; 202's own run at its last head dealt no shard.

The run tests run the shards' steps with stand-ins for cargo, so they never run the baseline or a mutant. Only a real shard does.

When this round is done, the plan deals a canary shard whenever the diff touches what a shard runs: `.github/workflows/mutants.yml`, `.github/scripts/mutants-floor.mjs`, `.github/scripts/lib/mutants-floor.mjs`, `.github/scripts/test-sandbox.sh`, `.github/scripts/test-sandbox-env.sh`, `.github/scripts/limits.sh`, `.github/scripts/tools.sh`, `.github/scripts/install-nextest.sh`, `.config/nextest.toml` and `.cargo/mutants.toml`, one list the policy holds. The canary is a shard like any other, dealt a small fixed set of real mutants of a function the plan names, so that its steps run end to end: the plan's download, cargo-mutants' install by its digest, the listing, the baseline of the whole workspace, the mutants, and the checks of their outcomes. The aggregate requires it as it requires every shard.

## Negative Scope

- No change to how the plan deals the diff's own mutants, or to the floor.
- No change to `.cargo/mutants.toml`'s exclusions.

## Invariants

- [ ] A diff that touches any file of the list deals the canary shard, and one that touches none deals none beside the diff's own, each planted.
- [ ] The canary runs the same steps as every shard, and the aggregate fails when it fails.
- [ ] The list is defined once, and the workflow policy refuses a file the shards run that the list leaves out.
- [ ] The canary's mutants are caught by the workspace's tests, so a canary that passes has run them.
- [ ] The `.github` changes go to Norman.
