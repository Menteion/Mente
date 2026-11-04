---
title: Algebraic Data Types, Flow-Sensitive Union Narrowing, and Null Safety
version: 0.1.0
status: active
date: 2026-11-04
---

# Algebraic Data Types, Flow-Sensitive Union Narrowing, and Null Safety

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Product types (`struct`) with embedded invariants (`Implemented` in `mente-syntax` and `mente-ast`)**:
   - `struct` declarations support named fields, default field values, and optional `invariant { ... }` blocks verified at construction and at the end of every `mut` projection.
2. **Sum types (`enum`) and ad-hoc type unions (`A | B`) (`Implemented` in parser, `Specified` flow narrowing)**:
   - Nominal tagged unions are declared via `enum`. Ad-hoc unions of existing types are written `A | B`.
   - `match` expressions MUST be exhaustive at compile time.
   - **Flow-sensitive type narrowing (`is`)**: Inside `if x is Packet.Data { ... }` or after `guard x is Packet.Data else { return }`, the static type of `x` narrows automatically to `Packet.Data` without requiring destructuring bindings.
3. **First-class optional types `?T` and elimination of `.unwrap()` (`Implemented`)**:
   - Nullable values are typed `?T` (equivalent to `T | none`) with automatic niche bit-packing (e.g., `?NonNullPtr` and `?u16[1..=65535]` occupy zero extra tag bytes).
   - **Prohibition of unchecked `.unwrap()`**: Neither the language nor the standard library provides a panicking `.unwrap()` method on `?T`. Accessing `T` from `?T` MUST proceed via compile-time flow narrowing (`guard val v = opt else { ... }`, `if opt != none`), safe navigation (`opt?.field`), or default coalescing (`opt ?? fallback`).

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **`Option<T>::unwrap()` / `.expect()` (Rust)** | In production codebases and AI-generated Rust code, `.unwrap()` is the single largest source of unexpected runtime panics, turning compile-time optionality back into unchecked `NullPointerException` crashes. Flow narrowing via `guard` is just as concise and never panics. |
| **Nominal-only enums without flow narrowing (Rust / Haskell)** | Forces repetitive `if let Some(x) = x` shadowing or single-arm `match` boilerplate even after an explicit check. Flow-sensitive narrowing (`if x != none`, `if x is Variant`) combines TypeScript ergonomics with static native layout. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="NoneLit," min="1" reason="Lexer defines NoneLit token for ?T optional types" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol=" -> ?usize" min="1" reason="Example 01 uses first-class ?usize optional return type" -->
<!-- @assert-absence target="examples" symbol=".unwrap()" glob="*.mn" reason="Mente forbids panicking .unwrap() on optional and error types" -->
