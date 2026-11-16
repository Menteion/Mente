---
id: evidence-against-the-tests-run
status: draft
type: feature
affectedFiles: [compiler/mente-cli/src/main.rs, compiler/mente-cli/tests/evidence.rs, briefs/evidence-against-the-tests-run.md, briefs/archive/evidence-against-the-tests-run.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**]
---

# Check each test named as evidence against the tests that run

## Intent

An `Implemented` claim's evidence is an `@assert-count` whose `symbol` names a test, `fn test_...`, and spec-guard counts the symbol as a substring of the target file. A test renamed with its old name left in a comment, deleted from one file but alive in another, or marked `#[ignore]` still counts. Every such directive is cross-checked against the list of tests nextest runs, which CI already writes, so that a claim whose test does not run fails.

## Negative Scope

- No change to spec-guard, whose repository is not this one: a gap found in it is reported to Norman.
- No change to `.github/`: what CI must hand the check, the list of tests run, is reported to the manager, for test-isolation.

## Invariants

- [ ] A directive naming a test that is renamed, deleted or ignored fails the check, each planted.
- [ ] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [ ] Every `fn test_` symbol of an `@assert-count` in the documents spec-guard reads names a test in nextest's list of the tests run, in the target file's crate.
