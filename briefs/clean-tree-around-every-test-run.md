---
id: clean-tree-around-every-test-run
status: active
trigger: when plan-outside-checkout's pull request (219) has merged
type: chore
affectedFiles: [.github/scripts/clean-tree.sh, .github/instrument-paths.txt, .github/scripts/test-sandbox-env.sh, .github/workflows/**, .github/gate-commands.txt, .github/scripts/policy/**, .github/scripts/policy-run.test.mjs, compiler/mente-cli/src/records.rs, compiler/mente-cli/tests/records.rs, compiler/mente-cli/tests/gates.rs, compiler/mente-cli/src/walk-skips.txt, briefs/clean-tree-around-every-test-run.md, briefs/archive/clean-tree-around-every-test-run.md]
protectedFiles: [spec/**, CLAUDE.md]
---

# Keep the tree the tests judge equal to the commit's, before and after every test run

## Intent

The escape `plan-downloaded-into-the-checkout`: a job that runs the tests did not keep its workspace equal to the commit's tree. Pull request 219 closed one writer, a download, and holds every download outside the checkout. The others stay:
- the Mutants plan writes `pr.diff`, its mutant lists, `deleted-re.txt`, `base-mutants.toml` and `missing.txt` into its checkout before its baseline, and `baseline.log` while the tests run (`tee`);
- each shard writes `shard-mutants.json` and `unlisted.txt` before its baseline, and `baseline.log` while it runs;
- `mutants-full.yml` writes `baseline.log` while its baseline runs;
- `fuzz.yml` writes `--out fuzz-out`, `mir-mutants.yml` writes `> gate.txt`, and `spike-tier1.yml` writes `| tee result.txt`, each into its checkout after its gate command, which an after-check would refuse (the review of 62f8265).

Every one of them moves under `$RUNNER_TEMP`.

None is Markdown, so no test fails on them today, but a test that walks the tree reads them: `test_repository_text_is_english` reads every file.

**This round removes the cause.** Every required job checks, right before each run of its tests and again right after, that the tree is the commit's; the working files move under `$RUNNER_TEMP`.

**Cause and guard (the Manager, 2026-11-23):** the check removes the cause, since no test then runs on a tree other than the commit's, whatever wrote into it and whatever the test reads. Reading git's list instead of the disk (`records-every-file`) only guards the tests that read the list, while the strays stay in the tree for every other reader. Both belong, each for its own reason; once this round holds in CI, `records-every-file`, which runs after it, re-weighs its CI list step, which it then no longer needs for CI's correctness, as its brief says.

## The check

`.github/scripts/clean-tree.sh` fails, naming each path escaped so that no file's name can form a workflow command, when `git -c core.fsmonitor= status --porcelain=v1 -z --untracked-files=all --ignored=matching` shows anything outside what the tree's walks leave out. `--ignored=matching` names an ignored directory once, as `prelim-out/` or `mutants.out/`, where the traditional mode would print every file under `target/` and `node_modules/` (the review of 62f8265):
- an untracked file, a tracked file changed, and an ignored file the walks read. `.gitignore` hides `mutants.out/`, `prelim-out/`, `bench-out/`, `fixtures-out/` and `build/`, where the harness writes `*.reply.md`, so the check reads ignored files too (the review of 996aea6);
- what it leaves out comes from the one definition the walks read (`records.rs:276` and `gates.rs:695` each spell `.git`, `target` and `node_modules` today): one list, read by both walks and by the script, or, if the shell cannot share it, a test that holds the copies equal.

**Before and after each test run:** every step that runs a gate command of its required job, and every test run of a testing job, runs it on the line right before its first such command and on the line right after its last, since `baseline.log` is written while the tests run. The lines stand in each step's script, where the policy reads them, not in `test-sandbox-env.sh`, where a check would run out of the policy's sight and in steps that run no gate command. A step of the gate's own setup alone, which runs before any code of the change or reads only files outside the tree (the toolchain's step, the tools' record, the sandbox's setup and proof, cargo-mutants' digest, the tests' times, mdBook's install), runs none; the policy names each with its reason (`CLEAN_TREE_SETUP`). The Commits job, which reads commit text alone, runs none either.

**The in-place mutate step has its own form:** `cargo mutants --in-place` changes a tracked file, one mutant at a time, by design. The check runs before the step and after it, when every mutated file must be restored; during it the one mutated file is the change under test.

## What it covers that the download rule does not

- a step of the shell's that writes into the checkout, `gh run download` among them;
- a local composite action (`uses: ./...`), which `workflows()` never reads;
- a cache restored into the checkout, and a checkout with a `path`.

## Negative Scope

- No change to what the walks read, beyond taking their exclusions from the one definition.
- No change to the sandbox, which already keeps the tests themselves from writing into the tree.

## Invariants

- [x] Every step of a required job that runs a gate command other than the gate's own setup, and every test run of a testing job, runs the check right before its first such command and right after its last; the workflow policy refuses a step without it before, without it after, with a writer between, or with it after the step's first gate command, each planted.
- [x] The check's exclusions are the walks' own, from one definition or held equal by a test.
- [x] A run test of `clean-tree.sh` in repositories it makes: a clean tree and files under the excluded directories pass; an untracked file, a changed tracked file and a file in an ignored directory the walks read fail, each named; a name with a quote, a newline or `::` is printed escaped.
- [x] The Mutants plan's and shards' working files, and `mutants-full.yml`'s, live under `$RUNNER_TEMP`; the plan's artifact keeps its layout.
- [x] The `.github` changes go to Norman.
