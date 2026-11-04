---
title: From-Scratch 64-Byte SoA Mente IR (MIR) and Multi-Backend Codegen
version: 0.1.0
status: active
date: 2026-11-04
---

# From-Scratch 64-Byte `SoA` Mente IR (`MIR`) and Multi-Backend Codegen

**Governs:** `compiler/mente-mir`, `compiler/mente-codegen`

## 1. Specification

1. **64-byte L1 cache-line `SoA` arena (`NodeId(u32)`, `Implemented` in `compiler/mente-mir/src/lib.rs`)**:
   - Every `MIR` instruction node is addressed by a 32-bit index handle (`NodeId(pub u32)`, `4` bytes) rather than a 64-bit heap pointer (`8` bytes).
   - `MirNodeBucket` (`#[repr(C, align(64))]`) packs 16-node SIMD tiles into exactly 64 bytes (`const _: () = assert!(core::mem::size_of::<MirNodeBucket>() == 64);`), matching a single CPU L1 cache line.
   - `MirGraph` stores instruction streams in parallel Structure-of-Arrays (`SoA`) vectors:
     - `ops: Vec<MirOp>` (`ConstInt`, `AddInt`, `SubInt`, `AddSatQ16`, `SubSatQ16`, `Select`, `CmpLt`, `CmpLe`, `Assume`, `BranchCond`, `Return`)
     - `lhs: Vec<NodeId>` and `rhs: Vec<NodeId>`
     - `lattices: Vec<ValueLattice>` (16-byte `[min_val, max_val]` interval per node)
     - `contracts: Vec<MirRegionContract>` (`FLAG_BRANCHLESS`, `FLAG_SIMD`, `FLAG_NO_ALLOC`, `FLAG_KERNEL`, `FLAG_TOTAL`)
2. **RVSDG canonicalization and Equality Saturation (`E-Graphs`, `Implemented` in `MirGraph::run_egraph_rewrite_pass`)**:
   - `MirGraph::run_egraph_rewrite_pass` performs non-destructive algebraic rewrites directly over the `SoA` arrays (e.g., folding identity additions while preserving `ValueLattice` bounds), avoiding phase-ordering cliffs.
3. **In-memory `MIR` mutation prover (`Implemented` in `MirGraph::run_mutation_gate`)**:
   - `mn test --mutate` mutates `MirOp` opcodes in place in memory (`CmpLt <-> CmpLe`, `AddInt <-> SubInt`) in microseconds and verifies that the module's `ensures` contracts and `@example` assertions detect and kill 100% of mutants.
4. **Multi-backend code generation and Dual-Track Peephole Parity (`Implemented` in `compiler/mente-codegen/src/lib.rs`, `Specified` Release Lowering — Engineering Guardrail #2)**:
   - **High-level semantic optimization lives 100% in `MIR`**: Bounds-check elimination (`ValueLattice::proves_in_bounds`), Ghost Contract `Assume` propagation, `AoSoA` (`Tiled<T, lanes: N>`) address calculation, and `@branchless` `SelectBranchless` canonicalization are completed inside `MirRegionSoA` before any target backend runs.
   - **Track A (Zero-Dependency Direct Native / SPIR-V Emitter)**: Emits x86-64 / AArch64 machine code with Carry-Flag (`CF`) error ABI (`CarryFlagAbiLowering`) and direct SPIR-V 1.6 binary words (`emit_spirv_binary`) in milliseconds with zero external compiler dependencies.
   - **Track B (E-Graph Micro-Architectural Peephole & Release Lowering)**: To match or surpass 25 years of LLVM target-specific instruction selection without inheriting C++ MLIR's pointer-graph overhead, `mente-codegen` pairs its E-Graph instruction combiner (`fma`, bmi2 bit-extract, AVX-512 mask registers) with an optional post-`MIR` machine-code emission pass after all Mente contracts and bounds checks have already been erased.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Upstream C++ `MLIR` / `LLVM` heap object graphs (`mlir::Operation*`)** | Upstream C++ MLIR allocates every operation, attribute, and use-def list as heap-allocated 64-bit pointer-linked C++ objects (120–256 bytes per op), causing severe L1/L2 cache thrashing during optimization passes, adding a multi-gigabyte C++ build dependency, and lacking native per-node refinement interval lattices (`ValueLattice`). |
| **Source-level file rewriting mutation testing (`cargo-mutants`)** | Mutating `.rs` or `.mn` source files on disk and invoking the full compiler + linker per mutant takes minutes to hours. Mutating `MirOp` bytes in memory inside `MirGraph` executes in microseconds (`< 100 µs`). |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="#[repr(align(64))]" min="1" reason="MirRegionSoA is aligned to 64 bytes (one CPU L1 cache line)" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub struct NodeId(pub u32)" min="1" reason="MIR nodes use 32-bit index handles instead of 64-bit heap pointers" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn eval_in_memory" min="1" reason="MirRegionSoA includes sub-microsecond in-memory evaluator over SoA streams" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn run_in_memory_mutation_gate" min="1" reason="mente-mir implements in-memory opcode mutation prover" -->
