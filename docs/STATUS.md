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
| Current milestone | Before M0: the declaration parser, the digests and projection, and the layout provers exist; function bodies are not parsed |
| Immediate next step | M0a (the documentation tools, CI and Rust hygiene), then M0b (design closure for M1), then M1, as [`ROADMAP.md`](../ROADMAP.md) sets out. The register entries M1 needs are decided during M0, in [`docs/design-review.md`](design-review.md) |
| Specification | One single-decision file per mechanism in [`spec/`](../spec/README.md), plus `README.md` and `philosophy.md`; `cargo run --bin mn -- spec` prints the current file and directive counts |
| Compiler crates | 6 Rust crates (`mente-syntax`, `mente-ast`, `mente-sema`, `mente-mir`, `mente-codegen`, `mente-cli`) |
| Showcase `.mn` modules | [`examples/`](../examples/README.md). `mn check` verifies their declarations only; their bodies show the intended language and are not checked |

---

## What the compiler does not do yet

These gaps hold across every crate, and every `spec/*.md` claim that depends on them is labelled `Specified`:

- **No body parser.** `mente-syntax` keeps each function and `@example` body as a flat token list, and types, contracts and effect clauses as raw strings. `mn check` accepts a body of arbitrary tokens.
- **No type checker.** Refinement intervals, units, `?T`, `T ! E`, effects, parameter modes and the Law of Exclusivity are not checked.
- **No lowering.** No `.mn` code reaches MIR. The MIR region `mn test` runs is built by hand in `compiler/mente-cli/src/main.rs`, with hand-written intervals.
- **No code generation.** `mente-codegen` emits no machine code, and its SPIR-V writer does not produce a valid module.
- **Silent lexer.** Characters the lexer does not know (`&`, `'`, `^`, `~`) are skipped without an error, so `&mut x` and `<'a>` are not rejected.

---

## Crate-by-Crate Maturity Ledger

| Crate | Path | Implemented today | Not yet (specified) |
| :--- | :--- | :--- | :--- |
| **`mente-syntax`** | [`compiler/mente-syntax/src/lib.rs`](../compiler/mente-syntax/src/lib.rs) | Lexer and declaration parser: `module`, `unit` (raw text), `struct` (fields, defaults, `@bytes`, `invariant` lines), `bitstruct` (`@bits`), `enum`, `trait` (method signatures as text), `fn` (generics, parameters with `mut`/`own`/`in` modes, `-> T ! E`, `from` provenance, `with` list, `requires`/`ensures`/`decreases` lines, `= expr` bodies), `@example`. | Statement and expression parsing of bodies; `extern` blocks; diagnostics for unknown characters. |
| **`mente-ast`** | [`compiler/mente-ast/src/lib.rs`](../compiler/mente-ast/src/lib.rs) | 128-bit two-lane FNV-style `spec_hash`, `instance_hash` and `body_hash` over the parsed declarations (a private field of a `pub struct` and a `from` clause are part of `spec_hash`; generic, trait-taking and `@inline` bodies go into `instance_hash`), and `project_spec_blueprint`. Projection median 45–92 µs per example module under the `ROADMAP.md` protocol (2026-11-05), 38–60% fewer tokens. | A typed AST; module dependency caching keyed on `spec_hash`. |
| **`mente-sema`** | [`compiler/mente-sema/src/lib.rs`](../compiler/mente-sema/src/lib.rs) | `@layout(size, align)` + `@bytes` prover (`E0410`–`E0419`, field alignment and `@packed` included; field sizes from primitives, whole-byte `uN`/`iN`, fixed-point, `sat`/`wrap`, arrays and the module's own bitstructs and `@layout` structs), `bitstruct` `@bits` prover (`E0420`–`E0425`, field type width included), and the `from` clause check (`E0311`). Token checks: `E0501` for a `@total` function with `while` and no `decreases`; `E0401`/`E0402` for a `@branchless` function containing `while`/`raise`. | Type checking, interval and DBM solving, units, MVS exclusivity, provenance, effects. The `@total` check does not verify the measure, and the `@branchless` check does not see `if`, `match` or calls. |
| **`mente-mir`** | [`compiler/mente-mir/src/lib.rs`](../compiler/mente-mir/src/lib.rs) | Straight-line `MirRegionSoA` (parallel `Vec`s indexed by `NodeId(u32)`; only the header is 64-byte aligned), 12 opcodes, `ValueLattice` record and `proves_in_bounds` predicate, `MorphicLayout::byte_offset`, `verify_branchless` (rejects `BranchCondJump`), `eval_in_memory`, `run_in_memory_mutation_gate`. | Control flow and nested regions (RVSDG), memory operations and calls, interval inference, e-graph rewriting, AST-to-MIR lowering. |
| **`mente-codegen`** | [`compiler/mente-codegen/src/lib.rs`](../compiler/mente-codegen/src/lib.rs) | `CarryFlagAbiLowering`, a table naming the registers and flag of the error ABI per target; `emit_spirv_binary`, a SPIR-V-shaped word stream that `spirv-val` would reject. Lowering MIR to Cranelift for native code (Phase 3). SPIR-V waits for `@kernel`'s execution model. |
| **`mente-cli`** | [`compiler/mente-cli/src/main.rs`](../compiler/mente-cli/src/main.rs) | `mn check`, `mn view --spec`, `mn test [--mutate]` (runs `mn check`, then the hand-written demo region and its mutation gate), and `mn spec` (front matter, at least one directive per file, every directive run and rejected if it cannot fail). | `mn build`, `mn fmt`, `mn lsp`, `mn prove`; executing `@example` bodies. |
