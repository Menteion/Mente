---
id: smt-trap-identity
status: draft
date: 2026-11-22
type: refactor
affectedFiles: [compiler/mente-cli/src/smt_independent.rs, compiler/mente-cli/tests/smt.rs, briefs/smt-trap-identity.md, briefs/archive/smt-trap-identity.md]
protectedFiles: [CLAUDE.md, spec/**, compiler/mente-sema/**, compiler/mente-interp/**, compiler/mente-smt/**, .github/**]
---

# Pair a check's traps one to one in the comparison with the base

## Intent

`mn smt --compare` ([`smt-independent-encoding`](../docs/design-review.md#smt-independent-encoding), pull request 191) holds each check Tier 1 erases to its base: one check, one function and one obligation, with the class of each of its traps. The traps themselves have no identity. `Erased::less_than` sorts each side's ranks, best first, and compares them place by place. So a trap lost on one side is masked by a gain on another trap of the same obligation: `[query, unreachable]` to `[unreachable, query]` passes, and so does `[query, unreachable]` to `[query, cut off, query]` (the Fable verdict on pull request 191, round 6).

When this round is done:
- The keys name each trap by a stable key: the trap's kind and its place, a structured pair the comparison reads. Never the encoder's free text.
- The comparison pairs the traps of one check one to one by that key. A trap on one side alone counts as lost, rank 0, on the side that lacks it. A check is verified less when any paired trap falls or any base trap is lost.
- A trap moved by the encoder, a column changed, reads as one lost and one gained. That fails the job until an `Smt-Drop:` trailer names the check and says why.

## Negative Scope

- The identity of a check, its file's text, its function and its obligation, does not change, and neither does the rule for a check Tier 1 erases in one run alone.
- The encoding and its traps do not change. This round only reads them.

## Not Empowered

- `compiler/mente-smt/**`, `spec/**`, `CLAUDE.md`, `.github/**`.

## Invariants

- [ ] The two cases of the verdict are tests and fail: `[query, unreachable]` to `[unreachable, query]` with the traps swapped, and `[query, unreachable]` to `[query, cut off, query]` with the unreachable trap lost.
- [ ] A trap moved to another place is one trap lost and one gained, and a waiver naming its check lets it through.
- [ ] Each check of the review's earlier rounds still fails, the multisets of round 6 among them. Each test is seen to fail with its fault planted.
- [ ] The keys' format is read strictly: a trap without its kind or its place is refused.
