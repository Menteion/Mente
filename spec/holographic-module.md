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
2. **Dual 128-bit structural hashing (`Implemented` in `compute_dual_hashes`, `compiler/mente-ast/src/lib.rs`)**:
   - `compiler/mente-ast` computes two non-cryptographic 128-bit digests (two 64-bit FNV-1a-style lanes, hex `String`s) from the parsed declarations:
     - `spec_hash` — the module path, `unit` declarations, every field of each `pub struct` (private fields and defaults included, since they decide the struct's size), `pub bitstruct` fields and `@bits`, `pub enum` variants, `pub trait` method signatures, and each `pub fn`'s attributes, parameters and modes, return and error types, `with` effects, `requires` / `ensures` / `decreases` clauses.
     - `body_hash` — the token kinds of every function body. Comments and whitespace are not tokens, so they change neither digest.
   - `Implemented`: editing a function body changes `body_hash` and leaves `spec_hash` unchanged, and changing a private field of a `pub struct` changes `spec_hash` (both tested in `mente-ast`).
   - **Zero-cascade incremental compilation (`Specified`)**: a build system that recompiles dependents only when a dependency's `spec_hash` changes. No build system exists yet.
3. **Blueprint projection (`Implemented` in `project_spec_blueprint` and `mn view --spec`)**:
   - `mn view --spec <file.mn>` prints the `pub` declarations, layouts, signatures, contracts and both hashes, and replaces each function body with `= ...`.
   - Measured on the four `examples/` modules (2026-11-04, release build, Windows, including reading the file): 0.3–0.9 ms per module, and 38–60% fewer tokens than the full source.
   - **Latency target (`Target`)**: under 0.1 ms per module, excluding file I/O. There is no measurement protocol yet.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Physical split files (`.mspec` + `.mbody` or C/C++ `.h` + `.cpp`)** | Forces engineers and AI agents to maintain two files in sync for every signature edit, doubles file-system navigation overhead, and introduces out-of-sync declaration drift. |
| **Whole-file text hashing (Rust / Go / TypeScript default)** | Any private comment or statement edit inside a function body invalidates the file hash and triggers cascading recompilation across downstream dependents. Dual AST hashing achieves physical split-file incremental build isolation inside a single `.mn` file. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="pub spec_hash: String" min="1" reason="DualSemanticHash tracks interface spec_hash independently of body_hash" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="pub body_hash: String" min="1" reason="DualSemanticHash tracks implementation body_hash independently of spec_hash" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="pub fn project_spec_blueprint" min="1" reason="mente-ast projects the spec blueprint printed by mn view --spec" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="fn test_private_field_change_changes_spec_hash" min="1" reason="A private field of a pub struct is part of spec_hash, because it decides the struct's size" -->
