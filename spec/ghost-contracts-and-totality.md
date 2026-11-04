---
title: Two-Tier SMT Ghost Contracts and Constructive Totality
version: 0.1.0
status: active
date: 2026-11-04
---

# Two-Tier SMT Ghost Contracts and Constructive Totality

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`

## 1. Specification

1. **First-class contract clauses (`requires`, `ensures`, `invariant`, `Implemented` in `mente-syntax` and `mente-ast`)**:
   - Functions declare preconditions via `requires: <bool-expr>` and postconditions via `ensures(ret): <bool-expr>`.
   - `struct` declarations declare data invariants via `invariant { <bool-expr> }`.
2. **Two-tier verification and 100% Ghost Erasure (`Implemented` in `mente-sema` and `mente-mir`)**:
   - **Tier 1 (`< 5 ms` incremental range/linear solver)**: Runs automatically on every save (`mn check`), propagating `ValueLattice` intervals (`[min_val, max_val]`) and linear inequalities across `guard` checks.
   - **Tier 2 (`mn prove` SMT bit-vector / quantifier-free solver)**: Verifies complex non-linear and bit-level contracts in parallel.
   - **Compile-time caller proof obligation**: Calling a function whose `requires` clause is not statically established in the caller's path context (e.g., via refined input types or a preceding `guard` statement) is a compile-time error.
   - **Ghost erasure and Contract-Guided Optimization (CGO)**: Once proven, `requires` and `ensures` emit **zero runtime instructions** (`0` bytes in the binary) and lower to `MirOp::Assume` nodes that allow `mente-mir` to eliminate downstream redundant bounds checks and dead branches.
3. **Constructive termination (`@total` and `decreases`, `Implemented` in `compiler/mente-sema/src/lib.rs`)**:
   - Functions annotated with `@total` MUST terminate on all valid inputs.
   - Every `while` loop or recursive call inside a `@total` function MUST specify a strictly decreasing well-founded measure (`decreases high - low`). Omitting `decreases` inside a `@total` function triggers a hard compile-time error (`E0501 [Totality Error]` in `compiler/mente-sema/src/lib.rs`).
4. **Tier-1 solver predictability and concrete counterexample diagnostics (`Specified` — Engineering Guardrail #1)**:
   - To eliminate the "SMT butterfly effect" (where a minor code edit causes unpredictable solver timeouts in Dafny/LiquidHaskell), Tier 1 MUST remain strictly within **polynomial-time decidable abstract domains**: Interval Arithmetic (`[min, max]`) plus the **Octagon / Difference-Bound Matrix (DBM) domain** ($\pm x \pm y \le c$, solved in $O(V^3)$ incremental Floyd-Warshall closure).
   - Whenever Tier 1 cannot discharge a `requires` or slice-bounds obligation, the compiler MUST NOT emit a generic "precondition might not hold" message. It MUST emit:
     1. A **concrete counterexample valuation** from the violated DBM boundary (e.g., `Counterexample: lo = 0, hi = 0 violates requires items.len > 0`).
     2. A **copy-pasteable 1-line `guard` prescription** (e.g., `Help: insert 'guard items.len > 0 else return none' before line 43 to narrow the interval`).
   - Adding the prescribed `guard` statement is guaranteed to narrow the Tier-1 DBM lattice deterministically in `< 1 ms` without requiring manual proof lemmas.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Runtime `assert!()` macros (Rust / C / Python)** | Either left enabled in production (wasting CPU branch cycles inside tight loops) or stripped in release builds (`debug_assert!`), leaving the optimizer unable to rely on the condition and exposing UB if violated. Proven Ghost Contracts cost 0 runtime cycles while actively accelerating codegen via `MirOp::Assume`. |
| **Unbounded whole-program SMT solving (Dafny / F* Z3 timeouts)** | Unstratified SMT verification suffers from brittle solver "butterfly effects" where minor edits cause 30-second Z3 timeouts. Separating `< 5 ms` Tier-1 interval/linear solving from explicit Tier-2 proofs keeps IDE latency instantaneous. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="E0501 [Totality Error]" min="1" reason="Sema rejects @total functions whose loops omit a decreases measure" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub struct ValueLattice" min="1" reason="MIR encodes proven contract intervals in ValueLattice for Contract-Guided Optimization" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="requires items.len > 0" min="1" reason="Example 01 uses requires ghost contract" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="@total" min="1" reason="Example 01 proves termination of binary search with @total and decreases" -->
