# Mente (`mn`) — Engineering Roadmap

**Status ledger:** [docs/STATUS.md](docs/STATUS.md)  
**Canonical specification:** [spec/README.md](spec/README.md) (one single-decision file per mechanism)
**Design review:** [docs/design-review.md](docs/design-review.md) — the design problems found on 2026-11-04 and how each was settled. The phases below were re-cut on 2026-11-05 around a core vertical slice, with native code through Cranelift (`roadmap-scope`).

---

## Phase 0 (`v0.0`): `@spec-family` Governance & Context Brief Pipeline (Step 0 — Immediate)

**Goal:** Wire the external `@spec-family` verification suite alongside `mn spec`, so that each specification directive is also proven able to fail and every cross-document link is checked.

- [x] Flatten specification into emoji-free, single-decision semantic files in [`spec/`](spec/README.md) with zero `DEC-XXX` or `Station` numbering.
- [x] Embed executable `<!-- @assert-present -->`, `<!-- @assert-count -->`, and `<!-- @assert-absence -->` directives across `spec/*.md`, run by `cargo run --bin mn -- spec`, which also rejects a directive that cannot fail.
- [x] Make the documents say what the compiler does (2026-11-04): maturity labels, names and measured numbers corrected across `spec/`, `docs/STATUS.md` and the crate comments.
- [ ] **Step 0.1 (`package.json`)**: private package, `engines` node `>=22`, devDependencies pinned exactly (no `^`): `@spec-family/spec-guard` `0.31.0`, `@spec-family/spec-graph` `0.21.0`, `@spec-family/spec-brief` `0.12.0`. Scripts: `spec-guard "spec/*.md" "docs/*.md"`, `spec-guard prove` over the same patterns, `spec-graph check`, `spec-brief lint --strict`, and `verify:docs` running the four in order. (`spec-graph` has no `verify` command, and `spec-brief` has no `verify` or `generate`.) `spec-guard` 0.31.0 and `spec-graph check` already pass on this tree, run from `C:Reposspecs` on 2026-11-04.
- [ ] **Step 0.2 (`.spec-graph.json`)**: `patterns` over `spec/*.md`, `docs/*.md`, `README.md`, `ROADMAP.md`, `CLAUDE.md` and `examples/README.md`, with `strict: true` (the keys `C:ReposMenteion.spec-graph.json` uses).
- [ ] **Step 0.25 (`.mn` comments in `spec-guard`)**: register the `.mn` comment syntax (`//`, `///`) in `spec-guard` (`C:\Repos\specs`), so it ignores comments in `.mn` files as `mn spec` already does; today it warns that the syntax is unknown and counts comments as code.
- [ ] **Step 0.3 (`.spec-brief.json` & `briefs/`)**: `spec-brief init`. `spec-brief` lints task briefs, the work orders handed to a session; each later step gets one. It does not generate per-crate context: the crate-to-spec routing stays in `CLAUDE.md`.

---

## Phase 1 (`v0.1`): Frontend and the Core Checks

**Goal:** the smallest language that shows Mente's core ideas end to end in the checker: refinements, units, layouts and mutable value semantics, proven by a predictable Tier 1.

**Governing specs:** [holographic-module.md](spec/holographic-module.md), [surface-syntax.md](spec/surface-syntax.md), [traits-and-generics.md](spec/traits-and-generics.md), [algebraic-types-and-null.md](spec/algebraic-types-and-null.md), [refinement-and-fixed-point.md](spec/refinement-and-fixed-point.md), [dimensional-units.md](spec/dimensional-units.md), [mutable-value-semantics.md](spec/mutable-value-semantics.md), [regions-and-pools.md](spec/regions-and-pools.md), [morphic-layouts.md](spec/morphic-layouts.md), [ghost-contracts-and-totality.md](spec/ghost-contracts-and-totality.md)

