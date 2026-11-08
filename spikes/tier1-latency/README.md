# Spike: Tier 1 against its latency protocol

The M0b spike of [`ROADMAP.md`](../../ROADMAP.md): Tier 1 on the generated module of the protocol "Tier-1 re-check after an edit: 99th percentile under 5 ms". A spike lives outside the compiler crates, is tested, and is never cited as evidence for a claim ([`docs/design-brief.md`](../../docs/design-brief.md), "Spikes before the milestone that needs them"); what it finds goes into the register.

## What it builds

A prototype of the path the protocol times, from an edit applied to the source in memory to Tier 1's diagnostics, for the part of M1 the generated module uses:

- **A lexer and parser** for that part of [`spec/grammar.md`](../../spec/grammar.md): functions with refined parameter and result types, `requires` and `ensures`, `val` and `var`, `while` with `invariant` clauses, `if`, `guard`, `return`, `break`, `continue`, calls, indexing and `.len`, with the grammar's precedence and newline rules.
- **The octagon domain over integers** (`src/octagon.rs`): a difference-bound matrix over `+x` and `-x`, kept tightly closed. Adding a constraint costs O(n²): a shortest-path relaxation for the new edge and its coherent twin, then tightening and strengthening. A test checks the closure against brute force over every integer point of a box: it keeps every point, finds emptiness exactly, and gives exact bounds.
- **Tier 1 for one function** (`src/tier1.rs`), as [`spec/core-calculus.md`](../../spec/core-calculus.md) item 8 and [`spec/ghost-contracts-and-totality.md`](../../spec/ghost-contracts-and-totality.md) items 2 and 4 describe it:
  - an obligation at every narrowing point, index, divisor and shift, and at every `requires`, `ensures` and written `invariant`;
  - contracts used modularly: a call checks the callee's `requires` and assumes its `ensures`;
  - loop-head facts by Houdini over candidates from the loop's own text, with no widening;
  - for an obligation it must prove and cannot, a valuation Tier 1 cannot rule out, read from the octagon, and a `guard` suggestion.
- **An incremental module** (`src/session.rs`). An edit re-parses only the function it falls in and re-runs Tier 1 on it. Callers see a function only through its header, so they are re-checked only when the header changes. A second architecture, kept for comparison, re-parses the whole module on every edit and re-checks the functions whose digests changed.
- **The generated module** (`src/generate.rs`) has 200 functions with 20 obligations each: two loops over an array, with indexing and loop bounds, then a call to the previous function on a narrower range, whose `requires` the caller must establish. Edits are random single-token changes inside a function body: an integer literal, a comparison, `+` or `-`, or a variable name.

The prototype is not the compiler: M1's checkpoint (b) builds Tier 1 in `compiler/mente-sema`, informed by what this spike found.

## Running it

The measurement is a benchmark, so it runs on GitHub Actions, in [`.github/workflows/spike-tier1.yml`](../../.github/workflows/spike-tier1.yml), on the three CI platforms, for a pull request that changes the spike, or by hand. The tests run with the workspace's `cargo test`.

```powershell
cargo test -p spike-tier1-latency
cargo run --release -p spike-tier1-latency -- measure
```
