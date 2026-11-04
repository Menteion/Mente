---
title: Lifetime-Free Provenance, Lexical Arenas, and Generational Pools
version: 0.1.0
status: active
date: 2026-11-04
---

# Lifetime-Free Provenance, Lexical Arenas, and Generational Pools

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Zero `<'a>` lifetime annotations (`Specified`)**:
   - Mente forbids tick-mark lifetime parameters (`<'a>`, `&'a T`) on structs, traits, and functions. Nothing rejects them yet: `mn check` accepts `pub struct S<'a>`, because the lexer skips `'` and keeps generics as a raw string.
   - When a function returns a slice or view derived from an input parameter (`fn subslice(data: Slice<u8>, start: usize) -> Slice<u8>`), the compiler automatically infers return provenance tied to the input argument's lexical scope (`Specified` in `mente-sema`).
2. **Lexical scoped arenas (`Arena.scoped`, `Specified`)**:
   - Temporary phase allocations use `Arena.scoped(capacity) { arena => ... }`.
   - Values allocated within `arena` cannot escape the lexical block via return values or outer `mut` bindings, and the entire region is reclaimed in `O(1)` (`1` bump-pointer reset instruction) upon scope exit.
3. **Generational `Pool<T>` and 64-bit `Handle<T>` (`Specified`; `examples/04_comptime_and_graph_pool.mn` uses them, and no standard library exists yet)**:
   - Graph topologies, doubly-linked structures, ECS entities, and UI trees use `Pool<T>` indexed by `Handle<T>` (`u32` slot index + `u32` generation tag = `64` bits).
   - `Handle<T>` is `Copy`, serializable across disk/network/GPU buffers, and free of reference cycles.
   - Indexing `pool[handle]` validates the 32-bit generation counter in `O(1)` and returns `?T` (or `mut` projection via `pool.get_mut(handle)`).
4. **Thread-aware `Shared<T>` (`Specified`)**:
   - Uses non-atomic reference increments while thread-local (`0` `LOCK XADD` bus-lock cost) and upgrades to atomic reference counting only when transferred (`own`) across a thread boundary.
5. **Provenance escape two-path diagnostic contract (`Specified` — Engineering Guardrail #3)**:
   - Because Mente forbids `<'a>` lifetime parameters in surface syntax, the programmer never solves lifetime constraints manually.
   - Whenever `mente-sema` detects that a borrowed slice or view would outlive its source binding or that two unrelated provenances are stored into the same long-lived struct field (`E0310 [Provenance Escape]`), the compiler MUST NOT expose internal region variable names (such as `'region#4`).
   - Instead, `E0310` MUST output a deterministic **two-path architectural fix**:
     1. **Lexical Region Path**: Add `in arena: Arena` to the function signature so the slice is allocated inside the caller's `Arena.scoped()` block.
     2. **Generational Handle Path**: Store `Handle<T>` (`64-bit Copy` index into `Pool<T>`) instead of a raw view inside long-lived or cyclic structs.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Viral `<'a, 'b>` lifetime parameters (Rust)** | Adding a borrowed slice field to a deep struct in Rust forces `<'a>` annotations onto every enclosing struct, trait, and function across the crate, and fails completely on cyclic graphs (forcing `Rc<RefCell<T>>` runtime panics or raw `unsafe` pointers). |
| **Global tracing garbage collection (Go / Java / C# / Python)** | Introduces unpredictable tail-latency pauses, 2–3x memory residency overhead, and loss of deterministic RAII resource destruction. |
| **Unchecked manual `malloc`/`free` or raw pointers (C / Zig)** | Susceptible to use-after-free and double-free vulnerabilities unless paired with generational validation (`Pool<T>`/`Handle<T>`). |

## 3. Verification assertions

<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="Pool<SceneNode>" min="1" reason="Example 04 models cyclic/hierarchical scene graphs via Pool<T>" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="Handle<SceneNode>" min="2" reason="Example 04 uses 64-bit generational Handle<T> values" -->
<!-- @assert-absence target="examples" symbol="Rc<RefCell" glob="*.mn" reason="Mente uses Pool<T>/Handle<T> rather than runtime-panicking Rc<RefCell<T>>" -->
