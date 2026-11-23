---
date: 2026-11-14
fixed-by: https://github.com/Menteion/Mente/pull/129
area: checker
missed-by: 'No lint: a walker''s `_` arm hid the operand kinds, and the checker rejected an operand that leaves its statement.'
guard: fn test_exhaustive_crates_deny_wildcard_arms in mente-cli
root: held
---

The fix denied `clippy::wildcard_enum_match_arm` at the root of the crates `EXHAUSTIVE_CRATES` names in `compiler/mente-cli/tests/lints.rs`, mente-sema, mente-interp, mente-mir and mente-codegen, a hard gate there. The cause's form still stands on `main` outside them: `Tree::children` in `compiler/mente-syntax/src/tree.rs`, the one place the tree's shape is read, ends its match over `StmtKind` with `_ => {}`, and mente-cli's walkers of the tree, `compiler/mente-cli/src/mir_run/coverage.rs` among them, match its kinds with `_` arms too. So the class is held, not removed. The brief `wildcard-arms-denied-in-every-walker` extends the deny to mente-syntax and to mente-cli's walkers; its pull request turns this record to `removed`.
