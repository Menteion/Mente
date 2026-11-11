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

**The spike held.** The evidence is workflow run 42993264331 for pull request 112, on 2026-11-11, on commit `b37a06d`. On all three targets and at both opt levels, Cranelift returned a tag and an 8-byte payload in two registers, with no setting beyond the flags `mente-codegen` already sets. Every round trip through three functions returned what it should:

- 156 checks on x86-64 Linux;
- 180 on AArch64 Linux;
- 132 on x86-64 Windows.

| Target | Tag and 8-byte payload | 16- and 24-byte payload as return values |
| :--- | :--- | :--- |
| x86-64 Linux, System V | `RAX`, `RDX` | refused in System V; `RAX`, `RCX`, `RDX`, `RSI` in `tail` |
| AArch64 Linux, AAPCS64 | `X0`, `X1` | `X0` to `X3` in AAPCS64; `X2` to `X5` in `tail` |
| x86-64 Windows, fastcall | `RAX`, `RDX` | refused in fastcall; `RAX`, `RCX`, `RDX`, `RSI` in `tail` |

The independent disassembler was GNU objdump: 2.42 on both Linux runners, 2.46 on Windows. The Rust comparison is rustc 1.97.1 at `-C opt-level=3`.

## Findings

1. **Two return values come back in two registers on every target.**
   - **x86-64 Linux (System V)**: tag in `RAX`, payload in `RDX`. That is how C returns a 16-byte struct of two integers, so rustc called `pair_outer` and `pair32_outer` directly as `extern "C"` functions returning `#[repr(C)] struct { tag, payload }`, with no wrapper, and read every value right.
   - **AArch64 Linux (AAPCS64)**: tag in `X0`, payload in `X1`. The direct calls from rustc worked here too.
   - **x86-64 Windows (fastcall)**: tag in `RAX`, payload in `RDX`. Windows' C convention returns one integer register, and a 16-byte struct through a hidden pointer. Cranelift's fastcall adds `RDX` as a second integer return register (its source says the Rust ABI for `i128` needs it) and refuses a third.
     - A Mente-to-Mente call has Cranelift code on both sides, so it is unaffected.
     - A C caller on Windows cannot read `RDX`, so a function exported to C needs the wrapper `spec/error-handling-abi.md` item 2 already plans, as the `_run` wrappers here do.
   - **Settings**: none. The ISA has only `mente-codegen`'s flags.
     - `enable_multi_ret_implicit_sret` is not needed for two values.
     - Neither is `enable_llvm_abi_extensions`. Cranelift's source says it would add `RCX` as a third System V return register; the spike did not try it.
   - **Opt levels**: the placement is the same at `opt_level` `none` and `speed`.
2. **Postfix `!` is a test and a branch, and propagating an error is the epilogue.** The tag and the payload are already in the registers the caller returns them in, so the propagating block moves nothing. `pair32_forward` (`{ val v = leaf(x)!; return v + 1 }`) as objdump shows it on x86-64 Linux:

   ```text
   push %rbp; mov %rsp,%rbp; call pair32_leaf
   test %eax,%eax; jne 972          # postfix !
   xor %eax,%eax; add $0x1,%rdx     # success: tag 0, payload v + 1
   mov %rbp,%rsp; pop %rbp; ret
   972: mov %rbp,%rsp; pop %rbp; ret   # propagate: the epilogue
   ```

   On AArch64 the test and branch is one `cbnz w0`, and propagating is `ldp x29, x30, [sp], #16; ret`. Windows is x86-64 Linux's code plus `sub`/`add $0x20,%rsp` for the shadow space.
   - **The success path pays the tag's test and branch, and the instruction that sets the function's own tag.** Measured against `plain_forward`, which differs only by `!`, that is three instructions on x86-64 (`test`, `jne`, `xor`) and two on AArch64 (`cbnz`, `mov w0, #0`), checked by `test_success_path_adds_only_the_tag_test_and_the_tag`.
   - **The tag should be an `i32`, not an `i8`.** Cranelift treats the bits of an `i8` above the eighth as undefined, so AArch64 masks the tag before branching (`tst w0, #0xff; b.ne`), one more instruction. On x86-64 `testb %al,%al` and `testl %eax,%eax` cost the same.
   - **Size**: the propagating block repeats the epilogue at each `!`. `pair32_forward` against `plain_forward`:

     | Target | `plain_forward` | `pair32_forward` |
     | :--- | :--- | :--- |
     | x86-64 Linux | 21 bytes | 36 bytes |
     | AArch64 | 24 bytes | 40 bytes |
     | Windows | 29 bytes | 48 bytes |
   - **Against Rust's `?` on the same shape** (`rust_q.rs`):
     - Rust returns `Result<i64, u64>` in the same registers on all three targets: `RAX`/`RDX` and `X0`/`X1`.
     - After the call, LLVM tests bit 0 of the tag. On x86-64 that is `mov %rax,%rcx; mov $1,%eax; test $1,%cl; jne`, four instructions against Cranelift's two. On AArch64 it is `tbz w0, #0`, one instruction like Cranelift's `cbnz`.
     - In `rust_forward` and AArch64's `rust_middle`, LLVM removed the branch, selecting the result instead.
     - Instruction counts, Cranelift's `pair32_forward`, `plain_forward` and `pair32_middle` against Rust's `rust_forward`, `rust_plain_forward` and `rust_middle`: 13, 7 and 22 against 8, 5 and 15 on x86-64 Linux; 10, 6 and 16 against 8, 6 and 12 on AArch64.
     - The difference is mostly Cranelift's code in general, not the error convention:
       - a frame pointer set up and torn down in every non-leaf function;
       - the epilogue repeated on each return path;
       - no conversion of branches into selects;
       - branches and immediates in their long encodings (`jne` with a 32-bit offset, `add $0x1` with a 32-bit immediate).
