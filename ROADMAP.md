# Mente (`mn`) — Engineering Roadmap

**Status ledger:** [docs/STATUS.md](docs/STATUS.md)  
**Canonical specification:** [spec/README.md](spec/README.md) (one single-decision file per mechanism)
**Design review:** [docs/design-review.md](docs/design-review.md) — open design problems. Phases 2 to 5 below are kept as written until `roadmap-scope` there is decided.

---

## Phase 0 (`v0.0`): `@spec-family` Governance & Context Brief Pipeline (Step 0 — Immediate)

**Goal:** Wire the external `@spec-family` verification suite alongside `mn spec`, so that each specification directive is also proven able to fail and every cross-document link is checked.

- [x] Flatten specification into emoji-free, single-decision semantic files in [`spec/`](spec/README.md) with zero `DEC-XXX` or `Station` numbering.
- [x] Embed executable `<!-- @assert-present -->`, `<!-- @assert-count -->`, and `<!-- @assert-absence -->` directives across `spec/*.md`, run by `cargo run --bin mn -- spec`, which also rejects a directive that cannot fail.
- [x] Make the documents say what the compiler does (2026-11-04): maturity labels, names and measured numbers corrected across `spec/`, `docs/STATUS.md` and the crate comments.
- [ ] **Step 0.1 (`package.json`)**: private package, `engines` node `>=22`, devDependencies pinned exactly (no `^`): `@spec-family/spec-guard` `0.31.0`, `@spec-family/spec-graph` `0.21.0`, `@spec-family/spec-brief` `0.12.0`. Scripts: `spec-guard "spec/*.md" "docs/*.md"`, `spec-guard prove` over the same patterns, `spec-graph check`, `spec-brief lint --strict`, and `verify:docs` running the four in order. (`spec-graph` has no `verify` command, and `spec-brief` has no `verify` or `generate`.) `spec-guard` 0.31.0 and `spec-graph check` already pass on this tree, run from `C:Reposspecs` on 2026-11-04.
- [ ] **Step 0.2 (`.spec-graph.json`)**: `patterns` over `spec/*.md`, `docs/*.md`, `README.md`, `ROADMAP.md`, `CLAUDE.md` and `examples/README.md`, with `strict: true` (the keys `C:ReposMenteion.spec-graph.json` uses).
- [ ] **Step 0.3 (`.spec-brief.json` & `briefs/`)**: `spec-brief init`. `spec-brief` lints task briefs, the work orders handed to a session; each later step gets one. It does not generate per-crate context: the crate-to-spec routing stays in `CLAUDE.md`.

---

## Phase 1 (`v0.1`): Holographic Frontend, Layout Prover, and 64B `SoA` `MIR` Core

**Governing specs:** [holographic-module.md](spec/holographic-module.md), [surface-syntax.md](spec/surface-syntax.md), [morphic-layouts.md](spec/morphic-layouts.md), [mir-and-codegen.md](spec/mir-and-codegen.md)

- [x] Zero-dependency `.mn` lexer and top-level structural parser (`compiler/mente-syntax`).
- [x] Dual 128-bit structural hashing (`spec_hash` vs. `body_hash`) and Holographic Spec Blueprint projection (`compiler/mente-ast`); measured 0.3–0.9 ms per example module, against a `< 0.1 ms` target.
- [x] Exact `@layout(size, align)` + `@bytes` and `bitstruct` + `@bits` verifier (`compiler/mente-sema`).
- [x] Straight-line `MirRegionSoA` (`NodeId(u32)`), `ValueLattice` record, in-memory interpreter, and opcode mutation gate (`compiler/mente-mir`), exercised on one hand-written region.
- [ ] Statement and expression parser for function bodies (a Pratt parser for nested binary and pipe expressions), replacing today's flat body token lists.
- [ ] Name resolution and a type checker over the parsed bodies. Lowering and every Phase 2 item depend on it.
- [ ] Complete statement/expression AST-to-`MirRegionSoA` lowering for arithmetic, `val`/`var`, `if`/`match`, `guard`, and `while` loops, which needs control flow and nested regions in MIR.
- [ ] Execute `@example` bodies in `mn test` through that lowering, replacing the hand-written demo region in `mente-cli`.

---

## Phase 2 (`v0.2`): Refinement Lattice, Dimensional Units, and Sigil-Free MVS

