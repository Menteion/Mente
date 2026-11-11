# Spike: `T ! E` returned in registers through Cranelift

The spike [`ROADMAP.md`](../../ROADMAP.md) schedules before M2: whether Cranelift's multi-value returns bring Mente's error union `T ! E` back in two registers, a tag and a payload, as Rust returns `Result<u64, u64>`. [`spec/error-handling-abi.md`](../../spec/error-handling-abi.md), item 2, wants a Carry-Flag convention eventually, but Cranelift cannot return a status in a flag, so the first native code returns this tagged form. A spike lives outside the compiler crates, is tested, and is never cited as evidence for a claim ([`docs/design-brief.md`](../../docs/design-brief.md), "Spikes before the milestone that needs them").

This directory is a Cargo workspace of its own, like [`spikes/cranelift-debug-info`](../cranelift-debug-info/README.md). It uses only the Cranelift crates Norman approved on 2026-11-09 ([`debug-info`](../../docs/design-review.md#debug-info), [`dependency-approval`](../../docs/design-review.md#dependency-approval)), at the versions `compiler/mente-codegen` pins: `cranelift-codegen`, `cranelift-frontend`, `cranelift-module`, `cranelift-object` and `cranelift-native` 0.137.1. Its `Cargo.lock` holds the same crates as the debug-info spike's.

## What it does

[`src/lib.rs`](src/lib.rs) writes by hand, in Cranelift IR, chains of three fallible functions, `leaf`, `middle` and `outer`. Each returns a tag (0 for success, 1 for an error) and a payload of one or more 64-bit words holding `T` or `E`. Each caller tests the tag after the call and returns an error unchanged, as postfix `!` does: it only propagates an identical error type. In Mente the chain would read:

```text
fn leaf(x: i64) -> i64 ! u64   { if x < 0 { raise 1000 - x }; return 2 * x }
fn middle(x: i64) -> i64 ! u64 { val v = leaf(x)!; if v > 100 { raise 2000 + v }; return v + 1 }
fn outer(x: i64) -> i64 ! u64  { val v = middle(x)!; if v % 5 == 0 { raise 3000 + v }; return 10 * v }
```

Overflow checks are left out: they do not touch the return convention. The chains differ in the payload's size and in how it travels:

| Chain | Payload | How it returns |
| :--- | :--- | :--- |
| `pair` | 8 bytes | tag (`i8`) and payload as two return values, in the target's C convention |
| `pair32` | 8 bytes | the same with an `i32` tag |
| `wide_platform`, `big_platform` | 16 and 24 bytes | tag and payload words as return values, in the C convention |
| `wide_tail`, `big_tail` | 16 and 24 bytes | the same in Cranelift's own `tail` convention |
| `wide_implicit`, `big_implicit` | 16 and 24 bytes | the same in the C convention with `enable_multi_ret_implicit_sret` |
| `wide_buffer`, `big_buffer` | 16 and 24 bytes | the tag as the one return value, the payload written to a buffer the caller passes |

Every chain also has:

- a probe, which returns a distinct constant in every position, so that its listing shows where each position travels;
- a wrapper in the C convention, `<chain>_run(x, out)`, which calls `outer` and stores the tag and the payload at `out`.

`pair_forward` and `pair32_forward` (`{ val v = leaf(x)!; return v + 1 }`) differ from `plain_forward` (`{ return plain_leaf(x) + 1 }` over an infallible `plain_leaf`) only by `!`, so their listings show what `!` costs. [`rust_q.rs`](rust_q.rs) writes the same shapes in Rust with `?`.

The ISA is built with the flags `mente-codegen` sets: position-independent code, inline stack probes, and `opt_level` `none` or `speed`.

## Running it

```powershell
cargo test --locked
bash check.sh
cargo run -- --target aarch64-unknown-linux-gnu
```

- The unit tests compile every chain for all three targets on any machine, and check where the probes put each position, what Cranelift refuses, the code after each call, and that every function gets unwind information.
- [`check.sh`](check.sh) builds the objects for the machine running it at both opt levels, and links them into [`host.rs`](host.rs), which runs every chain on six inputs and checks each tag and payload word against the rules it recomputes. On Linux it also calls `pair_outer` and `pair32_outer` directly, as C functions returning a 16-byte struct. Then it prints an independent disassembly of the object and rustc's assembly for [`rust_q.rs`](rust_q.rs).
- `cargo run -- --target <triple>` prints Cranelift's listing of every function for any of the three targets.

The checks run on GitHub Actions in [`.github/workflows/spike-error-return.yml`](../../.github/workflows/spike-error-return.yml), on x86-64 Linux, AArch64 Linux and x86-64 Windows.

## Result

Pending the workflow's first run on the pull request.
