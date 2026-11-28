---
id: wildcard-arms-denied-in-every-walker
status: deferred
trigger: when pull request 225, which changes compiler/mente-syntax/src/tree.rs, has merged
type: chore
affectedFiles: [compiler/mente-syntax/src/**, compiler/mente-cli/src/main.rs, compiler/mente-cli/src/mir_run/**, compiler/mente-cli/tests/lints.rs, docs/escapes/walker-wildcard-hid-operands.md, briefs/wildcard-arms-denied-in-every-walker.md, briefs/archive/wildcard-arms-denied-in-every-walker.md]
protectedFiles: [spec/**, CLAUDE.md, Cargo.toml]
---

# Deny the wildcard arm in every walker of the syntax tree

## Intent

The escape `walker-wildcard-hid-operands`: a walker's `_` arm hid operand kinds, and the checker refused an operand that leaves its statement. Its fix denied `clippy::wildcard_enum_match_arm` at the root of the crates `EXHAUSTIVE_CRATES` names in `compiler/mente-cli/tests/lints.rs`: mente-sema, mente-interp, mente-mir and mente-codegen. The same form still stands on `main` outside them, so the record is `held`:
- `compiler/mente-syntax/src/tree.rs`'s `Tree::children` ends its match over `StmtKind` with `_ => {}`. It is the one place the tree's shape is read, and `Tree::subtree` (the oracle's quiet set) and `Tree::too_deep` read it;
- mente-cli's walkers of the tree match its kinds with `_` arms too, `compiler/mente-cli/src/mir_run/coverage.rs` among them.

`lints.rs` says the deny stands at the root of each crate that walks the tree, and the decision [`guard-per-problem`](../docs/design-review.md#guard-per-problem) says the same; this round makes it so.

When this round is done:
- mente-syntax denies the lint at its root, and joins `EXHAUSTIVE_CRATES`;
- each module of mente-cli that matches an enum of mente-syntax's tree (`ExprKind`, `StmtKind`, `TypeKind` and the rest) denies it at its top. The test finds those modules by the enums they name, so a new walker cannot stand without the deny, planted;
- every `_` arm the lint finds is replaced by the variants it stood for, each walked or named as left out with why;
- the record `walker-wildcard-hid-operands` turns to `removed` and names this round's pull request in its prose.

Its owner is instruments-b, who built the walker lint, once pull request 225 has merged.

## Negative Scope

- No change to what any walker visits beyond the variants a `_` arm hid; a walk that changes is a finding of its own, with its test.
- No new lint and no change to the workspace's lint table in `Cargo.toml`.

## Invariants

- [ ] mente-syntax's root denies `clippy::wildcard_enum_match_arm`, and `EXHAUSTIVE_CRATES` names it.
- [ ] Every module of mente-cli that matches an enum of mente-syntax's tree denies the lint at its top, found by the test from the enums it names, with a module planted without it refused.
- [ ] No `_` arm over an enum stands in those crates and modules, and clippy passes with `-D warnings`.
- [ ] `docs/escapes/walker-wildcard-hid-operands.md` is `removed`, naming this round's pull request.
