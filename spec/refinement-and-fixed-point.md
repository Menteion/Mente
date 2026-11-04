---
title: First-Class Refinement Intervals, Dependent Slice Indexing, and Fixed-Point Algebra
version: 0.1.0
status: active
date: 2026-11-04
---

# First-Class Refinement Intervals, Dependent Slice Indexing, and Fixed-Point Algebra

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`

## 1. Specification

None of the type rules below are checked yet: `mente-syntax` keeps a type as a raw string and `mente-sema` has no type checker.

1. **Bounded interval types (`Specified`)**:
   - Any scalar primitive (`i8`..`i128`, `u8`..`u128`, `usize`, `f32`, `f64`) MAY be refined by a compile-time half-open `[min..max]` or closed `[min..=max]` interval:
     - `u32[0..1024]` (values `0 <= v < 1024`)
     - `u16[1..=65535]` (non-zero port number)
     - `f64[0.0..=1.0]` (normalized probability, implicitly forbidding `NaN` and `+-Inf`)
2. **Dependent slice index bounds-check elimination (`Specified`)**:
   - Slice indices may depend on the target slice's length (`i: usize[0..items.len]`).
   - When an index's interval lies inside `[0, slice_len)`, the compiler MUST omit runtime bounds-check instructions (`CMP` + `JA panic`) on `items[i]`.
   - `Implemented`: the predicate `ValueLattice::proves_in_bounds(slice_len)` in `mente-mir`. Nothing computes the intervals it is given or emits code that could omit a check.
3. **First-class fixed-point and decimal primitives (`Specified`)**:
   - Mente provides built-in binary and decimal fixed-point types: `q16.16` (signed 32-bit, 16 integer + 16 fractional bits), `q1.15` (signed 16-bit DSP), `q0.8` (unsigned 8-bit normalized weight), parameterized `fix<I, F>`, and financial decimal `dec<Scale>`.
   - Fixed-point arithmetic is deterministic and bit-identical across x86-64, AArch64, RISC-V, and GPU/NPU targets, avoiding IEEE-754 non-associativity and FMA rounding drift.
   - `Implemented`: the `@layout` prover knows the byte sizes of `q16.16`, `q1.15` and `q0.8`, and the MIR interpreter has saturating `AddSatQ16` / `SubSatQ16` opcodes.
4. **Explicit overflow semantics (`Specified`)**:
   - Standard integer and fixed-point arithmetic checks overflow in debug/verification modes; saturating (`sat<T>`, `+|`, `-|`) and modular wrapping (`wrap<T>`, `+%`, `-%`) behaviors are explicit in the type or operator. The lexer has no tokens for `+|`, `-|`, `+%` or `-%` yet.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Library wrapper structs (`NonZeroU32`, `Bounded<T, MIN, MAX>`)** | Library wrappers do not compose across arithmetic expressions (`a + b` cannot automatically infer `[MIN_A + MIN_B .. MAX_A + MAX_B]`) and require constant `.get()` / `TryFrom` unwrapping boilerplate. |
| **Unchecked indexing (`unsafe { get_unchecked(i) }` in Rust / `operator[]` in C++)** | Forces engineers to choose between runtime branch overhead on every inner-loop load or memory-unsafe UB. Dependent interval indexing (`usize[0..items.len]`) achieves C `operator[]` speed with mathematical safety. |
| **IEEE-754 floating-point only (`f32`/`f64`)** | Floating-point addition is non-associative and varies across CPU micro-architectures and compiler flags, breaking deterministic replay, lockstep physics, and quantized engine scoring. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub const fn proves_in_bounds" min="1" reason="ValueLattice statically proves slice index bounds in mente-mir" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="AddSatQ16" min="1" reason="MIR includes native saturating q16.16 fixed-point opcode" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="usize[0..items.len]" min="2" reason="Example 01 demonstrates dependent slice index refinement" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="sat<q16.16>" min="1" reason="Example 02 demonstrates saturating q16.16 fixed-point arithmetic" -->
