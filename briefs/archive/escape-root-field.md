---
id: escape-root-field
status: archived
trigger: when root-cause-first's pull request has merged
type: chore
awaits: [docs/design-review.md#root-cause-first]
affectedFiles: [compiler/mente-cli/src/records.rs, compiler/mente-cli/src/escapes.rs, compiler/mente-cli/tests/escapes.rs, compiler/mente-cli/tests/evidence.rs, compiler/mente-cli/src/front-matter/**, docs/escapes.md, docs/escapes/**, briefs/escape-root-field.md, briefs/archive/escape-root-field.md]
protectedFiles: [spec/**, CLAUDE.md, .github/**]
integrity: sha256-211a3c5b26dfeb500ccaeb415a75a7bad042ca9f04eab10437f616701f13b97c
---

<!-- spec-brief:banner -->
> **Archived 2026-11-22.**
> Recorded at commit `a974c39`.
> Relative links were rewritten to resolve from `briefs/archive/` (1); no other word changed.
> The body below describes the tree before execution and is not maintained.
<!-- /spec-brief:banner -->

# Say in every escaped bug's record whether its fix removed the root

## Intent

[`root-cause-first`](../../docs/design-review.md#root-cause-first) makes every pull request declare the root cause of what it fixes, and whether the fix removes it, in a form the Commits check holds. An escaped bug's record (`docs/escapes/<key>.md`) names the instrument that missed it and the guard its fix added, but not whether the fix removed the root or only holds the class at a guard. A reader of the records, or of `mn escapes`, cannot tell the two apart.

When this round is done, every record's front matter holds a sixth field, `root`, after `guard`. Its value is `removed`, when the fix took away the cause, or `held`, when the cause stays and the guard holds its class:
- `test_every_escape_has_a_key_an_area_and_a_guard` checks the field as it checks the other five: present, in its place, one of the two values;
- `mn escapes` counts the escapes of each value;
- every record already in `docs/escapes/` carries the field, each value read from its own fix's pull request.

## Negative Scope

- No change to the other five fields, their order or their forms.
- No change to the Commits check or its declaration; the record and the pull request each say it once, for their own reader.
- No record removed or renamed.

## Invariants

- [x] `front_matter` in `compiler/mente-cli/src/records.rs` reads six fields, `root` last, and the front-matter cases in `compiler/mente-cli/src/front-matter/` hold the field's form.
- [x] `test_every_escape_has_a_key_an_area_and_a_guard` fails on a record with no `root`, on one with `root` out of its place, and on one with any value but `removed` or `held`, each planted.
- [x] Every record of `docs/escapes/` has its `root`, and the pull request lists each with the line of its fix's pull request that the value rests on.
- [x] `docs/escapes.md` describes the field beside the other five.
- [x] `mn escapes` prints the count of each value, and `mn escapes --check` checks the field as the test does.
