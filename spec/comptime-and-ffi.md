---
title: Zero-Macro Compile-Time Metaprogramming, FFI Quarantine, and Toolchain
version: 0.1.0
status: active
date: 2026-11-04
---

# Zero-Macro Compile-Time Metaprogramming, FFI Quarantine, and Toolchain

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`, `compiler/mente-cli`

## 1. Specification

1. **Zero-macro `comptime` metaprogramming (`Implemented` in `mente-syntax` and `examples/04_comptime_and_graph_pool.mn`, `Specified` in `MIR` evaluator)**:
   - Mente has **no macro DSL** (`macro_rules!`, procedural token-stream macros, or `#define` preprocessor directives).
   - Compile-time type reflection and code specialization use standard `.mn` syntax (`comptime for`, `comptime if`, and `Type.of<T>().fields`):
     ```rust
     pub fn serialize_fields<comptime T: Type>(val item: T, mut sink: ByteSink) {
         comptime for field in Type.of<T>().fields {
             sink.write_u64(field.read_u64(item))
         }
     }
     ```
   - `comptime` blocks are strictly pure (`with` effects are forbidden at compile time), ensuring 100% deterministic, hermetic builds.
2. **Zero-compromise C / C++ / Rust FFI boundary quarantine (`Implemented` in `mente-syntax` and `mente-sema`)**:
   - External ABI functions are declared inside `extern "C"` blocks with mandatory `with FFI` capability effects and refined boundary types (`Slice<u8>[len: 1..=MAX]`).
   - Outbound calls prove preconditions statically before emitting a direct `CALL`; inbound return values undergo an automatic boundary check (`CMP` + `JC`) before entering Mente's refined value lattice.
   - **Zero-config ecosystem absorption (`mn bind`, `Specified` — Engineering Guardrail #4)**: To overcome Day-1 library ecosystem gravity without writing manual FFI wrappers, `mn bind <header.h | crate.rlib>` parses C headers and Rust C-ABI exports at build time and automatically synthesizes quarantined `extern "C"` declarations with `with FFI`, non-null pointer refinements, and `@layout` byte-offset assertions.
3. **Unified `mn` CLI toolchain (`Implemented` in `compiler/mente-cli/src/main.rs`)**:
   - `mn check`: Runs lexing, dual-hash AST construction, and `mente-sema` static verification.
   - `mn view <file.mn> --spec`: Projects the Holographic Blueprint (`spec_hash`, types, layouts, signatures, contracts) in `< 1 ms`.
   - `mn test --mutate`: Runs inline `@example` / `@property` checks and executes the in-memory `MIR` mutation prover (`MirGraph::run_mutation_gate`).
   - `mn spec [guard|prove|graph]`: Runs executable specification verification across `spec/*.md`.

## 2. Decision rationale and rejected alternatives

| Alternative considered | Reason for rejection |
| :--- | :--- |
| **Rust procedural macros (`syn` + `quote` + separate proc-macro crates) and `macro_rules!`** | Procedural macros operate on untyped token streams, require compiling separate host dynamic libraries (`proc-macro = true`), slow down build times dramatically, break IDE jump-to-definition, and allow arbitrary host filesystem/network side effects during compilation. |
| **Unchecked `unsafe extern "C"` blocks (Rust / C++)** | Trusting raw C return values without boundary checks allows a single malformed C return value to corrupt Mente's internal refinement assumptions. Automatic inbound boundary validation quarantines foreign faults at the exact FFI perimeter. |

## 3. Verification assertions

<!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="Comptime," min="1" reason="Lexer defines Comptime keyword for zero-macro metaprogramming" -->
<!-- @assert-count target="compiler/mente-cli/src/main.rs" symbol="cmd_spec" min="1" reason="mente-cli implements native mn spec guard/prove/graph verification" -->
<!-- @assert-count target="examples/04_comptime_and_graph_pool.mn" symbol="comptime for field in Type.of<T>().fields" min="1" reason="Example 04 demonstrates zero-macro comptime struct serialization" -->
<!-- @assert-absence target="examples" symbol="macro_rules!" glob="*.mn" reason="Mente forbids token-stream macros in favor of comptime" -->
