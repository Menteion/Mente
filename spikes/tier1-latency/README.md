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

## Result

On 2026-11-09 (tag `m0b`), in the workflow run for pull request 19, on GitHub's runners (4 vCPUs each): an AMD EPYC 7763 under Ubuntu 24.04 and under Windows Server 2025, and an Arm Neoverse-N2 under Ubuntu 24.04. The generated module has 200 functions, 105,649 bytes and 4,000 obligations, 2,200 of which must be proven; Tier 1 proves every one of those, and leaves 200 as run-time checks (the additions to an `i64` sum). Of the 1,000 timed edits, 471 made an obligation fail that Tier 1 must prove, 25 broke the parse, and 69 caused another diagnostic.

| Runner | Re-check after an edit, one function re-parsed: median / 99th percentile | Whole module re-parsed instead: median / 99th percentile | Cold check of the whole module |
| :--- | :--- | :--- | :--- |
| x86-64 Linux | 272 µs / 332 µs | 4.9 ms / 5.3 ms | 53 ms |
| x86-64 Windows | 305 µs / 479 µs | 7.4 ms / 13.4 ms | 63 ms |
| AArch64 Linux | 249 µs / 288 µs | 4.1 ms / 4.2 ms | 48 ms |

## Findings

1. **The target holds with room to spare when the re-check is per function.** Re-parsing and re-checking the edited function alone keeps the 99th percentile between 0.29 and 0.48 ms, a tenth of the 5 ms target, on every platform. Tier 1 itself costs about 0.25 ms for a function of 20 obligations, two loops and a call.
2. **Re-parsing the whole module on every edit misses the target.** Its 99th percentile is 4.2 to 13.4 ms, from just under the target to almost three times it, and it grows with the module. M1's incremental architecture therefore parses and checks per item, with each item's spans relative to its start, so that an edit shifts no other item's spans or diagnostics (the prototype shifts only the item boundaries).
3. **A counting loop needs a written invariant to bound its counter after the loop.** At a loop head Tier 1 keeps only the candidates that are inductive ([`tier1-loops`](../../docs/design-review.md#tier1-loops)). The condition `i < n` of the most common loop is not: it fails on the last iteration. So after `while i < n { i += 1 }` nothing bounds `i` by `n` unless the loop says `invariant i <= n`. Norman decided [`loop-condition-candidates`](../../docs/design-review.md#loop-condition-candidates) on 2026-11-09: the condition also proposes the non-strict form of each strict comparison in it, and the prototype now does so.
4. **Whether an unannotated binding takes a refinement is not said.** With `lo: usize[0..64]`, `val m = lo + 1` could have the type `usize[0..64]`, which `lo = 63` breaks, or `usize`. The generated module states every type; Norman decided [`refinement-inference`](../../docs/design-review.md#refinement-inference) on 2026-11-09: a binding takes a refined type only when it states one.
