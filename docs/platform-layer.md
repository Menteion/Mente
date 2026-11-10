---
title: Mente Platform Layer
version: 0.1.0
status: active
date: 2026-11-10
---

# Mente — The Platform Layer (`docs/platform-layer.md`)

The runtime and the standard library are written in Mente wherever the language allows, from the milestone that builds them ([docs/design-brief.md](design-brief.md) section 7.4). What cannot be forms this layer: each item, with the reason Mente cannot express it yet and the milestone that lets it move. Exit criterion 13 of every milestone checks that whatever the milestone added to the runtime is written in Mente or listed here.

`mente-codegen` itself, with Cranelift, is not part of this layer: it is the compiler's backend, which stays in Rust until the performance stage replaces it with one written in Mente (section 7.5).

## Native programs (since M1, checkpoint (c))

A native program holds the code of its Mente functions, each named `mente_f<index>_<name>`, and, beside it, the code below. `mente-codegen` generates every function of it in Cranelift IR ([`native-runtime`](design-review.md#native-runtime)), and the C library supplies the imports. A test, `test_every_symbol_outside_the_program_is_listed_in_the_platform_layer` in `compiler/mente-codegen/src/native.rs`, reads each target's object and fails on a symbol outside the program's own functions that this table does not list, and on a listed symbol no object has.

| Symbol | What it does | Why it is not Mente yet | It moves to Mente with |
| :--- | :--- | :--- | :--- |
| `main` | Reads the call's number from the first argument, sets up that call's arguments, makes the call, prints its result and each `mut` argument, and returns the exit status. | M1's Mente has no strings, no way to read a program's arguments, and no `extern "C"`. It is also the test harness of the three execution paths more than a program's own entry. | `extern "C"` with `with FFI` (M4), and a program entry the language defines. |
| `mente_rt_print` | Writes an integer, given by its sign and its 128-bit magnitude, in decimal by 32-bit limbs, to standard output for a result or to standard error in a trap report. | Writing calls the C library's `write`, which needs `extern "C"` and `with FFI`. The decimal conversion alone could be Mente, but a program cannot yet import a module that holds it: M1 has no `import`. | `import` and `extern "C"` with `with FFI` (M4). |
| `mente_rt_udivmod` | Divides 128-bit integers: one 64-bit division when both fit, three when the divisor fits in 32 bits, otherwise bit by bit. | Cranelift lowers integer division only up to 64 bits. Mente could write it over 64-bit halves, but no program can import it before `import` exists, and it must never be compiled through the 128-bit division it implements. | `import`, and a rule that marks it as the implementation of 128-bit `/` and `%`. |
| `write` | The C library's write, on Linux: the text of results and trap reports, and `mente_rt_print`'s digits. | A system call; it stays in the platform layer for good, reached through `extern "C"` once Mente has it. | Never: it is the operating system's. |
| `_write` | The C runtime's write, on Windows. | As `write`. | Never. |
| `exit` | Ends the process with status 1 once a trap is reported. | As `write`. | Never. |
| `memcpy` | Copies an array, as Cranelift's copy of a block of memory calls it. | Cranelift calls the C library's `memcpy` for a copy of a known size. A copy written in Mente would be compiled into the same call. | Never, while Cranelift lowers copies; a backend written in Mente may lower them itself. |

Inline code the backend emits is not runtime and is not listed: stack probes, checked 128-bit arithmetic, and each trap site's call of `mente_rt_print` and `exit`.
