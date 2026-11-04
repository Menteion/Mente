---
title: Micro-Architectural Hardware Contracts and Unified Kernel Compute
version: 0.1.0
status: active
date: 2026-11-04
---

# Micro-Architectural Hardware Contracts and Unified Kernel Compute

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`, `compiler/mente-codegen`

## 1. Specification

1. **Hard compile-time micro-architectural contracts (`Implemented` in `mente-syntax`, `mente-sema`, and `mente-mir`)**:
   - Hardware execution attributes in Mente are **hard static proofs**, never best-effort hints:
     - **`@branchless`**: Forbids conditional jump instructions (`Jcc`, `MirOp::BranchCond`) anywhere in the annotated function or block (`MirRegionContract::verify_branchless_region`). All `if`/`match` expressions inside `@branchless` MUST lower to `CMOV` / `CSEL` or SIMD mask `Select` instructions (`MirOp::Select`).
     - **`@simd` / `@simd(lanes: N)`**: Requires the annotated `for` loop (`for i in 0..n @simd`) to vectorize without loop-carried dependencies or scalar fallback.
     - **`@no_alloc`**: Transitively forbids heap allocation across the entire static call graph of the function.
     - **`@max_stack(bytes)`**: Enforces an exact compile-time upper bound on call-graph stack frame consumption.
2. **Unified heterogeneous compute (`@kernel`, `Implemented` in `mente-syntax`, `mente-codegen`, and `examples/02_mechanical_sympathy_simd.mn`)**:
   - Functions annotated with `@kernel` are written in standard `.mn` syntax with full access to refinement intervals (`u32[0..N]`), fixed-point types (`q16.16`), and dimensional units (`unit`).
   - `compiler/mente-codegen` lowers `@kernel` functions from `MIR` directly to native CPU SIMD loops (AVX-512 / NEON) and GPU/NPU SPIR-V 1.6 compute modules (`emit_spirv_compute_stub`) without external CUDA C++, HLSL, or WGSL string shaders.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Advisory compiler pragmas (`#pragma clang loop vectorize`, `#[inline]`)** | Advisory pragmas silently degrade to slow scalar branches when a minor code change introduces an alias or branch, causing sudden 10x production regressions without warning. Hard contracts abort compilation immediately with an actionable diagnostic. |
| **Separate GPU shading languages (CUDA / WGSL / Metal)** | Forces teams to duplicate data structures, constant tables, and numerical logic across host (Rust/C++) and device (CUDA/WGSL) codebases without shared refinement or unit checking. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn verify_branchless" min="1" reason="MIR verifies zero conditional branches inside @branchless regions" -->
<!-- @assert-count target="compiler/mente-codegen/src/lib.rs" symbol="pub fn emit_spirv_binary" min="1" reason="Codegen emits SPIR-V 1.6 compute words directly from MIR for @kernel functions" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="@branchless" min="1" reason="Example 02 enforces @branchless contract on integrate_cell" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="@kernel" min="1" reason="Example 02 declares a unified @kernel compute function" -->
