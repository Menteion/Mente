---
title: Algebraic Data Types, Flow-Sensitive Union Narrowing, and Null Safety
version: 0.1.0
status: active
date: 2026-11-04
---

# Algebraic Data Types, Flow-Sensitive Union Narrowing, and Null Safety

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Product types (`struct`) with embedded invariants**:
   - `Specified`: `struct` declarations have named fields, built by `Name(field: e, ...)` in the order of declaration (`core-calculus.md`, item 13). Field defaults and `invariant { ... }` blocks relating several fields are constructs of a later milestone: M2 has field refinements only ([`struct-invariants-m2`](../docs/design-review.md#struct-invariants-m2)).
   - `Implemented`: `parse_struct` parses a struct's fields, their defaults and the lines of its `invariant` block, keeping types and expressions as raw strings.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_struct_fields_defaults_and_invariants_are_parsed" min="1" reason="parse_struct keeps each field, default and invariant line" -->
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_field_default_starts_at_a_top_level_equals" min="1" reason="A default starts at the first = outside brackets" -->
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_invariant_block_may_hold_braces" min="1" reason="An invariant block may hold braces" -->
   - `Specified` (after M2): an invariant is proven at construction and at the end of every `mut` projection.
2. `Specified` **Sum types (`enum`)**:
   - Nominal tagged unions are declared via `enum`, whose variants hold nothing or a payload of named fields, and are always named with their enum (`core-calculus.md`, item 14).
   - `match` expressions MUST be exhaustive at compile time, and M2's patterns are its minimal set: variants with named bindings, integer and `bool` literals, `none` and a name for a `?T`, and `_` on integers only ([`match-patterns`](../docs/design-review.md#match-patterns)).
   - Ad-hoc unions of existing types (`A | B`) and narrowing a binding's type with `is` wait past M2 ([`optional-semantics`](../docs/design-review.md#optional-semantics)); a pattern binds a payload to a name of its own instead.
   - `Implemented`: `parse_enum` parses an enum's variant list, keeping each variant as raw text.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_enum_variants_are_parsed" min="1" reason="parse_enum keeps each variant of the list" -->
   - `Implemented`: a `match` on an enum, a `bool` or an integer is exhaustive at compile time, every variant named and `_` standing only on an integer, with M2's patterns; the patterns of `?T` come with `?T` itself (`core-calculus.md`, item 14).
     <!-- @assert-count target="compiler/mente-sema/src/check.rs" symbol="fn test_a_match_covers_every_value_once" min="1" reason="A match misses no value and reaches every arm, and _ stands only on integers" -->
3. `Specified` **First-class optional types `?T` and elimination of `.unwrap()`**:
   - Nullable values are typed `?T`, a tagged type of its own that does not flatten: `??T` is a type apart ([`optional-semantics`](../docs/design-review.md#optional-semantics)). It takes no tag byte where `T` has a niche the spec names, such as `?u16[1..=65535]` and `?bool` (`core-calculus.md`, item 21).
   - **Prohibition of unchecked `.unwrap()`**: Neither the language nor the standard library provides a panicking `.unwrap()` method on `?T`. A present value is reached by binding it to a name of its own, with `guard val v = opt else { ... }`, `if val v = opt { ... }` or `match`, or read through `opt?.field`, which only reads (`core-calculus.md`, item 15).
   - The lexer has `NoneLit`, `Question`, `QuestionDot` and `QuestionQuestion` tokens, which nothing parses yet; `examples/` is checked for `.unwrap()` by the directive below.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **`Option<T>::unwrap()` / `.expect()` (Rust)** | In production codebases and AI-generated Rust code, `.unwrap()` is the single largest source of unexpected runtime panics, turning compile-time optionality back into unchecked `NullPointerException` crashes. Flow narrowing via `guard` is just as concise and never panics. |
| **Flow-sensitive narrowing of a binding (`if x != none`, `if x is Variant`, as in TypeScript and Kotlin)** | One name would have two types in two regions of its block, against one name, one binding (`core-calculus.md`, item 6). `guard val v = x` and a pattern give the narrowed value a name of its own, at the cost of one more name. |
| **A flattened `?T`, the union of `T` and `none`** | `??T` would collapse into `?T`, so a list of optional values could not tell an empty list from a stored `none`, which is unsound for generic code. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="NoneLit," min="1" reason="Lexer defines NoneLit token for ?T optional types" -->
<!-- @assert-count target="examples/01_refinement_and_units.mn" symbol=" -> ?usize" min="1" reason="Example 01 uses first-class ?usize optional return type" -->
<!-- @assert-absence target="examples" symbol=".unwrap()" glob="*.mn" reason="Mente forbids panicking .unwrap() on optional and error types" -->
