# Escaped bugs

Every bug that escaped its author's own checks, found by a review, by CI or by a run, with the instrument that missed it and the guard its fix added ([`guard-per-problem`](design-review.md#guard-per-problem)). Norman decided on 2026-11-16 that they are recorded here, one row an escape, in the order their fixes merged. A row is never removed, and its guard stays in the tree.

`test_every_escape_has_its_id_its_day_its_area_and_a_guard_that_stands`, in `compiler/mente-cli/tests/escapes.rs`, checks every row from what the tree holds:

- The ids run `ESC-001`, `ESC-002` and on, so none repeats and none is reused.
- No day comes before the row above's.
- Each area is one of the record's.
- Each row says what missed it.
- Each guard is declared where its kind is: deleting a guard fails the test.

That each fix merged is a fact of the history, which the Commits check reads: there each fixing commit is to carry its id in a trailer, `Escaped: ESC-001`. That rule belongs to the Commits check's own pull request; the rows recorded before it name their fix by its pull request alone.

`mn escapes` prints how many escapes there were each week and in each area. No count is written here.

The columns:

- **Id**: `ESC-` and three digits, in order.
- **Date**: the day the fix merged, `YYYY-MM-DD`.
- **Fixed by**: the pull request that merged the fix, as a link.
- **Area**: where the bug was. It is one of `checker`, `Tier 1`, `interpreter`, `MIR`, `native`, `generator`, `local checks`, `policy` and `harness`, the list `compiler/mente-cli/src/escapes.rs` holds.
- **Missed by**: the instrument that should have caught it, and what it lacked.
- **Guard**: what now catches the class, in one of four forms, each found where it is declared:
  - a Rust test, `` `fn test_name` ``;
  - a check of the workflow policy, `policy check "name"`;
  - a Node test, `node test "name"`;
  - a quick check before a push, `quick check "name"`, by its name in `tools/quick-checks.json`.

| Id | Date | Fixed by | Area | Missed by | Guard |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `ESC-001` | 2026-11-14 | [#129](https://github.com/Menteion/Mente/pull/129) | checker | No lint: a walker's `_` arm hid the operand kinds, and the checker rejected an operand that leaves its statement. | `fn test_exhaustive_crates_deny_wildcard_arms` |
| `ESC-002` | 2026-11-14 | [#129](https://github.com/Menteion/Mente/pull/129) | MIR | The program generator, which wrote no value whose every branch leaves, so its lowering failed unseen. | `fn test_a_value_whose_every_branch_leaves_lowers_to_no_value` |
| `ESC-003` | 2026-11-15 | [#156](https://github.com/Menteion/Mente/pull/156) | local checks | The quick checks, a list each session chose from by hand: pull request 155 went out without its golden files and with two walker sites the lint flags. | quick check "lint list, golden files and conformance" |
| `ESC-004` | 2026-11-15 | [#155](https://github.com/Menteion/Mente/pull/155) | checker | The conformance frames, which held no `.len` of a place and no branch whose value follows an expression statement, so two mutants of the frame module survived. | `fn test_conformance_programs_give_the_frames_they_state` |
| `ESC-005` | 2026-11-16 | [#160](https://github.com/Menteion/Mente/pull/160) | harness | The broker's pricing tests, which priced only usage written by hand, so the first real usage, with a field they never showed, was counted at the worst in a run. | node test "the broker prices every usage the API has answered it with, at what its tokens cost" |
| `ESC-006` | 2026-11-16 | [#161](https://github.com/Menteion/Mente/pull/161) | harness | The broker's tests, whose fake answers named no model, so nothing compared the model an answer names with the one asked for. | node test "the broker hands back no answer from another model than the one asked: it refuses it at the worst it reserved, says so, and admits nothing more" |
| `ESC-007` | 2026-11-16 | [#161](https://github.com/Menteion/Mente/pull/161) | local checks | The quick checks' map, which ran the workflow policy only for the gates' files, not for every file it reads. | policy check "tools/quick.mjs runs this policy before a push for a change to any file, since the policy reads files across the whole tree" |
| `ESC-008` | 2026-11-16 | [#161](https://github.com/Menteion/Mente/pull/161) | policy | The policy's ownership check, which held the files the policy reads and the modules the secret jobs run, but not the modules the policy itself loads. | policy check "every module the workflow policy's tests load, directly or through another, is a gate file its owner holds" |
| `ESC-009` | 2026-11-16 | [#163](https://github.com/Menteion/Mente/pull/163) | harness | The harness tests, which never ran an entry script's top-level code, so `bench.ts` crashed in `retired()` on every run of `main`, found by run 3. | node test "every script that calls the API plans its runs with no broker and no key, as far as its first call" |
| `ESC-010` | 2026-11-16 | [#168](https://github.com/Menteion/Mente/pull/168) | policy | The policy's reading of the workflows, which listed what to refuse instead of what to admit, so a workflow file outside its pattern escaped every rule. | policy check "every file of .github/workflows is a workflow the policy reads" |
| `ESC-011` | 2026-11-16 | [#158](https://github.com/Menteion/Mente/pull/158) | interpreter | The debug tests, whose stack margin was larger than a release build's, so a deep recursion crashed only in release. | `fn test_a_segment_starts_from_the_least_a_switch_begins_with` |
| `ESC-012` | 2026-11-16 | [#158](https://github.com/Menteion/Mente/pull/158) | native | The program generator, which passes no argument outside its parameter's type, so no program reached a call counted after its arguments were narrowed. | `fn test_a_call_from_outside_is_counted_before_its_arguments_are_narrowed` |
| `ESC-013` | 2026-11-16 | [#158](https://github.com/Menteion/Mente/pull/158) | MIR | Nothing measured the verifier on a long function, whose table grew with the square of its blocks; CI's time limit caught it. | `fn test_the_verifier_reads_a_long_function_in_linear_time` |
| `ESC-014` | 2026-11-16 | [#158](https://github.com/Menteion/Mente/pull/158) | local checks | The quick checks, which ran no gate for a change to MIR, so the mutation gate's records named a mutant that no longer existed. | quick check "MIR mutation gate" |
