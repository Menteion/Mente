# The workflow policy

The workflow policy (docs/design-review.md, `guard-per-problem`) is what makes the instruments merge
gates of `main` that cannot pass without checking, and what keeps the workflows safe, checked over
`.github/workflows`. It runs in the Documentation job, a required check, as `npm run test:ci`:
first `.github/scripts/policy-collect.mjs`, which gives it git's facts, then `node --test` over
every `*.test.mjs` of this directory, with `policy-run.test.mjs` and `yaml.test.mjs` beside them.
`test_the_documentation_job_runs_the_workflow_policy` in `compiler/mente-cli` keeps it there.

What it holds:
- Every check `.github/required-checks.txt` names is produced by exactly one job, whose workflow
  runs on every pull request and in the merge queue with no `paths:` or `branches:` filter, and
  which nothing can skip, let fail, or give more than read access. Such a job that needs others is
  the aggregate of its workflow, as "Mutants in the diff" is of its plan and shards: it runs
  `if: always()`, needs every other job of the workflow, checks out nothing, reads every one's
  result in its one step, and passes only when they do; the jobs it needs are required jobs too.
- Every instrument workflow decides at its first step after the checkout whether it has anything
  to check (`.github/scripts/watch.mjs`); every later step runs unless the decision was "false",
  so a decision never written runs the instrument.
- Each instrument's list in `.github/instrument-paths.txt` names `Cargo.toml`, `Cargo.lock`,
  `rust-toolchain.toml`, its workflow, every file of every crate its jobs build, every repository
  path its scripts name with what they import, and the corpora the compiler reads for it.
- No script a required job runs gates its `main()` on the path it was started by.
- Every action is pinned to a full commit SHA, every checkout drops its credentials, no workflow
  runs on `pull_request_target` or `workflow_run`, no `run:` script and no `script:` input holds
  an expression, overflow checks stay on in every build, and a measured benchmark run proves its
  sandbox before any agent code runs.
- On Linux the testing jobs set up and prove the sandbox the code of a change runs in before any
  of it runs, and every step that runs it checks that it ran there.
- Every milestone `ROADMAP.md` marks done on the base has its tag, and every instrument
  `docs/design-brief.md` section 7.7 schedules for a finished milestone has its checks required.

## Its modules, one per concern

Each test module holds the checks of one concern, and a `FLOOR` equal to their number, which the
gates test holds. A check's own helpers and constants lie in its module. A value two concerns share
lies in one library of this directory, which the modules import, so that each fact has one
definition, and two changes to different concerns touch different files.

`modules.json` lists the policy's own files once: its test modules, its libraries, and the checks
the modules hold in all. The policy and the gates test hold this directory to exactly those files,
this README and the list, with no dotfile, which node's glob would not run; the modules' checks add
up to the list's total; and the pins exempt exactly the listed files, while any other script here
that a pinned job runs is pinned like every other. `PINNED_FILES` pins the list, so a new module, a
module renamed or a lower total changes a digest the owner holds. `tools/verify-policy-move.mjs`
compares the old `policy.test.mjs` with the modules, statement by statement.

| Module | Concern |
| :--- | :--- |
| `actions-and-permissions.test.mjs` | What every workflow may run and with what token: pinned actions, read-only permissions, no expression in a script. |
| `required-checks.test.mjs` | Every check `main` requires is one job that runs everywhere and cannot pass without checking; the milestones' tags. |
| `build-and-toolchain.test.mjs` | Overflow checks, the pinned toolchain, and nothing of a change running where cargo builds. |
| `test-runs.test.mjs` | The tests run through the pinned nextest, under the shared limits, from a list read as nextest writes it. |
| `sandbox.test.mjs` | Every job that runs code of a change runs it in the sandbox, or says why it does not. |
| `secrets-and-broker.test.mjs` | The jobs that hold the API key or the held-out token: how they start, and what they run. |
| `mutation.test.mjs` | cargo-mutants' settings, the sources it reads, and the floor of work of "Mutants in the diff". |
| `instruments.test.mjs` | Each instrument decides at its first step, and its path list names everything its jobs read. |
| `commits.test.mjs` | The Commits check. |
| `site.test.mjs` | The documentation site's publication. |
| `pins-and-owners.test.mjs` | The scripts and jobs the policy pins, the gate files CODEOWNERS gives the owner, and the modules the policy itself loads. |
| `local-tools.test.mjs` | The quick checks before a push and what each change calls for, the limits every local build, test and probe runs under on the laptop, and the hook that refuses a raw build. |
| `nightly-alarm.test.mjs` | Every workflow that runs on a schedule raises its one alarm, which keeps one issue per workflow, and no other job writes issues. |

| Library | What it holds |
| :--- | :--- |
| `repo.mjs` | The repository: its root, its files as git tracks them, the commit checked out and the collector's facts. |
| `workflows.mjs` | The workflows: each parsed, its jobs and their checks' names, the required ones, the forms a step's condition takes, the permissions GitHub knows, and the nightly alarm's jobs. |
| `pins.mjs` | The pins of the scripts and jobs that run the tests, the gate files, the quick checks' map, and the policy's own files as `modules.json` lists them. |
| `sandbox.mjs` | How jobs are classed for the tests' sandbox. |
| `toolchain.mjs` | What may run before the toolchain, in every required or pinned job. |
| `js.mjs` | The reading of JavaScript and TypeScript modules: their tokens and what they import. |
| `secrets.mjs` | The jobs that hold a secret, and their steps. |
