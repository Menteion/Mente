---
title: Colorless Concurrency and Lexical Structured Task Trees
version: 0.1.0
status: active
date: 2026-11-04
---

# Colorless Concurrency and Lexical Structured Task Trees

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Prohibition of `async` and `await` keywords (`Specified`)**:
   - The keywords `async` and `await` MUST NOT exist in Mente. The compiler does not reject them yet (they lex as identifiers); `examples/` is checked for `async fn` and `.await` by the directives below.
   - Higher-order combinators (`items.map(f)`, `filter(f)`) accept pure functions and `with IO, Net` functions uniformly without splitting the ecosystem into synchronous and asynchronous APIs.
2. **Effect-driven stackful fibers (`Specified`)**:
   - Functions carrying `with IO` or `with Net` may suspend at an effect operation; suspension is driven by OS completion queues (`io_uring` on Linux, `IOCP` on Windows, `kqueue` on macOS).
   - Each task runs on its own stack: a range of virtual memory reserved when the task starts, committed page by page as it is touched, and ended by an unmapped guard page. Suspending a task keeps its stack where it is, so there is no heap-allocated future per call and no stack copying.
   - A stack's size depends on no callee's body, so no caller's interface does either (`holographic-module.md`). Code that must run within a fixed stack declares `@max_stack(bytes)`, which composes from callees' declared bounds (`hardware-contracts.md`).
3. **Lexical structured concurrency (`parallel (...)` and `TaskScope`, `Specified`; the lexer has a `Parallel` token and `examples/03_colorless_concurrency_and_effects.mn` uses the syntax)**:
   - Concurrent fan-out is expressed via lexical `parallel (...)` blocks:
     ```rust
     val (left, right) = parallel (
         fetch_shard(primary)!,
         fetch_shard(replica)!,
     )
     ```
   - **Zero orphan tasks**: No child task spawned inside `parallel (...)` or `TaskScope` can outlive the lexical block.
   - **Automatic sibling cancellation**: If any branch raises an error via `!`, sibling tasks receive immediate cooperative cancellation at their next effect yield point, and all destructors complete before the scope propagates the error.
   - **Zero-`Arc` parent borrowing**: Because child tasks are strictly bounded by the lexical scope, they MAY read parent read-only bindings directly without `Arc<T>` heap reference counting or `'static` lifetime bounds.
   - **Exclusivity across branches**: the branches of one `parallel (...)` are checked by the Law of Exclusivity exactly as the arguments of one call are (`mutable-value-semantics.md`): any number of branches may read a binding, but a binding that one branch mutates no other branch may read or mutate. Passing `in arena` to several branches is a read (`regions-and-pools.md`).

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **`async fn` / `.await` function coloring (Rust / C# / TypeScript / Python)** | Splits libraries into incompatible sync/async worlds, requires `Pin<Box<dyn Future + Send + 'static>>` gymnastics in traits, and forces viral `Arc<Mutex<T>>` wrappers merely to satisfy `'static` bounds on spawned tasks. |
| **Stackless fibers sized by whole-program call-graph analysis** | Packing every suspension frame into the parent's stack needs every callee's frame size, which is a property of its body: separate compilation, `dyn` calls, FFI and recursion all leave it unknown, and a body edit in one module would resize tasks in another. Reserved virtual-memory stacks cost address space, not memory, on 64-bit targets. |
| **Unstructured `go f()` goroutines (Go)** | Detached background tasks easily leak when a parent request times out or errors early, causing silent memory/goroutine leaks and use-after-return hazards. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Parallel," min="1" reason="Lexer defines Parallel keyword for structured concurrent fan-out" -->
<!-- @assert-count target="examples/03_colorless_concurrency_and_effects.mn" symbol="parallel (" min="1" reason="Example 03 uses colorless structured parallel (...) block" -->
<!-- @assert-absence target="examples" symbol="async fn" glob="*.mn" reason="Mente forbids async/await function coloring" -->
<!-- @assert-absence target="examples" symbol=".await" glob="*.mn" reason="Mente forbids .await postfix syntax" -->
