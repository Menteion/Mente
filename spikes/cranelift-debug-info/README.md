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

On each, the debugger breaks on a line of `square`, prints the stack, and steps. The checks pass when it stops on that line, walks the stack back through `sum_squares` and `mente_main` to the right lines and into the host's `main`, and steps back to the caller's line. Unit tests read the object back:
- they check each function's line rows and its unwind information;
- on ELF, they decode the DWARF line program with `gimli`;
- on COFF, they decode the CodeView line tables and count the relocations.

## Running it

Debuggers and their output vary by platform, so the checks run on GitHub Actions in [`.github/workflows/spike-cranelift.yml`](../../.github/workflows/spike-cranelift.yml), on x86-64 Linux, AArch64 Linux and x86-64 Windows.

```powershell
cargo test --locked
bash check.sh
```
