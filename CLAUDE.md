# CLAUDE.md — Project Switchboard

This file is the switchboard, not the rulebook.

1. **Live implementation status**: Read [`docs/STATUS.md`](docs/STATUS.md) first. It is the single source of truth for what is `Implemented` vs. `Specified` across all 6 compiler crates and sets **Phase 0 (Step 0: `@spec-family` configuration)** as the immediate entry point.
2. **Language and compiler specification**: Read [`spec/README.md`](spec/README.md). Each file in `spec/*.md` defines exactly **one orthogonal mechanism and design decision** (Specification + Decision Rationale & Rejected Alternatives + Executable `<!-- @assert-* -->` assertions). Load only the specific `spec/<topic>.md` file governing the crate you are editing.
3. **Engineering roadmap**: Read [`ROADMAP.md`](ROADMAP.md) for Phase 0 (`v0.0`) through Phase 5 (`v1.0`) milestones.

---

## Non-Negotiable Architectural Invariants

1. **Single-decision atomic specs (`spec/*.md`)**:
   - Never re-introduce numeric filename prefixes (`00_`..`08_`), nested subdirectories inside `spec/`, `DEC-XXX` numbers, or decorative emoji warning headers.
   - Every claim in `spec/*.md` must carry a maturity label (`Implemented`, `Specified`, `Target`, or `Hypothesis`) and be backed by an `<!-- @assert-present -->`, `<!-- @assert-count -->`, or `<!-- @assert-absence -->` directive.
2. **Forbidden language anti-patterns in `.mn` code**:
   - Never write `let` or `let mut` (use `val` and `var`).
   - Never write `&T`, `&mut T`, or `<'a>` lifetimes (use MVS parameter modes: default read-only, `mut`, `own`, `in`).
   - Never write `async` or `await` (use `with IO, Net` effects and `parallel (...)`).
   - Never write `.unwrap()` or `::<>` turbofish.
   - Never link upstream C++ `LLVM/MLIR` into `compiler/mente-mir` (keep `MirRegionSoA` 100% self-contained, 64-byte cache-line aligned, and indexed by `NodeId(u32)`).
3. **Four Mandatory Engineering Guardrails (Phases 2–3)**:
   - **Guardrail #1 (Solver Predictability — [`ghost-contracts-and-totality.md`](spec/ghost-contracts-and-totality.md))**: Tier-1 stays strictly within polynomial-time Interval + Octagon/DBM ($\pm x \pm y \le c$) domains and MUST emit a concrete counterexample valuation plus a copy-pasteable 1-line `guard` prescription whenever an obligation fails.
   - **Guardrail #2 (Dual-Track Codegen — [`mir-and-codegen.md`](spec/mir-and-codegen.md))**: Complete all high-level semantic optimizations (bounds erasure, Ghost Contract `Assume`, `AoSoA` offsets, `@branchless` select) in `MirRegionSoA` before lowering to fast native/SPIR-V or release peephole backends.
   - **Guardrail #3 (Provenance Escape Diagnostics — [`regions-and-pools.md`](spec/regions-and-pools.md))**: Never expose internal region IDs in errors; `E0310` MUST prescribe either `in arena: Arena` or `Pool<T>` / `Handle<T>`.
   - **Guardrail #4 (Ecosystem Absorption — [`comptime-and-ffi.md`](spec/comptime-and-ffi.md))**: `mn bind` auto-synthesizes quarantined `extern "C"` + `with FFI` + `@layout` bindings from C headers and Rust C-ABI libraries.

---

## Verification Commands (Must Pass Before Every Commit)

```powershell
# 1. Run all unit & integration tests across the 6 compiler crates (including spec assertions)
cargo test

# 2. Verify all 19 spec/*.md files (61 @assert-* directives + counterfactual proofs)
cargo run --bin mn -- spec

# 3. Verify all .mn showcase modules and run the in-memory MIR Mutation Prover
cargo run --bin mn -- test --mutate

# 4. Inspect Holographic Spec Blueprint projection (< 0.1 ms, strips bodies, shows spec_hash/body_hash)
cargo run --bin mn -- view --spec examples/02_mechanical_sympathy_simd.mn
```

---

## Crate-to-Spec Routing Table

| Crate | Governing `spec/*.md` files |
| :--- | :--- |
| `compiler/mente-syntax` | [`holographic-module.md`](spec/holographic-module.md), [`surface-syntax.md`](spec/surface-syntax.md) |
| `compiler/mente-ast` | [`holographic-module.md`](spec/holographic-module.md) |
| `compiler/mente-sema` | [`refinement-and-fixed-point.md`](spec/refinement-and-fixed-point.md), [`dimensional-units.md`](spec/dimensional-units.md), [`algebraic-types-and-null.md`](spec/algebraic-types-and-null.md), [`traits-and-generics.md`](spec/traits-and-generics.md), [`mutable-value-semantics.md`](spec/mutable-value-semantics.md), [`regions-and-pools.md`](spec/regions-and-pools.md), [`morphic-layouts.md`](spec/morphic-layouts.md), [`ghost-contracts-and-totality.md`](spec/ghost-contracts-and-totality.md), [`algebraic-effects.md`](spec/algebraic-effects.md) |
| `compiler/mente-mir` | [`mir-and-codegen.md`](spec/mir-and-codegen.md), [`hardware-contracts.md`](spec/hardware-contracts.md), [`disjoint-parallelism.md`](spec/disjoint-parallelism.md) |
| `compiler/mente-codegen` | [`error-handling-abi.md`](spec/error-handling-abi.md), [`hardware-contracts.md`](spec/hardware-contracts.md), [`mir-and-codegen.md`](spec/mir-and-codegen.md) |
| `compiler/mente-cli` | [`comptime-and-ffi.md`](spec/comptime-and-ffi.md), [`holographic-module.md`](spec/holographic-module.md) |
