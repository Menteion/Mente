---
date: 2026-11-23
fixed-by: https://github.com/Menteion/Mente/pull/219
area: policy
missed-by: No shard ran its tests on pull request 202's tree at its last head, since 202 changed no Rust line and its plan dealt no shard; the run test of a shard's steps ran them with stand-ins for cargo, so it never ran the baseline or any test in the tree where the plan landed; and review read the download's path without asking what the tests would see there.
guard: policy check "every artifact a workflow downloads lands outside the checkout, so the tree the tests judge is the commit's"
root: held
---

Pull request 202 made "Mutants in the diff" the aggregate of a plan and the shards it deals. Each shard downloaded the plan's artifact to `path: mutants-plan`, inside its checkout, and then ran the whole workspace's tests once without a mutant. The plan's files stood in the tree those tests judged, `plan/summary.md` among them, which no declaration of the records holds, so `test_every_markdown_file_is_declared_once` failed every shard of every pull request based on 9e6858d before any mutant ran. Pull request 216 at 77aba83 showed all seven shards red; 217 escaped it only because its shards were skipped. 202's probes had run shards with the plan in the checkout, but on a base from before 0927729, which holds every document of the tree to a declaration and reached `main` after them.

Root cause: a job that runs the tests does not keep its workspace equal to the commit's tree. A download is one writer into it of several: the Mutants plan and shards also write their lists there, and `tee baseline.log` while the tests run.

Not removable at the root: in this fix, which closes the download's route alone. The shell's writers are the brief `clean-tree-around-every-test-run`, whose check, before and after each test run, removes the class.

Guard: the policy check named above, in the required Documentation job, refuses a download whose path is not a directory of its own under `${{ runner.temp }}`, its default included, in any workflow and by any action of the kind. The run test of a shard's steps takes the plan's place from the download step itself and makes the plan under another name, so that a step reading it from anywhere else finds nothing.
