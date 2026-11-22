---
date: 2026-11-23
fixed-by: https://github.com/Menteion/Mente/pull/219
area: policy
missed-by: No shard ran its tests on pull request 202's tree, since 202 changed no Rust line and its plan dealt no shard; the run test of a shard's steps ran them in a tree that held no plan downloaded into it, so it never ran the tests where the plan landed; and review read the download's path without asking what the tests would see there.
guard: policy check "every artifact a workflow downloads lands outside the checkout, so the tree the tests judge is the commit's"
---

Pull request 202 made "Mutants in the diff" the aggregate of a plan and the shards it deals. Each shard downloaded the plan's artifact to `path: mutants-plan`, inside its checkout, and then ran the whole workspace's tests once without a mutant. The plan's files stood in the tree those tests judged, `plan/summary.md` among them, which no declaration of the records holds, so `test_every_markdown_file_is_declared_once` failed every shard of every pull request based on 9e6858d before any mutant ran. Pull request 216 at 77aba83 showed all seven shards red; 217 escaped it only because its shards were skipped.

Root cause: a step downloaded an artifact into the checkout, so the tree the tests judged was not the commit's tree.

Removed at the root: every artifact a workflow downloads lands under `${{ runner.temp }}`, the plan at `${{ runner.temp }}/mutants-plan`, and the policy check named above, in the required Documentation job, refuses a download whose path is in the checkout, its default included, in any workflow.

The run test of a shard's steps now makes the plan under another name and puts it under the runner's temporary directory, so that a step that read the plan from the tree would find nothing.
