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

<!-- @assert-present file="Cargo.toml,compiler/mente-syntax/src/lib.rs,compiler/mente-ast/src/lib.rs,compiler/mente-sema/src/lib.rs,compiler/mente-mir/src/lib.rs,compiler/mente-codegen/src/lib.rs,compiler/mente-cli/src/main.rs" reason="All six core Mente compiler crates exist in the workspace" -->
