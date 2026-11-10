# Tight-loop kernels

M1's performance baselines for Mente's native code, with hardware counters, and the first gaps of the tight-loop benchmark: typical kernels written in Mente, C, C++ and Rust, measured side by side ([`tight-loops`](../docs/design-review.md#tight-loops), [`performance-stage`](../docs/design-review.md#performance-stage)). The protocol is "Tight-loop kernels" under "Measurement protocols" in [ROADMAP.md](../ROADMAP.md).

Until self-hosting is done, a figure about the speed of generated code is a baseline, never a claim ([docs/design-brief.md](../docs/design-brief.md)). Baselines may not regress; the gaps are reported so that later optimisation goes where they are, and are not enforced.

**Benchmarks run on GitHub Actions only, never on a laptop.** The workflow [`.github/workflows/perf.yml`](../.github/workflows/perf.yml) runs them.

## The kernels

Each kernel is one function over integers, `bool` and fixed arrays, in [`kernels/`](kernels): `<name>.mn`, and its equivalents `<name>.c` (C17), `<name>.cpp` (C++17) and `<name>.rs` (Rust 2021). Each takes a work count, builds its input from a fixed linear congruential generator, and returns a checksum, which every language must agree on.

| Kernel | Work | What it exercises | Work count | Checks Tier 1 leaves in Mente |
| :--- | :--- | :--- | ---: | :--- |
| `sum` | sum of 4,096 `u32` into a `u64` | a reduction a C compiler vectorises | 200,000 rounds | the sum's narrowing into `u64` |
| `dot` | dot product of two arrays of 4,096 `u32` | a multiply-add reduction | 200,000 rounds | the sum's narrowing into `u64` |
| `prefix` | running sums of 4,096 `u32` | a loop-carried dependency and a store per element | 150,000 rounds | the running sum's narrowing into `u32` |
| `matmul` | 32 by 32 `u32` matrix product, row by row | nested loops, index arithmetic, a strided load | 15,000 rounds | the accumulator's narrowing into `u32`; the checksum's into `u64` |
| `sieve` | primes below about 262,144, sieve of Eratosthenes | strided stores over 256 KiB of `bool`, beyond the L1 cache | 600 rounds | an index and `p * p`, both outside the inner loop; the count |
| `bsearch` | 4,096 binary searches in 4,096 sorted `u32` | data-dependent, unpredictable branches | 3,000 rounds | the index `a[mid]`; two sums |
| `popcount` | bits set in 4,096 `u64`, by shifts and masks | straight-line bit arithmetic | 25,000 rounds | the first subtraction; the total |
| `isort` | insertion sort of a copy of 256 `u32` | an inner loop with two exits, an array copy | 25,000 rounds | four indexes in the inner loop |
| `collatz` | Collatz steps of every start below a limit | scalar division, multiplication and branches | limit 1,000,000 | `3 * x + 1` and the counters |

The last column is what `mn view --checks` prints for each kernel at the commit of the first result: everything else, every other bound and overflow check, Tier 1 proved and erased.

How the kernels are written:

- **The same work on the same inputs.** The C, C++ and Rust versions follow the Mente one statement for statement, with the same integer types, so that no value wraps in any of them; the Rust version's debug build, with overflow checks, gives the same checksums.
- **Long enough to time.** Each function repeats its work a number of rounds, or works up to a limit, that the run takes a tenth of a second or more in every language.
- **Nothing can be computed once.** After every round one input changes, from the running checksum, or the round's limit does, so no compiler can hoist a round's work out of the loop over rounds, and the checksum depends on every round.
- **No call in a loop.** Each kernel is one function: Mente's compiler does not inline yet, and inlining is a measurement of its own.
- **As the language card teaches.** The Mente code adds no `invariant` or refinement only to erase a check, so what Tier 1 erases on its own is part of the baseline, and the checks it leaves are part of the gap.
- **Inputs at run time.** C, C++ and Rust read the work count from the command line. The Mente executable has its call built in by `mn build --call`, and its `main` makes the call through a function the backend compiles separately and never inlines, so the count is equally unknown to the code.

## How they are measured

[`run.mjs`](run.mjs) builds every kernel in every language, checks that all the checksums agree, and measures:

- **Compilers.** Mente through `mn build`, which compiles through Cranelift at its speed level for the baseline CPU; C and C++ with clang 18 at `-O2`; Rust with the rustc that `rust-toolchain.toml` pins, at `-C opt-level=2`, without overflow checks as a release build has them, and again with `-C overflow-checks=on`, reported beside the others. All target the baseline CPU of their architecture: no `-march=native`.
- **Time.** 11 rounds; each round runs every executable once, starting one further along the list each time, so that drift on the machine falls on every language alike. The figure is the median, with the spread as half the range over the median.
- **Hardware counters.** Every timed run goes through `perf stat` when the runner has it: the wall time is then perf's `duration_time`, and cycles, instructions, branches, branch misses, cache references and cache misses are counted where the platform exposes them. Without perf, the wall time comes from the parent process. The report says which.
- **The gap.** Mente's median time over the fastest of C, C++ and Rust, kernel by kernel. Rust with overflow checks is never counted in it.
- **The regression gate.** The base branch's compiler and the change's build each kernel, at one path, and both executables are measured in the same rounds. The change fails when its median time exceeds the base's by more than 10%, or, where perf counts instructions, its median instruction count exceeds the base's by more than 2%. Byte-identical executables cannot regress, so they always pass, and their ratios show the noise.

The tolerances, and the measurements that justify them, are under "Results" below.

To run the script by hand, on a Linux machine that is not a laptop, with clang 18 and perf:

```bash
cargo build --release --bin mn
node perf/run.mjs --mn target/release/mn --out perf-results.json
node --test perf/run.test.mjs
```

## Results

None yet.
