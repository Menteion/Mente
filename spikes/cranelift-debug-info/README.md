# Spike: debug and unwind information through Cranelift

The M0b spike of [`ROADMAP.md`](../../ROADMAP.md) and [`debug-info`](../../docs/design-review.md#debug-info): how much of the debug and unwind information native code needs Cranelift provides, and whether debuggers read what it and the spike produce, with DWARF on Linux and a PDB built from CodeView on Windows. A spike lives outside the compiler crates, is tested, and is never cited as evidence for a claim ([`docs/design-brief.md`](../../docs/design-brief.md), "Spikes before the milestone that needs them").

This directory is a Cargo workspace of its own, so that the compiler's dependencies stay as they are until M1's checkpoint (c). On 2026-11-09 Norman approved its external crates for it: `cranelift-codegen`, `cranelift-frontend`, `cranelift-module`, `cranelift-object` and `cranelift-native` 0.137.1, `gimli` 0.33 and `object` 0.40. `Cargo.lock` lists everything they bring in.

## What it does

[`demo.mn`](demo.mn) is a three-function program: `mente_main` calls `sum_squares`, whose loop calls `square`. `src/lib.rs` writes it by hand as Cranelift IR, each instruction tagged with its line of `demo.mn`, compiles it for the host, and writes an object file:

- **From Cranelift**:
  - the machine code;
  - the code range of every source location (`MachSrcLoc`);
  - the symbols, through `cranelift-object`;
  - per-function unwind information: System V CFI, which `cranelift-object` writes as `.eh_frame` on ELF, and Windows x64 `UNWIND_INFO`.
- **Added by the spike on ELF**: the DWARF compilation unit, its subprograms and line table, written with `gimli`, with relocations against the function symbols and the debug sections.
- **Added by the spike on COFF**:
  - `.pdata` and `.xdata`, built from Cranelift's Windows unwind information;
  - CodeView symbols and line tables in `.debug$S`, which the linker turns into a PDB. No Rust crate writes CodeView, so the spike encodes the records itself.

[`check.sh`](check.sh) links the object into [`host.rs`](host.rs) with `rustc -C debuginfo=2`, runs it, and drives the debuggers:

- **On Linux**: gdb in batch mode, then lldb.
- **On Windows**: cdb.

On each, the debugger breaks on a line of `square` and prints the stack, and gdb and lldb then step. The checks pass when it stops on that line, walks the stack back through `sum_squares` and `mente_main` to the lines of their calls and into the host's `main`, and steps out of `square` into its caller. Unit tests read the object back:
- they check each function's line rows and its unwind information;
- on ELF, they decode the DWARF line program with `gimli`;
- on COFF, they decode the CodeView line tables and count the relocations.

## Running it

Debuggers and their output vary by platform, so the checks run on GitHub Actions in [`.github/workflows/spike-cranelift.yml`](../../.github/workflows/spike-cranelift.yml), on x86-64 Linux, AArch64 Linux and x86-64 Windows.

```powershell
cargo test --locked
bash check.sh
```

## Result

On 2026-11-09 (tag `m0b`), in the workflow runs for pull request 21, every check passed on all three platforms:

- **gdb 15.1 and lldb 18.1.3, on x86-64 and AArch64 Linux**: both stopped at `square () at demo.mn:5`, and showed `sum_squares` at line 12, `mente_main` at line 19, and then `host::main` and the Rust runtime below them. gdb's `next` left `square` for line 13 of `sum_squares`, since it steps on from the middle of line 12 to the start of the next line; lldb's step-over stopped at line 12.
- **cdb 10.0.26100, on x86-64 Windows Server 2025**: a breakpoint on `demo.mn:5` stopped in `demo!square [demo.mn @ 5]`. `k` walked `demo!sum_squares [demo.mn @ 12]` and `demo!mente_main [demo.mn @ 19]` into `demo!host::main`, the Rust runtime and the C runtime's start-up, through the spike's `.pdata` and the PDB the linker built from its CodeView.

## Findings

1. **Cranelift gives everything the information is made from, but writes little of it.** It gives:
   - the code range of every source location;
   - the symbols;
   - unwind information in both encodings: System V CFI and Windows x64 `UNWIND_INFO`.

   It writes no DWARF debug information, no CodeView, and no `.pdata` or `.xdata`. `mente-codegen` writes them, as the spike does in about 300 lines: `gimli` writes the DWARF, and CodeView is encoded by hand, since no Rust crate writes it.
2. **`cranelift-object`'s own `.eh_frame` cannot be linked into a position-independent executable.** It encodes each FDE's address as an absolute 64-bit pointer (`R_X86_64_64`). Executables are position-independent by default on Linux, and `rust-lld` rejects that relocation in them. The spike writes `.eh_frame` itself from the same unwind information, with PC-relative 32-bit pointers, and so does `mente-codegen` until `cranelift-object` changes.
3. **A PDB from Cranelift's output works.** The linker merges the hand-written CodeView into the PDB, and cdb reads lines, symbols and the stack. So `cdb` joins the CI debugger session on Windows from M1's checkpoint (c), as [`debug-info`](../../docs/design-review.md#debug-info) set as the condition.
4. **Not every function has a prologue line.** On AArch64, the leaf function `square` has no instruction without a source location, so its line table has no row for the declaration line. Line tables are built from the code ranges as they are, never from an assumed layout.
5. **Open for M1's checkpoint (c)**:
   - Mente has no DWARF language code, so the spike uses `DW_LANG_C11`, which keeps the debuggers' expression evaluators usable.
   - `perf` was not tried, since the demo runs too briefly to sample; it belongs in the scripted session, with a longer program.
   - Variables, types and inlined frames are outside this spike, and come with the constructs that need them.
