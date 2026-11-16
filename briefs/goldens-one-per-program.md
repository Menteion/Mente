---
id: goldens-one-per-program
status: draft
type: chore
awaits: [docs/design-review.md#records-one-per-file]
affectedFiles: [compiler/mente-cli/tests/golden/**, compiler/mente-cli/tests/reproducible.rs, compiler/mente-cli/src/records.rs, compiler/mente-cli/tests/records.rs, briefs/goldens-one-per-program.md, briefs/archive/goldens-one-per-program.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**, compiler/mente-cli/tests/lints.rs, compiler/mente-cli/tests/gates.rs]
---

# Keep each program's golden outputs in a file of its own

## Intent

The golden outputs `tests/golden/mir.txt`, `tests/golden/diagnostics.txt` and `tests/golden/projections.txt` each hold every program of the corpus, so every change that adds a program or changes its lowering edits the same files. `mn pressure` ranked `diagnostics.txt` among the files the most merged pull requests change. Each becomes one file a program, under a directory per kind of output, named by the key of the program's path: `records::key_of` of its names, without `.mn`, each split into words at `-` and `_`, so that `conformance/core-calculus/array_beyond_4_gib.mn` is `conformance-z-core-calculus-z-array-beyond-z4-gib`. Since a name's `-` and `_` give one separator, a test refuses two programs whose paths give one key, and says to rename one. `records.rs` declares them as kinds and drops them from `SHARED`.

## Negative Scope

- No change to what is compared: the same outputs, byte for byte, split by program.
- No change to the program corpus or to how a golden file is blessed (`MENTE_BLESS=1`).
- The file list above is cleared with the manager against stack-budget's before the round starts.

## Invariants

- [ ] Every program of the corpus has its golden file and every golden file has its program: a test refuses an orphan either way.
- [ ] The reproducibility tests pass on every runner of the matrix.
- [ ] `node tools/quick.mjs` passes before the push.
