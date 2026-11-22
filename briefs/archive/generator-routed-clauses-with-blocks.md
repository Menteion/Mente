---
id: generator-routed-clauses-with-blocks
status: archived
type: chore
affectedFiles: [compiler/mente-cli/src/mir_run/generate.rs, compiler/mente-cli/src/mir_run/coverage.rs, compiler/mente-cli/src/mir_run/differential.rs, docs/escapes/oracle-claimed-inside-routed-clauses.md, briefs/generator-routed-clauses-with-blocks.md, briefs/archive/generator-routed-clauses-with-blocks.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**, compiler/mente-sema/**]
integrity: sha256-f2466f3a3dc01661bdb047e5911a18e0d084358cc7c208f05628216159b0ad39
---

<!-- spec-brief:banner -->
> **Archived 2026-11-22.**
> Generated programs hold @checked and @prove functions whose requires, ensures, loop invariants and measures hold an if or a match with a block arm, each way of which runs take, and @checked requires that trap a call; the coverage test requires each kind, form and attribute, each way taken and a trap.
> Recorded at commit `3f43a61`.
> Relative links were rewritten to resolve from `briefs/archive/` (1); no other word changed.
> The body below describes the tree before execution and is not maintained.
<!-- /spec-brief:banner -->

# Generate routed clauses that hold blocks

## Intent

The containment oracle ([`tier1-containment`](../../docs/design-review.md#tier1-containment)) makes no claim inside a clause Tier 1 walks nowhere: an `ensures` routed by `@checked` or `@prove`, which a run evaluates all the same. Its first version reported a false alarm in such a clause when it held a block, an `if` or a `match` with a block arm, and nothing had caught it: no conformance program and no generated program held one. Its fix added a conformance program, `routed_clauses_holding_blocks.mn`, and the oracle's unit test; the program generator still writes none. When this round is done, it writes `@checked` and `@prove` functions whose `ensures`, `requires`, written invariants outside the fragment and measures hold an `if`, a `match` with an expression arm and one with a block arm, each a clause that holds on every call the comparison makes, so that the comparison of the three paths and the oracle run on them at every seed. The pull request that brought the generator into step with the checker, which changed `generate.rs`, has merged (pull request 204, 2e404cc), so the round can start.

## Negative Scope

- No change to Tier 1, to the oracle, or to which clauses Tier 1 walks.
- No clause that fails on a call: a routed clause that fails stops the run at a trap that hides what follows.

## Invariants

- [x] Every generated program still passes every check, Tier 1 included: the generator's self-test over its seeds holds.
- [x] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [x] `test_generated_programs_reach_every_construct_of_the_grammar` counts a routed `ensures`, `requires`, invariant and measure holding an `if` and a block-armed `match`, and fails, seen to, when the generator writes none.
- [x] The oracle, run on the seeds that hold one, reports nothing.
