---
title: From-Scratch 64-Byte SoA Mente IR (MIR) and Multi-Backend Codegen
version: 0.1.0
status: active
date: 2026-11-04
---

# From-Scratch 64-Byte `SoA` Mente IR (`MIR`) and Multi-Backend Codegen

**Governs:** `compiler/mente-mir`, `compiler/mente-codegen`

## 1. Specification

1. `Implemented` **Index-addressed structure-of-arrays region** (`MirRegionSoA`, `compiler/mente-mir/src/lib.rs`):
   <!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="fn test_nodes_are_rows_addressed_by_index" min="1" reason="Each node is one row of the parallel arrays, addressed by its NodeId" -->
   <!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="fn test_interpreter_compares_and_selects" min="1" reason="The interpreter's comparisons, select and jump" -->
   <!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="fn test_branchless_integrate_cell_mir_and_mutation_prover" min="1" reason="The interpreter evaluates a region over its arrays" -->
   - Every `MIR` instruction node is addressed by a 32-bit index handle (`NodeId(pub u32)`, `4` bytes) rather than a 64-bit heap pointer (`8` bytes).
   - `MirRegionSoA` stores one straight-line region in parallel vectors: `opcodes: Vec<MirOp>`, `types: Vec<MirType>` (`I64`, `Q16_16`, `Bool`), `inputs: Vec<[NodeId; 3]>`, `lattices: Vec<ValueLattice>` (an `i128` interval and an alignment per node), `lines: Vec<u32>`, plus `is_branchless_guaranteed: bool` and `ret_node`.
   - `MirOp` today: `Param`, `Const`, `Add`, `Sub`, `AddSatQ16`, `SubSatQ16`, `Shr`, `CmpGe`, `CmpLt`, `CmpEq`, `SelectBranchless`, `BranchCondJump`. There are no loads, stores, calls, loops or nested regions.
   - `#[repr(align(64))]` on `MirRegionSoA` aligns the struct header only; the node arrays are ordinary heap `Vec`s.
   - `Specified`: packing nodes into 64-byte cache-line buckets.
2. `Specified` **RVSDG canonicalization and Equality Saturation (`E-Graphs`)**:
   - A rewrite pass performs non-destructive algebraic rewrites directly over the `SoA` arrays (e.g., folding identity additions while preserving `ValueLattice` bounds), avoiding phase-ordering cliffs. Neither RVSDG structure nor an e-graph exists yet.
3. **In-memory `MIR` mutation prover**:
   - `Implemented`: `run_in_memory_mutation_gate` flips `Add` ↔ `Sub`, `AddSatQ16` ↔ `SubSatQ16` and `CmpGe` ↔ `CmpLt`, swaps the arms of each `SelectBranchless`, and counts the mutants that the supplied `(inputs, expected)` vectors kill. `mn test --mutate` runs it on one hand-written region with two vectors.
     <!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="fn test_mutation_gate_counts_survivors" min="1" reason="The gate counts each mutant and the ones the vectors kill" -->
     <!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="fn test_branchless_integrate_cell_mir_and_mutation_prover" min="1" reason="The demo region's four mutants are all killed" -->
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_test_mutate_runs_the_demo_gate" min="1" reason="mn test --mutate runs the gate on the demo region" -->
     <!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="fn test_mutation_gate_flips_each_comparison" min="1" reason="CmpLt and CmpGe flip into each other" -->
   - `Specified`: running it on regions lowered from `.mn` source, with the module's `ensures` contracts and `@example` assertions as the oracle.
   - `Specified` **What passing means**: some mutants cannot be killed by any test because they change nothing observable (an equivalent mutant: flipping `x + 0` to `x - 0`). So the gate does not demand that every mutant die. It passes when every mutant is either killed or marked equivalent with a written reason; `mn test --mutate` lists each survivor with its source line. A project may instead set a kill-rate threshold, and then the report still lists the survivors.
