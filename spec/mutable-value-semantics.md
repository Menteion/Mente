---
title: Sigil-Free Mutable Value Semantics and the Law of Exclusivity
version: 0.1.0
status: active
date: 2026-11-04
---

# Sigil-Free Mutable Value Semantics and the Law of Exclusivity

**Governs:** `compiler/mente-syntax`, `compiler/mente-ast`, `compiler/mente-sema`

## 1. Specification

1. `Specified` **Elimination of reference sigils (`&T`, `&mut T`, `*T`)**:
   - Mente surface syntax has no address-of `&` or dereference `*` sigils on user types. Nothing rejects them yet: the lexer skips `&` without an error.
   - Instead, every function parameter specifies one of three explicit **parameter ownership modes**, plus the region mode `in` specified in `regions-and-pools.md`.
   - `Implemented`: for declarations, `parse_param_list` records each parameter's mode, `ParamMode::ReadOnly`, `Mut`, `Own` or `RegionIn` (`compiler/mente-syntax/src/lib.rs`).
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_parameter_modes_are_recorded" min="1" reason="Each declared parameter's mode is recorded" -->
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_parameters_split_only_at_top_level_commas" min="1" reason="Parameters split only at commas outside brackets and function types" -->
   - `Specified`: the semantics and machine ABI of each mode:

| Mode | Declaration syntax | Call-site syntax | Semantic contract | Machine ABI lowering |
| :--- | :--- | :--- | :--- | :--- |
| **Shared projection (`ReadOnly`)** | `x: T` | `f(a)` | Read-only immutable view; caller retains ownership; zero deep copies. | Register value if `sizeof(T) <= 16B` and `T` is plain; otherwise a `const T*` pointer. |
| **In-place projection (`Mut`)** | `mut x: T` | `f(mut a)` | Exclusive access to a value for the call's duration, defined by copy-in/copy-out (item 4); no other argument to `f` may alias `a`. | Non-aliasing `T* restrict` pointer (`noalias`) to the caller's storage when `a` is stored contiguously; otherwise a temporary written back on return. |
| **Consuming transfer (`Own`)** | `own x: T` | `f(own a)` | Ownership transferred to `f`; `a` is statically invalidated at caller after call. | Register / bitwise move transfer; drop responsibility moves to callee. |

   - `Specified` **A read-only `?T` is passed as an address or none** ([`stack-budget`](../docs/design-review.md#stack-budget)): a read-only parameter of a type `?T` that is over 16 bytes, whether or not `T` has a niche, is one word: the address of the present value, or none. The callee reads the value through the address, and reads none as `none`. A caller passes the address of the value where it holds a present one, in a place of type `T` or of type `?T`, or as a value of its own; the address a projection `p?.f` is (`core-calculus.md`, item 15); or none. It builds no `?T` to pass, so an argument that is a `T` and one that is a projection cross in one way, and a read-only argument is read in place. A `?T` of 16 bytes or less is passed in registers, as the table has any small value, the caller reading it first, where it is plain. An owning one, which only arrays of no element make so small, as `?[0]String`, is passed as an address or none, as one over 16 bytes is, and any other read-only value of owning type and 16 bytes or less, as `struct S { a: [0]String, n: u64 }`, is passed by its address, so that nothing owning is copied implicitly (`core-calculus.md`, item 12). A `mut` parameter of a type `?T` is the address of its caller's place, tag and all, since the callee may change whether a value is present, and a projection is never passed so, since `?.` only reads. An `own` parameter receives the `?T` itself.

2. **Mandatory dual-site annotation**:
   - `Specified`: both `mut` and `own` MUST be written at the function declaration site (`fn step(mut p: Particle)`) **and** at every call site (`step(mut p)`). A call that changes its receiver is marked the same way, `mut xs.push(v)`, since the receiver is an argument ([`exclusivity-projections`](../docs/design-review.md#exclusivity-projections)). The compiler checks the call-site rule for `mut` since M1, and for `own` and receivers from M2.
   - `Implemented`: the declaration site is parsed, and `parse_param_list` records `mut` and `own`.
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_parameter_modes_are_recorded" min="1" reason="mut and own are recorded at the declaration site" -->
3. `Specified` **Law of Exclusivity and move invalidation**:
   - Within any call expression, the root binding of an argument passed as `mut` or `own`, a marked receiver included, MUST NOT appear in any other argument, whatever the projections: `f(mut s.a, mut s.b)` is rejected as `f(mut a[i], a[j])` is ([`exclusivity-projections`](../docs/design-review.md#exclusivity-projections)). `core-calculus.md` (items 7 and 20) states the rule the checker implements.
   - Any binding passed with `own` MUST NOT be read or mutated on any subsequent path unless re-initialized: a binding moved on any path is dead where the paths join, only whole local bindings and `own` parameters move, and no flag is kept at run time ([`own-invalidation`](../docs/design-review.md#own-invalidation); `core-calculus.md`, item 20).
4. `Specified` **`mut` is defined by copy-in/copy-out**:
   - The meaning of `f(mut a)` is: `a`'s value is copied into the callee's parameter, the callee works on that copy, and the final value is copied back into `a` when `f` returns. The write-back happens on every exit from `f`, including `raise`, `!` propagation and task cancellation, so a caller sees the same result whichever way `f` ends.
   - Because the Law of Exclusivity forbids any other access to `a` while `f` runs, no program can tell copy-in/copy-out from passing a pointer to `a`'s storage. The compiler passes a pointer whenever `a` is stored contiguously, and pays for a copy only where it must.
   - **Projections.** `a` may be any projection path: a binding, a field (`mut cell.potential`), an element (`mut items[i]`), or an element of a container whose element is not stored contiguously. An element of `SoA<T>` or `Tiled<T, lanes: N>` is gathered from its field arrays into a temporary `T` and scattered back on return; the same holds for each iteration variable of `parallel_for p in mut batch` over such a container, where the compiler may instead vectorize the loop body over the field arrays directly, since no program can observe the difference.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Pointer/reference sigils `&T` and `&mut T` (Rust / C++)** | Forces programmers to choose physical pointer vs. value passing manually even for small register types (`&u64` adds unnecessary pointer indirection), and leaks reference wrapper types into generic signatures. MVS lets the compiler pick register vs. pointer ABI automatically based on target size while enforcing identical alias exclusivity. |
| **Declaration-only `inout` / `T&` without call-site marker (C++)** | Looking at `process(buffer)` in C++ gives zero indication whether `buffer` is read, mutated in place, or moved from. Requiring `process(mut buffer)` and `consume(own buffer)` makes mutation and ownership transfer 100% locally visible. |
| **Defining `mut` as a pointer to the caller's storage (C++ `T&`, Rust `&mut T`)** | A pointer needs the value to sit contiguously in memory, so it cannot express a `mut` element of `SoA<T>` or `Tiled<T, lanes>`, whose fields live in separate arrays; those containers would need proxy types and a second API. Copy-in/copy-out (as in Swift `inout` and Hylo) covers every projection, and exclusivity lets the compiler still pass a pointer wherever the storage allows. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="pub enum ParamMode" min="1" reason="Syntax AST encodes ReadOnly, Mut, Own, and RegionIn parameter modes" -->
<!-- @assert-count target="examples/02_mechanical_sympathy_simd.mn" symbol="integrate_cell(mut cell" min="1" reason="Example 02 marks in-place mutation explicitly at the call site" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="mut pool: Pool<SceneNode>" min="1" reason="Example 04 marks in-place mutation explicitly at the declaration site" -->
