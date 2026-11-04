# Mente (`mn`) — Engineering Roadmap

**Status ledger:** [docs/STATUS.md](docs/STATUS.md)  
**Canonical specification:** [spec/README.md](spec/README.md) (17 atomic single-decision specification files)

---

## Phase 0 (`v0.0`): `@spec-family` Governance & Context Brief Pipeline (Step 0 — Immediate)

**Goal:** Wire the external `@spec-family` verification suite alongside `mn spec` so every specification claim, cross-document link, and per-crate context brief is machine-enforced before compiler passes expand.

- [x] Flatten specification into 19 emoji-free, single-decision semantic files in [`spec/`](spec/README.md) with zero `DEC-XXX` or `Station` numbering.
- [x] Embed 61 executable `<!-- @assert-present -->`, `<!-- @assert-count -->`, and `<!-- @assert-absence -->` directives across `spec/*.md`, verified by `cargo run --bin mn -- spec`.
- [ ] **Step 0.1 (`package.json`)**: Configure `@spec-family/spec-guard` (`^0.30.0`), `@spec-family/spec-graph` (`^0.21.0`), and `@spec-family/spec-brief` (`^0.12.0`) with `npm run verify:docs` (`spec-guard && spec-guard prove && spec-graph verify && spec-brief verify --strict`).
- [ ] **Step 0.2 (`.spec-graph.json`)**: Configure front-matter schema validation and cross-link checking across `spec/*.md` and `docs/*.md`.
- [ ] **Step 0.3 (`.spec-brief.json` & `briefs/`)**: Map each compiler crate (`mente-syntax`, `mente-ast`, `mente-sema`, `mente-mir`, `mente-codegen`, `mente-cli`) to its governing atomic `spec/*.md` files and generate `briefs/*.md`.

---

## Phase 1 (`v0.1`): Holographic Frontend, Layout Prover, and 64B `SoA` `MIR` Core

**Governing specs:** [holographic-module.md](spec/holographic-module.md), [surface-syntax.md](spec/surface-syntax.md), [morphic-layouts.md](spec/morphic-layouts.md), [mir-and-codegen.md](spec/mir-and-codegen.md)

- [x] Zero-dependency `.mn` lexer and top-level structural parser (`compiler/mente-syntax`).
- [x] Dual 128-bit semantic hashing (`spec_hash` vs. `body_hash`) and `< 0.1 ms` Holographic Spec Blueprint projection (`compiler/mente-ast`).
- [x] Exact `@layout(size, align)` + `@bytes` and `bitstruct` + `@bits` verifier (`compiler/mente-sema`).
- [x] 64-byte cache-line-aligned `MirRegionSoA` (`NodeId(u32)`), `ValueLattice`, in-memory evaluator, and microsecond mutation gate (`compiler/mente-mir`).
- [ ] Complete statement/expression AST-to-`MirRegionSoA` lowering for arithmetic, `val`/`var`, `if`/`match`, `guard`, and `while` loops.

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
- [ ] Vectorizer and `@simd(width: N)` loop dependency prover lowering `SoA<T>` and `Tiled<T, lanes: N>` to AVX-512 / NEON instructions.
- [ ] Hermetic `comptime` interpreter executing `comptime for field in Type.of<T>().fields` directly on `MirRegionSoA`.
- [ ] **Engineering Guardrail #4 (Zero-Config Ecosystem Absorption)**: Implement `mn bind` to auto-import C headers and Rust C-ABI exports into quarantined `extern "C"` blocks with `with FFI`, non-null pointer refinements, and `@layout` checks.
- [ ] Production SPIR-V 1.6 compute module emission for `@kernel` functions.

---

## Phase 4 (`v0.4`): Colorless Concurrency, Algebraic Effects, and Disjoint Parallelism

**Governing specs:** [algebraic-effects.md](spec/algebraic-effects.md), [structured-concurrency.md](spec/structured-concurrency.md), [disjoint-parallelism.md](spec/disjoint-parallelism.md)

- [ ] Static capability subset checking (`with IO, Net, Clock, Rand, FFI`) and zero-cost handler monomorphization.
- [ ] One-line Deterministic Simulation Testing (DST) harness (`VirtualClock`, `MockNet`) in `mn test`.
- [ ] Static-stack hybrid fiber scheduler and lexical `parallel (...)` task tree with automatic sibling cancellation.
- [ ] SMT disjoint interval partition checker for lock-free `parallel_for` over `.chunks_mut(N)` and `Tiled<T, lanes: N>`.

---

## Phase 5 (`v1.0`): Self-Hosting and First Flagship Workload Validation

- [ ] Self-host `mente-syntax`, `mente-sema`, and `mente-mir` in `.mn`.
- [ ] Validate Mente against high-frequency deterministic systems workloads (including porting critical `Menteion` 64-byte cache-line and branchless fixed-point kernels to `.mn` and benchmarking L1 miss rate, branch mispredictions, and lines of code).
