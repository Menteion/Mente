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
   - **Which tier handles a clause**: every `requires`, `ensures` and `invariant` clause is classified by its syntax alone, so the classification is visible in the signature and in `mn view --spec`:
     - **Tier-1 fragment**: a conjunction (`and`) of comparisons of the form `±x ±y <op> c` or `±x <op> c`, in any equivalent arrangement (`lo < hi`, `items.len > 0`, `i + 1 <= n`), where `x` and `y` are integer or fixed-point bindings, field paths or `.len`, `c` is a compile-time constant, and `<op>` is `<`, `<=`, `>`, `>=` or `==`; plus single-variable bounds on floating-point values (`result >= 0.0_joule`). `mn check` proves these at every definition and every call.
     - **Any other clause** — a call (`is_sorted(items)`), a product of variables, `or`, `!=`, a quantifier — MUST be routed explicitly. With `@prove` on the function, `mn prove` discharges the clause by Tier 2 at the definition and at every call site. With `@checked`, the clause compiles to a runtime check at entry (`requires`) or exit (`ensures`) that traps when it fails. A clause outside the fragment on a function with neither is an error (`E0503`) whose help offers both. `or` and `!=` are outside because octagons are convex.
   - **Ghost erasure and Contract-Guided Optimization (CGO)**: Once proven, `requires` and `ensures` emit **zero runtime instructions** and become assumptions that let `mente-mir` eliminate downstream redundant bounds checks and dead branches. MIR has no assumption node yet.
3. **Constructive termination (`@total` and `decreases`)**:
   - Functions annotated with `@total` MUST terminate on all valid inputs (`Specified`).
   - Every `while` loop or recursive call inside a `@total` function MUST specify a strictly decreasing well-founded measure (`decreases high - low`) (`Specified`). For recursion the measure is a `decreases` clause in the signature, and each recursive call must make it smaller; mutual recursion shares one measure.
   - A `@total` function may call only functions that are `@total` themselves, including trait methods and `extern` declarations that declare it (`hardware-contracts.md`), and may iterate only over finite ranges and collections (`Specified`). An effect operation that waits (`Net`, `Clock`) still counts as returning: `@total` promises termination of the computation, not of the outside world.
   - `Implemented`, as a token check only: `verify_function_contracts` in `compiler/mente-sema/src/lib.rs` raises `E0501 [Totality Error]` when a `@total` function's body contains `while` and neither its signature nor its body contains `decreases`. It does not check that the measure is non-negative or decreases, does not look at recursion, and accepts `while true { i = i + 1 }` once `decreases` appears.
4. **Tier-1 solver predictability and concrete counterexample diagnostics (`Specified` — Engineering Guardrail #1)**:
   - To eliminate the "SMT butterfly effect" (where a minor code edit causes unpredictable solver timeouts in Dafny/LiquidHaskell), Tier 1 MUST remain strictly within **polynomial-time decidable abstract domains**: Interval Arithmetic (`[min, max]`) plus the **Octagon / Difference-Bound Matrix (DBM) domain** ($\pm x \pm y \le c$, solved in $O(V^3)$ incremental Floyd-Warshall closure).
   - **Contracts are used modularly**: at each call, Tier 1 checks the callee's `requires` and then assumes its `ensures`; it never analyses the callee's body, which keeps every caller's proof independent of callee bodies (`holographic-module.md`). A fact the two domains cannot derive is supplied by a function whose `ensures` states it inside them and whose body is proven once by Tier 2.
   - The standard library's `midpoint` is the canonical case:
     ```rust
     pub fn midpoint(lo: usize, hi: usize) -> usize
       requires lo < hi
       ensures lo <= result and result < hi
     = lo + (hi - lo) / 2
     ```
     Its body divides by 2, a coefficient no octagon constraint can hold, so Tier 1 alone would lose the relation between `mid` and `hi`. Through the contract, a binary search's `items[mid]` is proven in bounds from octagon facts only (`examples/01_refinement_and_units.mn`).
   - Whenever Tier 1 cannot discharge a `requires` or slice-bounds obligation, the compiler MUST NOT emit a generic "precondition might not hold" message. It MUST emit:
     1. A **concrete counterexample valuation** from the violated DBM boundary (e.g., `Counterexample: lo = 0, hi = 0 violates requires items.len > 0`).
     2. A **copy-pasteable 1-line `guard` prescription** (e.g., `Help: insert 'guard items.len > 0 else return none' before line 43 to narrow the interval`).
   - Adding the prescribed `guard` statement narrows the Tier-1 DBM lattice deterministically without requiring manual proof lemmas; doing so in under 1 ms is a `Target`.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Runtime `assert!()` macros (Rust / C / Python)** | Either left enabled in production (wasting CPU branch cycles inside tight loops) or stripped in release builds (`debug_assert!`), leaving the optimizer unable to rely on the condition and exposing UB if violated. Proven Ghost Contracts cost 0 runtime cycles while actively accelerating codegen as MIR assumptions. |
| **Letting the solver decide which tier a clause needs** | Whether a clause is proven at every save, only by `mn prove`, or at run time would then depend on the solver's internals and on timeouts, and could change between compiler versions. A syntactic classification is fixed by the clause as written, and `@prove` / `@checked` put the choice in the signature. |
| **Widening Tier 1 to general linear arithmetic (polyhedra, or a linear solver with a time budget)** | Would prove `lo + (hi - lo) / 2 < hi` directly, but polyhedra are exponential in the worst case and a budget makes the answer depend on machine speed, which is the unpredictability Guardrail #1 exists to rule out. A contract on a small library function moves the hard fact to Tier 2 once and leaves every call site in the predictable domains. |
| **Unbounded whole-program SMT solving (Dafny / F* Z3 timeouts)** | Unstratified SMT verification suffers from brittle solver "butterfly effects" where minor edits cause 30-second Z3 timeouts. Separating `< 5 ms` Tier-1 interval/linear solving from explicit Tier-2 proofs keeps IDE latency instantaneous. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="E0501 [Totality Error]" min="1" reason="Sema rejects a @total function with a while loop and no decreases token" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub struct ValueLattice" min="1" reason="MIR has an interval record per node, the carrier Tier-1 facts are specified to fill" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="requires items.len > 0" min="1" reason="Example 01 uses requires ghost contract" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="@total" min="1" reason="Example 01 proves termination of binary search with @total and decreases" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="= midpoint(lo, hi)" min="1" reason="Example 01's binary search takes mid from midpoint's contract, which octagons can hold" -->
