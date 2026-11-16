---
id: brief-file-scope-check
status: draft
type: feature
awaits: [docs/design-review.md#brief-file-scope, docs/design-review.md#records-one-per-file]
affectedFiles: [compiler/mente-cli/src/briefs.rs, compiler/mente-cli/src/lib.rs, compiler/mente-cli/src/main.rs, compiler/mente-cli/tests/briefs.rs, compiler/mente-cli/tests/register.rs, ROADMAP.md, docs/design-review.md, briefs/**]
protectedFiles: [.github/**, CLAUDE.md, spec/**]
---

# Hold each pull request to its brief's files, and each decided entry to its place in the plan

## Intent

[`brief-file-scope`](../docs/design-review.md#brief-file-scope), decided on 2026-11-15, is held so far by memory alone. Two rules are added.

The first fails a pull request that names no brief, or that changes a file outside its brief's `affectedFiles` or inside its `protectedFiles`. This round writes the side that reads the briefs and compares a list of changed files with them. The required check that runs it lives in `.github/`, which the test-isolation session owns and wires.

The second rule is in the register's test: every `decided` entry is linked from an item of `ROADMAP.md` or from a live brief, so that no decision drops out of the plan. On 2026-11-17, 8 of the 22 decided entries appeared nowhere in `ROADMAP.md`.

## Negative Scope

- No change to `.github/`: what the required check must wire is reported to the manager, for test-isolation.
- No new decision: an entry that is linked nowhere gets a link to where the plan already does its work, or a brief, never a new rule.

## Invariants

- [ ] Each rule has a test seen to fail without it, with plants: no brief named, a file outside the scope, a file inside the protected files, a decided entry linked from nowhere.
- [ ] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [ ] A function reads a pull request's brief and its changed files and returns each file out of scope, by the patterns spec-brief reads.
- [ ] `tests/register.rs` refuses a decided entry that no ROADMAP item or live brief links.
- [ ] The manager has the list of what test-isolation must wire.
