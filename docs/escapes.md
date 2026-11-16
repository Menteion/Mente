# Escaped bugs

Every bug that escaped its author's own checks, found by a review, by CI or by a run, with the instrument that missed it and the guard its fix added ([`guard-per-problem`](design-review.md#guard-per-problem)). Norman decided on 2026-11-16 that they are recorded here, one row an escape. A row is never removed, and its guard stays in the tree.

A row is named by a key, as the register's entries are: lowercase words joined by hyphens, never a number (Norman, 2026-11-17). One rule, `is_key` in `compiler/mente-cli/src/records.rs`, holds the register's keys and these.

`test_every_escape_has_a_key_an_area_and_a_guard_that_stands`, in `compiler/mente-cli/tests/escapes.rs`, checks every row from what the tree holds:

- Each key is a key and no other row's; since no row is removed, no key is reused.
- Each day is a day of the calendar.
- Each area is one of the record's.
- Each row says what missed it.
- Each guard is declared where its kind is: deleting a guard fails the test.

The rows' order is free: their days order them, and two fixes in flight each add their own row.

That each fix merged is a fact of the history, which the Commits check reads: there each fixing commit is to carry its row's key in a trailer, `Escaped: <key>`. That rule belongs to the Commits check's own pull request; the rows recorded before it name their fix by its pull request alone.

`mn escapes` prints how many escapes there were each week and in each area. No count is written here.

The columns:

- **Key**: the row's key.
- **Date**: the day the escape was found, `YYYY-MM-DD`. The rows recorded on 2026-11-17 from earlier pull requests carry the day their fix merged.
- **Fixed by**: the pull request that merged the fix, as a link.
- **Area**: where the bug was. It is one of `checker`, `Tier 1`, `interpreter`, `MIR`, `native`, `generator`, `local checks`, `policy` and `harness`, the list `compiler/mente-cli/src/escapes.rs` holds.
- **Missed by**: the instrument that should have caught it, and what it lacked.
- **Guard**: what now catches the class, in one of four forms, each found where it is declared:
  - a Rust test, `` `fn test_name` ``;
  - a check of the workflow policy, `policy check "name"`;
  - a Node test, `node test "name"`;
  - a quick check before a push, `quick check "name"`, by its name in `tools/quick-checks.json`.

| Key | Date | Fixed by | Area | Missed by | Guard |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `walker-wildcard-hid-operands` | 2026-11-14 | [#129](https://github.com/Menteion/Mente/pull/129) | checker | No lint: a walker's `_` arm hid the operand kinds, and the checker rejected an operand that leaves its statement. | `fn test_exhaustive_crates_deny_wildcard_arms` |
| `every-branch-leaves-unlowered` | 2026-11-14 | [#129](https://github.com/Menteion/Mente/pull/129) | MIR | The program generator, which wrote no value whose every branch leaves, so its lowering failed unseen. | `fn test_a_value_whose_every_branch_leaves_lowers_to_no_value` |
| `quick-checks-chosen-by-hand` | 2026-11-15 | [#156](https://github.com/Menteion/Mente/pull/156) | local checks | The quick checks, a list each session chose from by hand: pull request 155 went out without its golden files and with two walker sites the lint flags. | quick check "lint list, golden files and conformance" |
| `frame-mutants-survived` | 2026-11-15 | [#155](https://github.com/Menteion/Mente/pull/155) | checker | The conformance frames, which held no `.len` of a place and no branch whose value follows an expression statement, so two mutants of the frame module survived. | `fn test_conformance_programs_give_the_frames_they_state` |
| `broker-priced-handwritten-usage` | 2026-11-16 | [#160](https://github.com/Menteion/Mente/pull/160) | harness | The broker's pricing tests, which priced only usage written by hand, so the first real usage, with a field they never showed, was counted at the worst in a run. | node test "the broker prices every usage the API has answered it with, at what its tokens cost" |
| `answer-from-another-model` | 2026-11-16 | [#161](https://github.com/Menteion/Mente/pull/161) | harness | The broker's tests, whose fake answers named no model, so nothing compared the model an answer names with the one asked for. | node test "the broker hands back no answer from another model than the one asked: it refuses it at the worst it reserved, says so, and admits nothing more" |
| `quick-map-skipped-test-ci` | 2026-11-16 | [#161](https://github.com/Menteion/Mente/pull/161) | local checks | The quick checks' map, which ran the workflow policy only for the gates' files, not for every file it reads. | policy check "tools/quick.mjs runs this policy before a push for a change to any file, since the policy reads files across the whole tree" |
| `policy-unowned-imports` | 2026-11-16 | [#161](https://github.com/Menteion/Mente/pull/161) | policy | The policy's ownership check, which held the files the policy reads and the modules the secret jobs run, but not the modules the policy itself loads. | policy check "every module the workflow policy's tests load, directly or through another, is a gate file its owner holds" |
| `bench-retired-misplaced-paren` | 2026-11-16 | [#163](https://github.com/Menteion/Mente/pull/163) | harness | The harness tests, which never ran an entry script's top-level code, so `bench.ts` crashed in `retired()` on every run of `main`, found by run 3. | node test "every script that calls the API plans its runs with no broker and no key, as far as its first call" |
| `workflow-files-outside-pattern` | 2026-11-16 | [#168](https://github.com/Menteion/Mente/pull/168) | policy | The policy's reading of the workflows, which listed what to refuse instead of what to admit, so a workflow file outside its pattern escaped every rule. | policy check "every file of .github/workflows is a workflow the policy reads" |
| `release-stack-margin` | 2026-11-16 | [#158](https://github.com/Menteion/Mente/pull/158) | interpreter | The debug tests, whose stack margin was larger than a release build's, so a deep recursion crashed only in release. | `fn test_a_segment_starts_from_the_least_a_switch_begins_with` |
| `call-counted-after-narrowing` | 2026-11-16 | [#158](https://github.com/Menteion/Mente/pull/158) | native | The program generator, which passes no argument outside its parameter's type, so no program reached a call counted after its arguments were narrowed. | `fn test_a_call_from_outside_is_counted_before_its_arguments_are_narrowed` |
| `verifier-quadratic-in-blocks` | 2026-11-16 | [#158](https://github.com/Menteion/Mente/pull/158) | MIR | Nothing measured the verifier on a long function, whose table grew with the square of its blocks; CI's time limit caught it. | `fn test_the_verifier_reads_a_long_function_in_linear_time` |
| `quick-checks-skipped-mir-gate` | 2026-11-16 | [#158](https://github.com/Menteion/Mente/pull/158) | local checks | The quick checks, which ran no gate for a change to MIR, so the mutation gate's records named a mutant that no longer existed. | quick check "MIR mutation gate" |
