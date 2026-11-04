---
title: Angle-Bracket Generics and Static-Default Trait Polymorphism
version: 0.1.0
status: active
date: 2026-11-04
---

# Angle-Bracket Generics and Static-Default Trait Polymorphism

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Angle-bracket generics `<T>` without turbofish `::<>`**:
   - Type parameters on types and functions use angle brackets: `List<T>`, `Pool<SceneNode>`, `fn largest<T>(a: T, b: T) -> T` (`Implemented` for declarations: `mente-syntax` keeps a declaration's `<...>` as a raw string).
   - **One spelling for compile-time parameters (`Specified`)**: a bare name in `<...>` is a type parameter. A compile-time value parameter is written `comptime N: K` with an optional default: `struct RingWindow<T, comptime CAP: usize = 64>`, `fn mean<comptime U: Unit>(xs: []f64<U>) -> f64<U>`. There is no `const` parameter form, and no `comptime T: Type` form for an ordinary type parameter.
   - Explicit generic function calls are written `parse<i32>(input)` and **never** `parse::<i32>(input)` (`Specified`; there is no expression parser to apply the rule yet).
   - **Telling type arguments from comparisons (`Specified`, adapted from C#)**: when the tokens after a name parse as a type-argument list `<...>`, they are taken as type arguments only if the token after the closing `>` is one of `(`, `)`, `[`, `]`, `}`, `:`, `,`, `.`, `?`, `==`, `!=`; otherwise `<` is a comparison. So `parse<i32>(input)` and `Type.of<T>().fields` are calls with type arguments, `a < b and c > d` is two comparisons (`b and c` is not a type list), and `f(a < b, c > (d))` calls `a<b, c>` with `d`, as in C#. Inside a type (`f64<meter / sec>`, a parameter's type) `<` always opens type arguments.
2. **`trait` declarations and structural/nominal bounds (`Implemented` in `parse_trait`, method signatures kept as raw strings)**:
   - Shared behavior contracts are declared with `trait`:
     ```rust
     pub trait ByteSink {
         fn write_u64(mut self, v: u64)
     }
     ```
3. **Static monomorphization by default (`x: Trait`) vs. explicit dynamic dispatch (`dyn Trait`) (`Specified`; `examples/04_comptime_and_graph_pool.mn` uses the syntax, and the lexer has a `Dyn` token)**:
   - Writing a trait name directly as a parameter type (`fn encode(mut sink: ByteSink)`) MUST default to **compile-time static monomorphization** (equivalent to Rust `impl ByteSink` or C++20 `concept`), enabling 100% inlining and zero vtable indirection.
   - Such a function is generic even without `<...>`, so its body is instantiated by dependents and tracked by `instance_hash` (`holographic-module.md`). `Implemented` for traits declared in the same module (tested in `mente-ast`); a trait from another module needs name resolution and is `Specified`.
   - Runtime virtual dispatch via a 16-byte fat pointer `(data_ptr, vtable_ptr)` MUST be requested explicitly with the `dyn` keyword (`loggers: []dyn Logger`).

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
<!-- @assert-absence target="examples" symbol="const " glob="*.mn" reason="A compile-time value parameter is written comptime N: K, never const" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="fn test_trait_typed_parameter_makes_a_function_instantiable" min="1" reason="A function taking a module trait by name is generic, so its body goes into instance_hash" -->
