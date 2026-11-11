---
title: Zero-Cost Physical and Domain Dimensional Unit Algebra
version: 0.1.0
status: active
date: 2026-11-04
---

# Zero-Cost Physical and Domain Dimensional Unit Algebra

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Unit declaration syntax**:
   - `Specified`: base units, scaled units, and composite derived units are declared at module scope using the `unit` keyword:
     ```rust
     unit sec, ms = 0.001 * sec
     unit meter, kg
     unit newton = kg * meter / (sec * sec)
     unit token, byte
     ```
   - `Implemented`: the parser keeps each `unit` declaration's text (`TokenKind::Unit`, `ItemDecl::Unit`); nothing interprets it yet.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_unit_declarations_keep_their_text" min="1" reason="Each unit declaration is kept as its text" -->
2. `Specified` **A unit is a dimension and a scale**:
   - Numeric types parameterized by a unit (`f64<meter>`, `f64<meter / sec>`, `u64<byte>`) carry a unit with two parts:
     - a **dimension**: integer exponents over the base units (`newton` normalizes to $\text{kg}^1 \cdot \text{meter}^1 \cdot \text{sec}^{-2}$);
     - a **scale**: an exact rational factor relative to the unit built from base units alone (`ms` has dimension $\text{sec}^1$ and scale $1/1000$). A decimal in a `unit` declaration (`0.001 * sec`) is read as the exact rational it spells.
   - Two units are the same unit when both their dimensions and their scales are equal; `ms` and `sec` are different units of the same dimension.
3. `Specified` **Arithmetic on quantities**:
   - Addition, subtraction and every comparison (`+`, `-`, `<`, `<=`, `>`, `>=`, `==`, `!=`) require the same unit, scale included. `5.0_ms + 1.0_sec` is a compile-time error (`E0601`) whose help line offers both explicit conversions: `5.0_ms.to<sec>() + 1.0_sec` and `5.0_ms + 1.0_sec.to<ms>()`.
   - Multiplication and division combine units: dimensions add and subtract, scales multiply and divide (`f64<meter> / f64<sec>` is `f64<meter / sec>`; `f64<ms> * f64<meter>` is `f64<ms * meter>`). A quotient of the same dimension is a plain number only when the scales cancel (`f64<meter> / f64<meter>` is `f64`); otherwise convert an operand first.
   - Exponents are integers. `sqrt` is defined on a quantity only when every exponent is even.
4. `Specified` **Explicit conversion with `.to<U>()`**:
   - `x.to<U>()` is the only conversion between units. It requires the same dimension (`E0602` otherwise) and multiplies by the ratio of the scales, computed exactly at compile time.
   - When that ratio or its reciprocal is an integer, the conversion is a single multiplication or division by that integer, so `500.0_ms.to<sec>()` is exactly `0.5_sec`.
   - On an integer quantity, the ratio must be an integer (`5_sec.to<ms>()` is `5000_ms`); converting towards a coarser unit (`u64<ms>` to `u64<sec>`) is an error (`E0603`), because it would discard a remainder. Divide explicitly or convert to a fixed-point or floating type first.
5. `Specified` **Functions generic over a unit**:
   - A unit parameter is a compile-time parameter of kind `Unit`: `fn mean<comptime U: Unit>(xs: []f64<U>) -> f64<U>`. The function's body is checked once for every `U`.
6. `Specified` **Compile-time erasure**:
   - After `mente-sema` verifies unit consistency, all `<unit>` parameters are erased. At the `MIR` and machine-code levels, `f64<newton>` is an unboxed `f64` in an `XMM` / `ZMM` register with zero wrapper overhead. The only instructions units ever add are the multiplications or divisions written as `.to<U>()`. No `.mn` code is lowered to MIR yet.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Newtype struct wrappers (`struct Meter(f64)`, `struct Second(f64)`)** | Cannot express arbitrary derived algebraic combinations (`meter / (sec * sec)`) without manually implementing dozens of `Mul` and `Div` trait impls for every pair of units. |
| **C++ `std::chrono` / Boost.Units template metaprogramming** | Produces hundreds of lines of unreadable template instantiation diagnostics on type mismatch and slows compilation by instantiating deep recursive class templates. Native compiler abelian-exponent normalization resolves in `O(1)` vector operations. |
| **Comparing dimensions only, so `ms + sec` adds raw numbers** | Type-checks `5.0_ms + 1.0_sec` and returns `6.0` of no meaningful unit: exactly the class of error units exist to catch. |
| **Implicit conversion to a common unit on `+` (as Python's Pint does; F# instead rejects mixed units)** | Hides a multiplication and, in floating point, a rounding inside an operator that reads as plain addition, and leaves open which unit `a + b` has. One explicit `.to<U>()` keeps every conversion visible in the source and in the instruction stream. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Unit," min="1" reason="Lexer defines first-class Unit keyword" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="ItemDecl::Unit" min="1" reason="AST represents top-level unit declarations" -->
<!-- @assert-count target="examples/05_units_and_generics.mn" symbol="unit newton = kg * meter / (sec * sec)" min="1" reason="Example 05 declares composite physical units" -->
<!-- @assert-count target="examples/05_units_and_generics.mn" symbol="500.0_ms.to<sec>()" min="1" reason="Example 05 converts milliseconds to seconds explicitly before adding them" -->
