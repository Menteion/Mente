---
title: Capability Effects, Tail-Resumptive Handlers, and Deterministic Simulation Testing
version: 0.1.0
status: active
date: 2026-11-04
---

# Capability Effects, Tail-Resumptive Handlers, and Deterministic Simulation Testing

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`

## 1. Specification

1. `Specified` **Pure-by-default functions and explicit `with` clauses**:
   - A function without a `with ...` clause is statically guaranteed to be pure with respect to external I/O, network sockets, wall-clock time, entropy, and foreign C calls.
   - Side-effecting capabilities MUST be declared in the function signature using `with`:
     ```rust
     pub fn fetch_telemetry(endpoint: Endpoint) -> Telemetry ! NetError
         with Net, Clock
     ```
   - `with` appears only in signatures, where it declares effects. It never opens a block.
   - `Implemented`: `parse_fn` records the effects a `with` clause lists in `FnDecl.effects`.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_with_clause_lists_effects" min="1" reason="parse_fn records every effect a with clause lists" -->
2. `Specified` **Effects and handlers**:
   - An effect is a named set of operations, declared like a trait: `effect Clock { fn now_ms() -> u64 }`. The standard effects are `IO`, `Net`, `Clock`, `Rand` and `FFI`; user code may declare its own.
   - Code declared `with Clock` calls an operation through the effect's name: `Clock.now_ms()`.
   - A handler is a value that implements an effect's operations. `handle Clock = SimClock(fixed_ms: 5000), Net = SimNet(drop_rate: 0.05) { ... }` installs handlers for the block: inside it, those effects' operations run the handlers' methods, and the block no longer needs those effects from its enclosing function.
   - **Handlers resume at once (tail-resumptive)**: an operation call runs the handler's method and returns its result to the caller, exactly like a function call. A handler cannot capture the rest of the computation, resume it later, or resume it twice. A standard handler that must wait (`StdNet` on a socket) suspends the whole task's fiber (`structured-concurrency.md`); that is the scheduler's doing, not a continuation the program can hold.
3. `Specified` **Static capability subset verification**:
   - If function `f` calls function `g`, `f`'s declared `with` effect set MUST be a superset of `g`'s `with` effect set, less the effects handled by `handle` blocks around the call.
   - `compiler/mente-sema` verifies capability containment across every call site. No checker exists yet: call sites sit inside unparsed bodies.
4. `Specified` **Effect variables**:
   - A function generic over the effects of a function it receives takes an effect-set parameter, a compile-time parameter of kind `Effects` (`traits-and-generics.md`): `fn map<T, U, comptime E: Effects>(xs: []T, f: fn(T) -> U with E) -> List<U> with E`.
   - `map(xs, parse)` with a pure `parse` is pure; `map(urls, fetch)` with `fetch` declared `with Net` is `with Net`. One `map` serves both, which is what keeps libraries free of a synchronous / asynchronous split (`structured-concurrency.md`).
   - An effect set may combine variables and named effects: `with E, Clock`.
5. `Specified` **Zero-cost static handlers**:
   - Because handlers resume at once, a handler known at compile time is an ordinary call target: in production builds the standard handlers (`StdNet`, `StdClock`, `StdIO`) monomorphize into direct OS calls, with `0` heap closure allocation and `0` dynamic dispatch.
6. `Specified` **Deterministic Simulation Testing**:
   - Under `mn test`, wrapping a call in simulated handlers (`handle Clock = SimClock(seed: 42), Net = SimNet(drop_rate: 0.05) { ... }`) intercepts every `Clock` and `Net` operation deterministically in memory, enabling FoundationDB-style **Deterministic Simulation Testing (DST)** without mocking frameworks or dependency-injection boilerplate.
   - **Scheduling is an effect too**: the order in which tasks run, fibers resume and `parallel_for` workers take iterations is decided by a `Sched` handler. Production installs a work-stealing pool; `mn test` installs `SimSched(seed)`, which runs every task on one OS thread in an order fixed by the seed, so a failing interleaving replays from its seed. `Sched` is installed by the program's entry point and by `mn test`, and is never written in a signature.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Unrestricted ambient authority (Rust / C++ / Go / Python)** | Any transitive helper function or third-party package can silently read `~/.ssh`, open a network socket, or call `SystemTime::now()`, breaking reproducibility and sandboxing. |
| **Monadic `IO a` wrappers (Haskell)** | Forces monadic lifting (`MonadTrans`, `liftIO`) across every expression and duplicates higher-order collection combinators (`map` vs `mapM`). Mente's `with` capabilities are orthogonal to the return type `T`. |
| **General algebraic effect handlers with first-class continuations (Koka, Eff, OCaml 5)** | A handler that can capture the rest of the computation and resume it later, or twice, needs that computation reified as a heap object or a copied stack, which rules out compiling handlers to direct calls and breaks `own` (a continuation resumed twice would use a moved value twice). Every use Mente has for handlers (capabilities, simulation, scheduling) resumes at once. |
| **`with` for both effect clauses and handler blocks** | One keyword meant a declaration in signatures, an installation in bodies, and, in `with Arena.scoped() as arena`, an arena scope. `with` now only declares, `handle` installs, and arenas open with `Arena.scoped`. |
| **Effect sets without variables** | Every higher-order function would need one copy per effect set, or would have to demand the largest one, which is the function colouring the language set out to remove. |
| **Simulating only time and the network** | Tasks, fibers and `parallel_for` workers would still interleave in whatever order the OS chose, so a test could pass or fail from run to run with the same clock and network. Making the scheduler a handled effect puts the interleaving under the seed too. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="With," min="1" reason="Lexer defines With keyword for effect clauses" -->
<!-- @assert-count target="examples/03_colorless_concurrency_and_effects.mn" symbol="with Net, Clock" min="1" reason="Example 03 declares explicit Net and Clock capability effects" -->
<!-- @assert-count target="examples/03_colorless_concurrency_and_effects.mn" symbol="handle Clock = SimClock(" min="1" reason="Example 03 installs simulated handlers with handle" -->
<!-- @assert-absence target="examples" symbol="with Clock =" glob="*.mn" reason="with only declares effects; handlers are installed with handle" -->
