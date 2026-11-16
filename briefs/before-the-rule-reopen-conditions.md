---
id: before-the-rule-reopen-conditions
status: deferred
trigger: when the pull request that writes the queues as briefs merges
type: chore
affectedFiles: [docs/design-review.md, compiler/mente-cli/tests/register.rs, ROADMAP.md, briefs/before-the-rule-reopen-conditions.md, briefs/archive/before-the-rule-reopen-conditions.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**]
---

# Give every decided entry before the rule its reopen condition

## Intent

The register's test exempts the entries decided before the reopen rule, the list `BEFORE_THE_RULE` in `tests/register.rs`, from naming what would reopen them, with no deadline. Norman set one on 2026-11-17: the list is empty by the M2 review. This round adds an item to the M2 review in `ROADMAP.md` that says so, with the first batch. The conditions are drafted in batches of about twenty for Norman to sign, and each signed batch leaves the list in one change.

## Negative Scope

- No condition is written as signed before Norman signs it.
- No change to an entry's decision: only its reopen condition is added.

## Invariants

- [ ] The list only shrinks, and the register's test holds every entry that leaves it to its condition.
- [ ] `node tools/quick.mjs` passes before the push.