**Governing specs:** [refinement-and-fixed-point.md](spec/refinement-and-fixed-point.md), [dimensional-units.md](spec/dimensional-units.md), [mutable-value-semantics.md](spec/mutable-value-semantics.md), [regions-and-pools.md](spec/regions-and-pools.md), [ghost-contracts-and-totality.md](spec/ghost-contracts-and-totality.md)

- [ ] **Engineering Guardrail #1 (Solver Predictability)**: Tier-1 `< 5 ms` incremental Octagon / Difference-Bound Matrix ($\pm x \pm y \le c$) solver in `mente-sema` that emits concrete counterexample valuations and copy-pasteable 1-line `guard` prescriptions whenever a `requires` or slice-bounds obligation is unproved.
- [ ] Abelian exponent dimensional `unit` checker (`f64<newton>`, `f64<meter / sec>`) with 100% compile-time erasure before `MIR` emission.
- [ ] Interprocedural Law of Exclusivity (`mut` / `own` disjoint root-path checking) and automatic return-slice provenance inference without `<'a>`.
- [ ] **Engineering Guardrail #3 (Provenance Escape Diagnostics)**: Lexical `Arena.scoped()` escape verifier (`E0310`) that never exposes internal region IDs and always prescribes either `in arena: Arena` or `Pool<T>` / `Handle<T>`.

---

## Phase 3 (`v0.3`): Native Codegen, Carry-Flag ABI, and `comptime` VM

**Governing specs:** [error-handling-abi.md](spec/error-handling-abi.md), [hardware-contracts.md](spec/hardware-contracts.md), [comptime-and-ffi.md](spec/comptime-and-ffi.md), [mir-and-codegen.md](spec/mir-and-codegen.md)

- [ ] **Engineering Guardrail #2 (Dual-Track Codegen & Peephole Parity)**: Native x86-64 and AArch64 machine-code backend lowering `T ! E` to CPU Carry Flag (`CF` / `NZCV.C`) with zero stack tag overhead, paired with E-Graph target peephole rules (`fma`, `bmi2`, AVX-512 masks) after all `MIR` bounds and ghost contracts are erased.
- [ ] Vectorizer and `@simd(lanes: N)` loop dependency prover lowering `SoA<T>` and `Tiled<T, lanes: N>` to AVX-512 / NEON instructions.
- [ ] Hermetic `comptime` interpreter executing `comptime for field in Type.of<T>().fields` directly on `MirRegionSoA`.
- [ ] **Engineering Guardrail #4 (Zero-Config Ecosystem Absorption)**: Implement `mn bind` to auto-import C headers and Rust C-ABI exports into quarantined `extern "C"` blocks with `with FFI`, non-null pointer refinements, and `@layout` checks.
- [ ] An execution model for `@kernel` (index space, memory spaces, barriers, relation to `parallel_for`), then production SPIR-V 1.6 compute module emission for `@kernel` functions. `@kernel` is a `Hypothesis` until the model exists.

---

## Phase 4 (`v0.4`): Colorless Concurrency, Algebraic Effects, and Disjoint Parallelism

**Governing specs:** [capability-effects.md](spec/capability-effects.md), [structured-concurrency.md](spec/structured-concurrency.md), [disjoint-parallelism.md](spec/disjoint-parallelism.md)

- [ ] Static capability subset checking (`with IO, Net, Clock, Rand, FFI`) and zero-cost handler monomorphization.
- [ ] One-line Deterministic Simulation Testing (DST) harness (`SimClock`, `SimNet`, and a seeded `SimSched` scheduler) in `mn test`.
- [ ] Static-stack hybrid fiber scheduler and lexical `parallel (...)` task tree with automatic sibling cancellation.
- [ ] SMT disjoint interval partition checker for lock-free `parallel_for` over `.chunks_mut(N)` and `Tiled<T, lanes: N>`.

---

## Phase 5 (`v1.0`): Self-Hosting and First Flagship Workload Validation

- [ ] Self-host `mente-syntax`, `mente-sema`, and `mente-mir` in `.mn`.
- [ ] Validate Mente against high-frequency deterministic systems workloads (including porting critical `Menteion` 64-byte cache-line and branchless fixed-point kernels to `.mn` and benchmarking L1 miss rate, branch mispredictions, and lines of code).
