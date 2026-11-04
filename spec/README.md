---
title: Mente Language Specification Index
version: 0.1.0
status: active
date: 2026-11-04
---

# Mente Language Specification

**Canonical specification of the Mente programming language (`.mn`) and the Mente Intermediate Representation (`MIR`).**

| Document control | |
| :--- | :--- |
| Version | 0.1.0 |
| Status | Active (living specification; verified by `mn spec` and `@spec-family`) |
| Date | 2026-11-04 |
| Implementation status | [docs/STATUS.md](../docs/STATUS.md) |
| Requirement language | The key words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are to be interpreted as described in BCP 14 (RFC 2119, RFC 8174). |
| License | MIT |

## How to read this specification

Each file in `spec/` defines **one orthogonal architectural mechanism and design decision** of Mente, combining its normative specification, its rationale and rejected alternatives, and its executable `<!-- @assert-* -->` verification directives in a single self-contained document.

Every claim carries one of four **maturity labels**:

| Label | Meaning | Evidence required |
| :--- | :--- | :--- |
| **Implemented** | Exists in `compiler/` or `examples/` today and is verified by a unit test, a compile-time assertion, or an executable `<!-- @assert-* -->` directive. | Source path plus a test or `@assert-*` directive. |
| **Specified** | The grammar, type rule, or ABI invariant is fixed in `spec/`, awaiting full lowering in a subsequent milestone. | A normative section in `spec/*.md`. |
| **Target** | A measurable performance or latency goal with a stated measurement protocol. | A benchmark protocol in [ROADMAP.md](../ROADMAP.md). |
| **Hypothesis** | An architectural assumption to be validated empirically against a baseline corpus. | A documented evaluation criterion. |

Where a specification document and the source tree disagree, the source tree is authoritative. Claims about the source tree are executable via `cargo run --bin mn -- spec` (`guard`, `prove`, and `graph`).

<!-- @assert-present file="spec/philosophy.md,spec/holographic-module.md,spec/surface-syntax.md,spec/refinement-and-fixed-point.md,spec/dimensional-units.md,spec/algebraic-types-and-null.md,spec/traits-and-generics.md,spec/mutable-value-semantics.md,spec/regions-and-pools.md,spec/morphic-layouts.md,spec/hardware-contracts.md,spec/error-handling-abi.md,spec/ghost-contracts-and-totality.md,spec/algebraic-effects.md,spec/structured-concurrency.md,spec/disjoint-parallelism.md,spec/comptime-and-ffi.md,spec/mir-and-codegen.md" reason="All eighteen canonical single-decision specification files exist in flat spec/ directory" -->

## Atomic specification files and crate authority map

| Specification file | Orthogonal mechanism / decision | Governed crates |
| :--- | :--- | :--- |
| [philosophy.md](philosophy.md) | First-principles design philosophy and the five founding axioms | Workspace-wide |
| [holographic-module.md](holographic-module.md) | Single `.mn` file model and AST dual hashing (`spec_hash` vs `body_hash`) | `compiler/mente-syntax`, `compiler/mente-ast` |
| [surface-syntax.md](surface-syntax.md) | `{}` semicolon-free blocks, `val`/`var`, `->`, `guard`, `\|>`, `and`/`or`/`not` | `compiler/mente-syntax`, `compiler/mente-ast` |
| [refinement-and-fixed-point.md](refinement-and-fixed-point.md) | Bounded interval types (`u32[0..N]`), dependent slice indexing, and fixed-point (`q16.16`) | `compiler/mente-sema`, `compiler/mente-mir` |
| [dimensional-units.md](dimensional-units.md) | Zero-cost physical and domain dimensional unit algebra (`unit`) | `compiler/mente-syntax`, `compiler/mente-sema` |
| [algebraic-types-and-null.md](algebraic-types-and-null.md) | `struct`, `enum`, flow-sensitive union narrowing (`is`), and `.unwrap()`-free `?T` | `compiler/mente-syntax`, `compiler/mente-sema` |
| [traits-and-generics.md](traits-and-generics.md) | Angle-bracket generics `<T>` (no `::<>`) and static-default `trait` / explicit `dyn Trait` | `compiler/mente-syntax`, `compiler/mente-sema` |
| [mutable-value-semantics.md](mutable-value-semantics.md) | Sigil-free Mutable Value Semantics (`Let` / `mut` / `own`) and Law of Exclusivity | `compiler/mente-ast`, `compiler/mente-sema` |
| [regions-and-pools.md](regions-and-pools.md) | Lifetime-free provenance inference, lexical `Arena`, and generational `Pool<T>`/`Handle<T>` | `compiler/mente-sema` |
| [morphic-layouts.md](morphic-layouts.md) | Morphic containers (`AoS`/`SoA`/`Tiled`), 64B `@layout`/`@bytes`, and `bitstruct`/`@bits` | `compiler/mente-sema`, `compiler/mente-mir` |
| [hardware-contracts.md](hardware-contracts.md) | Micro-architectural contracts (`@branchless`, `@simd`, `@no_alloc`, `@max_stack`) and `@kernel` | `compiler/mente-sema`, `compiler/mente-mir`, `compiler/mente-codegen` |
| [error-handling-abi.md](error-handling-abi.md) | Orthogonal `T ! E` error model and CPU Carry-Flag (`CF`) zero-overhead register ABI | `compiler/mente-sema`, `compiler/mente-codegen` |
| [ghost-contracts-and-totality.md](ghost-contracts-and-totality.md) | Two-tier SMT ghost contracts (`requires`/`ensures` erasure) and `@total`/`decreases` | `compiler/mente-sema`, `compiler/mente-mir` |
| [algebraic-effects.md](algebraic-effects.md) | Algebraic capability effects (`with IO, Net, Clock, Rand`) and single-line DST | `compiler/mente-sema` |
| [structured-concurrency.md](structured-concurrency.md) | Colorless concurrency (no `async`/`await`) and lexical `parallel (...)` task trees | `compiler/mente-syntax`, `compiler/mente-sema` |
| [disjoint-parallelism.md](disjoint-parallelism.md) | SMT-proven lock-free disjoint slice parallelism (`parallel_for`), `Channel`, and `Mutex<T>` | `compiler/mente-sema`, `compiler/mente-mir` |
| [comptime-and-ffi.md](comptime-and-ffi.md) | Zero-macro pure `.mn` `comptime` reflection, `extern` FFI quarantine, and `mn` CLI | `compiler/mente-sema`, `compiler/mente-cli` |
| [mir-and-codegen.md](mir-and-codegen.md) | 64B `SoA` `Mente IR (MIR)` (`NodeId(u32)`), RVSDG/E-Graphs, mutation prover, and SPIR-V | `compiler/mente-mir`, `compiler/mente-codegen` |
