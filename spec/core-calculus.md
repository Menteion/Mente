---
title: The Core Calculus of M1
version: 0.1.0
status: active
date: 2026-11-08
---

# The Core Calculus of M1

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`, and the reference interpreter `mente-interp` once it exists

This file is normative for the language of milestone M1 ([`core-calculus-home`](../docs/design-review.md#core-calculus-home)). It says which programs M1 accepts, what they compute, and when they trap. The reference interpreter implements it rule by rule; the conformance suite tests it; MIR and native code must agree with it, since the three execution paths are compared (`ROADMAP.md`, M1). Each milestone extends it. Its concrete syntax is in `grammar.md`; where another spec file says more about a mechanism, this file states only what M1 takes from it.

## 1. Specification

1. `Specified` **What M1 contains**: integers, `bool`, fixed-size arrays `[N]T` held as values, functions with read-only and `mut` parameters, `val` and `var`, `if`, `while` with `break` and `continue` ([`loop-exits`](../docs/design-review.md#loop-exits)), `guard`, `return`, `requires` and `ensures`, refined integer types, and `todo`.
   - A program that uses anything else (structs, enums, `?T`, `T ! E`, effects, floating-point and fixed-point values, units, `own` and `in` parameters, views, generics, traits, `comptime`, `wrap<T>` and `sat<T>`, parallelism) is rejected with a diagnostic that names the construct and the milestone that brings it. Those mechanisms stay specified in their own files.
2. `Specified` **Integer types are ranges** ([`integer-conversions`](../docs/design-review.md#integer-conversions)):
   - An integer value is a mathematical integer. An integer type names a range of them: `uN` holds `0 ..= 2^N - 1` and `iN` holds `-2^(N-1) ..= 2^(N-1) - 1`, for `N` in 8, 16, 32, 64 and 128. On every M1 target `usize` is 64 bits wide and unsigned.
   - There are no conversions between integer types. Any integer expression may flow into any integer type, and is checked where it is narrowed (item 4).
   - A refined type `T[a..b]` or `T[a..=b]` names the part of `T`'s range inside the interval. Its bounds are integer literals or names of bindings that cannot change in the refined value's scope: a `val` or a read-only parameter, or `.len` of an array (`refinement-and-fixed-point.md`, item 1). `T[a..b]` is a subtype of `T`.
   - A plain type is a range checked at run time where Tier 1 cannot prove it; a refined type is a range proven at compile time (item 8).
3. `Specified` **`bool` and arrays**:
   - `bool` holds `true` and `false`. No integer converts to `bool` or back.
   - `[N]T` holds `N` values of type `T`, where `N` is a non-negative integer literal and `T` is any M1 type, arrays included. `a.len` is `N`. An array is a value: binding, passing and returning it copies it, and no two bindings ever share one.
4. `Specified` **Integer arithmetic is exact, and checked where it is narrowed** ([`overflow-check-cost`](../docs/design-review.md#overflow-check-cost)):
   - `+`, `-`, `*`, `/`, `%` and unary `-` compute the exact mathematical result, whatever the integer types of their operands. `/` truncates toward zero, `%` takes the sign of its left operand, and a zero divisor traps.
   - A value is narrowed, and checked against the type it enters, at a binding's initializer, an assignment, an argument, a returned value, an element of an array literal, and an element store. A value outside the type's range traps.
   - The bit operations `&`, `|`, `^` and `~` and the shifts `<<` and `>>` are narrowing points too. The operands of `&`, `|` and `^` have one integer type, and the operation works on their two's-complement bits in that type. `x << k` is `x · 2^k`, then narrowed to `x`'s type; `x >> k` is `x / 2^k` rounded toward negative infinity. A shift amount outside `0 ..= width - 1` of `x`'s type traps.
   - An operation whose exact result, computed from the operands' types alone, might not fit in 128 bits is itself a narrowing point, checked against its operands' type; so `i128` and `u128` arithmetic is checked at every operator.
   - Comparisons (`==`, `!=`, `<`, `<=`, `>`, `>=`) compare the exact values, across integer types.
5. `Specified` **The type of an unannotated binding** ([`integer-inference`](../docs/design-review.md#integer-inference)):
   - `val x = e` and `var x = e` take `e`'s type. When the integer operands of `e` all have one type, `e` has that type, so `a + b` with `a` and `b` of type `u8` is a `u8`, and a sum above 255 traps at the binding.
   - An integer literal takes the type of the other operands, or of the place it is narrowed into; an expression of literals alone is an `i64`. A literal with a suffix (`255_u8`) has the suffix's type.
   - When `e`'s integer operands have different types, the binding must state its type. The diagnostic suggests the smallest standard type that holds both operands' ranges: `u32` for a `u8` and a `u32`, `i64` for an `i32` and a `u32`.
6. `Specified` **Bindings, blocks and statements**:
   - `val` binds a value that never changes; `var` binds one that assignment may change. A name is declared once in a function, its parameters and inner blocks included ([`no-shadowing`](../docs/design-review.md#no-shadowing)); a second declaration is an error whose fix suggests a new name.
   - Assignment `x = e` and element assignment `a[i] = e` apply to a `var` or a `mut` parameter, and `x += e` means `x = x + e`, likewise for the other arithmetic and bit operators.
   - A block runs its statements in order. Its value is its last statement when that is an expression; a function with a result type must produce a value of that type on every path that does not leave it by `return` or a trap.
   - `if c { a } else { b }` evaluates `c`, then one branch, and is an expression whose type both branches share. Without `else`, `if` is a statement.
   - `while c { body }` repeats `body` while `c` holds. `break` leaves the innermost loop, and `continue` starts its next iteration.
   - `guard c else { body }` continues when `c` holds. Its `body` must not reach its end: it leaves by `return`, `break`, `continue` or `todo`. After a `guard`, `c` is a fact Tier 1 holds.
   - `and` and `or` evaluate their right operand only when it decides the result; `not` negates.
   - `todo("reason")` has any type and traps when it runs (`surface-syntax.md`, item 7).
7. `Specified` **Functions and parameter modes** (`mutable-value-semantics.md`):
   - A function's parameters are read-only by default. A read-only parameter is a value the function cannot change.
   - A `mut` parameter receives a place, marked at the call site: `f(mut x)` or `f(mut a[i])`. Its value is copied in when the call starts and copied back on every return, so the caller's place holds the parameter's final value.
   - Within one call, a `mut` argument's root binding may not appear in any other argument: `f(mut a[i], a[j])` and `f(mut a[i], mut a[j])` are errors, whatever `i` and `j` are.
   - Arguments are evaluated left to right, each narrowed to its parameter's type, then the call runs. A function may call itself; running out of stack traps.
   - `requires` clauses are obligations at every call, and `ensures` clauses at every return of the function, where `result` names the returned value (`ghost-contracts-and-totality.md`, item 2). A clause outside the Tier-1 fragment needs `@checked`, which makes it a run-time check that traps when it fails, or `@prove`, which makes it a trusted axiom until `mn prove` runs.
8. `Specified` **What Tier 1 must prove, and what may stay a run-time check** ([`gradual-verification`](../docs/design-review.md#gradual-verification)):
   - Must be proven at compile time, or the program is rejected with the valuation Tier 1 cannot rule out and a `guard` suggestion: a value entering a refined type, a `requires` clause at a call, an `ensures` clause at a return, and a written loop `invariant`.
   - May stay a run-time check where Tier 1 cannot prove it, and is erased where it can: a value entering a plain integer type, an index inside its array, a non-zero divisor, and a shift amount in range. Tier 1 runs on refined and unrefined code alike.
   - Tier 1's facts come from the refinements of parameters and bindings, `requires` clauses of the function, `ensures` clauses of its callees, `if`, `while` and `guard` conditions, assignments, and the inductive facts at each loop head, where a `break` is one more exit of the loop (`ghost-contracts-and-totality.md`, items 2 and 4).
9. `Specified` **Traps**:
   - A trap ends the program. M1's traps are: a value outside the type it is narrowed into, a zero divisor, an index outside its array, a shift amount out of range, a `todo` reached, a failed `@checked` clause, and running out of stack.
   - The reference interpreter reports the trap's kind and the span of the operation that trapped. MIR and native code may defer checks within a region that has no visible effect and report the region instead (`refinement-and-fixed-point.md`, item 5); the three execution paths agree when they produce the same value, or trap of the same kind in the same region.
10. `Specified` **Evaluation order and the program's result**:
    - Operands, arguments and the elements of an array literal are evaluated left to right. An element assignment `a[i] = e` evaluates `i`, then `e`, then checks the index.
    - A program runs its `main` function, which takes no parameters. When `main` has a result type, which in M1 is an integer type, `bool` or an array of them, its value is the program's result; otherwise a program that does not trap has no result.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Explicit conversions between integer types (Rust's `as`)** | Under exact arithmetic, `val x: u8 = a + b` is already checked at the binding, so `val x: u8 = y` for a `u32` `y` is the same narrowing. A cast would mark one case and not the other, and Rust's `as`, which truncates silently, is a bug agents write often. With no casts, nothing truncates silently: a plain type is checked at run time and a refined type is proven. |
| **Widening an unannotated binding to hold its expression's static range (`u8 + u8` as `u16`)** | It never traps at the binding, but the binding's type would change with the way the expression is written, which neither a reader nor an agent can predict. When the operands share a type, keeping it matches what agents expect from other languages; when they differ, no type is obviously right, so the programmer chooses. |
| **Defaulting every unannotated integer binding to `i64`** | Simple, but arithmetic on `u8` values would yield `i64` bindings, against what the code says. `i64` is kept only for expressions of literals alone. |
| **Shadowing, as in Rust (`let x = x + 1`)** | Refinement bounds name bindings (`usize[0..n]`). With a second `n` in scope, neither a reader nor an agent can tell at a glance which `n` a bound means. One name, one binding, at the cost of a new name after a transformation. |
| **Leaving `break` and `continue` to a later milestone** | Agents reach for them in almost every loop, and the first agent benchmark, at the end of M1, would otherwise score how well they work around their absence. They add no type rule, and Tier 1 counts a `break` as one more exit of its loop. |
| **Flooring division, as in Python** | Hardware, C, Rust and the languages agents know best truncate toward zero; a flooring function can be added beside `/` without changing what `/` means. |
| **Exclusivity by index (allowing `f(mut a[i], a[j])` when `i != j` is proven)** | It would make the Law of Exclusivity depend on Tier 1 proving a disequality, which octagons cannot hold in general. M1 takes the simple rule by root binding; a relaxation can come later without breaking programs that M1 accepts. |
| **A small-step semantics written as inference rules** | The rules here are prose, one claim each, so the tools hold each one to its evidence as it is implemented. A mechanised model is not built in stage 1 ([`mechanised-model`](../docs/design-review.md#mechanised-model)). |

## 3. Verification assertions

<!-- @assert-absence target="examples" symbol=" as u" glob="*.mn" reason="Integers convert where they are narrowed; there is no as cast" -->
<!-- @assert-absence target="examples" symbol=" as i" glob="*.mn" reason="Integers convert where they are narrowed; there is no as cast" -->
<!-- @assert-present file="conformance/README.md" reason="The conformance suite tests this calculus as M1 implements it" -->
