---
title: First-Principles Design Philosophy and Founding Axioms
version: 0.1.0
status: active
date: 2026-11-04
---

# First-Principles Design Philosophy and Founding Axioms

**Governs:** Workspace-wide architecture (`compiler/*`, `examples/*`, `spec/*`)

## 1. Mission statement

Mente (`.mn`) is a universal, statically verified, hardware-sympathetic programming language designed from first principles to resolve three chronic engineering failure modes of existing systems and application languages:

1. **The ergonomic vs. mechanical-sympathy divide**: High-level languages (Python, TypeScript, Go, C#) sacrifice cache locality, deterministic deallocation, and SIMD guarantees for developer velocity, whereas low-level languages (C++23, Rust, Zig) burden application logic with lifetime sigils (`<'a>`), borrow-checker archaeology, or unchecked pointer arithmetic.
2. **Runtime boundary and contract redundancy**: Traditional languages re-evaluate array bounds, null checks, and defensive preconditions at runtime on every call instead of proving them once at compile time and erasing them from the instruction stream.
3. **Opaque compiler intermediate representations**: Upstream C++ compiler infrastructures (`LLVM` / `MLIR`) represent programs as pointer-heavy heap object graphs that cannot natively reason about value refinement intervals, hardware cache-line alignment, or branchless execution contracts.

## 2. The five founding axioms

1. **Zero-compromise universality**: A single language syntax and type system MUST scale continuously from high-level domain pipelines and network services down to 64-byte L1 cache-line structures and `@branchless` SIMD loops. Reaching GPU/NPU compute through `@kernel` is a `Hypothesis` until it has an execution model (`hardware-contracts.md`).
2. **Proof-erased zero-cost safety**: Every memory, bounds, dimensional unit, and precondition check (`requires` / `ensures`) MUST be discharged statically at compile time whenever provable, emitting zero runtime branch instructions and feeding proven intervals into the optimizer (`Contract-Guided Optimization`).
3. **Sigil-free local reasoning**: Ownership (read-only by default, `mut`, `own`), mutation, error propagation (`!`), and side-effect capabilities (`with IO, Net`) MUST be explicit at function signatures and call sites without reference sigils (`&`, `&mut`, `*`) or viral lifetime parameters (`<'a>`).
4. **Hardware mechanical sympathy as a static contract**: Data layout (`AoS`, `SoA`, `Tiled`, `@layout`, `bitstruct`) and micro-architectural execution constraints (`@branchless`, `@simd`, `@no_alloc`, `@max_stack`) are compile-time verified invariants, never silent compiler hints.
5. **Canonical single-path representation**: Every semantic concept has exactly one canonical syntactic form, making `.mn` source code unambiguous for both human engineers and automated synthesis/verification agents.

## 3. Goal: the general-purpose language best suited to AI agents

- Mente aims to be the general-purpose programming language best suited to AI agents writing code, while people review the result and decide whether to trust it ([`ai-first-purpose`](../docs/design-review.md#ai-first-purpose), decided 2026-11-08).
- "Best suited" is a claim, so it is measured: by the agent benchmark of [`success-workload`](../docs/design-review.md#success-workload), against Rust and a control language, under its protocol in `ROADMAP.md`.
- The goal does not narrow axiom 1. Mechanical sympathy stays a principle of the whole language: defaults fit the hardware, hardware costs are visible to agents and reviewers, and the hardware contracts are its strongest form. Agents are weakest at exactly this knowledge, which other languages leave implicit, so making it explicit and checked is part of what suits Mente to them.
- **The core subset.** No model has been trained on Mente, so an agent learns every mechanism from its context, and the language's surface area is the main cost of the goal. An agent can write correct, reasonably fast code using only the core of the language: types, `val` and `var`, parameter modes, `?T`, `T ! E` and effects. Refinements, units, explicit layouts and hardware contracts appear only where the code asks for them, and a refinement or a contract asks for a compile-time proof only where it is written. The agent benchmark scores tasks that need only the core apart from those that use the other mechanisms.

<!-- @assert-present file="Cargo.toml,compiler/mente-syntax/src/lib.rs,compiler/mente-ast/src/lib.rs,compiler/mente-sema/src/lib.rs,compiler/mente-mir/src/lib.rs,compiler/mente-codegen/src/lib.rs,compiler/mente-cli/src/main.rs" reason="All six core Mente compiler crates exist in the workspace" -->
