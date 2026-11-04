---
title: Two-Tier SMT Ghost Contracts and Constructive Totality
version: 0.1.0
status: active
date: 2026-11-04
---

# Two-Tier SMT Ghost Contracts and Constructive Totality

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`

## 1. Specification

1. **First-class contract clauses (`requires`, `ensures`, `invariant`)**:
   - Functions declare preconditions via `requires <bool-expr>` and postconditions via `ensures <bool-expr>`, one clause per line, where `result` names the return value (`Implemented` in `parse_fn`: each clause is kept as a raw string up to the end of its line).
   - `struct` declarations declare data invariants via `invariant { <bool-expr> }` (`Implemented` in `parse_struct`, as raw strings).
   - Nothing checks or evaluates a clause yet.
2. **Two-tier verification and Ghost Erasure (`Specified`)**:
   - **Tier 1 (incremental range/linear solver)**: Runs automatically on every save (`mn check`), propagating `ValueLattice` intervals (`[min_val, max_val]`) and linear inequalities across `guard` checks. Its latency target, under 5 ms per edit, is a `Target` with no measurement protocol yet.
   - **Tier 2 (`mn prove` SMT bit-vector / quantifier-free solver)**: Verifies complex non-linear and bit-level contracts in parallel.
   - **Compile-time caller proof obligation**: Calling a function whose `requires` clause is not statically established in the caller's path context (e.g., via refined input types or a preceding `guard` statement) is a compile-time error.
   - **Ghost erasure and Contract-Guided Optimization (CGO)**: Once proven, `requires` and `ensures` emit **zero runtime instructions** and become assumptions that let `mente-mir` eliminate downstream redundant bounds checks and dead branches. MIR has no assumption node yet.
3. **Constructive termination (`@total` and `decreases`)**:
   - Functions annotated with `@total` MUST terminate on all valid inputs (`Specified`).
   - Every `while` loop or recursive call inside a `@total` function MUST specify a strictly decreasing well-founded measure (`decreases high - low`) (`Specified`).
   - `Implemented`, as a token check only: `verify_function_contracts` in `compiler/mente-sema/src/lib.rs` raises `E0501 [Totality Error]` when a `@total` function's body contains `while` and neither its signature nor its body contains `decreases`. It does not check that the measure is non-negative or decreases, does not look at recursion, and accepts `while true { i = i + 1 }` once `decreases` appears.
4. **Tier-1 solver predictability and concrete counterexample diagnostics (`Specified` — Engineering Guardrail #1)**:
   - To eliminate the "SMT butterfly effect" (where a minor code edit causes unpredictable solver timeouts in Dafny/LiquidHaskell), Tier 1 MUST remain strictly within **polynomial-time decidable abstract domains**: Interval Arithmetic (`[min, max]`) plus the **Octagon / Difference-Bound Matrix (DBM) domain** ($\pm x \pm y \le c$, solved in $O(V^3)$ incremental Floyd-Warshall closure).
   - Whenever Tier 1 cannot discharge a `requires` or slice-bounds obligation, the compiler MUST NOT emit a generic "precondition might not hold" message. It MUST emit:
     1. A **concrete counterexample valuation** from the violated DBM boundary (e.g., `Counterexample: lo = 0, hi = 0 violates requires items.len > 0`).
     2. A **copy-pasteable 1-line `guard` prescription** (e.g., `Help: insert 'guard items.len > 0 else return none' before line 43 to narrow the interval`).
   - Adding the prescribed `guard` statement narrows the Tier-1 DBM lattice deterministically without requiring manual proof lemmas; doing so in under 1 ms is a `Target`.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Runtime `assert!()` macros (Rust / C / Python)** | Either left enabled in production (wasting CPU branch cycles inside tight loops) or stripped in release builds (`debug_assert!`), leaving the optimizer unable to rely on the condition and exposing UB if violated. Proven Ghost Contracts cost 0 runtime cycles while actively accelerating codegen as MIR assumptions. |
| **Unbounded whole-program SMT solving (Dafny / F* Z3 timeouts)** | Unstratified SMT verification suffers from brittle solver "butterfly effects" where minor edits cause 30-second Z3 timeouts. Separating `< 5 ms` Tier-1 interval/linear solving from explicit Tier-2 proofs keeps IDE latency instantaneous. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="E0501 [Totality Error]" min="1" reason="Sema rejects a @total function with a while loop and no decreases token" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub struct ValueLattice" min="1" reason="MIR has an interval record per node, the carrier Tier-1 facts are specified to fill" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="requires items.len > 0" min="1" reason="Example 01 uses requires ghost contract" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="@total" min="1" reason="Example 01 proves termination of binary search with @total and decreases" -->