4. `Specified` **Code generation through Cranelift (Engineering Guardrail #2)**:
   - Today `compiler/mente-codegen` has only `CarryFlagAbiLowering`, a table of the registers the error ABI would use, and `emit_spirv_binary`, a SPIR-V-shaped word stream that is not a valid module.
   - **High-level semantic optimization lives 100% in `MIR`**: Bounds-check elimination (`ValueLattice::proves_in_bounds`), Ghost Contract `Assume` propagation, `AoSoA` (`Tiled<T, lanes: N>`) address calculation, and `@branchless` `SelectBranchless` canonicalization are completed inside `MirRegionSoA` before any target backend runs.
   - **Native code**: `mente-codegen` lowers optimised MIR to Cranelift IR and lets Cranelift emit x86-64 and AArch64 machine code. Cranelift is written in Rust, so no C++ toolchain enters the build; `mente-mir` itself depends on nothing, and only `mente-codegen` depends on Cranelift.
   - **What Cranelift does, and what Mente does** (checked against `cranelift-codegen` 0.135.8): Cranelift selects instructions, runs its e-graph mid-end and allocates registers. Mente decides in MIR the two things Cranelift leaves to its user. Inlining: Cranelift performs only the inlines it is asked for, and Mente asks across modules only for `@inline` and generic callees (`holographic-module.md`), and inside a module by a size heuristic. Vectorization: Cranelift has no auto-vectorizer, so `@simd(lanes: N)` loops are lowered to vector operations in MIR.
   - **Vectors are 128 bits wide**: Cranelift's x64 and AArch64 backends handle vector values of at most 128 bits, so `mente-codegen` splits a wider lane group into 128-bit operations (`hardware-contracts.md`).
   - **No floating-point or vector `select` reaches Cranelift from `@branchless` code**, because on x64 Cranelift emits those as a jump around a move. `mente-codegen` lowers a vector select to `bitselect` over a comparison mask and a scalar float select to an integer `select` on the bit pattern; integer selects become `cmov` / `csel`. After emission, `mn` checks each `@branchless` function's machine code (`E0403`, `hardware-contracts.md`).
   - **Errors return as a two-register tagged value**: Cranelift has no flags type and no calling convention that returns a status in the carry flag, so `T ! E` is lowered to a tag and a payload in two return registers through Cranelift's multi-value returns. The Carry-Flag convention of `error-handling-abi.md` is tested later, on a custom emitter or a Cranelift fork.
   - **A backend written in Mente (stage 3)**: the performance stage replaces Cranelift with a native backend written in Mente, for x86-64 and AArch64, as [`long-term-order`](../docs/design-review.md#long-term-order) decided. It becomes the default only when it gives the same results as Cranelift on the whole corpus and neither its generated code, its compile latency nor its debug and profiler information is worse; until then Cranelift is the only backend.
     - `Hypothesis`: its code matches LLVM-compiled equivalents, measured program against program.
   - **SPIR-V** waits for `@kernel`'s execution model (`hardware-contracts.md`).

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Writing the native backend from scratch first** | Register allocation, instruction selection and object-file emission for two architectures are years of work before the first example runs natively, and they test none of Mente's own ideas, which all live above MIR. Cranelift supplies them in pure Rust; the backend written in Mente arrives in stage 3, after self-hosting, behind the same MIR. |
| **Upstream C++ `MLIR` / `LLVM` heap object graphs (`mlir::Operation*`)** | Upstream C++ MLIR allocates every operation, attribute, and use-def list as heap-allocated 64-bit pointer-linked C++ objects (120–256 bytes per op), causing severe L1/L2 cache thrashing during optimization passes, adding a multi-gigabyte C++ build dependency, and lacking native per-node refinement interval lattices (`ValueLattice`). |
| **Requiring 100% of mutants killed** | Unreachable in general: an equivalent mutant survives every test, so the rule would push authors to write meaningless tests or to stop running the gate. Accounting for every survivor keeps the gate strict without demanding the impossible. |
| **Source-level file rewriting mutation testing (`cargo-mutants`)** | Mutating `.rs` or `.mn` source files on disk and invoking the full compiler + linker per mutant takes minutes to hours. Mutating `MirOp` values in memory inside `MirRegionSoA` executes in microseconds (34 µs for the demo region's four mutants, measured 2026-11-04). |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="#[repr(align(64))]" min="1" reason="The MirRegionSoA header is aligned to 64 bytes; its node arrays are not" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub struct NodeId(pub u32)" min="1" reason="MIR nodes use 32-bit index handles instead of 64-bit heap pointers" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn eval_in_memory" min="1" reason="MirRegionSoA includes an in-memory interpreter over its SoA arrays" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub fn run_in_memory_mutation_gate" min="1" reason="mente-mir implements in-memory opcode mutation prover" -->
