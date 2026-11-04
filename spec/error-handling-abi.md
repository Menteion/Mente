---
title: Orthogonal Error Union Model and Carry-Flag Zero-Overhead ABI
version: 0.1.0
status: active
date: 2026-11-04
---

# Orthogonal Error Union Model and Carry-Flag Zero-Overhead ABI

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-codegen`

## 1. Specification

1. **Strict separation of `?T` (absence) and `T ! E` (error) (`Implemented` in `mente-syntax` and `examples/03_colorless_concurrency_and_effects.mn`)**:
   - `?T` represents expected optionality (`none` vs `T`); `T ! E` represents recoverable operational failure.
   - Inside a function returning `T ! E`:
     - The normal `return expr` path returns `T` directly **without** `Ok(expr)` wrapping.
     - Error paths exit via `raise ErrVariant`.
     - Callers propagate errors with postfix `!` (`val hdr = decode(buf)!`) or handle them inline with `catch { err => ... }`.
2. **CPU Carry-Flag (`CF`) zero-overhead register ABI (`Implemented` in `compiler/mente-codegen/src/lib.rs`)**:
   - Rather than packing `T ! E` into a tagged memory union (`sizeof(Result<T, E>) = max(sizeof(T), sizeof(E)) + tag_padding`), Mente lowers `T ! E` directly to hardware status flags (`CarryFlagAbiLowering` in `compiler/mente-codegen/src/lib.rs`):
     - **Success return (`CF = 0`)**: Primary return register (`RAX` / `XMM0` on x86-64; `X0` on AArch64) carries `T`; `CLC` clears the Carry Flag.
     - **Error return (`CF = 1`)**: Secondary return register (`RDX` / `X1`) carries the compact error payload `E`; `STC` sets the Carry Flag.
     - **Postfix `!` propagation**: Lowers to a single 2-byte conditional jump `JC error_landing_pad` ( predicted not-taken on the hot path, `0` stack union spills, `0` tag shifts).

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Stack-tagged `Result<T, E>` enum (Rust)** | Wrapping a 64-bit value `u64` with a 16-byte error struct inflates `Result<u64, Error>` to 24 bytes, forcing the return value out of `RAX` onto stack memory (`sret`) and adding tag-branching overhead on every call even when no error occurs. |
| **DWARF stack-unwinding exceptions (`try`/`throw`/`catch` in C++ / Java / Python)** | Violates zero-cost determinism on the error path, hides control flow from function signatures, and bloats binaries with `.eh_frame` unwinding tables. |
| **Go `val, err := f(); if err != nil` tuple returns** | Pollutes 50%+ of application lines with repetitive boilerplate and allows programmers to accidentally read `val` when `err != nil`. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-codegen/src/lib.rs" symbol="CarryFlagAbiLowering" min="1" reason="Codegen lowers T ! E returns to CPU Carry-Flag (CF) register convention" -->
<!-- @assert-count target="compiler/mente-codegen/src/lib.rs" symbol="happy_path_tag_overhead_bytes" min="1" reason="Carry-Flag ABI guarantees 0 tag overhead bytes on the happy path" -->
<!-- @assert-count target="examples/03_colorless_concurrency_and_effects.mn" symbol="! GatewayError" min="2" reason="Example 03 demonstrates unwrapped T ! E return signature with raise and postfix !" -->
