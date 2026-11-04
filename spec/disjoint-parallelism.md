---
title: Disjoint Partition Parallelism, Ownership Channels, and Mutex
version: 0.1.0
status: active
date: 2026-11-04
---

# Disjoint Partition Parallelism, Ownership Channels, and `Mutex<T>`

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`

## 1. Specification

1. **Lock-free disjoint slice parallelism (`parallel_for`, `Specified`; the lexer has a `ParallelFor` token and `examples/02_mechanical_sympathy_simd.mn` uses the syntax)**:
   - `parallel_for` distributes independent iterations across the hardware worker pool for shared-memory data parallelism:
     ```rust
     parallel_for tile in (mut scores).chunks_mut(64) {
         normalize_tile(mut tile)
     }
     ```
   - **Structural partitions need no proof**: when a mutable collection (`mut scores`) is partitioned by `.chunks_mut(S)`, `.split_at_mut(mid)` or the tiles of a `Tiled<T, lanes: N>`, the pieces are disjoint by construction, and `mn check` accepts the loop without solving anything.
   - **Index-arithmetic partitions go to Tier 2**: when iteration `i` instead writes `scores[i * S .. (i + 1) * S]`, the compiler must prove, for all distinct worker indices $i \neq j$:
     $[i \cdot S, (i + 1) \cdot S) \cap [j \cdot S, (j + 1) \cdot S) = \emptyset$
     The product `i * S` is outside Tier 1's octagons, so the function MUST carry `@prove`, and `mn prove` discharges the obligation (`ghost-contracts-and-totality.md`). Without `@prove` the loop is rejected, with a help line suggesting `.chunks_mut(S)`.
   - Each worker thread mutates its assigned slice in place with **zero locks (`0` `Mutex`), zero `RwLock`, and zero atomic bus-locking instructions**.
2. **Deterministic phased barriers (`Phase.barrier()`, `Specified`)**:
   - Multi-step parallel simulations alternate cleanly between an immutable previous generation (`prev: []T`) and a disjoint mutable next generation (`mut next: []T`), guaranteeing bit-identical output regardless of OS thread scheduling order.
3. **Zero-copy ownership `Channel<T, cap>` and data-wrapping `Mutex<T>` (`Specified`)**:
   - Sending a message over a channel requires `ch.send(own msg)`. Because MVS invalidates `msg` at the sender call site (`own`), large buffers and structs transfer across threads by pointer in `O(1)` with zero deep cloning and zero data races.
   - Standalone `lock()` / `unlock()` methods are forbidden; `Mutex<T>` wraps the protected payload `T` directly (`mutex.with_lock { mut state => ... }`), ensuring no reference to `state` can escape the critical section.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Per-element `Arc<Mutex<T>>` or atomic loops in parallel workloads** | Cache-line ping-ponging (`MESI` invalidation storms) and `LOCK CMPXCHG` instructions degrade multi-core scaling by 10–50x compared to proven disjoint slice partitioning. |
| **Proving every partition with a solver, the title's former "SMT-proven"** | `chunks_mut` and `split_at_mut` are disjoint because of how they are built; asking a solver to rediscover that costs time on every check and ties a routine loop to Tier 2, which runs only under `mn prove`. Only index arithmetic needs a proof. |
| **Unprotected OpenMP `#pragma omp parallel for` (C / C++ / Fortran)** | Silently compiles data races when two loop iterations write to overlapping indices, resulting in non-deterministic Heisenbugs. Mente rejects `parallel_for` bodies at compile time unless write intervals are proven disjoint. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="ParallelFor," min="1" reason="Lexer defines ParallelFor keyword for disjoint data parallelism" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="parallel_for p in mut batch" min="1" reason="Example 02 demonstrates lock-free disjoint tile mutation via parallel_for" -->
