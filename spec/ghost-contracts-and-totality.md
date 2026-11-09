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
   - `Specified`: functions declare preconditions via `requires <bool-expr>` and postconditions via `ensures <bool-expr>`, one clause per line, where `result` names the return value.
   - `Implemented`: `parse_fn` keeps each `requires`, `ensures` and `decreases` clause as a raw string, up to the end of its line.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_contract_clauses_are_kept_line_by_line" min="1" reason="parse_fn keeps each contract clause as its own line" -->
   - `Specified`: `struct` declarations declare data invariants via `invariant { <bool-expr> }`.
   - `Implemented`: `parse_struct` keeps each line of an `invariant` block as a raw string.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_struct_fields_defaults_and_invariants_are_parsed" min="1" reason="parse_struct keeps each invariant line" -->
   - The declaration parser checks no clause; the body parser parses each clause of a function with a body, which Tier 1 proves and the reference interpreter checks at run time (item 2; `core-calculus.md`, items 8 and 9).
2. `Specified` **Two-tier verification and Ghost Erasure**:
   - **Tier 1 (incremental range/linear solver)**: Runs automatically on every save (`mn check`), propagating `ValueLattice` intervals (`[min_val, max_val]`) and linear inequalities across `guard` checks.
   - `Specified` **Facts at a loop head** ([`tier1-loops`](../docs/design-review.md#tier1-loops)): Tier 1 keeps exactly the candidate facts that are inductive, true on entry and preserved by the body. Candidates come only from the loop's own text: its condition, with the non-strict form of each strict comparison in it (`i < n` also proposes `i <= n`, [`loop-condition-candidates`](../docs/design-review.md#loop-condition-candidates)), its `guard`s, the refinements of the variables it touches, and an optional `invariant` clause. Tier 1 drops failing candidates until the set is stable, in polynomial time and with no widening, so a reader can tell from the loop which facts can survive. A written `invariant` is itself checked, on entry and across the body.
   - `Implemented`: at each loop head Tier 1 keeps, by Houdini, the candidates of the loop's text that hold on entry and after every round of the body: the conjuncts of its condition with the non-strict form of each strict comparison, its written invariants, and the conditions of the `guard`s in its body outside inner loops; the bindings the loop changes keep only their types and refinements. A written `invariant` that fails on entry or across the body is an error (`E0506`).
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_houdini_keeps_only_inductive_candidates" min="1" reason="Houdini keeps the inductive candidates of the loop's text" -->
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_written_invariants_hold_on_entry_and_are_preserved" min="1" reason="A written invariant is checked on entry and across the body" -->
   - `Specified`: when an obligation inside a loop fails, the diagnostic lists the facts held at the loop head and the candidate that was dropped, with the assignment that breaks it, and offers an `invariant` line or a `guard` as fixes.
   - `Implemented`: an obligation inside a loop that Tier 1 cannot prove names, in a note at the loop, the candidates it holds at the loop's head and those it dropped, and offers a `guard`.
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_obligations_inside_a_loop_name_its_head_facts" min="1" reason="A failed obligation in a loop names the loop head's facts" -->
   - `Target`: a Tier-1 re-check after an edit takes under 5 ms at the 99th percentile, under [its protocol](../ROADMAP.md#tier-1-re-check-after-an-edit-99th-percentile-under-5-ms); it has no result yet: Tier 1 runs in `mn check` since M1's checkpoint (b), but has not been measured under the protocol.
   - **Tier 2 (`mn prove` SMT bit-vector / quantifier-free solver)**: Verifies complex non-linear and bit-level contracts in parallel. Until `mn prove` runs, a clause routed to it, and a body such as `midpoint`'s that only it could prove, is a trusted axiom: assumed, not proven, and called so in the spec, the examples and the diagnostics. `mn prove` never reports a clause proven before it can prove it ([`tier2-trusted`](../docs/design-review.md#tier2-trusted)).
   - `Specified`: `mn check` lists every trusted clause with its place, as it lists every `todo`.
   - **Compile-time caller proof obligation**: Calling a function whose `requires` clause is not statically established in the caller's path context (e.g., via refined input types or a preceding `guard` statement) is a compile-time error.
   - **Which tier handles a clause**: every `requires`, `ensures` and `invariant` clause is classified by its syntax alone, so the classification is visible in the signature and in `mn view --spec`:
     - **Tier-1 fragment**: a conjunction (`and`) of comparisons of the form `±x ±y <op> c` or `±x <op> c`, in any equivalent arrangement (`lo < hi`, `items.len > 0`, `i + 1 <= n`), where `x` and `y` are integer or fixed-point bindings, field paths or `.len` (the `.len` of a fixed-size array `[N]T` is the constant `N`), `c` is a compile-time constant, and `<op>` is `<`, `<=`, `>`, `>=` or `==`; plus single-variable bounds on floating-point values (`result >= 0.0_joule`). `mn check` proves these at every definition and every call.
     - **Any other clause** — a call (`is_sorted(items)`), a product of variables, `or`, `!=`, a quantifier — MUST be routed explicitly. With `@prove` on the function, `mn prove` discharges the clause by Tier 2 at the definition and at every call site. With `@checked`, the clause compiles to a runtime check at entry (`requires`) or exit (`ensures`) that traps when it fails. A clause outside the fragment on a function with neither is an error (`E0503`) whose help offers both. `or` and `!=` are outside because octagons are convex.
   - `Implemented`: `mn check` classifies each `requires`, `ensures` and loop `invariant` clause by its syntax. One outside the Tier-1 fragment, on a function with neither `@checked` nor `@prove`, is an error (`E0503`) that names what puts it outside and offers both; Tier 1 proves such a clause nowhere, and assumes it where it holds: on entry, after a call, at a loop head.
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_clauses_outside_the_fragment_are_routed" min="1" reason="A clause outside the fragment is E0503 unless routed, and assumed when routed" -->
   - **Ghost erasure and Contract-Guided Optimization (CGO)**: Once proven, `requires` and `ensures` emit **zero runtime instructions** and become assumptions that let `mente-mir` eliminate downstream redundant bounds checks and dead branches. MIR has no assumption node yet.
3. **Constructive termination (`@total` and `decreases`)**:
   - `Specified`: functions annotated with `@total` MUST terminate on all valid inputs.
   - `Specified`: every `while` loop or recursive call inside a `@total` function MUST specify a strictly decreasing well-founded measure (`decreases high - low`). For recursion the measure is a `decreases` clause in the signature, and each recursive call must make it smaller; mutual recursion shares one measure.
   - `Specified`: outside a `@total` function a measure is optional, and one that is written is proven all the same, as a claim the code makes (`core-calculus.md`, item 8; [`loop-decreases`](../docs/design-review.md#loop-decreases)).
   - `Specified`: a `@total` function may call only functions that are `@total` themselves, including trait methods and `extern` declarations that declare it (`hardware-contracts.md`), and may iterate only over finite ranges and collections. An effect operation that waits (`Net`, `Clock`) still counts as returning: `@total` promises termination of the computation, not of the outside world.
   - `Implemented`, as a token check only: `verify_function_contracts` in `compiler/mente-sema/src/lib.rs` reports `E0501` when a `@total` function's body contains `while` and neither its signature nor its body contains `decreases`. It does not check that the measure is non-negative or decreases, does not look at recursion, and accepts `while true { i = i + 1 }` once `decreases` appears.
     <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_total_while_without_decreases_is_rejected" min="1" reason="A @total function with a while loop and no decreases is rejected with E0501" -->
4. `Specified` **Tier-1 solver predictability and concrete counterexample diagnostics (Engineering Guardrail #1)**:
   - To eliminate the "SMT butterfly effect" (where a minor code edit causes unpredictable solver timeouts in Dafny/LiquidHaskell), Tier 1 MUST remain strictly within **polynomial-time decidable abstract domains**: Interval Arithmetic (`[min, max]`) plus the **Octagon / Difference-Bound Matrix (DBM) domain** ($\pm x \pm y \le c$, solved in $O(V^3)$ incremental Floyd-Warshall closure).
   - **Contracts are used modularly**: at each call, Tier 1 checks the callee's `requires` and then assumes its `ensures`; it never analyses the callee's body, which keeps every caller's proof independent of callee bodies (`holographic-module.md`). A fact the two domains cannot derive is supplied by a function whose `ensures` states it inside them and whose body is proven once by Tier 2, a trusted axiom until `mn prove` runs.
   - The standard library's `midpoint` is the canonical case:
     ```rust
     pub fn midpoint(lo: usize, hi: usize) -> usize
       requires lo < hi
       ensures lo <= result and result < hi
     = lo + (hi - lo) / 2
     ```
     Its body divides by 2, a coefficient no octagon constraint can hold, so Tier 1 alone would lose the relation between `mid` and `hi`. Through the contract, a binary search's `items[mid]` is proven in bounds from octagon facts only (`examples/01_refinement_and_units.mn`).
   - Whenever Tier 1 cannot discharge a `requires` or slice-bounds obligation, the compiler MUST NOT emit a generic "precondition might not hold" message. It MUST emit:
     1. A **concrete valuation Tier 1 cannot rule out**, read from the violated DBM boundary (e.g., `Tier 1 cannot rule out lo = 0, hi = 0, which violates requires items.len > 0`). Octagons over-approximate, notably after the join at the end of an `if`, so no execution may reach the valuation, and the diagnostic never calls it an input that fails ([`tier1-counterexample`](../docs/design-review.md#tier1-counterexample)).
     2. A **copy-pasteable 1-line `guard` prescription** (e.g., `Help: insert 'guard items.len > 0 else return none' before line 43 to narrow the interval`). It is the diagnostic's fix, with applicability `suggestion`: a `guard` changes what the function does on the inputs it excludes (`diagnostics.md`).
   - `Implemented`: Tier 1 works over an octagon of the function's integer bindings, call arguments and results, a difference-bound matrix kept tightly closed, to which a constraint is added in O(n^2). It uses contracts modularly: a call proves the callee's `requires` and assumes its `ensures`, reading the callee's header only, and an edit to a callee's header proves its callers again under `mn check --watch`.
     <!-- @assert-count target="compiler/mente-sema/src/octagon.rs" symbol="fn test_incremental_closure_agrees_with_brute_force" min="1" reason="The incremental closure keeps every point and finds emptiness exactly" -->
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_contracts_are_used_modularly" min="1" reason="Callees are read through their headers only" -->
     <!-- @assert-count target="compiler/mente-sema/src/incremental.rs" symbol="fn test_a_callee_contract_edit_proves_its_callers_again" min="1" reason="A callee's header edit proves its callers again" -->
   - `Implemented`: an obligation Tier 1 must prove and cannot is an error that names a valuation of the bindings in scope Tier 1 cannot rule out, read from the failing octagon, and never calls it an input that fails; its fix, of applicability `suggestion`, inserts a one-line `guard` on its own line before the statement that holds the obligation, whose `else` leaves by `todo` in a function with a result and by `return` in one without.
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_a_failed_refinement_names_a_valuation_and_a_guard" min="1" reason="A failed obligation names a valuation and a guard" -->
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_guard_fixes_go_on_their_own_line" min="1" reason="The guard goes on its own line, indented like its statement" -->
   - Adding the prescribed `guard` statement narrows the Tier-1 DBM lattice deterministically without requiring manual proof lemmas; the re-check it causes falls under the 5 ms target above.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Runtime `assert!()` macros (Rust / C / Python)** | Either left enabled in production (wasting CPU branch cycles inside tight loops) or stripped in release builds (`debug_assert!`), leaving the optimizer unable to rely on the condition and exposing UB if violated. Proven Ghost Contracts cost 0 runtime cycles while actively accelerating codegen as MIR assumptions. |
| **Letting the solver decide which tier a clause needs** | Whether a clause is proven at every save, only by `mn prove`, or at run time would then depend on the solver's internals and on timeouts, and could change between compiler versions. A syntactic classification is fixed by the clause as written, and `@prove` / `@checked` put the choice in the signature. |
| **Widening Tier 1 to general linear arithmetic (polyhedra, or a linear solver with a time budget)** | Would prove `lo + (hi - lo) / 2 < hi` directly, but polyhedra are exponential in the worst case and a budget makes the answer depend on machine speed, which is the unpredictability Guardrail #1 exists to rule out. A contract on a small library function moves the hard fact to Tier 2 once and leaves every call site in the predictable domains. |
| **Unbounded whole-program SMT solving (Dafny / F* Z3 timeouts)** | Unstratified SMT verification suffers from brittle solver "butterfly effects" where minor edits cause 30-second Z3 timeouts. Separating `< 5 ms` Tier-1 interval/linear solving from explicit Tier-2 proofs keeps IDE latency instantaneous. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="E0501" min="1" reason="Sema rejects a @total function with a while loop and no decreases token" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub struct ValueLattice" min="1" reason="MIR has an interval record per node, the carrier Tier-1 facts are specified to fill" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="requires items.len > 0" min="1" reason="Example 01 uses requires ghost contract" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="@total" min="1" reason="Example 01 proves termination of binary search with @total and decreases" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="= midpoint(lo, hi)" min="1" reason="Example 01's binary search takes mid from midpoint's contract, which octagons can hold" -->
