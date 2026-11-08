---
title: Micro-Architectural Hardware Contracts and Kernel Compute
version: 0.1.0
status: active
date: 2026-11-04
---

# Micro-Architectural Hardware Contracts and Kernel Compute

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`, `compiler/mente-codegen`

## 1. Specification

1. `Specified` **Hard compile-time micro-architectural contracts**:
   - Hardware execution attributes in Mente are **hard static proofs**, never best-effort hints:
     - **`@branchless`**: Forbids every conditional branch whose condition depends on data in the annotated function or block. Concretely:
       - All `if`/`match` expressions MUST lower to `CMOV` / `CSEL` or SIMD mask select instructions. Lowering to a select evaluates every arm, so each arm MUST be total and effect-free: no call that may trap, no `raise`, no `with` effect, no `mut` argument.
       - A loop is allowed only with a compile-time trip count (`for i in 0..N` with `N` a compile-time value), and is fully unrolled; `while` is rejected.
       - Every runtime check that would branch (bounds, overflow, division by zero, a `?T` narrowing) MUST be proven away by Tier 1, or the operation written in a non-trapping form (`sat<T>`, `wrap<T>`, a select).
       - Every callee MUST itself be `@branchless`, or be inlined and meet these rules; a call instruction itself is not a branch.
       - Whether a select stays branch-free is finally the backend's choice, so after code generation `mn` disassembles each `@branchless` function's machine code and rejects any conditional jump, naming the source line (`E0403`).
     - **`@simd(lanes: N)`**: Attached to a loop, after its iterable (`for p in mut batch @simd(lanes: 16)`, `parallel_for p in mut batch @simd(lanes: 16)`), never to a function. It requires the loop body to run `N` iterations at a time in vector registers, with no loop-carried dependency and no scalar fallback. A trip count that is not a multiple of `N` runs its last partial group under a lane mask. On a `parallel_for`, each worker vectorizes its own share; over a `Tiled<T, lanes: N>` container the tiles are the unit both of distribution and of vectorization. `N` counts lanes, not register width: the backend splits `N` lanes into as many native vector registers as it needs (four 128-bit registers for 16 `f32` lanes through Cranelift, one `ZMM` register on a backend with AVX-512), and the contract holds either way.
     - **`@no_alloc`**: Forbids heap allocation in the function and everything it calls. It composes from signatures, like every contract here: a `@no_alloc` function may call only functions that declare `@no_alloc` themselves.
     - **`@max_stack(bytes)`**: Enforces a compile-time upper bound on call-graph stack consumption. The bound composes from signatures: a `@max_stack` function may call only functions that declare their own `@max_stack`, and its bound must cover its own frame plus the largest bound among its callees. Recursion and `dyn` calls are rejected inside a `@max_stack` call graph unless the trait method declares the bound.
   - **Contracts through `dyn`, traits and FFI**: a caller only ever sees a callee's signature (`holographic-module.md`), so `@branchless`, `@no_alloc`, `@max_stack` and `@total` reach across a call only when the callee declares them. A trait method that declares a contract requires every implementation to meet it, which is what lets a `dyn` call or a call through a trait-typed parameter satisfy the contract. An `extern` declaration may declare a contract on the foreign function's behalf; the compiler cannot check foreign code, so that declaration is trusted and sits in the FFI quarantine (`comptime-and-ffi.md`).
   - `Implemented` for `@branchless` only, and only as two partial checks: `verify_function_contracts` in `mente-sema` rejects a `@branchless` function whose body tokens contain `while` (`E0401`) or `raise` (`E0402`), and `MirRegionSoA::verify_branchless` in `mente-mir` rejects a region containing a `BranchCondJump` node (`E0401`). Neither sees an `if`, a `match` or a call, and no `.mn` body is lowered to MIR yet. `@simd`, `@no_alloc` and `@max_stack` are parsed as attributes and checked by nothing.
     <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_rejects_branchless_with_while_loop" min="1" reason="A @branchless body with while is rejected with E0401" -->
     <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_rejects_branchless_with_raise" min="1" reason="A @branchless body with raise is rejected with E0402" -->
     <!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="fn test_branchless_region_rejects_conditional_jump" min="1" reason="A @branchless MIR region with a BranchCondJump node is rejected" -->
2. `Hypothesis` **Heterogeneous compute (`@kernel`)**:
   - The hypothesis: functions annotated with `@kernel` can be written in standard `.mn` syntax, with refinement intervals (`u32[0..N]`), fixed-point types (`q16.16`) and dimensional units (`unit`), and lowered from `MIR` to both CPU SIMD loops (AVX-512 / NEON) and GPU/NPU SPIR-V 1.6 compute modules without external CUDA C++, HLSL, or WGSL string shaders.
   - It stays a hypothesis until an execution model is written: the index space a kernel runs over and how it is launched, the memory spaces it may touch (global, workgroup, private), barriers, and how a kernel relates to `parallel_for` and `@simd`. No example uses `@kernel` until then.
   - Today `mn check` only counts `@kernel` functions, and nothing emits SPIR-V.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Advisory compiler pragmas (`#pragma clang loop vectorize`, `#[inline]`)** | Advisory pragmas silently degrade to slow scalar branches when a minor code change introduces an alias or branch, causing sudden 10x production regressions without warning. Hard contracts abort compilation immediately with an actionable diagnostic. |
| **`@branchless` as "no conditional jump instruction anywhere"** | Also forbids every loop (its back-edge is a conditional branch) and every bounds or overflow check the compiler inserts, and says nothing about arms that trap once both are evaluated. "No data-dependent branch" is the property timing-predictable and constant-time code needs, and it can be checked: constant trip counts unroll, proven checks vanish, total arms select safely. |
| **`@simd(width: N)` on a function, beside `@kernel` and an inner `parallel_for`** | Three attributes each claimed the same loop at a different level (GPU grid, vector lanes, worker threads) with no rule relating them. `@simd(lanes: N)` belongs to one loop, and how a worker's share and a tile map onto lanes is written above; `@kernel` waits for its own model. |
| **Separate GPU shading languages (CUDA / WGSL / Metal)** | Forces teams to duplicate data structures, constant tables, and numerical logic across host (Rust/C++) and device (CUDA/WGSL) codebases without shared refinement or unit checking. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn verify_branchless" min="1" reason="MIR rejects a @branchless region that contains a BranchCondJump node" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="@branchless" min="1" reason="Example 02 enforces @branchless contract on integrate_cell" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="parallel_for p in mut batch @simd(lanes: 16)" min="1" reason="Example 02 vectorizes a parallel_for over a Tiled container with @simd(lanes: N) on the loop" -->
<!-- @assert-absence target="examples" symbol="@simd(width" glob="*.mn" reason="The vector width is written @simd(lanes: N), on a loop" -->
<!-- @assert-absence target="examples" symbol="@kernel" glob="*.mn" reason="No example uses @kernel until it has an execution model" -->
