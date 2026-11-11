---
title: Lifetime-Free Provenance, Lexical Arenas, and Generational Pools
version: 0.1.0
status: active
date: 2026-11-04
---

# Lifetime-Free Provenance, Lexical Arenas, and Generational Pools

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. `Specified` **Zero `<'a>` lifetime annotations**:
   - Mente forbids tick-mark lifetime parameters (`<'a>`, `&'a T`) on structs, traits, and functions. Nothing rejects them yet: `mn check` accepts `pub struct S<'a>`, because the lexer skips `'` and keeps generics as a raw string.
   - `Specified` **Return provenance is read from the signature, never inferred from the body**. When a function returns a view (`[]T`, `str`, or a value holding one), the parameters it may borrow from are:
     - by default, every read-only parameter that is or holds a view, and from M3 every `in` arena. This is safe without annotation: the caller keeps all of them alive, and unchanged, for as long as it keeps the result. A `mut` parameter is never a source, since it may be passed through a temporary that the call's return ends, and neither is an owner such as an array, a `List` or a `String`, which a caller views with `xs[..]` ([`view-provenance-sources`](../docs/design-review.md#view-provenance-sources));
     - with a `from p, q` clause after the return type, only the named parameters: `fn subslice(data: []u8, key: []u8, start: usize) -> []u8 from data`. The clause comes after any `! E` and before `with`.
   - A body that returns a view borrowed from somewhere the signature does not allow is an error (`E0310`, with the two-path fix below). Because provenance is part of the signature, it is part of `spec_hash` (`holographic-module.md`), and a body edit cannot change it.
   - `Implemented`: `parse_fn` records the `from` list (`FnDecl.provenance`), `spec_hash` covers it, `mn view --spec` prints it, and `verify_function_contracts` rejects a `from` that names no parameter or an `own` parameter (`E0311`; a view cannot borrow from a value the callee consumes).
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_from_clause_names_return_provenance" min="1" reason="parse_fn records the from list" -->
     <!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="fn test_from_clause_is_part_of_spec_hash" min="1" reason="spec_hash covers the from list" -->
     <!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="fn test_projection_keeps_signatures_and_drops_bodies" min="1" reason="The projection prints the from list" -->
     <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_from_clause_must_name_a_kept_parameter" min="1" reason="A from clause must name a parameter the caller keeps, never an own one" -->
   - `Specified`: checking a body against its `from` clause, which needs a body parser.
2. `Specified` **Views and owned values**:
   - `str` is a view of UTF-8 text and `[]T` a view of a contiguous run of `T`; both borrow, and follow the provenance rules of item 1. `String` and `List<T>` own their storage, and can be stored, returned and moved with no provenance.
   - A string literal is a `str` view of static storage, valid for the whole program, so a value holding only literals carries no provenance constraint.
   - A struct with a view field holds a view: it follows the same rules, and storing it where it would outlive its source is `E0310` (item 6). A field of type `String` or `List<T>` avoids the question.
   - A `str` becomes a `String` only explicitly (`String.from(s)`), because that allocates and copies.
3. `Specified` **Lexical scoped arenas (`Arena.scoped`)**:
   - Temporary phase allocations use `Arena.scoped(capacity) { arena => ... }`, with `capacity` optional. This is the only form that opens an arena.
   - Values allocated within `arena` cannot escape the lexical block via return values or outer `mut` bindings, and the entire region is reclaimed in `O(1)` upon scope exit.
   - **Passing an arena**: a function receives one as `in arena: Arena` and is called as `f(in arena)`, the same two-site marking as `mut` and `own` (`mutable-value-semantics.md`). `in` grants allocation only: the callee may allocate into the arena and return values whose provenance is the arena (`from arena`), but cannot reset or free it. For the Law of Exclusivity an `in` argument is a read, so several calls and several tasks may hold the same arena at once.
   - **Concurrent allocation**: when the branches of `parallel (...)` or the iterations of a `parallel_for` allocate in one arena, each task takes its own chunk from the arena and bumps within it, so the fast path has no atomic instruction and no task waits on another to allocate. Chunks are released with the arena.
   - **No destructors in an arena**: a type with a destructor (one that owns a resource or heap storage, such as a file, `String` or `List<T>`) cannot be allocated in an arena (`E0312`), so the `O(1)` reset never skips cleanup. Arena data is plain values and views into the arena itself (`str`, `[]T`).
