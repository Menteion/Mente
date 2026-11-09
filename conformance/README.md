# Conformance suite

Programs that must pass and programs that must fail with a given code, one directory per spec file: `conformance/morphic-layouts/` holds the programs for `spec/morphic-layouts.md`. They test the language as the specification states it, not one compiler, so the self-hosted compiler will run the same suite unchanged (`docs/design-brief.md`, "The quality system").

## A program

The first line states what checking it must report:

```text
// expect: pass
// expect: E0413
```

`pass` means no diagnostic at all. Codes, separated by spaces, mean exactly those diagnostics, in that order: one code for one diagnostic, the usual case, and more for a program that also uses a construct of a later milestone (`E0109`), as `// expect: E0402 E0109 E0109` does. The rest of the file is an ordinary `.mn` module; keep each program to the one thing it tests.

## The runner

`compiler/mente-cli/tests/conformance.rs` runs every program through `mn check --json` and compares the reported codes with the first line. It reads nothing else from the compiler, so it can run against another implementation of the same command:

```powershell
cargo test -p mente-cli --test conformance
$env:MENTE_COMPILER = "path/to/another/mn"; cargo test -p mente-cli --test conformance
```

Every code the compiler reports has at least one program here, in the directory of the spec that governs it; a test in `compiler/mente-cli/src/catalogue.rs` holds the suite to that. A new spec claim brings its programs, both ones that must pass and ones that must fail.
