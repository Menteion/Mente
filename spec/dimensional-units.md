---
title: Zero-Cost Physical and Domain Dimensional Unit Algebra
version: 0.1.0
status: active
date: 2026-11-04
---

# Zero-Cost Physical and Domain Dimensional Unit Algebra

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Unit declaration syntax (`Implemented` in `compiler/mente-syntax/src/lib.rs`)**:
   - Base units, scaled units, and composite derived units are declared at module scope using the `unit` keyword (`TokenKind::KwUnit`, `Item::UnitDecl`):
     ```rust
     unit sec, ms = 0.001 * sec
     unit meter, kg
     unit newton = kg * meter / (sec * sec)
     unit token, byte
     ```
2. **Dimensional algebra and normalization (`Specified` in `mente-sema`)**:
   - Numeric types parameterized by a unit (`f64<meter>`, `f64<meter / sec>`, `u64<byte>`) represent abelian group exponents over base dimensions (e.g., `newton` normalizes to $\text{kg}^1 \cdot \text{meter}^1 \cdot \text{sec}^{-2}$).
   - Addition, subtraction, and relational comparison (`+`, `-`, `<`, `<=`, `==`) require identical normalized dimension vectors; violating this is a hard compile-time error.
   - Multiplication and division (`*`, `/`) add and subtract dimension exponent vectors automatically (`f64<meter> / f64<sec>` yields `f64<meter / sec>`).
3. **100% compile-time erasure (`Implemented` in `MIR` lowering model)**:
   - After `mente-sema` verifies dimensional consistency, all `<unit>` parameters are completely erased. At the `MIR` and machine-code levels, `f64<newton>` is an unboxed `f64` in an `XMM` / `ZMM` register with zero wrapper overhead.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Newtype struct wrappers (`struct Meter(f64)`, `struct Second(f64)`)** | Cannot express arbitrary derived algebraic combinations (`meter / (sec * sec)`) without manually implementing dozens of `Mul` and `Div` trait impls for every pair of units. |
| **C++ `std::chrono` / Boost.Units template metaprogramming** | Produces hundreds of lines of unreadable template instantiation diagnostics on type mismatch and slows compilation by instantiating deep recursive class templates. Native compiler abelian-exponent normalization resolves in `O(1)` vector operations. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Unit," min="1" reason="Lexer defines first-class Unit keyword" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="ItemDecl::Unit" min="1" reason="AST represents top-level unit declarations" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol="unit newton = kg * meter / (sec * sec)" min="1" reason="Example 01 declares composite physical units" -->
