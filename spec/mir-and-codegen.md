---
title: From-Scratch 64-Byte SoA Mente IR (MIR) and Multi-Backend Codegen
version: 0.1.0
status: active
date: 2026-11-04
---

# From-Scratch 64-Byte `SoA` Mente IR (`MIR`) and Multi-Backend Codegen

**Governs:** `compiler/mente-mir`, `compiler/mente-codegen`

## 1. Specification

1. **Index-addressed structure-of-arrays region (`Implemented` in `MirRegionSoA`, `compiler/mente-mir/src/lib.rs`)**:
   - Every `MIR` instruction node is addressed by a 32-bit index handle (`NodeId(pub u32)`, `4` bytes) rather than a 64-bit heap pointer (`8` bytes).
   - `MirRegionSoA` stores one straight-line region in parallel vectors: `opcodes: Vec<MirOp>`, `types: Vec<MirType>` (`I64`, `Q16_16`, `Bool`), `inputs: Vec<[NodeId; 3]>`, `lattices: Vec<ValueLattice>` (an `i128` interval and an alignment per node), `lines: Vec<u32>`, plus `is_branchless_guaranteed: bool` and `ret_node`.
   - `MirOp` today: `Param`, `Const`, `Add`, `Sub`, `AddSatQ16`, `SubSatQ16`, `Shr`, `CmpGe`, `CmpLt`, `CmpEq`, `SelectBranchless`, `BranchCondJump`. There are no loads, stores, calls, loops or nested regions.
   - `#[repr(align(64))]` on `MirRegionSoA` aligns the struct header only; the node arrays are ordinary heap `Vec`s. Packing nodes into 64-byte cache-line buckets is `Specified`.
2. **RVSDG canonicalization and Equality Saturation (`E-Graphs`, `Specified`)**:
   - A rewrite pass performs non-destructive algebraic rewrites directly over the `SoA` arrays (e.g., folding identity additions while preserving `ValueLattice` bounds), avoiding phase-ordering cliffs. Neither RVSDG structure nor an e-graph exists yet.
3. **In-memory `MIR` mutation prover**:
   - `Implemented` in `run_in_memory_mutation_gate`: flips `Add` ↔ `Sub`, `AddSatQ16` ↔ `SubSatQ16` and `CmpGe` ↔ `CmpLt`, swaps the arms of each `SelectBranchless`, and counts the mutants that the supplied `(inputs, expected)` vectors kill. `mn test --mutate` runs it on one hand-written region with two vectors.
   - `Specified`: running it on regions lowered from `.mn` source, with the module's `ensures` contracts and `@example` assertions as the oracle.
4. **Multi-backend code generation and Dual-Track Peephole Parity (`Specified` — Engineering Guardrail #2; matching LLVM's code quality is a `Hypothesis`)**:
   - Today `compiler/mente-codegen` has only `CarryFlagAbiLowering`, a table of the registers the error ABI would use, and `emit_spirv_binary`, a SPIR-V-shaped word stream that is not a valid module.
   - **High-level semantic optimization lives 100% in `MIR`**: Bounds-check elimination (`ValueLattice::proves_in_bounds`), Ghost Contract `Assume` propagation, `AoSoA` (`Tiled<T, lanes: N>`) address calculation, and `@branchless` `SelectBranchless` canonicalization are completed inside `MirRegionSoA` before any target backend runs.
   - **Track A (Zero-Dependency Direct Native / SPIR-V Emitter)**: Emits x86-64 / AArch64 machine code with Carry-Flag (`CF`) error ABI (`CarryFlagAbiLowering`) and direct SPIR-V 1.6 binary words (`emit_spirv_binary`) in milliseconds with zero external compiler dependencies.
   - **Track B (E-Graph Micro-Architectural Peephole & Release Lowering)**: To match or surpass 25 years of LLVM target-specific instruction selection without inheriting C++ MLIR's pointer-graph overhead, `mente-codegen` pairs its E-Graph instruction combiner (`fma`, bmi2 bit-extract, AVX-512 mask registers) with an optional post-`MIR` machine-code emission pass after all Mente contracts and bounds checks have already been erased.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Upstream C++ `MLIR` / `LLVM` heap object graphs (`mlir::Operation*`)** | Upstream C++ MLIR allocates every operation, attribute, and use-def list as heap-allocated 64-bit pointer-linked C++ objects (120–256 bytes per op), causing severe L1/L2 cache thrashing during optimization passes, adding a multi-gigabyte C++ build dependency, and lacking native per-node refinement interval lattices (`ValueLattice`). |
| **Source-level file rewriting mutation testing (`cargo-mutants`)** | Mutating `.rs` or `.mn` source files on disk and invoking the full compiler + linker per mutant takes minutes to hours. Mutating `MirOp` values in memory inside `MirRegionSoA` executes in microseconds (34 µs for the demo region's four mutants, measured 2026-11-04). |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="#[repr(align(64))]" min="1" reason="The MirRegionSoA header is aligned to 64 bytes; its node arrays are not" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub struct NodeId(pub u32)" min="1" reason="MIR nodes use 32-bit index handles instead of 64-bit heap pointers" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn eval_in_memory" min="1" reason="MirRegionSoA includes an in-memory interpreter over its SoA arrays" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn run_in_memory_mutation_gate" min="1" reason="mente-mir implements in-memory opcode mutation prover" -->
