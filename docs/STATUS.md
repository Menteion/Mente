---
title: Mente Implementation Status
version: 0.1.0
status: active
date: 2026-11-04
---

# Mente — Implementation Status (`docs/STATUS.md`)

**Single source of live implementation status across `compiler/`, `examples/`, and `spec/`.** It says what the compiler does today; `spec/*.md` says what the language is meant to be.

| Field | Value |
| :--- | :--- |
| Current milestone | M1, checkpoint (a), under way: the lexer of M1 and the body parser are done. Before it, M0b, done (tag `m0b`): the core calculus and the grammar of M1, the agent-benchmark corpus, the Tier-1 latency spike and the Cranelift debug-information spike, after M0a's documentation tools, CI, claim evidence, Rust hygiene, structured diagnostics, conformance suite and benchmarks (tag `m0a`). The plan was reviewed against M0's evidence on 2026-11-09 and stands ([`milestone-reviews.md`](milestone-reviews.md)). |
| Immediate next step | M1, the thin slice, from checkpoint (a): the syntax probe on the public M1 tasks before anything builds on the syntax, name resolution and the type checker, `todo`, the reference interpreter `mente-interp`, the incremental architecture with `mn check --watch`, and `mn explain`, as [`ROADMAP.md`](../ROADMAP.md) sets out. The register entries M1 needs are decided during M0, in [`docs/design-review.md`](design-review.md) |
| Specification | One single-decision file per mechanism in [`spec/`](../spec/README.md), plus `README.md` and `philosophy.md`; `cargo run --bin mn -- spec` prints the current file and directive counts |
| Compiler crates | 6 Rust crates (`mente-syntax`, `mente-ast`, `mente-sema`, `mente-mir`, `mente-codegen`, `mente-cli`) |
| Showcase `.mn` modules | [`examples/`](../examples/README.md). Their declarations pass `mn check`; their bodies use constructs of later milestones, and `mn test` checks that each reports exactly what its `.expected` file lists ([`example-bodies`](design-review.md#example-bodies)) |

---

## What the compiler does not do yet

These gaps hold across every crate, and every `spec/*.md` claim that depends on them is labelled `Specified`:

- **Bodies are parsed, not checked.** The body parser reads every function with a body under M1's grammar and reports what M1 does not accept; `@example` bodies stay token lists, and the declaration parser keeps types, contracts and effect clauses as raw strings for the digests.
- **No type checker.** Refinement intervals, units, `?T`, `T ! E`, effects, parameter modes and the Law of Exclusivity are not checked.
- **No lowering.** No `.mn` code reaches MIR. The MIR region `mn test` runs is built by hand in `compiler/mente-cli/src/main.rs`, with hand-written intervals.
- **No code generation.** `mente-codegen` is empty until M1 lowers MIR to Cranelift.
- **No idiom diagnostics yet.** The lexer reports a character Mente does not use (`E0105`), so `<'a>` is rejected; `&mut x` and `let` reach the body parser as an expression it does not expect, not yet as the idioms of `surface-syntax.md`, item 8.

---

## Crate-by-Crate Maturity Ledger

| Crate | Path | Implemented today | Not yet (specified) |
| :--- | :--- | :--- | :--- |
| **`mente-syntax`** | [`compiler/mente-syntax/src/lib.rs`](../compiler/mente-syntax/src/lib.rs) | The lexer of `spec/grammar.md` item 1, with its lexical errors (`E0105`–`E0108`), and the declaration parser: `module`, `unit` (raw text), `struct` (fields, defaults, `@bytes`, `invariant` lines), `bitstruct` (`@bits`), `enum`, `trait` (method signatures as text), `fn` (generics, parameters with `mut`/`own`/`in` modes, `-> T ! E`, `from` provenance, `with` list, `requires`/`ensures`/`decreases` lines, `= expr` bodies), `@example`. Spans for declarations, fields and attributes; the structured `Diagnostic` with text and JSON forms and fixes (`spec/diagnostics.md`). | Statement and expression parsing of bodies; `extern` blocks. |
| **`mente-ast`** | [`compiler/mente-ast/src/lib.rs`](../compiler/mente-ast/src/lib.rs) | 128-bit two-lane FNV-style `spec_hash`, `instance_hash` and `body_hash` over the parsed declarations (a private field of a `pub struct` and a `from` clause are part of `spec_hash`; generic, trait-taking and `@inline` bodies go into `instance_hash`), and `project_spec_blueprint`. Projection median 45–92 µs per example module under the `ROADMAP.md` protocol (2026-11-05), 38–60% fewer tokens. | A typed AST; module dependency caching keyed on `spec_hash`. |
| **`mente-sema`** | [`compiler/mente-sema/src/lib.rs`](../compiler/mente-sema/src/lib.rs) | `@layout(size, align)` + `@bytes` prover (`E0410`–`E0419`, field alignment and `@packed` included; field sizes from primitives, whole-byte `uN`/`iN`, fixed-point, `sat`/`wrap`, arrays and the module's own bitstructs and `@layout` structs), `bitstruct` `@bits` prover (`E0420`–`E0425`, field type width included), and the `from` clause check (`E0311`). Token checks: `E0501` for a `@total` function with `while` and no `decreases`; `E0401`/`E0402` for a `@branchless` function containing `while`/`raise`. | Type checking, interval and DBM solving, units, MVS exclusivity, provenance, effects. The `@total` check does not verify the measure, and the `@branchless` check does not see `if`, `match` or calls. |
| **`mente-mir`** | [`compiler/mente-mir/src/lib.rs`](../compiler/mente-mir/src/lib.rs) | Straight-line `MirRegionSoA` (parallel `Vec`s indexed by `NodeId(u32)`; only the header is 64-byte aligned), 12 opcodes, `ValueLattice` record and `proves_in_bounds` predicate, `MorphicLayout::byte_offset`, `verify_branchless` (rejects `BranchCondJump`), `eval_in_memory`, `run_in_memory_mutation_gate`. | Control flow and nested regions (RVSDG), memory operations and calls, interval inference, e-graph rewriting, AST-to-MIR lowering. |
| **`mente-codegen`** | [`compiler/mente-codegen/src/lib.rs`](../compiler/mente-codegen/src/lib.rs) | Nothing: the crate is empty. | Lowering MIR to Cranelift for native code (M1, checkpoint (c)). SPIR-V waits for `@kernel`'s execution model. |
| **`mente-cli`** | [`compiler/mente-cli/src/main.rs`](../compiler/mente-cli/src/main.rs) | `mn check` (`--json` for the versioned diagnostic form), `mn explain` (the error-code catalogue), `mn view --spec`, `mn test [--mutate]` (runs `mn check`, then the hand-written demo region and its mutation gate), and `mn spec` (front matter, at least one directive per file, every directive run and rejected if it cannot fail). | `mn build`, `mn fmt`, `mn lsp`, `mn prove`; executing `@example` bodies. |
