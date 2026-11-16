# Escaped bugs

Every bug that escaped its author's own checks, found by a review, by CI or by a run, with the instrument that missed it and the guard its fix added ([`guard-per-problem`](design-review.md#guard-per-problem)). Norman decided on 2026-11-16 that they are recorded, and on 2026-11-17 that each is recorded in a file of its own, `docs/escapes/<key>.md`, so that two fixes in flight never touch one file. A file is never removed, and its guard stays in the tree.

A file's name is the escape's key, as the register's entries are named: lowercase words joined by hyphens, no word of digits alone, so never a number, and no name Windows reserves for a device. One rule, `is_key` in `compiler/mente-cli/src/records.rs`, holds the register's keys and these.

A file starts with a front matter of five fields, each on one line `name: value`, in this order, between two lines `---`; any prose may follow it. Each value is plain, or in single quotes, with `''` for a quote, when YAML would read it otherwise: when it holds `: ` or ` #`, starts with a quote, a backtick or another of YAML's indicators, or is a null, a boolean or a number. No tab and no carriage return stands anywhere in the file. So the front matter reads as YAML too. `front_matter` in `compiler/mente-cli/src/records.rs` reads it, and the cases in `compiler/mente-cli/src/front-matter/` hold how, for every reader of the form.

- **date**: the day the escape was found, `YYYY-MM-DD`. The escapes recorded on 2026-11-17 from earlier pull requests carry the day their fix merged.
- **fixed-by**: the address of the pull request that merged the fix, `https://github.com/Menteion/Mente/pull/N`, its number with no zero before it.
- **area**: where the bug was. It is one of `checker`, `Tier 1`, `interpreter`, `MIR`, `native`, `generator`, `local checks`, `policy` and `harness`, the list `compiler/mente-cli/src/escapes.rs` holds.
- **missed-by**: the instrument that should have caught it, and what it lacked.
- **guard**: what now catches the class, in one of four forms, each found in what its runner runs:
  - a Rust test of a package, `fn test_name in mente-interp`, that nextest runs; the package is named, since one test's name may stand in two crates;
  - a check of the workflow policy, `policy check "name"`, that the policy's run ran;
  - a Node test, `node test "name"`, that a Node run ran;
  - a quick check before a push, `quick check "name"`, by its name in `tools/quick-checks.json`.

`test_every_escape_has_a_key_an_area_and_a_guard`, in `compiler/mente-cli/tests/escapes.rs`, checks every file's form:

- The directory holds only files `<key>.md` whose name is a key, so each key is unique, and since no file is removed, never reused.
- Each front matter has the five fields in their order, each value in its form.
- Each date is a day of the calendar, and each fix the address of a pull request.
- Each area is one of the record's.
- Each escape says what missed it.
- Each guard is of one of the four forms.

Whether each guard runs is what the runners run, not what the sources spell. `mn evidence` reads the tools' own output, in JSON: nextest's list of the tests from each runner, as CI's profile takes them; our reporter's report of each Node run, `tools/evidence-reporter.mjs`; and spec-guard's report of the directives it runs. It fails a guard that no runner runs, an ignored, filtered, skipped or left-to-do test included, and every test an `@assert-count` names as evidence that nextest does not run from exactly its target file (`compiler/mente-cli/src/evidence.rs`).

That each fix merged is a fact of the history, which the Commits check reads: there each fixing commit is to carry its escape's key in a trailer, `Escaped: <key>`. That rule belongs to the Commits check's own pull request; the escapes recorded before it name their fix by its pull request alone.

`mn escapes` prints how many escapes there were each week and in each area, and `mn escapes --check` checks the files as the test does. No count is written here.