4. `Specified` **Generational `Pool<T>` and 64-bit `Handle<T>`**:
   - Graph topologies, doubly-linked structures, ECS entities, and UI trees use `Pool<T>` indexed by `Handle<T>` (`u32` slot index + `u32` generation tag = `64` bits).
   - `Handle<T>` is `Copy`, serializable across disk/network/GPU buffers, and free of reference cycles.
   - `pool[h]` validates the 32-bit generation counter in `O(1)` and yields `?T`: the element while `h` is live, `none` once its slot has been freed or reused. `pool[mut h]` is the mutable projection, also `?T`, and writes through it follow copy-in/copy-out (`mutable-value-semantics.md`).
   - Reading or writing a field therefore needs narrowing (`guard val node = pool[h] else ...`): `?.` only reads, so a write through a stale handle cannot silently do nothing ([`optional-semantics`](../docs/design-review.md#optional-semantics)). There is no `get_mut` and no panicking index.
   - `examples/04_comptime_and_graph_pool.mn` uses them; no standard library exists yet.
5. `Specified` **Reference-counted sharing: `Local<T>` and `Shared<T>`**:
   - `Local<T>` counts references without atomic instructions (`0` `LOCK XADD` bus-lock cost). It is **task-bound**: a value whose type contains a `Local<T>` cannot be sent over a channel, moved into another task, or named inside a branch of `parallel (...)` or the body of a `parallel_for` (`E0320`), because each of those may run on another thread.
   - `Shared<T>` counts atomically and may cross any task boundary; its `T` must not be task-bound.
   - No conversion leaves clones of the other kind behind: `Local.from(own value)` and `Shared.from(own value)` start from an owned value, and `local.take()` yields `?T`, the value only when no other clone of that `Local<T>` exists.
   - A clone is always explicit (`x.clone()`), so every count change is visible in the source.
6. `Specified` **Provenance escape two-path diagnostic contract (Engineering Guardrail #3)**:
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
| **One `Shared<T>` that counts non-atomically until moved across threads, then atomically** | The clones left in the first thread keep updating the same counter non-atomically while the second thread updates it atomically: a data race. Biased reference counting repairs this with a second counter and an owner-thread test on every operation, a runtime cost hidden behind one type. Two types put the cost in the type and let the compiler reject the race. |
| **One arena bump pointer shared by all tasks through an atomic add** | Puts a contended atomic on every allocation of every branch, the cost arenas exist to avoid. A chunk per task keeps each allocation a plain add. |
| **Running destructors when an arena resets** | The reset would walk a list of finalizers, so reclaiming an arena would no longer be `O(1)`, and a type's cleanup would run at a time its owner does not control. |
| **Global tracing garbage collection (Go / Java / C# / Python)** | Introduces unpredictable tail-latency pauses, 2–3x memory residency overhead, and loss of deterministic RAII resource destruction. |
| **Unchecked manual `malloc`/`free` or raw pointers (C / Zig)** | Susceptible to use-after-free and double-free vulnerabilities unless paired with generational validation (`Pool<T>`/`Handle<T>`). |

## 3. Verification assertions

<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="Pool<SceneNode>" min="1" reason="Example 04 models cyclic/hierarchical scene graphs via Pool<T>" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="Handle<SceneNode>" min="2" reason="Example 04 uses 64-bit generational Handle<T> values" -->
<!-- @assert-absence target="examples" symbol="Rc<RefCell" glob="*.mn" reason="Mente uses Pool<T>/Handle<T> rather than runtime-panicking Rc<RefCell<T>>" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="guard val root_node = scene[root]" min="1" reason="Pool indexing yields ?T, so example 04 narrows before reading a field" -->
<!-- @assert-absence target="examples" symbol="get_mut" glob="*.mn" reason="The mutable pool projection is pool[mut h], not a get_mut method" -->
<!-- @assert-count target="examples/03_colorless_concurrency_and_effects.mn" symbol="(uid, in arena)!" min="2" reason="An arena is passed with in at the call site, as mut and own are" -->
<!-- @assert-absence target="examples" symbol="in: arena" glob="*.mn" reason="The call-site marker is in arena, not a named argument" -->
