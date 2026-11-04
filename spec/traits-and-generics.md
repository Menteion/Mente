---
title: Angle-Bracket Generics and Static-Default Trait Polymorphism
version: 0.1.0
status: active
date: 2026-11-04
---

# Angle-Bracket Generics and Static-Default Trait Polymorphism

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Angle-bracket generics `<T>` without turbofish `::<>` (`Implemented` in `compiler/mente-syntax/src/lib.rs`)**:
   - Type parameters on types and functions use angle brackets: `List<T>`, `Pool<SceneNode>`, `fn serialize<comptime T: Type>(...)`.
   - Because Mente's LL(k) expression grammar separates type arguments from relational comparison via lexical/syntactic context, explicit generic function calls are written `parse<i32>(input)` and **never** `parse::<i32>(input)`.
2. **`trait` declarations and structural/nominal bounds (`Implemented` in parser)**:
   - Shared behavior contracts are declared with `trait`:
     ```rust
     pub trait ByteSink {
         fn write_u64(mut self, v: u64)
     }
     ```
3. **Static monomorphization by default (`x: Trait`) vs. explicit dynamic dispatch (`dyn Trait`) (`Implemented` in `examples/04_comptime_and_graph_pool.mn`, `Specified` in codegen)**:
   - Writing a trait name directly as a parameter type (`fn encode(mut sink: ByteSink)`) MUST default to **compile-time static monomorphization** (equivalent to Rust `impl ByteSink` or C++20 `concept`), enabling 100% inlining and zero vtable indirection.
   - Runtime virtual dispatch via a 16-byte fat pointer `(data_ptr, vtable_ptr)` MUST be requested explicitly with the `dyn` keyword (`loggers: Slice<dyn Logger>`).

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Square-bracket generics `List[T]` (Scala / Python / Mojo)** | Collides directly with Mente's first-class refinement interval syntax `u32[0..1024]` and array indexing `items[i]`. Keeping `<T>` for type parameters and `[min..max]` exclusively for value intervals and indexing ensures instant visual disambiguation. |
| **Rust `::<T>` turbofish syntax** | Adds noisy punctuation (`::<>`) solely to work around C/Rust parser ambiguity. |
| **Implicit vtable dispatch on bare interface names (Go / Java / C#)** | Silently defeats inlining, causes heap escape of interface receivers, and triggers indirect branch mispredictions in hot loops. Making static dispatch the default (`x: Trait`) and requiring `dyn Trait` for vtables aligns syntax cost with hardware cost. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Trait," min="1" reason="Lexer defines Trait keyword" -->
<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Dyn," min="1" reason="Lexer defines Dyn keyword for explicit dynamic vtable dispatch" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="mut sink: ByteSink" min="1" reason="Example 04 uses static-default trait parameter without impl/dyn boilerplate" -->
<!-- @assert-absence target="examples" symbol="::<" glob="*.mn" reason="Mente generics forbid turbofish ::<> syntax" -->
