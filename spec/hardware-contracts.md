---
title: Micro-Architectural Hardware Contracts and Unified Kernel Compute
version: 0.1.0
status: active
date: 2026-11-04
---

# Micro-Architectural Hardware Contracts and Unified Kernel Compute

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`, `compiler/mente-codegen`

## 1. Specification

1. **Hard compile-time micro-architectural contracts (`Specified`)**:
   - Hardware execution attributes in Mente are **hard static proofs**, never best-effort hints:
     - **`@branchless`**: Forbids conditional jump instructions (`Jcc`) anywhere in the annotated function or block. All `if`/`match` expressions inside `@branchless` MUST lower to `CMOV` / `CSEL` or SIMD mask select instructions.
     - **`@simd` / `@simd(lanes: N)`**: Requires the annotated `for` loop (`for i in 0..n @simd`) to vectorize without loop-carried dependencies or scalar fallback.
     - **`@no_alloc`**: Transitively forbids heap allocation across the entire static call graph of the function.
     - **`@max_stack(bytes)`**: Enforces an exact compile-time upper bound on call-graph stack frame consumption.
   - `Implemented` for `@branchless` only, and only as two partial checks: `verify_function_contracts` in `mente-sema` rejects a `@branchless` function whose body tokens contain `while` (`E0401`) or `raise` (`E0402`), and `MirRegionSoA::verify_branchless` in `mente-mir` rejects a region containing a `BranchCondJump` node (`E0401`). Neither sees an `if`, a `match` or a call, and no `.mn` body is lowered to MIR yet. `@simd`, `@no_alloc` and `@max_stack` are parsed as attributes and checked by nothing.
2. **Unified heterogeneous compute (`@kernel`, `Specified`)**:
   - Functions annotated with `@kernel` are written in standard `.mn` syntax with full access to refinement intervals (`u32[0..N]`), fixed-point types (`q16.16`), and dimensional units (`unit`).
   - `compiler/mente-codegen` lowers `@kernel` functions from `MIR` directly to native CPU SIMD loops (AVX-512 / NEON) and GPU/NPU SPIR-V 1.6 compute modules without external CUDA C++, HLSL, or WGSL string shaders.
   - Today `mn check` only counts `@kernel` functions. `emit_spirv_binary` in `mente-codegen` writes a SPIR-V-shaped word stream that is not a valid module.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Advisory compiler pragmas (`#pragma clang loop vectorize`, `#[inline]`)** | Advisory pragmas silently degrade to slow scalar branches when a minor code change introduces an alias or branch, causing sudden 10x production regressions without warning. Hard contracts abort compilation immediately with an actionable diagnostic. |
| **Separate GPU shading languages (CUDA / WGSL / Metal)** | Forces teams to duplicate data structures, constant tables, and numerical logic across host (Rust/C++) and device (CUDA/WGSL) codebases without shared refinement or unit checking. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn verify_branchless" min="1" reason="MIR rejects a @branchless region that contains a BranchCondJump node" -->
<!-- @assert-count target="compiler/mente-codegen/src/lib.rs" symbol="pub fn emit_spirv_binary" min="1" reason="Codegen has a placeholder SPIR-V word writer; it is not a valid module" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="@branchless" min="1" reason="Example 02 enforces @branchless contract on integrate_cell" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="@kernel" min="1" reason="Example 02 declares a unified @kernel compute function" -->
