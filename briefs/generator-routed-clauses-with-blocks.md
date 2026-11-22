---
id: generator-routed-clauses-with-blocks
status: active
type: chore
affectedFiles: [compiler/mente-cli/src/mir_run/generate.rs, compiler/mente-cli/src/mir_run/coverage.rs, compiler/mente-cli/src/mir_run/differential.rs, docs/escapes/oracle-claimed-inside-routed-clauses.md, briefs/generator-routed-clauses-with-blocks.md, briefs/archive/generator-routed-clauses-with-blocks.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**, compiler/mente-sema/**]
---

# Generate routed clauses that hold blocks

## Intent

The containment oracle ([`tier1-containment`](../docs/design-review.md#tier1-containment)) makes no claim inside a clause Tier 1 walks nowhere: an `ensures` routed by `@checked` or `@prove`, which a run evaluates all the same. Its first version reported a false alarm in such a clause when it held a block, an `if` or a `match` with a block arm, and nothing had caught it: no conformance program and no generated program held one. Its fix added a conformance program, `routed_clauses_holding_blocks.mn`, and the oracle's unit test; the program generator still writes none. When this round is done, it writes `@checked` and `@prove` functions whose `ensures`, `requires`, written invariants outside the fragment and measures hold an `if`, a `match` with an expression arm and one with a block arm, each a clause that holds on every call the comparison makes, so that the comparison of the three paths and the oracle run on them at every seed. The pull request that brought the generator into step with the checker, which changed `generate.rs`, has merged (pull request 204, 2e404cc), so the round can start.

## Negative Scope

- No change to Tier 1, to the oracle, or to which clauses Tier 1 walks.
- No clause that fails on every call that reaches it, and none that fails under `@prove`: a routed clause that fails stops the run at a trap that hides what follows. A `@checked` function's routed `requires` may fail on its parameter's lowest value and hold on the others, so that the run-time check `@checked` asks for traps the calls that pass that value and lets the rest through.

## Invariants

- [x] Every generated program still passes every check, Tier 1 included: the generator's self-test over its seeds holds.
- [x] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [x] `test_generated_programs_reach_every_construct_of_the_grammar` counts a routed `ensures`, `requires`, invariant and measure holding an `if` and a block-armed `match`, and fails, seen to, when the generator writes none.
- [x] The oracle, run on the seeds that hold one, reports nothing.
