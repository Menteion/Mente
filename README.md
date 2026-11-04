# Mente (`mn`)

**The design of a universal, statically verified, hardware-sympathetic programming language (`.mn`), with an early prototype compiler and its own structure-of-arrays intermediate representation (`MIR`).**

The highlights below are the language's design. Most of it is not implemented: [`docs/STATUS.md`](docs/STATUS.md) says what the compiler does today, and [`docs/design-review.md`](docs/design-review.md) lists the design problems still being corrected.

| Resource | Entry point |
| :--- | :--- |
| Language Specification | [`spec/README.md`](spec/README.md) (one single-decision file per mechanism) |
| Implementation Status | [`docs/STATUS.md`](docs/STATUS.md) |
| Design Review | [`docs/design-review.md`](docs/design-review.md) |
| Engineering Roadmap | [`ROADMAP.md`](ROADMAP.md) |
| Showcase `.mn` Programs | [`examples/README.md`](examples/README.md) |
| Agent Switchboard | [`CLAUDE.md`](CLAUDE.md) |

---

## Core Architectural Highlights

1. **Holographic `.mn` Module & Dual AST Hashing ([`spec/holographic-module.md`](spec/holographic-module.md))**:
   - Single `.mn` source file per module. The compiler computes separate `spec_hash` (public interface, layouts, contracts) and `body_hash` (implementation bodies) digests so body edits trigger **zero downstream recompilation**, and `mn view --spec` projects a compact interface blueprint (target: `< 0.1 ms`).
2. **Refinement Intervals, Fixed-Point, and Zero-Cost Units ([`spec/refinement-and-fixed-point.md`](spec/refinement-and-fixed-point.md), [`spec/dimensional-units.md`](spec/dimensional-units.md))**:
   - First-class bounded intervals (`u32[0..1024]`, dependent slice indexing `usize[0..items.len]` for zero bounds checks), cross-platform deterministic fixed-point (`q16.16`, `q1.15`, `sat<T>`), and compile-time erased physical units (`unit newton = kg * meter / (sec * sec)`).
3. **Sigil-Free Mutable Value Semantics & Generational Pools ([`spec/mutable-value-semantics.md`](spec/mutable-value-semantics.md), [`spec/regions-and-pools.md`](spec/regions-and-pools.md))**:
   - Memory safety without `&`/`&mut` reference sigils, without `<'a>` lifetime annotations, and without a tracing garbage collector via explicit `mut` / `own` call-site markers, lexical `Arena.scoped()`, and 64-bit generational `Pool<T>` / `Handle<T>`.
4. **Hardware Mechanical Sympathy as a Compile-Time Contract ([`spec/morphic-layouts.md`](spec/morphic-layouts.md), [`spec/hardware-contracts.md`](spec/hardware-contracts.md))**:
   - Container-level `List<T>` (`AoS`), `SoA<T>`, and `Tiled<T, lanes: 16>` (`AoSoA`); byte-exact `@layout(size: 64, align: 64)` + `@bytes` verification; `bitstruct` + `@bits`; hard `@branchless`, `@simd`, `@no_alloc` contracts; and, as a hypothesis still without an execution model, unified CPU/GPU `@kernel` compute.
5. **Carry-Flag (`CF`) Error ABI, Ghost Contracts, and Colorless Concurrency ([`spec/error-handling-abi.md`](spec/error-handling-abi.md), [`spec/ghost-contracts-and-totality.md`](spec/ghost-contracts-and-totality.md), [`spec/structured-concurrency.md`](spec/structured-concurrency.md))**:
   - `T ! E` lowers to the hardware CPU Carry Flag (`CF`) with `0` stack union tag bytes on the happy path; proven `requires`/`ensures` ghost contracts erase to `0` runtime instructions; `@total` + `decreases` proves termination; and `parallel (...)` + `with IO, Net` replace `async`/`await`.
6. **From-Scratch 64-Byte `SoA` `Mente IR (MIR)` ([`spec/mir-and-codegen.md`](spec/mir-and-codegen.md))**:
   - Built from scratch in Rust without upstream C++ `MLIR`: 32-bit `NodeId(u32)` handles, 64-byte cache-line-aligned `MirRegionSoA`, microsecond in-memory execution, in-memory mutation testing (`mn test --mutate`), and direct SPIR-V 1.6 emission.

---

## Quick Start

```powershell
# Run all compiler & specification verification tests
cargo test

# Check spec/*.md front matter and run every executable @assert-* directive
cargo run --bin mn -- spec

# Check the examples' declarations, then run the demo MIR region and its mutation gate
cargo run --bin mn -- test --mutate

# Project a .mn module's Holographic Spec Blueprint
cargo run --bin mn -- view --spec examples/02_mechanical_sympathy_simd.mn
```
