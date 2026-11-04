---
title: Zero-Macro Compile-Time Metaprogramming, FFI Quarantine, and Toolchain
version: 0.1.0
status: active
date: 2026-11-04
---

# Zero-Macro Compile-Time Metaprogramming, FFI Quarantine, and Toolchain

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`, `compiler/mente-cli`

## 1. Specification

1. **Zero-macro `comptime` metaprogramming (`Specified`; the lexer has a `Comptime` token and `examples/04_comptime_and_graph_pool.mn` uses the syntax)**:
   - Mente has **no macro DSL** (`macro_rules!`, procedural token-stream macros, or `#define` preprocessor directives).
   - Compile-time type reflection and code specialization use standard `.mn` syntax (`comptime for`, `comptime if`, and `Type.of<T>().fields`):
     ```rust
     pub fn serialize_fields<T>(item: T, mut sink: ByteSink) {
         comptime for field in Type.of<T>().fields {
             comptime if field.ty == u64 {
                 sink.write_u64(item.(field.name))
             }
         }
     }
     ```
   - `item.(field.name)` reads the field whose name is the compile-time string `field.name`. It is resolved when the loop is expanded, so each expanded line is an ordinary, type-checked field access.
   - `comptime` blocks are strictly pure (`with` effects are forbidden at compile time), ensuring 100% deterministic, hermetic builds.
2. **Zero-compromise C / C++ / Rust FFI boundary quarantine (`Specified`; the lexer has an `Extern` token, and the parser has no `extern` item yet)**:
   - External ABI functions are declared inside `extern "C"` blocks with mandatory `with FFI` capability effects, refined boundary types (`fd: i32[0..=65535]`) and length contracts (`buf: []u8` with `requires buf.len >= 1`).
   - Outbound calls prove preconditions statically before emitting a direct `CALL`; inbound return values undergo an automatic boundary check (`CMP` + `JC`) before entering Mente's refined value lattice.
   - **Zero-config ecosystem absorption (`mn bind`, `Specified` — Engineering Guardrail #4)**: To overcome Day-1 library ecosystem gravity without writing manual FFI wrappers, `mn bind <header.h | crate.rlib>` parses C headers and Rust C-ABI exports at build time and automatically synthesizes quarantined `extern "C"` declarations with `with FFI`, non-null pointer refinements, and `@layout` byte-offset assertions.
3. **Unified `mn` CLI toolchain (`compiler/mente-cli/src/main.rs`)**:
   - `mn check` (`Implemented`): parses each `.mn` file's declarations, computes both hashes, and runs `mente-sema`: the `@layout` and `bitstruct` provers and the token-level `@total` / `@branchless` checks.
   - `mn view --spec <file.mn>` (`Implemented`): prints the Holographic Blueprint (`spec_hash`, types, layouts, signatures, contracts); see `holographic-module.md` for measured time.
   - `mn test --mutate` (`Implemented` as a demo): runs `mn check`, then runs the hand-written MIR region `integrate_cell` built in `main.rs` against two test vectors and, with `--mutate`, `run_in_memory_mutation_gate` on it. `@example` and `@property` bodies are not executed (`Specified`).
   - `mn spec` (`Implemented`): checks each `spec/*.md` for YAML front matter and at least one `@assert-*` directive, and runs the directives, rejecting any that cannot fail. Proving each rule can fail against a mutated tree is left to `spec-guard prove`.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Rust procedural macros (`syn` + `quote` + separate proc-macro crates) and `macro_rules!`** | Procedural macros operate on untyped token streams, require compiling separate host dynamic libraries (`proc-macro = true`), slow down build times dramatically, break IDE jump-to-definition, and allow arbitrary host filesystem/network side effects during compilation. |
| **Unchecked `unsafe extern "C"` blocks (Rust / C++)** | Trusting raw C return values without boundary checks allows a single malformed C return value to corrupt Mente's internal refinement assumptions. Automatic inbound boundary validation quarantines foreign faults at the exact FFI perimeter. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Comptime," min="1" reason="Lexer defines Comptime keyword for zero-macro metaprogramming" -->
<!-- @assert-count target="compiler/mente-cli/src/main.rs" symbol="cmd_spec" min="1" reason="mente-cli implements mn spec: front matter and @assert-* directives" -->
<!-- @assert-count target="compiler/mente-cli/src/main.rs" symbol="fn test_spec_directives_that_cannot_fail_are_rejected" min="1" reason="mn spec rejects a directive that cannot fail" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="comptime for field in Type.of<T>().fields" min="1" reason="Example 04 demonstrates zero-macro comptime struct serialization" -->
<!-- @assert-absence target="examples" symbol="macro_rules!" glob="*.mn" reason="Mente forbids token-stream macros in favor of comptime" -->
