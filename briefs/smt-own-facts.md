---
id: smt-own-facts
status: draft
date: 2026-11-22
type: refactor
affectedFiles: [compiler/mente-smt/src/**, compiler/mente-cli/src/smt_independent.rs, compiler/mente-cli/tests/smt.rs, docs/design-review.md, docs/STATUS.md, briefs/smt-own-facts.md, briefs/archive/smt-own-facts.md]
protectedFiles: [CLAUDE.md, spec/**, compiler/mente-sema/**, compiler/mente-interp/**, compiler/mente-smt/Cargo.toml, .github/**]
---

# Derive the independent encoding's types inside mente-smt

## Intent

The independent SMT cross-check ([`smt-independent-encoding`](../docs/design-review.md#smt-independent-encoding), pull request 191) shares no code with Tier 1. But it takes every expression's type from the checker: `mente_smt::Facts`, whose `exprs`, `literals`, `bindings` and `mixed` `smt_independent::facts` copies out of `CheckedFn::typing`. Tier 1 reads the same types. So a wrong type from the checker is common-mode: given a literal of the wrong type, or the wrong join of an operator's operands, Tier 1 and the encoding agree on the same wrong program, and the cross-check cannot see it (the Fable review of pull request 191, item 6).

When this round is done, mente-smt reads from the checker only the types of the leaves, as `compiler/mente-interp/src/kinds.rs` does for the interpreter:
- a parameter's, a stated binding's, a field's and a call's return type;
- the binding each name resolves to;
- a suffixed literal's type.

It derives everything else itself, with a join of its own taken from the ranges of the types (`core-calculus.md`, items 4 and 5):
- the type of every operator, branch, arm, array literal and indexed element;
- the type the literals of an expression of literals alone take;
- whether an expression's integers are mixed;
- an unstated binding's type, from its initializer's.

A slip in the checker's derivation then becomes a disagreement between two derivations that share no code. Tier 1 proves a check under the checker's type, and the encoding writes its query under its own: the query's model fails the check on the interpreter, whose derivation is a third, and the bug is confirmed.

## Negative Scope

- mente-smt depends on `mente-syntax` alone (`test_crates_depend_only_on_allowed_crates`), and shares no code with mente-interp's `kinds.rs` or with mente-sema. This is a second derivation, written from `core-calculus.md`, items 4 and 5, not a copy of either.
- The checker, Tier 1 and the interpreter do not change, even where the derivations disagree. Each disagreement is a finding: write its case into the governing spec item before any path changes (CLAUDE.md, invariant 6), and report it.
- The queries' text, the keys `mn smt --independent --keys` writes and the comparison with the base do not change.

## Not Empowered

- `spec/**`, `CLAUDE.md`, `compiler/mente-sema/**`, `compiler/mente-interp/**`, `compiler/mente-smt/Cargo.toml` and `.github/**`.
- The leaves' types stay the checker's, as they are the interpreter's. Taking them from the syntax tree alone would need name resolution in mente-smt, which is a decision for the register, not this round.

## Invariants

- [ ] `Facts` holds the leaves' types alone. `smt_independent::facts` reads from `CheckedFn::typing` and `CheckedFn::resolution` only what a leaf needs, and a test names each field it reads.
- [ ] Each rule of the derivation has a unit test against `core-calculus.md`, items 4 and 5:
  - every arithmetic and bit operator;
  - a shift, which takes its left operand's type;
  - a comparison, which gives no integer;
  - `if` and `match`, which join their branches, and an `if` without `else`, which gives no integer;
  - an array literal, a repeat and an index;
  - literals alone;
  - the mixed join.
- [ ] A test hands the encoding leaf types alone for the construct programs, and its queries are the same as before this round. The encoding reads no type the checker derives.
- [ ] The agreement tests, `test_each_query_holds_exactly_of_the_inputs_whose_run_traps_there` and the corpus's, find no disagreement. The CI comparison with the base shows no check verified less, or a `Smt-Drop:` trailer names each one with its reason.
- [ ] The register entry `smt-independent-encoding` says which types the encoding takes from the checker and which it derives.
