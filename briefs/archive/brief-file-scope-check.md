---
id: brief-file-scope-check
status: archived
type: feature
awaits: [docs/design-review.md#brief-file-scope, docs/design-review.md#records-one-per-file]
affectedFiles: [compiler/mente-cli/tests/register.rs, ROADMAP.md, docs/design-review.md, briefs/brief-file-scope-check.md, briefs/archive/brief-file-scope-check.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**]
integrity: sha256-ec4feae1757d6a52122d49755ff6ba60d27a885db3b52f0ae18db6fb2dbe8818
---

<!-- spec-brief:banner -->
> **Archived 2026-11-21.**
> The register's test holds every decided entry to its place in the plan, an item of a list in ROADMAP.md, a milestone's Decided first line or a live brief, by a rule of the register's own; the scope check reuses spec-brief's, which test-isolation wires.
> Merged in pull request [#177](https://github.com/Menteion/Mente/pull/177).
> Recorded at commit `0a58eb4`.
> Relative links were rewritten to resolve from `briefs/archive/` (1); no other word changed.
> The body below describes the tree before execution and is not maintained.
<!-- /spec-brief:banner -->

# Hold each pull request to its brief's files, and each decided entry to its place in the plan

## Intent

[`brief-file-scope`](../../docs/design-review.md#brief-file-scope), decided on 2026-11-15, is held so far by memory alone. Two rules are added.

The first fails a pull request that names no brief, or that changes a file outside its brief's `affectedFiles` or inside its `protectedFiles`. spec-brief already holds a round to its scope (`spec-brief archive <brief> --dry-run --strict --base <rev>`), with the scope's globs defined once, in the tool, so the check reuses it rather than copying its globs into Rust (the manager, 2026-11-17). The required check that runs it lives in `.github/`, which the test-isolation session owns and wires; this round writes down what it must wire.

The second rule is in the register's test: every `decided` entry is linked from an item of `ROADMAP.md`, from a milestone's line of decisions taken first, or from a live brief, so that no decision drops out of the plan.

## Negative Scope

- No change to `.github/`: what the required check must wire is reported to the manager, for test-isolation.
- No new decision: an entry that is linked nowhere gets a link to where the plan already does its work, or a brief, never a new rule.

## Invariants

- [x] The register's rule has tests seen to fail without it, with plants: a decided entry linked from nowhere, from prose only, from a longer key or from an archived brief, and a placement taken out of `ROADMAP.md`.
- [ ] The scope check has tests seen to fail without it, with plants: no brief named, a file outside the scope, a file inside the protected files.

  **Delegated** to test-isolation, which wires spec-brief's own scope check in `.github/` and plants its cases there.
- [x] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [ ] A function reads a pull request's brief and its changed files and returns each file out of scope, by the patterns spec-brief reads.

  **Rejected**: spec-brief's `archive --dry-run --strict --base <rev>` already returns each file out of scope and each protected file changed, by its own patterns; a Rust copy would define the scope's globs twice.
- [x] `tests/register.rs` refuses a decided entry that no ROADMAP item or live brief links.
- [x] The manager has the list of what test-isolation must wire.
