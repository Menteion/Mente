---
title: Orthogonal Error Union Model and Carry-Flag Zero-Overhead ABI
version: 0.1.0
status: active
date: 2026-11-04
---

# Orthogonal Error Union Model and Carry-Flag Zero-Overhead ABI

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-codegen`

## 1. Specification

1. **Strict separation of `?T` (absence) and `T ! E` (error) (`Specified`; `parse_return_and_error_sig` records a function's `! E` error type, `Implemented`)**:
   - `?T` represents expected optionality (`none` vs `T`); `T ! E` represents recoverable operational failure.
   - Inside a function returning `T ! E`:
     - The normal `return expr` path returns `T` directly **without** `Ok(expr)` wrapping.
     - Error paths exit via `raise ErrVariant`.
     - Callers propagate errors with postfix `!` (`val hdr = decode(buf)!`) or handle them inline with `catch { err => ... }`.
2. **CPU Carry-Flag (`CF`) zero-overhead register ABI (`Specified`; its speed advantage is a `Hypothesis` with no benchmark yet)**:
   - Today `CarryFlagAbiLowering` in `compiler/mente-codegen/src/lib.rs` only names the registers and flag below for each target; nothing emits machine code.
   - **Not in the first native backend**: Cranelift, which Phase 3 uses, cannot return a status in the carry flag, so Phase 3 lowers `T ! E` to the two-register tagged return this hypothesis is measured against. The Carry-Flag convention waits for a custom emitter or a Cranelift fork (`mir-and-codegen.md`).
   - Rather than packing `T ! E` into a tagged memory union (`sizeof(Result<T, E>) = max(sizeof(T), sizeof(E)) + tag_padding`), Mente lowers `T ! E` directly to hardware status flags:
     - **Success return (`CF = 0`)**: Primary return register (`RAX` / `XMM0` on x86-64; `X0` on AArch64) carries `T`; `CLC` clears the Carry Flag.
     - **Error return (`CF = 1`)**: Secondary return register (`RDX` / `X1`) carries the compact error payload `E`; `STC` sets the Carry Flag.
     - **Postfix `!` propagation**: Lowers to a single conditional jump `JC error_landing_pad` (`B.CS` on AArch64), with `0` stack union spills and `0` tag shifts.
   - **Sizes**: `E` travels in the secondary register when it fits 8 bytes. A larger `E` is written to a buffer the caller provides, as an oversized `T` already is (`sret`); the flag still says which one was written, so the caller never reads a tag from memory.
   - **Keeping the flag intact**: between the `CLC` / `STC` and the `RET`, a function's epilogue MUST NOT execute an instruction that changes the flag. On x86-64 that rules out `add rsp, N`; the epilogue uses `lea rsp, [rsp + N]` and `pop`. On AArch64 the epilogue's `add sp, sp, #N` without `S` leaves `NZCV` alone.
   - **Where the convention does not apply**: a function exported to or imported from C (`extern "C"`) returns a tagged value in C's own convention, because C callers cannot read a flag; the compiler writes the wrapper. A `@kernel` would do the same, since SPIR-V has no flags register. A function type records its convention, so a `fn(T) -> U ! E` value is always called the way it was compiled.
   - **The hypothesis and how to test it**: on call-heavy code where errors are rare, this convention is faster than the best tagged alternative, which is not a stack-allocated `Result` but Rust's: a `Result<u64, u64>`-sized value returned in two registers and tested with `test` + `jne` after each call. The benchmark compares the two on the same lowered code with the same error rates; until it runs, the speed claim is a `Hypothesis`.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Tagged `Result<T, E>` returned in registers (Rust)** | When `T` and `E` each fit a register, Rust returns `Result` in `RAX`/`RDX` and the caller tests the tag after every call; the Carry Flag replaces that `test` and frees the tag's register bits. When the error is larger than a register (a 16-byte error makes `Result<u64, Error>` 24 bytes), Rust returns through memory (`sret`), as Mente does for a large `E`, so the difference there is only that Mente's tag stays in the flag. Whether that is worth a custom convention is the hypothesis above. |
| **DWARF stack-unwinding exceptions (`try`/`throw`/`catch` in C++ / Java / Python)** | Violates zero-cost determinism on the error path, hides control flow from function signatures, and bloats binaries with `.eh_frame` unwinding tables. |
| **Go `val, err := f(); if err != nil` tuple returns** | Pollutes 50%+ of application lines with repetitive boilerplate and allows programmers to accidentally read `val` when `err != nil`. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-codegen/src/lib.rs" symbol="CarryFlagAbiLowering" min="1" reason="Codegen names the registers and flag of the specified Carry-Flag (CF) error ABI" -->
<!-- @assert-count target="compiler/mente-codegen/src/lib.rs" symbol="happy_path_tag_overhead_bytes" min="1" reason="The ABI descriptor records 0 tag bytes on the happy path" -->
<!-- @assert-count target="examples/03_colorless_concurrency_and_effects.mn" symbol="! GatewayError" min="2" reason="Example 03 demonstrates unwrapped T ! E return signature with raise and postfix !" -->
