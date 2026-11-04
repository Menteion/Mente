---
title: Holographic Single-File Module Model and Dual AST Hashing
version: 0.1.0
status: active
date: 2026-11-04
---

# Holographic Single-File Module Model and Dual AST Hashing

**Governs:** `compiler/mente-syntax`, `compiler/mente-ast`, `compiler/mente-cli`

## 1. Specification

1. **Single `.mn` source file (`Specified`; `mente-syntax` parses one file per module, `Implemented`)**:
   - Every Mente module resides in a single `.mn` source file. Separate `.h` / `.cpp` or `.mspec` / `.mbody` split files MUST NOT exist.
   - Public types, memory layouts, function signatures, capability effects (`with ...`), and ghost contracts (`requires` / `ensures` / `invariant`) are written inline alongside function bodies.
2. **The interface is the signature (`Specified`)**:
   - Everything a dependent module compiles against MUST be written in, or derived from, the declarations alone: types and their layouts, signatures, parameter modes, return-view provenance (`from`, `regions-and-pools.md`), effects, contracts, and contract attributes such as `@max_stack(bytes)` (`hardware-contracts.md`).
   - The compiler MUST NOT infer any of these from a function body. A rule that needs a callee's body to type-check or size its caller is a design error.
   - The one exception is a body that dependents compile into themselves: a generic `pub fn` (instantiated per use) or an `@inline` one (inlined across modules). Such bodies are tracked by their own digest, so only the dependents that use them are affected. No other body may reach a dependent; release-mode whole-program inlining (`mir-and-codegen.md`, Track B) is outside the incremental guarantee.
3. **Three 128-bit structural digests (`Implemented` in `compute_dual_hashes`, `compiler/mente-ast/src/lib.rs`)**:
   - `compiler/mente-ast` computes three non-cryptographic 128-bit digests (two 64-bit FNV-1a-style lanes, hex `String`s) from the parsed declarations:
     - `spec_hash` — the module path, `unit` declarations, every field of each `pub struct` (private fields and defaults included, since they decide the struct's size), `pub bitstruct` fields and `@bits`, `pub enum` variants, `pub trait` method signatures, and each `pub fn`'s attributes, parameters and modes, return and error types, `from` provenance, `with` effects, `requires` / `ensures` / `decreases` clauses.
     - `instance_hash` — the bodies of `pub` functions that are generic: with a `<...>` list, or taking one of the module's traits by name (`traits-and-generics.md` §3); and of those marked `@inline`.
     - `body_hash` — the token kinds of every function body. Comments and whitespace are not tokens, so they change no digest.
   - `Implemented` and tested in `mente-ast`: editing a plain function body changes only `body_hash`; editing a generic body changes `instance_hash` but not `spec_hash`; changing a private field of a `pub struct` or a function's `from` list changes `spec_hash`.
   - **Zero-cascade incremental compilation (`Specified`)**: a build system that recompiles a dependent only when a dependency's `spec_hash` changes, or its `instance_hash` changes and the dependent instantiates or inlines from it. No build system exists yet.
4. **Blueprint projection (`Implemented` in `project_spec_blueprint` and `mn view --spec`)**:
   - `mn view --spec <file.mn>` prints the `pub` declarations, layouts, signatures, contracts and the three digests, and replaces each function body with `= ...`.
   - Measured on the four `examples/` modules (2026-11-04, release build, Windows, including reading the file): 0.3–0.9 ms per module, and 38–60% fewer tokens than the full source.
   - **Latency target (`Target`)**: under 0.1 ms per module, excluding file I/O. There is no measurement protocol yet.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Physical split files (`.mspec` + `.mbody` or C/C++ `.h` + `.cpp`)** | Forces engineers and AI agents to maintain two files in sync for every signature edit, doubles file-system navigation overhead, and introduces out-of-sync declaration drift. |
| **Whole-file text hashing (Rust / Go / TypeScript default)** | Any private comment or statement edit inside a function body invalidates the file hash and triggers cascading recompilation across downstream dependents. Dual AST hashing achieves physical split-file incremental build isolation inside a single `.mn` file. |
| **Inferring interface facts from bodies (provenance, stack size, effects)** | Each inferred fact makes a body edit able to change what callers compile against, which breaks the one-digest test for recompilation and makes an error in one module surface as a failure in another. Writing the fact in the signature, with a conservative default where one is safe, keeps every interface readable in `mn view --spec`. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="pub spec_hash: String" min="1" reason="DualSemanticHash tracks interface spec_hash independently of body_hash" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="pub body_hash: String" min="1" reason="DualSemanticHash tracks implementation body_hash independently of spec_hash" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="pub fn project_spec_blueprint" min="1" reason="mente-ast projects the spec blueprint printed by mn view --spec" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="fn test_private_field_change_changes_spec_hash" min="1" reason="A private field of a pub struct is part of spec_hash, because it decides the struct's size" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="fn test_instantiable_bodies_go_into_instance_hash" min="1" reason="Generic and @inline bodies are tracked by instance_hash, not spec_hash" -->
