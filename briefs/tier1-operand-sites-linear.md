---
id: tier1-operand-sites-linear
status: deferred
trigger: when pull request 216 merges
type: chore
affectedFiles: [compiler/mente-sema/src/tier1.rs, compiler/mente-sema/src/octagon.rs, briefs/tier1-operand-sites-linear.md, briefs/archive/tier1-operand-sites-linear.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**, compiler/mente-syntax/**]
---

# Find each operand's site variables in time linear in the function

## Intent

Tier 1's setup, `Fx::new` in `compiler/mente-sema/src/tier1.rs`, gives the right operand of each `and` and `or` the site variables its evaluation writes (`sites_of_operands`). For each such operand it scans every expression of the function and keeps those inside the operand's span. Its work is the number of `and`s and `or`s times the number of expressions: the square of a function's size, which the nesting limit does not bound, since the operands need not nest. A long function with an `and` in each of many statements pays it at every check, and `mn check --watch` at every edit of the function. Issue 220's triage found it, beside the tree past the limit its pull request removed.

When this round is done, the variables are found in one walk of the function: each expression's variables are written once, and each operand reads the range of those inside it. A deterministic count holds the setup linear: the expressions it visits, counted on the meter pull request 216 gives Tier 1, so that a function of twice the statements visits at most about twice the expressions. Its owner is stack-budget, once pull request 216, which rewrites `tier1.rs`, has merged.

## Negative Scope

- No change to which variables an operand's evaluation writes, or to what Tier 1 proves.
- No wall-clock bound: the guard is a count, the same on every runner.

## Invariants

- [ ] Tier 1's diagnostics on the conformance suite and the examples are unchanged, as the golden diagnostics show.
- [ ] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [ ] A test holds the setup's count of visited expressions linear in a function of many `and`s in many statements, and fails on today's scan.
- [ ] The work golden of pull request 216 moves within its band, or the pull request says why.
