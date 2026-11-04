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
     - **`@branchless`**: Forbids every conditional branch whose condition depends on data in the annotated function or block. Concretely:
       - All `if`/`match` expressions MUST lower to `CMOV` / `CSEL` or SIMD mask select instructions. Lowering to a select evaluates every arm, so each arm MUST be total and effect-free: no call that may trap, no `raise`, no `with` effect, no `mut` argument.
       - A loop is allowed only with a compile-time trip count (`for i in 0..N` with `N` a compile-time value), and is fully unrolled; `while` is rejected.
       - Every runtime check that would branch (bounds, overflow, division by zero, a `?T` narrowing) MUST be proven away by Tier 1, or the operation written in a non-trapping form (`sat<T>`, `wrap<T>`, a select).
       - Every callee MUST itself be `@branchless`, or be inlined and meet these rules; a call instruction itself is not a branch.
     - **`@simd` / `@simd(lanes: N)`**: Requires the annotated `for` loop (`for i in 0..n @simd`) to vectorize without loop-carried dependencies or scalar fallback.
     - **`@no_alloc`**: Transitively forbids heap allocation across the entire static call graph of the function.
     - **`@max_stack(bytes)`**: Enforces a compile-time upper bound on call-graph stack consumption. The bound composes from signatures: a `@max_stack` function may call only functions that declare their own `@max_stack`, and its bound must cover its own frame plus the largest bound among its callees. Recursion and `dyn` calls are rejected inside a `@max_stack` call graph unless the trait method declares the bound.
   - `Implemented` for `@branchless` only, and only as two partial checks: `verify_function_contracts` in `mente-sema` rejects a `@branchless` function whose body tokens contain `while` (`E0401`) or `raise` (`E0402`), and `MirRegionSoA::verify_branchless` in `mente-mir` rejects a region containing a `BranchCondJump` node (`E0401`). Neither sees an `if`, a `match` or a call, and no `.mn` body is lowered to MIR yet. `@simd`, `@no_alloc` and `@max_stack` are parsed as attributes and checked by nothing.
2. **Unified heterogeneous compute (`@kernel`, `Specified`)**:
   - Functions annotated with `@kernel` are written in standard `.mn` syntax with full access to refinement intervals (`u32[0..N]`), fixed-point types (`q16.16`), and dimensional units (`unit`).
   - `compiler/mente-codegen` lowers `@kernel` functions from `MIR` directly to native CPU SIMD loops (AVX-512 / NEON) and GPU/NPU SPIR-V 1.6 compute modules without external CUDA C++, HLSL, or WGSL string shaders.
   - Today `mn check` only counts `@kernel` functions. `emit_spirv_binary` in `mente-codegen` writes a SPIR-V-shaped word stream that is not a valid module.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Advisory compiler pragmas (`#pragma clang loop vectorize`, `#[inline]`)** | Advisory pragmas silently degrade to slow scalar branches when a minor code change introduces an alias or branch, causing sudden 10x production regressions without warning. Hard contracts abort compilation immediately with an actionable diagnostic. |
| **`@branchless` as "no conditional jump instruction anywhere"** | Also forbids every loop (its back-edge is a conditional branch) and every bounds or overflow check the compiler inserts, and says nothing about arms that trap once both are evaluated. "No data-dependent branch" is the property timing-predictable and constant-time code needs, and it can be checked: constant trip counts unroll, proven checks vanish, total arms select safely. |
| **Separate GPU shading languages (CUDA / WGSL / Metal)** | Forces teams to duplicate data structures, constant tables, and numerical logic across host (Rust/C++) and device (CUDA/WGSL) codebases without shared refinement or unit checking. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn verify_branchless" min="1" reason="MIR rejects a @branchless region that contains a BranchCondJump node" -->
<!-- @assert-count target="compiler/mente-codegen/src/lib.rs" symbol="pub fn emit_spirv_binary" min="1" reason="Codegen has a placeholder SPIR-V word writer; it is not a valid module" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="@branchless" min="1" reason="Example 02 enforces @branchless contract on integrate_cell" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="@kernel" min="1" reason="Example 02 declares a unified @kernel compute function" -->
