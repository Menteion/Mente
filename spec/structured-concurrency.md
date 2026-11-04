---
title: Colorless Concurrency and Lexical Structured Task Trees
version: 0.1.0
status: active
date: 2026-11-04
---

# Colorless Concurrency and Lexical Structured Task Trees

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. **Prohibition of `async` and `await` keywords (`Implemented` in `compiler/mente-syntax/src/lib.rs`)**:
   - The keywords `async` and `await` MUST NOT exist in Mente.
   - Higher-order combinators (`items.map(f)`, `filter(f)`) accept pure functions and `with IO, Net` functions uniformly without splitting the ecosystem into synchronous and asynchronous APIs.
2. **Effect-driven static-stack hybrid fibers (`Specified`)**:
   - Functions carrying `with IO` or `with Net` automatically compile to suspendable state/fiber frames backed by OS completion queues (`io_uring` on Linux, `IOCP` on Windows, `kqueue` on macOS).
   - Using whole-program call-graph stack sizing (`@max_stack`), non-escaping child suspension frames are packed contiguously into the parent task's stack arena, avoiding per-call heap `Box<dyn Future>` allocations and evitar Go-style dynamic stack-copying pauses.
3. **Lexical structured concurrency (`parallel (...)` and `TaskScope`, `Implemented` in `mente-syntax` and `examples/03_colorless_concurrency_and_effects.mn`)**:
   - Concurrent fan-out is expressed via lexical `parallel (...)` blocks:
     ```rust
     val (left, right) = parallel (
         fetch_shard(primary)!,
         fetch_shard(replica)!,
     )
     ```
   - **Zero orphan tasks**: No child task spawned inside `parallel (...)` or `TaskScope` can outlive the lexical block.
   - **Automatic sibling cancellation**: If any branch raises an error via `!`, sibling tasks receive immediate cooperative cancellation at their next effect yield point, and all destructors complete before the scope propagates the error.
   - **Zero-`Arc` parent borrowing**: Because child tasks are strictly bounded by the lexical scope, they MAY read parent `Let` bindings directly without `Arc<T>` heap reference counting or `'static` lifetime bounds.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **`async fn` / `.await` function coloring (Rust / C# / TypeScript / Python)** | Splits libraries into incompatible sync/async worlds, requires `Pin<Box<dyn Future + Send + 'static>>` gymnastics in traits, and forces viral `Arc<Mutex<T>>` wrappers merely to satisfy `'static` bounds on spawned tasks. |
| **Unstructured `go f()` goroutines (Go)** | Detached background tasks easily leak when a parent request times out or errors early, causing silent memory/goroutine leaks and use-after-return hazards. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Parallel," min="1" reason="Lexer defines Parallel keyword for structured concurrent fan-out" -->
<!-- @assert-count target="examples/03_colorless_concurrency_and_effects.mn" symbol="parallel (" min="1" reason="Example 03 uses colorless structured parallel (...) block" -->
<!-- @assert-absence target="examples" symbol="async fn" glob="*.mn" reason="Mente forbids async/await function coloring" -->
<!-- @assert-absence target="examples" symbol=".await" glob="*.mn" reason="Mente forbids .await postfix syntax" -->