3. **A payload over 8 bytes cannot come back in the platform's registers on x86-64.** Explicit `StructReturn` cannot carry the tag either. Four ways were tried:
   - **More return values in the platform convention**:
     - On x86-64, System V and fastcall refuse a third integer return value: "Too many return values to fit in registers. Use a StructReturn argument instead. (#9510)".
     - AArch64's C convention has eight integer return registers, so it returns 16 and 24 bytes in `X0` to `X3`.
   - **Explicit `StructReturn`**: Cranelift panics with "No return values are allowed when using StructReturn" on all three targets. With a `StructReturn` buffer, the tag would have to go to memory too, which item 2's sizes rule rules out.
   - **Cranelift's `tail` convention**: every value in registers on all three targets.
     - On x86-64: `RAX`, `RCX`, `RDX`, `RSI`, of up to eight registers by Cranelift's source.
     - On AArch64: `X2` to `X5`. `tail` reserves `X0` for a return area and `X1` for an indirect callee, so it has six return registers.
     - It is the smallest code for a wide payload: `wide_tail_middle` is 86 bytes on x86-64 against 130 for the buffer.
     - Unwind information is made for it on Windows too.
     - But it is Cranelift's own convention, unlike C's on every target. By Cranelift's source, on Windows it uses System V's callee-saved registers.
   - **`enable_multi_ret_implicit_sret`**: the first two values come back in `RAX`/`RDX` and the rest through a return area.
     - The caller passes the area's address in the first argument register, `RDI` or `RCX`, and the arguments move one register on.
     - Cranelift documents the setting as deprecated, not conforming to platform ABIs, and to be removed.
     - It made the largest code: `wide_implicit_middle` is 165 bytes on x86-64 Linux and 170 on Windows. AArch64 needs no return area for these sizes.
   - **A buffer the caller passes as an ordinary pointer parameter**: the tag comes back in `RAX` or `X0`, and the payload is written to the buffer. This is the shape item 2 sets for a large `E`.
     - Since `!` only propagates an identical error type, a caller can hand its own buffer down. An error the callee writes is then already where the caller's caller reads it, so propagating stays a test and a branch to the epilogue, with no copy.
     - The success path loads `T` from the buffer and stores the result.
     - The buffer pointer is kept in a callee-saved register across the call, which costs a save and a restore.
     - `wide_buffer_middle` is 130 bytes on x86-64 Linux and 128 on AArch64.
4. **Every function gets unwind information** in every style: System V CFI on Linux, Windows x64 unwind information on Windows (`test_every_function_gets_unwind_information`). So `mente-codegen`'s writer needs nothing new for any of these conventions.

## What M2's lowering of `T ! E` should do

1. **When `T` and `E` each fit 8 bytes**: two Cranelift return values in the target's default convention, with no setting.
   - An `i32` tag, 0 for success and 1 for an error, comes first. Then one `i64` word holding `T` or `E`.
   - They come back in `RAX`/`RDX` on x86-64 and `X0`/`X1` on AArch64.
   - Postfix `!` is `brif tag, propagate, continue`, where `propagate` returns the call's results unchanged; Cranelift makes it the epilogue.
   - A `T` or `E` held in a float register was not tried here.
2. **When the payload is larger**: a caller's buffer passed as an ordinary pointer parameter, with the tag still returned in a register, in the default convention on every target alike.
   - The caller hands its own buffer down when the callee's payload fits it.
   - Never an explicit `StructReturn`, which leaves no register for the tag.
   - Never `enable_multi_ret_implicit_sret`, which is deprecated.
   - The `tail` convention made smaller code, but would add a second convention that differs from C on every target. It is worth measuring once the cost report shows large error unions on hot paths.
3. **A function exported to C**: on Linux, the pair is already C's return of a 16-byte two-integer struct. On Windows it needs the wrapper, since C reads only `RAX`.
4. **The Carry-Flag hypothesis**: what this lowering emits, a tag in a register tested with `test` and `jne` after each call, is the tagged convention item 2's hypothesis is to be measured against.
