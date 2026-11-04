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
   - **Return provenance is read from the signature, never inferred from the body (`Specified`)**. When a function returns a view (`[]T`, `str`, or a value holding one), the parameters it may borrow from are:
     - by default, every read-only, `mut` or `in` parameter that is or holds a view or an arena. This is safe without annotation: the caller keeps all of them alive for as long as it keeps the result;
     - with a `from p, q` clause after the return type, only the named parameters: `fn subslice(data: []u8, key: []u8, start: usize) -> []u8 from data`. The clause comes after any `! E` and before `with`.
   - A body that returns a view borrowed from somewhere the signature does not allow is an error (`E0310`, with the two-path fix below). Because provenance is part of the signature, it is part of `spec_hash` (`holographic-module.md`), and a body edit cannot change it.
   - `Implemented`: `parse_fn` records the `from` list (`FnDecl.provenance`), `spec_hash` covers it, `mn view --spec` prints it, and `verify_function_contracts` rejects a `from` that names no parameter or an `own` parameter (`E0311`; a view cannot borrow from a value the callee consumes). Checking the body against the clause needs a body parser and is `Specified`.
2. **Views and owned values (`Specified`)**:
   - `str` is a view of UTF-8 text and `[]T` a view of a contiguous run of `T`; both borrow, and follow the provenance rules of item 1. `String` and `List<T>` own their storage, and can be stored, returned and moved with no provenance.
   - A string literal is a `str` view of static storage, valid for the whole program, so a value holding only literals carries no provenance constraint.
   - A struct with a view field holds a view: it follows the same rules, and storing it where it would outlive its source is `E0310` (item 6). A field of type `String` or `List<T>` avoids the question.
   - A `str` becomes a `String` only explicitly (`String.from(s)`), because that allocates and copies.
3. **Lexical scoped arenas (`Arena.scoped`, `Specified`)**:
   - Temporary phase allocations use `Arena.scoped(capacity) { arena => ... }`.
   - Values allocated within `arena` cannot escape the lexical block via return values or outer `mut` bindings, and the entire region is reclaimed in `O(1)` (`1` bump-pointer reset instruction) upon scope exit.
4. **Generational `Pool<T>` and 64-bit `Handle<T>` (`Specified`; `examples/04_comptime_and_graph_pool.mn` uses them, and no standard library exists yet)**:
   - Graph topologies, doubly-linked structures, ECS entities, and UI trees use `Pool<T>` indexed by `Handle<T>` (`u32` slot index + `u32` generation tag = `64` bits).
   - `Handle<T>` is `Copy`, serializable across disk/network/GPU buffers, and free of reference cycles.
   - `pool[h]` validates the 32-bit generation counter in `O(1)` and yields `?T`: the element while `h` is live, `none` once its slot has been freed or reused. `pool[mut h]` is the mutable projection, also `?T`, and writes through it follow copy-in/copy-out (`mutable-value-semantics.md`).
   - Reading a field therefore needs narrowing (`guard val node = pool[h] else ...`), and `pool[mut h]?.field = v` does nothing when `h` is stale. There is no `get_mut` and no panicking index.
5. **Thread-aware `Shared<T>` (`Specified`)**:
   - Uses non-atomic reference increments while thread-local (`0` `LOCK XADD` bus-lock cost) and upgrades to atomic reference counting only when transferred (`own`) across a thread boundary.
6. **Provenance escape two-path diagnostic contract (`Specified` — Engineering Guardrail #3)**:
   - Because Mente forbids `<'a>` lifetime parameters in surface syntax, the programmer never solves lifetime constraints manually.
   - Whenever `mente-sema` detects that a borrowed slice or view would outlive its source binding or that two unrelated provenances are stored into the same long-lived struct field (`E0310 [Provenance Escape]`), the compiler MUST NOT expose internal region variable names (such as `'region#4`).
   - Instead, `E0310` MUST output a deterministic **two-path architectural fix**:
     1. **Lexical Region Path**: Add `in arena: Arena` to the function signature so the slice is allocated inside the caller's `Arena.scoped()` block.
     2. **Generational Handle Path**: Store `Handle<T>` (`64-bit Copy` index into `Pool<T>`) instead of a raw view inside long-lived or cyclic structs.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Inferring return provenance from the body** | Lets a body edit change which arguments a caller must keep alive, so an edit in one module can break a caller in another while every signature stays the same, and `spec_hash` can no longer tell which edits reach dependents. A conservative default plus an optional `from` clause gives the same convenience with the fact on the page. |
| **Viral `<'a, 'b>` lifetime parameters (Rust)** | Adding a borrowed slice field to a deep struct in Rust forces `<'a>` annotations onto every enclosing struct, trait, and function across the crate, and fails completely on cyclic graphs (forcing `Rc<RefCell<T>>` runtime panics or raw `unsafe` pointers). |
| **Global tracing garbage collection (Go / Java / C# / Python)** | Introduces unpredictable tail-latency pauses, 2–3x memory residency overhead, and loss of deterministic RAII resource destruction. |
| **Unchecked manual `malloc`/`free` or raw pointers (C / Zig)** | Susceptible to use-after-free and double-free vulnerabilities unless paired with generational validation (`Pool<T>`/`Handle<T>`). |

## 3. Verification assertions

<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="Pool<SceneNode>" min="1" reason="Example 04 models cyclic/hierarchical scene graphs via Pool<T>" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="Handle<SceneNode>" min="2" reason="Example 04 uses 64-bit generational Handle<T> values" -->
<!-- @assert-absence target="examples" symbol="Rc<RefCell" glob="*.mn" reason="Mente uses Pool<T>/Handle<T> rather than runtime-panicking Rc<RefCell<T>>" -->
<!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_from_clause_must_name_a_kept_parameter" min="1" reason="A from clause must name a parameter the caller keeps, never an own one" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="guard val root_node = scene[root]" min="1" reason="Pool indexing yields ?T, so example 04 narrows before reading a field" -->
<!-- @assert-absence target="examples" symbol="get_mut" glob="*.mn" reason="The mutable pool projection is pool[mut h], not a get_mut method" -->
