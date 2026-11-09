---
title: Zero-Macro Compile-Time Metaprogramming, FFI Quarantine, and Toolchain
version: 0.1.0
status: active
date: 2026-11-04
---

# Zero-Macro Compile-Time Metaprogramming, FFI Quarantine, and Toolchain

**Governs:** `compiler/mente-syntax`, `compiler/mente-sema`, `compiler/mente-mir`, `compiler/mente-cli`

## 1. Specification

1. `Specified` **Zero-macro `comptime` metaprogramming**:
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
   - `comptime` blocks are strictly pure (`with` effects are forbidden at compile time), ensuring deterministic, hermetic builds.
   - **Termination**: each `comptime` evaluation runs under a step budget, by default 10^7 evaluation steps. Exceeding it is a compile-time error (`E0701`) that names the loop or call still running; `@comptime_budget(steps: N)` on a function raises the budget for evaluations it starts. A build can therefore never hang in `comptime` code.
   - The lexer has a `Comptime` token and `examples/04_comptime_and_graph_pool.mn` uses the syntax; nothing parses or evaluates it yet.
2. `Specified` **Zero-compromise C / C++ / Rust FFI boundary quarantine**:
   - External ABI functions are declared inside `extern "C"` blocks with mandatory `with FFI` capability effects, refined boundary types (`fd: i32[0..=65535]`) and length contracts (`buf: []u8` with `requires buf.len >= 1`).
   - Outbound calls prove preconditions statically before emitting a direct `CALL`; inbound return values undergo an automatic boundary check before entering Mente's refined value lattice.
   - **A failed inbound check is an error value**: an `extern` function whose result type is refined MUST declare an error type (`-> u16[1..=65535] ! FfiError`), and a value outside the refinement raises it at the boundary. Nothing foreign enters the lattice unchecked, and nothing traps.
   - **Raw pointers stay in quarantine**: a C pointer has the type `RawPtr<T>`. It may be passed, stored and compared only in code declared `with FFI`, and becomes a Mente value only through a checked conversion (`ptr.read(len)` yields `[]T ! FfiError`, failing on null). A pointer is nullable unless the binding says otherwise.
   - The lexer has an `Extern` token, and the parser has no `extern` item yet.
   - **Ecosystem absorption (`mn bind`, Engineering Guardrail #4)**: to reduce hand-written FFI, `mn bind <header.h>` parses a C header at build time and synthesizes quarantined `extern "C"` declarations with `with FFI` and `@layout` byte-offset assertions for its structs.
     - A pointer is bound as nullable unless the header annotates it (`_Nonnull`) or a Mente sidecar file next to the header says otherwise; C headers do not otherwise say which pointers may be null.
     - A Rust library is bound through the C header that `cbindgen` generates for its `extern "C"` exports; a `.rlib` carries no C-ABI description to read.
     - The C dialect `mn bind` accepts (preprocessor, compiler extensions, platform headers) is written down before the feature is promised for real libraries; parsing arbitrary system headers without libclang is a project of its own.
3. **Unified `mn` CLI toolchain (`compiler/mente-cli/src/main.rs`)**:
   - How `mn` reports a problem, `mn check --json` and `mn explain` are specified in `diagnostics.md`.
   - `Implemented`: `mn check` parses each `.mn` file's declarations, computes its digests, runs `mente-sema` (the `@layout` and `bitstruct` provers, the token-level `@total` / `@branchless` checks and the `from` check), and parses every function with a body under M1's grammar (`grammar.md`); a file that fails any of them fails the command.
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_check_proves_a_layout_and_rejects_broken_modules" min="1" reason="mn check passes a clean module and fails one that breaks a prover" -->
     <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_report_counts_contract_functions" min="1" reason="The report counts @total, @branchless and @kernel functions and examples" -->
   - `Implemented`: `mn view --spec <file.mn>` prints the Holographic Blueprint (`spec_hash`, types, layouts, signatures, contracts); see `holographic-module.md` for measured time.
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_view_spec_prints_the_blueprint" min="1" reason="mn view --spec prints the digests and signatures and drops bodies" -->
   <!-- @spec-item id="mn-view-checks" -->
   - `Implemented`: `mn view --checks <file.mn>` lists, for each function whose body checks, each run-time check Tier 1 leaves, one per line as `line:col kind detail` (a narrowing into a type, an index into an array, a divisor, a shift amount), then how many run-time checks Tier 1 erased.
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_view_checks_lists_the_run_time_checks_tier1_leaves" min="1" reason="mn view --checks lists the checks Tier 1 leaves and counts those it erased" -->
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_run_time_checks_say_what_they_check" min="1" reason="Each check says what it checks against" -->
   - `Implemented`: `mn test` checks that each module reports exactly the diagnostics its `.expected` file lists, one `line:col code` per line, or none when it has no such file ([`example-bodies`](../docs/design-review.md#example-bodies)); as a demo, it then runs the hand-written MIR region `integrate_cell` built in `main.rs` against two test vectors and, with `--mutate`, `run_in_memory_mutation_gate` on it.
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_test_mutate_runs_the_demo_gate" min="1" reason="mn test runs the demo region, and --mutate adds its mutation gate" -->
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_test_compares_each_module_with_its_expected_file" min="1" reason="mn test compares each module with its .expected file" -->
   - `Specified`: `mn test` runs `@example` and `@property` bodies; today they are not executed.
   - `Implemented`: `mn run <file.mn>` runs `main` on the reference interpreter, or the call `--call "f(1, mut [2, 3])"` names, whose `mut` marks must match the parameters. It prints the result and what each `mut` argument is left holding, or the trap and where it happened (exit 1), or the diagnostics of a module that does not check (exit 2).
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_run_prints_a_result_or_a_trap" min="1" reason="mn run prints a result, what mut arguments leave, or a trap" -->
   - `Implemented`: `mn run --mir` runs the same call through MIR, lowered from the checked module and run on the MIR interpreter, and `mn run --native` as native code, built for the host and linked by the system linker; both print what the reference interpreter prints: the result, what each `mut` argument leaves, or the same trap at the same place.
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_run_mir_prints_what_the_interpreter_prints" min="1" reason="mn run --mir prints what the reference interpreter prints" -->
   - `Implemented`: `mn build <file.mn> [--call "f(args)"] [-o <exe>]` builds the call, `main` by default, as an executable with debug information, by default at the source's path without `.mn`, its object file and on Windows its PDB beside it. Started, it makes the call and prints what `mn run` prints, a trap with the file's name. CI builds a program with it and steps through it, with gdb on Linux and cdb on Windows (`compiler/mente-cli/tests/debuggers/check.sh`).
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_build_writes_an_executable_that_prints_what_mn_run_prints" min="1" reason="mn build writes an executable that prints what mn run prints" -->
     <!-- @assert-present file="compiler/mente-cli/tests/debuggers/check.sh,compiler/mente-cli/tests/debuggers/demo.mn" reason="The debugger sessions CI runs on native code built from a .mn program" -->
   - `Implemented`: `mn check --watch <file.mn>` checks a module item by item, and again each time its file changes. It splits the module into its top-level items, parses and checks each on its own with spans relative to the item, and keeps the results by the item's text, so an edit re-parses only the items it changed and re-checks only their functions, or every function when a name or a signature changes ([`tier1-latency-spike`](../docs/design-review.md#tier1-latency-spike)). It reports exactly what `mn check` reports for the module, in the same order; a module that declares more than functions is checked whole.
     <!-- @assert-count target="compiler/mente-sema/src/incremental.rs" symbol="fn test_a_session_reports_what_the_whole_module_check_does" min="1" reason="Item by item, every module in the repository reports what the whole check does" -->
     <!-- @assert-count target="compiler/mente-sema/src/incremental.rs" symbol="fn test_an_edit_parses_and_checks_again_only_what_it_changes" min="1" reason="An edit re-parses and re-checks only what it changed" -->
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_a_module_splits_into_its_top_level_items" min="1" reason="A module splits into its top-level items" -->
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_check_watch_checks_item_by_item" min="1" reason="mn check --watch reports what it re-parsed and re-checked" -->
   - `Implemented`: `mn dump --tokens|--tree|--typed <file.mn>` prints one phase's output in its canonical form, versioned text that two implementations of the phase must print byte for byte alike ([`canonical-forms`](../docs/design-review.md#canonical-forms)): the tokens one per line, each function's syntax tree as an S-expression, and the typed functions with each binding's and expression's type, followed by the diagnostics. MIR's form comes with lowering in checkpoint (c).
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_dump_prints_canonical_forms" min="1" reason="mn dump prints each phase's canonical form, the same on every run" -->
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_tokens_and_trees_dump_in_their_canonical_forms" min="1" reason="Tokens and trees dump in their canonical forms" -->
     <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_typed_functions_dump_with_their_diagnostics" min="1" reason="Typed functions dump with their diagnostics" -->
   - `Implemented`: `mn spec` checks each `spec/*.md` for YAML front matter and at least one `@assert-*` directive, and runs the directives, rejecting any that cannot fail. On a `.mn` target it removes `//` comments first, so a comment can neither satisfy a count nor trip an absence. Proving each rule can fail against a mutated tree is left to `spec-guard prove`.
     <!-- @assert-count target="compiler/mente-cli/tests/cli.rs" symbol="fn test_mn_spec_checks_front_matter_and_directives" min="1" reason="mn spec fails a spec without front matter or directives, and a directive that does not hold" -->
     <!-- @assert-count target="compiler/mente-cli/src/main.rs" symbol="fn test_spec_directives_that_cannot_fail_are_rejected" min="1" reason="mn spec rejects a directive that cannot fail" -->
     <!-- @assert-count target="compiler/mente-cli/src/main.rs" symbol="fn test_spec_directives_on_mn_files_ignore_comments" min="1" reason="mn spec removes comments from a .mn target first" -->
     <!-- @assert-count target="compiler/mente-cli/src/main.rs" symbol="fn test_strip_mn_comments_keeps_code_strings_and_lines" min="1" reason="Comments go, strings and line breaks stay" -->

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
<!-- @assert-count target="compiler/mente-cli/src/main.rs" symbol="fn test_spec_directives_on_mn_files_ignore_comments" min="1" reason="A directive on a .mn file is met by code, never by a comment" -->
