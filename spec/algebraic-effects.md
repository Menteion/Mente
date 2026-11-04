---
title: Algebraic Capability Effects and Deterministic Simulation Testing
version: 0.1.0
status: active
date: 2026-11-04
---

# Algebraic Capability Effects and Deterministic Simulation Testing

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Pure-by-default functions and explicit `with` clauses (`Implemented` in `compiler/mente-sema/src/lib.rs`)**:
   - A function without a `with ...` clause is statically guaranteed to be pure with respect to external I/O, network sockets, wall-clock time, entropy, and foreign C calls.
   - Side-effecting capabilities MUST be declared in the function signature using `with`:
     ```rust
     pub fn fetch_telemetry(endpoint: Endpoint) -> Telemetry ! NetError
         with Net, Clock
     ```
2. **Static capability subset verification (`Implemented` in `check_effect_capabilities`)**:
   - If function `f` calls function `g`, `f`'s declared `with` effect set MUST be a superset of `g`'s `with` effect set (unless `f` installs a local `handle` block that discharges that effect).
   - `compiler/mente-sema` verifies capability containment across every call site (`check_effect_capabilities`).
3. **Zero-cost static handler monomorphization and 1-line DST (`Specified`)**:
   - In production builds, standard OS handlers (`StdNet`, `StdClock`, `StdIO`) monomorphize statically into direct OS syscalls (`0` heap closure allocation, `0` dynamic dispatch).
   - In test and verification builds (`mn test`), wrapping a call in a virtual handler (`with SimClock(seed: 42), SimNet(drop_rate: 0.05) { ... }`) intercepts every `Clock` and `Net` operation deterministically in memory, enabling FoundationDB-style **Deterministic Simulation Testing (DST)** without mocking frameworks or dependency-injection boilerplate.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Unrestricted ambient authority (Rust / C++ / Go / Python)** | Any transitive helper function or third-party package can silently read `~/.ssh`, open a network socket, or call `SystemTime::now()`, breaking reproducibility and sandboxing. |
| **Monadic `IO a` wrappers (Haskell)** | Forces monadic lifting (`MonadTrans`, `liftIO`) across every expression and duplicates higher-order collection combinators (`map` vs `mapM`). Mente's `with` capabilities are orthogonal to the return type `T`. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="With," min="1" reason="Lexer defines With keyword for algebraic effect clauses" -->
<!-- @assert-count target="examples/03_colorless_concurrency_and_effects.mn" symbol="with Net, Clock" min="1" reason="Example 03 declares explicit Net and Clock capability effects" -->
