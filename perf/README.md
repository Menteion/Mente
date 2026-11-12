# Tight-loop kernels

M1's performance baselines for Mente's native code, with hardware counters, and the first gaps of the tight-loop benchmark: typical kernels written in Mente, C, C++ and Rust, measured side by side ([`tight-loops`](../docs/design-review.md#tight-loops), [`performance-stage`](../docs/design-review.md#performance-stage)). The protocol is "Tight-loop kernels" under "Measurement protocols" in [ROADMAP.md](../ROADMAP.md).

Until self-hosting is done, a figure about the speed of generated code is a baseline, never a claim ([docs/design-brief.md](../docs/design-brief.md)). Baselines may not regress; the gaps are reported so that later optimisation goes where they are, and are not enforced.

**Benchmarks run on GitHub Actions only, never on a laptop.** The workflow [`.github/workflows/perf.yml`](../.github/workflows/perf.yml) runs them.

## The kernels

Each kernel is one function over integers, `bool` and fixed arrays, in [`kernels/`](kernels): `<name>.mn`, and its equivalents `<name>.c` (C17), `<name>.cpp` (C++17) and `<name>.rs` (Rust 2021). Each takes a work count, builds its input from a fixed linear congruential generator, and returns a checksum, which every language must agree on.

| Kernel | Work | What it exercises | Work count | Checks Tier 1 leaves in Mente |
| :--- | :--- | :--- | ---: | :--- |
| `sum` | sum of 4,096 `u32` into a `u64` | a reduction a C compiler vectorises | 300,000 rounds | the sum's narrowing into `u64` |
| `dot` | dot product of two arrays of 4,096 `u32` | a multiply-add reduction | 200,000 rounds | the sum's narrowing into `u64` |
| `prefix` | running sums of 4,096 `u32` | a loop-carried dependency and a store per element | 150,000 rounds | the running sum's narrowing into `u32` |
| `matmul` | 32 by 32 `u32` matrix product, row by row | nested loops, index arithmetic, a strided load | 15,000 rounds | the accumulator's narrowing into `u32`; the checksum's into `u64` |
| `sieve` | primes below about 262,144, sieve of Eratosthenes | strided stores over 256 KiB of `bool`, beyond the L1 cache | 600 rounds | an index and `p * p`, both outside the inner loop; the count |
| `bsearch` | 4,096 binary searches in 4,096 sorted `u32` | data-dependent, unpredictable branches | 400 rounds | the index `a[mid]`; two sums |
| `popcount` | bits set in 4,096 `u64`, by shifts and masks | straight-line bit arithmetic | 35,000 rounds | the first subtraction; the total |
| `isort` | insertion sort of a copy of 256 `u32` | an inner loop with two exits, an array copy | 25,000 rounds | four indexes in the inner loop |
| `collatz` | Collatz steps of every start below a limit | scalar division, multiplication and branches | limit 1,200,000 | `3 * x + 1` and the counters |

The last column is what `mn view --checks` prints for each kernel at the commit of the first result: everything else, every other bound and overflow check, Tier 1 proved and erased.

How the kernels are written:

- **The same work on the same inputs.** The C, C++ and Rust versions follow the Mente one statement for statement, with the same integer types, so that no value wraps in any of them: the Rust version built with overflow checks, which runs beside the others, would stop on the first that did.
- **Long enough to time.** Each function repeats its work a number of rounds, or works up to a limit, so that the fastest language's run takes about a tenth of a second or more. `bsearch` is the exception: its count keeps Mente's run within the workflow's time, for the reason under "Where the gap comes from", so its C run lasts only 30 to 60 ms.
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
- **The regression gate.** The base branch's compiler and the change's build each kernel, at one path, and both executables are measured in the same rounds. The change fails when its median instruction count exceeds the base's by more than 1%; a median time above the base's by more than 5% is reported in the run's summary and does not fail it, since shared runners' times vary from run to run. Instructions are counted on AArch64 alone, where the job runs `perf/run.mjs --require-instructions`, so a run there that counted none fails instead of passing on nothing; on x86-64, whose runner counts nothing, the job checks the kernels' checksums and reports the times. Byte-identical executables cannot regress, so they pass the comparison, and their ratios show the noise. Both jobs are required checks of `main`.

The tolerances, and the measurements that justify them, are under "Results" below.

To run the script by hand, on a Linux machine that is not a laptop, with clang 18 and perf:

```bash
cargo build --release --bin mn
node perf/run.mjs --mn target/release/mn --out perf-results.json
node --test perf/run.test.mjs
```

## Where the gap comes from

What the disassembly of the x86-64 code `mn build` writes shows, at the commit of the first result (examined in an object built on Windows, whose code differs from Linux's only in the calling convention), together with the AArch64 counters below. These are observations to direct later work, not decisions: [`tight-loops`](../docs/design-review.md#tight-loops) is still proposed.

- **`bsearch`'s midpoint was a call of the bit-serial 128-bit division.** `(lo + hi) / 2` of two `usize` is an exact intermediate that may need 65 bits, so it is computed in 128 bits, and its division by 2 called the generated runtime's 128-step shift-and-subtract loop, about 6,000 instructions, on every step of every search. Since commit "Divide 128-bit values without the bit-serial routine where possible", a division by a constant power of two is a shift and a mask, and the runtime divides with one 64-bit division when both operands fit in 64 bits and with three when the divisor fits in 32. `bsearch` went from 12,098 ms to 159 ms on x86-64 and from 9,110 ms to 178 ms on AArch64 ("The gate's first comparison" below). Its work count stays at 400 rounds for now, so its C run is still short.
- **Every `var` lives in memory.** Each loop variable is a stack slot that every iteration loads and stores; Cranelift does not promote stack slots to registers.
- **Exact intermediates in 128 bits.** A `u64` sum such as `total += a[j]` is computed as a pair of 64-bit halves and range-checked with a compare-and-select sequence before it is stored.
- **No vectorisation.** Cranelift has no auto-vectoriser, and `@simd` comes in M5. The references execute about one instruction per element of `sum` and `dot` on AArch64, as code vectorised with the baseline's NEON does; Mente executes about 37 and 43.
- **The checks are the smaller part.** Rust with overflow checks costs between nothing (`isort`) and five times (`sum` on AArch64, where the checks stop vectorisation) against Rust without, while Mente trails the fastest reference by 3 to 31 times outside `bsearch`. Most of the gap is the code around the checks.

## Results

### The gate's first comparison, and its one exception

2026-11-10, PR #69 (commit "Build the base compiler apart from the change in the Performance job", `83e19ff`) against `main` at `6305c99`, workflow run 42752671802: the first run in which the base's compiler and the change's were built apart, so the first that compared two compilers. x86-64: an AMD EPYC 7763; AArch64: a Neoverse-N2. Change over base, in median time and, on AArch64, in instructions:

| Kernel | x86-64 time | AArch64 time | AArch64 instructions | Gap now, x86-64 | Gap now, AArch64 |
| :--- | ---: | ---: | ---: | ---: | ---: |
| `sum` | 0.926 | 0.940 | 0.958 | 18.5× | 29.5× |
| `dot` | 0.801 | 1.002 | **1.033** | 9.5× | 18.1× |
| `prefix` | 0.828 | 0.938 | 0.963 | 4.5× | 5.4× |
| `matmul` | 0.835 | 0.979 | 0.920 | 5.3× | 7.6× |
| `sieve` | 1.002 | 1.004 | 1.000 | 6.9× | 9.4× |
| `bsearch` | 0.013 | 0.020 | 0.011 | 2.2× | 6.3× |
| `popcount` | 0.919 | 0.982 | 0.968 | 5.9× | 4.9× |
| `isort` | 0.989 | 0.987 | 0.925 | 9.6× | 10.1× |
| `collatz` | 0.920 | 0.995 | 1.000 | 3.3× | 4.7× |

The kernels other than `bsearch` gained too, since their seeds and checksums divide by constants (`% 2147483648` is now a shift).

**The exception.** `dot` on AArch64 executed 3.3% more instructions than the base, at the same time, beyond the gate's 1%, so the gate failed. Its hot loop, `total += a[j] * b[j]`, divides nothing; the change rewrote only its setup's divisions and the one of each round. Built with both compilers for x86-64 and disassembled, `dot`'s function shows its registers and stack slots shifted throughout, not only at the divisions: Cranelift's register allocator answered the different code elsewhere in the function, and the loop landed on a slightly different allocation. Norman accepted the regression on 2026-11-10, and the change was merged with the gate failing for this kernel alone. The tolerances stay as they are, and this change is the base every later comparison measures against.

### First result

2026-11-10, commit "Recalibrate the kernels' work counts and the gate's tolerances" (`666ec4b`, PR #65) against `main` at `8898ec2`, workflow run 42744696016 on GitHub's hosted runners. Both runners: Ubuntu 24.04.5, kernel 7.0.0-1017-azure, 4 cores; clang 18.1.3; rustc 1.97.1 with LLVM 22.1.6; perf 7.0.14.

Median wall time of Mente, of the fastest of C, C++ and Rust, and the gap between them:

| Kernel | x86-64, AMD EPYC 9V45: Mente | fastest | gap | AArch64, Neoverse-N2: Mente | fastest | gap |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| `sum` | 1,288 ms | 148 ms, Rust | 8.7× | 2,935 ms | 93.7 ms, C | 31.3× |
| `dot` | 1,005 ms | 116 ms, Rust | 8.7× | 2,215 ms | 123 ms, C | 18.1× |
| `prefix` | 584 ms | 151 ms, C | 3.9× | 1,591 ms | 278 ms, C | 5.7× |
| `matmul` | 576 ms | 97.8 ms, Rust | 5.9× | 1,260 ms | 162 ms, Rust | 7.8× |
| `sieve` | 586 ms | 113 ms, C | 5.2× | 1,353 ms | 146 ms, Rust | 9.3× |
| `bsearch` | 6,917 ms | 57.3 ms, C | 120.7× | 9,113 ms | 28.6 ms, C | 318.2× |
| `popcount` | 556 ms | 103 ms, C | 5.4× | 954 ms | 192 ms, C | 5.0× |
| `isort` | 1,004 ms | 143 ms, Rust | 7.0× | 1,691 ms | 165 ms, Rust | 10.2× |
| `collatz` | 393 ms | 125 ms, Rust | 3.1× | 538 ms | 114 ms, Rust | 4.7× |

Rust with overflow checks took 1.0 to 2.2 times as long as Rust without on x86-64, and 1.0 to 5.2 times on AArch64, the most on `sum`, whose checks stop vectorisation.

**Hardware counters.** The AArch64 runner counted every event asked for. The x86-64 runner counted none: with `perf_event_paranoid` at -1, `perf stat` answers "The cycles event is not supported", since the virtual machine exposes no performance-monitoring unit, so on x86-64 the baselines are wall time alone. On AArch64, against the fastest reference by time:

| Kernel | Mente: instructions | IPC | branch misses | Fastest: instructions | IPC | branch misses |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| `sum` | 44.97 G | 4.51 | 544 k | 1.39 G | 4.42 | 308 k |
| `dot` | 34.93 G | 4.65 | 300 k | 1.13 G | 2.74 | 209 k |
| `prefix` | 26.19 G | 4.85 | 315 k | 3.69 G | 3.92 | 160 k |
| `matmul` | 20.90 G | 4.89 | 641 k | 2.42 G | 4.44 | 508 k |
| `sieve` | 16.58 G | 3.61 | 14.0 M | 0.96 G | 1.96 | 79 k |
| `bsearch` | 125.24 G | 4.05 | 11.2 M | 0.18 G | 1.92 | 144 k |
| `popcount` | 13.93 G | 4.30 | 82 k | 1.19 G | 1.83 | 44 k |
| `isort` | 24.06 G | 4.20 | 6.1 M | 2.52 G | 4.52 | 3.3 M |
| `collatz` | 5.48 G | 3.01 | 27.1 M | 1.13 G | 2.95 | 1.1 M |

Mente's code runs at a high IPC but executes 5 to 32 times the instructions outside `bsearch`, which is the gap above. `sieve` and `collatz` also take far more branch misses than the references. Cache misses are alike where the data outgrows the cache: `sieve` misses 87.9 M times in Mente and 86.6 M in each reference.

**Noise, and the gate's tolerances.** In both runs so far the base's and the change's compilers built byte-identical executables, so their comparison measured the noise alone. That was the workflow's doing, not the change's: both compilers were built into one target directory, where cargo took the base's fresh build for the change's and reused it, so the job measured one compiler twice (fixed by commit "Build the base compiler apart from the change in the Performance job"). The figures below still measure noise: one executable against itself. Their medians differed by at most 0.9% on x86-64 and 0.2% on AArch64, and their instruction counts agreed to five digits. A single executable's rounds spread by up to ±9% on x86-64, which is why the gate compares medians of interleaved rounds. The time tolerance, 5%, is five times the largest difference seen; the instruction tolerance, 1%, is far above the variation of a count that agreed to five digits. The figures above are for one CPU: the first run (commit "Measure tight-loop kernels against C, C++ and Rust on Actions", `6e81fe8`, with the earlier work counts) drew an AMD EPYC 9V74 for x86-64 instead, and its gaps were 18.7× for `sum`, 11.7× for `dot`, 5.5× for `prefix`, 7.0× for `matmul`, 6.5× for `sieve`, 542.7× for `bsearch`, 6.3× for `popcount`, 8.5× for `isort` and 3.3× for `collatz`. Hosted runners do not promise a CPU, which is why the gate only compares two compilers on one runner, and a gap is always quoted with its CPU.

**Cost.** A run took 9 minutes on x86-64 and 13 on AArch64, the two in parallel, building both compilers and measuring for 7 and 10 minutes.