- [x] Zero-dependency `.mn` lexer and top-level structural parser (`compiler/mente-syntax`).
- [x] Three 128-bit structural digests (`spec_hash`, `instance_hash`, `body_hash`) and Holographic Spec Blueprint projection (`compiler/mente-ast`); median 45–92 µs per example module, meeting the `< 0.1 ms` median target (protocol below).
- [x] Exact `@layout(size, align)` + `@bytes` prover with field alignment and `@packed`, and `bitstruct` + `@bits` prover with field widths (`compiler/mente-sema`).
- [x] Return-view provenance read from the signature: the `from` clause is parsed, hashed and checked against the parameter list (`E0311`).
- [ ] Statement and expression parser for function bodies (a Pratt parser for nested binary and pipe expressions, the C#-style type-argument rule), replacing today's flat body token lists, with diagnostics for unknown characters.
- [ ] Name resolution and a type checker: generics with the one compile-time parameter form, traits with static default and `dyn`, `?T` narrowing, `T ! E` with `raise` and `!`, `enum` with exhaustive `match`.
- [ ] Refinements: subtyping, interval arithmetic, bounds only over immutable bindings, overflow and division traps unless proven.
- [ ] Units as dimension plus scale, with `.to<U>()` the only conversion.
- [ ] Mutable value semantics: parameter modes at call sites, the Law of Exclusivity (across `parallel` branches too), `own` invalidation, and bodies checked against their `from` clause.
- [ ] **Engineering Guardrail #1 (Solver Predictability)**: Tier-1 intervals plus octagons in `mente-sema`, using callee contracts modularly, classifying clauses for `@prove` / `@checked`, and emitting a concrete counterexample and a one-line `guard` prescription for every unproved obligation; measured under the Tier-1 protocol below.
- [ ] **Engineering Guardrail #3 (Provenance Escape Diagnostics)**: `E0310`, which never exposes internal region IDs and always prescribes either `in arena: Arena` or `Pool<T>` / `Handle<T>`.

---

## Phase 2 (`v0.2`): MIR, Lowering and Execution

**Goal:** every example runs: checked `.mn` lowers to MIR and executes in the interpreter, and the mutation gate works on real code.

**Governing specs:** [mir-and-codegen.md](spec/mir-and-codegen.md), [hardware-contracts.md](spec/hardware-contracts.md)

- [x] Straight-line `MirRegionSoA` (`NodeId(u32)`), `ValueLattice` record, in-memory interpreter, and opcode mutation gate (`compiler/mente-mir`), exercised on one hand-written region.
- [ ] Control flow and nested regions in MIR, memory operations and calls.
- [ ] AST-to-`MirRegionSoA` lowering for arithmetic, `val`/`var`, `if`/`match`, `guard`, loops, calls and projections (copy-in/copy-out included).
- [ ] Proven contracts and bounds become MIR assumptions; bounds-check elimination from Tier-1 intervals rather than hand-written ones.
- [ ] `mn test` executes `@example` bodies through the interpreter, replacing the hand-written demo region in `mente-cli`; the mutation gate runs on lowered code and accounts for every survivor.
- [ ] `@branchless`, `@no_alloc`, `@max_stack` and `@total` checked as specified, composing from signatures.

---

## Phase 3 (`v0.3`): Native Code through Cranelift

**Goal:** the examples compile to native x86-64 and AArch64 code without writing a backend from scratch.

**Governing specs:** [mir-and-codegen.md](spec/mir-and-codegen.md), [error-handling-abi.md](spec/error-handling-abi.md), [morphic-layouts.md](spec/morphic-layouts.md), [regions-and-pools.md](spec/regions-and-pools.md)

- [ ] **Engineering Guardrail #2**: every Mente-level optimisation (bounds erasure, contract assumptions, `AoSoA` offsets, `@branchless` selects) completes in `MirRegionSoA`; `mente-codegen` then lowers MIR to Cranelift IR for x86-64 and AArch64. Cranelift is pure Rust, so no C++ toolchain enters the build, and `mente-mir` keeps no dependency at all.
- [ ] Runtime: arenas with a chunk per task, `Pool<T>` / `Handle<T>`, `Local<T>` / `Shared<T>`, `String` / `List<T>`.
- [ ] `@layout` structs in native code: alignment, little-endian bytes, `@packed` and `@endian(big)`.
- [ ] `@simd(lanes: N)` loops through Cranelift's 128-bit vector types, splitting wider lane groups, over `SoA<T>` and `Tiled<T, lanes: N>`.
- [ ] `@branchless` in native code: floating-point and vector selects lowered without Cranelift's branching `select`, and the machine-code check (`E0403`).
- [ ] `T ! E` as a two-register tagged return through Cranelift's multi-value returns, with the generated `extern "C"` wrappers.
- [ ] Inlining decided in MIR: `@inline` and generic callees across modules, a size heuristic inside one.

---

## Phase 4 (`v0.4`): Effects, Concurrency and Simulation

**Governing specs:** [capability-effects.md](spec/capability-effects.md), [structured-concurrency.md](spec/structured-concurrency.md), [disjoint-parallelism.md](spec/disjoint-parallelism.md), [comptime-and-ffi.md](spec/comptime-and-ffi.md)

- [ ] Capability checking with effect variables, `handle` blocks, and tail-resumptive handlers compiled to direct calls.
- [ ] Stackful fibers on reserved virtual-memory stacks, and lexical `parallel (...)` task trees with sibling cancellation.
- [ ] `parallel_for` over structural partitions (`.chunks_mut(N)`, `.split_at_mut(mid)`, `Tiled` tiles), whose disjointness holds by construction.
- [ ] Deterministic Simulation Testing in `mn test`: `SimClock`, `SimNet`, and the seeded `SimSched` scheduler.
- [ ] Hermetic `comptime` interpreter on `MirRegionSoA`, under its step budget.
- [ ] `extern "C"` blocks: checked inbound values, `RawPtr<T>`, and the tagged-return wrappers.

---

## Later, Unscheduled

Each of these starts with an entry in [docs/design-review.md](docs/design-review.md) and is scheduled only once the phases above are done.

- [ ] An execution model for `@kernel` (index space, memory spaces, barriers, relation to `parallel_for`), then SPIR-V 1.6 compute modules. `@kernel` is a `Hypothesis` until the model exists.
- [ ] A from-scratch native emitter with E-graph peephole rules (`fma`, `bmi2`, AVX-512 masks) and 256- / 512-bit vectors, pursued only if Cranelift's code is measured short of what a workload needs.
- [ ] The Carry-Flag ABI experiment of `error-handling-abi.md`, on that emitter or on a Cranelift fork: implement it beside the two-register return, run the benchmark, and keep it only if it wins.
- [ ] **Engineering Guardrail #4**: `mn bind` for C headers, and Rust libraries through their `cbindgen` headers, once the accepted C dialect is written down.
- [ ] Index-arithmetic `parallel_for` partitions (`scores[i * S .. (i + 1) * S]`), which need a prover beyond Tier 1.
- [ ] Self-host `mente-syntax`, `mente-sema`, and `mente-mir` in `.mn`.
- [ ] Validate Mente against high-frequency deterministic systems workloads (including porting critical `Menteion` 64-byte cache-line and branchless fixed-point kernels to `.mn` and benchmarking L1 miss rate, branch mispredictions, and lines of code).

---

## Measurement protocols

A `Target` in `spec/*.md` is a number with a protocol here. A figure without a protocol is not a target, and is removed or relabelled `Hypothesis`.

### `mn view --spec` projection: median under 0.1 ms per module

- **Timed**: parsing a module whose source is already in memory, computing its three digests, and projecting the blueprint. File reading and process start are excluded.
- **Corpus**: every module in `examples/`, and the largest standard-library module once one exists.
- **Method**: release build; per module, 100 warm-up runs, then 1,000 timed runs; report the median and the 99th percentile. The harness is the ignored test `view_projection_protocol` in `compiler/mente-cli`:
  ```powershell
  cargo test --release -p mente-cli -- --ignored --nocapture view_projection_protocol
  ```
- **Machine**: record the CPU, OS and power mode with each result.
- **Last result** (2026-11-05, AMD Ryzen 7 7735HS, Windows 11): median 45–92 µs, 99th percentile 110–208 µs. The median meets the target; nothing is promised for the 99th percentile.

### Tier-1 re-check after an edit: 99th percentile under 5 ms

- **Timed**: in a running `mn check --watch` or language-server process, from an edit being applied to the source in memory until Tier 1 has re-proven every obligation the edit can affect and produced its diagnostics.
- **Corpus**: the `examples/` modules, plus a generated module of 200 functions with 20 obligations each (loop bounds, slice indexing, chains of `requires`). Each run applies one random single-token edit inside one function body.
- **Method**: release build; 100 warm-up edits, then 1,000 timed edits; report the median and the 99th percentile.
- **Machine**: as above.
- **Last result**: none yet; Tier 1 does not exist.
