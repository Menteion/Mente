---
id: smt-narrowing-shortcut
status: draft
date: 2026-11-22
type: defect
affectedFiles: [compiler/mente-smt/src/exec.rs, compiler/mente-cli/src/smt_independent.rs, briefs/smt-narrowing-shortcut.md, briefs/archive/smt-narrowing-shortcut.md]
protectedFiles: [CLAUDE.md, spec/**, compiler/mente-sema/**, compiler/mente-interp/**, compiler/mente-smt/Cargo.toml, .github/**]
---

# Restore the shortcut for narrowing an array whose elements narrow nothing

## Intent

Round 7 of pull request 191 removed `narrows` from `narrow_plain` in the independent encoding ([`smt-independent-encoding`](../docs/design-review.md#smt-independent-encoding)). It was taken for an equivalent mutant: an element that narrows nothing, a `bool` or a struct, checks nothing. Fable's review found two differences:
- such an array now walks its elements, and each element adds a `select` term toward the encoding's term limit;
- such an array past 256 elements, whose shape differs from its place's, is now cut off as a construct the encoding lacks.

Both only cut more, and never verify wrongly. When this round is done, the shortcut is back, and a test tells it apart: an array whose elements narrow nothing is narrowed with no term made and no cut.

## Negative Scope

- No other change to `narrow_plain` or to what it checks.

## Not Empowered

- `compiler/mente-sema/**`, `compiler/mente-interp/**`, `spec/**`, `CLAUDE.md`, `.github/**`.

## The Defect, Measured

Encode a function that narrows an array of 300 `bool`s into a place of another shape. Before this round, the path is cut off as lacked. Encode one that narrows an array of `bool`s of its own shape. Before this round, it makes one `select` term per element.

## Invariants

- [ ] Narrowing an array whose elements narrow nothing makes no term and cuts no path, at any length; a test holds both cases above.
- [ ] Each test fails with the shortcut planted away, which is how Mutants first saw it as equivalent.
