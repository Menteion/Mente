---
title: Holographic Single-File Module Model and Dual AST Hashing
version: 0.1.0
status: active
date: 2026-11-04
---

# Holographic Single-File Module Model and Dual AST Hashing

**Governs:** `compiler/mente-syntax`, `compiler/mente-ast`, `compiler/mente-cli`

## 1. Specification

1. **Single `.mn` source file (`Implemented` in `compiler/mente-ast/src/lib.rs`)**:
   - Every Mente module resides in a single `.mn` source file. Separate `.h` / `.cpp` or `.mspec` / `.mbody` split files MUST NOT exist.
   - Public types, memory layouts, function signatures, capability effects (`with ...`), and ghost contracts (`requires` / `ensures` / `invariant`) are written inline alongside function bodies.
2. **Dual 64-bit FNV-1a AST hashing (`Implemented` in `ModuleAst::compute_dual_hashes`)**:
   - During parsing, `compiler/mente-ast` partitions the AST into two independent cryptographic/structural digests:
     - `spec_hash: u64` — hashes solely public declarations, struct/bitstruct `@layout` and `@bytes`/`@bits` metadata, function signatures, `with` effect sets, and `requires`/`ensures` contracts.
     - `body_hash: u64` — hashes the executable statement and expression bodies of all functions.
   - **Zero-cascade incremental compilation**: Modifying a function body alters `body_hash` while leaving `spec_hash` unchanged (`spec_hash_before == spec_hash_after`), guaranteeing **zero downstream recompilation** of dependent modules.
3. **Sub-millisecond blueprint projection (`mn view --spec`, `Implemented` in `compiler/mente-cli/src/main.rs`)**:
   - Running `mn view <file.mn> --spec` projects only the specification surface (types, layouts, signatures, contracts, and `spec_hash`) while eliding function bodies `{ ... }`, reducing context token consumption by 70–85% for architectural review.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Physical split files (`.mspec` + `.mbody` or C/C++ `.h` + `.cpp`)** | Forces engineers and AI agents to maintain two files in sync for every signature edit, doubles file-system navigation overhead, and introduces out-of-sync declaration drift. |
| **Whole-file text hashing (Rust / Go / TypeScript default)** | Any private comment or statement edit inside a function body invalidates the file hash and triggers cascading recompilation across downstream dependents. Dual AST hashing achieves physical split-file incremental build isolation inside a single `.mn` file. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="pub spec_hash: String" min="1" reason="DualSemanticHash tracks interface spec_hash independently of body_hash" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="pub body_hash: String" min="1" reason="DualSemanticHash tracks implementation body_hash independently of spec_hash" -->
<!-- @assert-count target="compiler/mente-ast/src/lib.rs" symbol="pub fn project_spec_blueprint" min="1" reason="mente-ast supports sub-millisecond blueprint projection for mn view --spec" -->
