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
   - **Entering and leaving a refinement**: `T[a..b]` is a subtype of `T`, so a refined value is accepted wherever `T` is, and widening needs no conversion. A value enters a refinement only when its interval is known to lie inside it: a literal in range, the result of arithmetic on refined operands (`u8[0..10] + u8[0..10]` has the interval `[0, 18]`), or a value narrowed by a `guard` or `if` condition. Anything else is a compile-time error carrying Tier 1's counterexample and `guard` prescription (`ghost-contracts-and-totality.md`).
   - **Bounds that name other bindings**: a bound may name another binding (`usize[0..items.len]`, `usize[0..CAP]`) only when that binding cannot change during the refined value's scope: a read-only parameter, a `val`, or a compile-time parameter. A bound over a `var` or a `mut` parameter is an error, since a push or a reassignment would silently invalidate every value refined by it. The refined binding itself may be a `var` (`var lo: usize[0..=items.len]`).
2. **Dependent slice index bounds-check elimination (`Specified`)**:
   - Slice indices may depend on the target slice's length (`i: usize[0..items.len]`).
   - When an index's interval lies inside `[0, slice_len)`, the compiler MUST omit runtime bounds-check instructions (`CMP` + `JA panic`) on `items[i]`.
   - `Implemented`: the predicate `ValueLattice::proves_in_bounds(slice_len)` in `mente-mir`. Nothing computes the intervals it is given or emits code that could omit a check.
3. **First-class fixed-point and decimal primitives (`Specified`)**:
   - Mente provides built-in binary and decimal fixed-point types: `q16.16` (signed 32-bit, 16 integer + 16 fractional bits), `q1.15` (signed 16-bit DSP), `q0.8` (unsigned 8-bit normalized weight), parameterized `fix<I, F>`, and financial decimal `dec<Scale>`.
   - Fixed-point arithmetic is deterministic and bit-identical across x86-64, AArch64, RISC-V, and GPU/NPU targets, avoiding IEEE-754 non-associativity and FMA rounding drift.
   - `Implemented`: the `@layout` prover knows the byte sizes of `q16.16`, `q1.15` and `q0.8`, and the MIR interpreter has saturating `AddSatQ16` / `SubSatQ16` opcodes.
4. **Overflow and division semantics (`Specified`)**:
   - Integer and fixed-point `+`, `-`, `*`, shifts and narrowing conversions trap on overflow in every build, debug and release alike, unless Tier 1 proves the result in range, in which case no check is emitted. `/` and `%` trap on a zero divisor under the same rule. A trap ends the program (under `mn test`, the test); it is not an error value, so code that must recover checks first with `guard`.
   - Wrapping and saturation are never implicit: saturating arithmetic is written with `sat<T>` or the operators `+|`, `-|`, and modular arithmetic with `wrap<T>` or `+%`, `-%`. The lexer has no tokens for these operators yet.
   - Floating-point arithmetic follows IEEE-754 and does not trap. A refined float (`f64[0.0..=1.0]`) excludes `NaN` and the infinities, so a result entering it follows the refinement rules in item 1.
   - Code that may not branch (`@branchless`, `@kernel`) must have every such check proven away, since a trap is a branch.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Library wrapper structs (`NonZeroU32`, `Bounded<T, MIN, MAX>`)** | Library wrappers do not compose across arithmetic expressions (`a + b` cannot automatically infer `[MIN_A + MIN_B .. MAX_A + MAX_B]`) and require constant `.get()` / `TryFrom` unwrapping boilerplate. |
| **Unchecked indexing (`unsafe { get_unchecked(i) }` in Rust / `operator[]` in C++)** | Forces engineers to choose between runtime branch overhead on every inner-loop load or memory-unsafe UB. Dependent interval indexing (`usize[0..items.len]`) achieves C `operator[]` speed with mathematical safety. |
| **Wrapping silently on overflow in release builds (Rust's default) or undefined behaviour (signed overflow in C)** | Makes a release build compute something other than the build that was tested, and turns the bug a checked build would have caught into a wrong answer. Because Tier 1 removes every check it can prove, trapping costs only where the range is genuinely unknown. |
| **Bounds over mutable bindings (`usize[0..items.len]` with `var items`)** | A `push` or reassignment would change the bound under every value refined by it, so each mutation would have to re-check or invalidate them. Restricting bounds to bindings that cannot change keeps a refinement true for its whole scope. |
| **IEEE-754 floating-point only (`f32`/`f64`)** | Floating-point addition is non-associative and varies across CPU micro-architectures and compiler flags, breaking deterministic replay, lockstep physics, and quantized engine scoring. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="pub const fn proves_in_bounds" min="1" reason="ValueLattice statically proves slice index bounds in mente-mir" -->
<!-- @assert-count target="compiler/mente-mir/src/lib.rs" symbol="AddSatQ16" min="1" reason="MIR includes native saturating q16.16 fixed-point opcode" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="usize[0..items.len]" min="2" reason="Example 01 demonstrates dependent slice index refinement" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="sat<q16.16>" min="1" reason="Example 02 demonstrates saturating q16.16 fixed-point arithmetic" -->
