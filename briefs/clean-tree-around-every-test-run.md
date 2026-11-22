---
id: clean-tree-around-every-test-run
status: deferred
trigger: when plan-outside-checkout's pull request (219) has merged
type: chore
affectedFiles: [.github/scripts/clean-tree.sh, .github/scripts/test-sandbox-env.sh, .github/workflows/**, .github/gate-commands.txt, .github/scripts/policy/**, .github/scripts/policy-run.test.mjs, compiler/mente-cli/src/records.rs, compiler/mente-cli/tests/records.rs, compiler/mente-cli/tests/gates.rs, compiler/mente-cli/src/walk-skips.txt, briefs/records-every-file.md, briefs/clean-tree-around-every-test-run.md, briefs/archive/clean-tree-around-every-test-run.md]
protectedFiles: [spec/**, CLAUDE.md]
---

# Keep the tree the tests judge equal to the commit's, before and after every test run

## Intent

The escape `plan-downloaded-into-the-checkout`: a job that runs the tests did not keep its workspace equal to the commit's tree. Pull request 219 closed one writer, a download, and holds every download outside the checkout. The others stay:
- the Mutants plan writes `pr.diff`, its mutant lists, `deleted-re.txt`, `base-mutants.toml` and `missing.txt` into its checkout before its baseline, and `baseline.log` while the tests run (`tee`);
- each shard writes `shard-mutants.json` and `unlisted.txt` before its baseline, and `baseline.log` while it runs;
- `mutants-full.yml` writes `baseline.log` while its baseline runs.

None is Markdown, so no test fails on them today, but a test that walks the tree reads them: `test_repository_text_is_english` reads every file.

**This round removes the cause.** Every required job checks, right before each run of its tests and again right after, that the tree is the commit's; the working files move under `$RUNNER_TEMP`.

**Cause and guard (the Manager, 2026-11-23):** the check removes the cause, since no test then runs on a tree other than the commit's, whatever wrote into it and whatever the test reads. Reading git's list instead of the disk (`records-every-file`) only guards the tests that read the list, while the strays stay in the tree for every other reader. Both belong, each for its own reason; once this round holds in CI, `records-every-file` re-weighs its CI list step, which it then no longer needs for CI's correctness. This round adds that note to its brief.

## The check

`.github/scripts/clean-tree.sh` fails, naming each path escaped so that no file's name can form a workflow command, when `git -c core.fsmonitor= status --porcelain=v1 -z --untracked-files=all --ignored` shows anything outside what the tree's walks leave out:
- an untracked file, a tracked file changed, and an ignored file the walks read. `.gitignore` hides `mutants.out/`, `prelim-out/`, `bench-out/`, `fixtures-out/` and `build/`, where the harness writes `*.reply.md`, so the check reads ignored files too (the review of 996aea6);
- what it leaves out comes from the one definition the walks read (`records.rs:276` and `gates.rs:695` each spell `.git`, `target` and `node_modules` today): one list, read by both walks and by the script, or, if the shell cannot share it, a test that holds the copies equal.

**Before and after each test run:** a test step runs it first, and again after its tests, since `baseline.log` is written while they run. The steps that source `test-sandbox-env.sh` get both through it; the required jobs outside the sandbox (docs, pages, generated programs, execution paths, Z3, fuzz, the MIR gate, perf, the harness) run it as steps around their gate commands.

**The in-place mutate step has its own form:** `cargo mutants --in-place` changes a tracked file, one mutant at a time, by design. The check runs before the step and after it, when every mutated file must be restored; during it the one mutated file is the change under test.

## What it covers that the download rule does not

- a step of the shell's that writes into the checkout, `gh run download` among them;
- a local composite action (`uses: ./...`), which `workflows()` never reads;
- a cache restored into the checkout, and a checkout with a `path`.

## Negative Scope

- No change to what the walks read, beyond taking their exclusions from the one definition.
- No change to the sandbox, which already keeps the tests themselves from writing into the tree.

## Invariants

- [ ] Every required job runs the check before its first command of `.github/gate-commands.txt` and after its last, each test step of a sandboxed job through `test-sandbox-env.sh`; the workflow policy refuses a job without it, and one with it after its first gate command, each planted.
- [ ] The check's exclusions are the walks' own, from one definition or held equal by a test.
- [ ] A run test of `clean-tree.sh` in repositories it makes: a clean tree and files under the excluded directories pass; an untracked file, a changed tracked file and a file in an ignored directory the walks read fail, each named; a name with a quote, a newline or `::` is printed escaped.
- [ ] The Mutants plan's and shards' working files, and `mutants-full.yml`'s, live under `$RUNNER_TEMP`; the plan's artifact keeps its layout.
- [ ] `briefs/records-every-file.md` says that its CI list step is to be re-weighed.
- [ ] The `.github` changes go to Norman.
