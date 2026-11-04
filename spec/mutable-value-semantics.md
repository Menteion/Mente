---
title: Sigil-Free Mutable Value Semantics and the Law of Exclusivity
version: 0.1.0
status: active
date: 2026-11-04
---

# Sigil-Free Mutable Value Semantics and the Law of Exclusivity

**Governs:** `compiler/mente-syntax`, `compiler/mente-ast`, `compiler/mente-sema`

## 1. Specification

1. **Elimination of reference sigils (`&T`, `&mut T`, `*T`) (`Implemented` in `mente-syntax` and `mente-ast`)**:
   - Mente surface syntax has no address-of `&` or dereference `*` sigils on user types.
   - Instead, every function parameter specifies one of three explicit **parameter ownership modes** (`ParamMode` in `compiler/mente-ast/src/lib.rs`):

| Mode | Declaration syntax | Call-site syntax | Semantic contract | Machine ABI lowering |
| :--- | :--- | :--- | :--- | :--- |
| **Shared projection (`Let`)** | `x: T` | `f(a)` | Read-only immutable view; caller retains ownership; zero deep copies. | Register value if `sizeof(T) <= 16B`; `const T*` pointer if `> 16B`. |
| **In-place projection (`Mut`)** | `mut x: T` | `f(mut a)` | Exclusive mutable projection; no other argument to `f` may alias `a`. | Non-aliasing `T* restrict` pointer (`noalias`). |
| **Consuming transfer (`Own`)** | `own x: T` | `f(own a)` | Ownership transferred to `f`; `a` is statically invalidated at caller after call. | Register / bitwise move transfer; drop responsibility moves to callee. |

2. **Mandatory dual-site annotation (`Implemented` in `mente-syntax`)**:
   - Both `mut` and `own` MUST be written at the function declaration site (`fn step(mut p: Particle)`) **and** at every call site (`step(mut p)`).
3. **Law of Exclusivity and move invalidation (`Implemented` in `compiler/mente-sema/src/lib.rs`)**:
   - Within any call expression, two arguments passed as `mut` (or one `mut` and one `Let`/`own`) MUST NOT overlap in root storage path (`check_call_exclusivity`).
   - Any binding passed with `own` is recorded in `moved_vars` and MUST NOT be read or mutated on any subsequent path unless re-initialized.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Pointer/reference sigils `&T` and `&mut T` (Rust / C++)** | Forces programmers to choose physical pointer vs. value passing manually even for small register types (`&u64` adds unnecessary pointer indirection), and leaks reference wrapper types into generic signatures. MVS lets the compiler pick register vs. pointer ABI automatically based on target size while enforcing identical alias exclusivity. |
| **Declaration-only `inout` / `T&` without call-site marker (C++)** | Looking at `process(buffer)` in C++ gives zero indication whether `buffer` is read, mutated in place, or moved from. Requiring `process(mut buffer)` and `consume(own buffer)` makes mutation and ownership transfer 100% locally visible. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="pub enum ParamMode" min="1" reason="Syntax AST encodes ReadOnly, Mut, Own, and RegionIn parameter modes" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="integrate_cell(mut cell" min="1" reason="Example 02 marks in-place mutation explicitly at the call site" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="mut pool: Pool<SceneNode>" min="1" reason="Example 04 marks in-place mutation explicitly at the declaration site" -->
