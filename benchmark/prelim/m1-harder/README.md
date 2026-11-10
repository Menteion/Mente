# Preliminary runs of the harder M1 tasks

The third preliminary run of [`benchmark-difficulty`](../../../docs/design-review.md#benchmark-difficulty). The core and the non-core M1 tasks had both saturated ([`../m1`](../m1/README.md), [`../m1-non-core`](../m1-non-core/README.md)), so on 2026-11-10 Norman chose to add six harder public tasks before the first measured run, and to check with a preliminary run that they no longer saturate. It ran twice: the first run found a bug in the harness, and the second ran again the tasks the bug had voided. These runs are not a benchmark result, and they never opened the held-out set.

## How it ran

- **Tasks**: the six harder public M1 solve tasks.
  - Core: `spiral`, `multiply-digits`, `next-permutation`.
  - Non-core: `bucket`, `merge-unique`, `day-of-year`. Their refined signatures need the checker's help.
- **Models**: `claude-sonnet-5-5` and `claude-haiku-5-5`, as in the earlier runs, each with its default thinking and effort, Messages API version `2026-10-21`.
- **What the model saw**: the [language card](../../../docs/language-card.md) of the run's commit as its system prompt, then each task: its description, its signature in Mente and its examples.
- **Repairs**: as in the earlier runs, following [`benchmark/harness/prelim.ts`](../../harness/prelim.ts).
  - After each program, the harness showed the agent what `mn check` reported, or else each example's result on the reference interpreter.
  - The agent could revise the whole function up to 10 times.
  - The hidden tests scored the last program.
- **First run**, [`run-1`](run-1/): workflow "Preliminary run", run 42724081517, on commit `aa216cf`, all six tasks, 5 runs per task and model.
  - It stopped at its US$5 budget after 57 of the 60 runs: three of Haiku's `spiral` runs were never made.
  - The 130 Sonnet calls used 1,518,669 input and 169,879 output tokens; the 129 Haiku calls used 1,614,107 and 190,019.
- **Second run**, [`run-2`](run-2/): run 42730172881, on commit `32f212c`, after the harness fix.
  - `merge-unique`, `next-permutation` and `spiral`, 5 runs per task and model, 30 in all.
  - It cost US$0.37 of a second US$5 budget Norman approved.
  - The 15 Sonnet calls used 59,245 input and 22,710 output tokens; the 15 Haiku calls used 59,245 and 42,764.

Each `<model>-<task>-<run>-<attempt>.mn` holds a program as checked, the matching `.reply.md` holds the reply it came from, and each run's `results.json` records it in full.

## The harness bug, and what the first run voided

- **The bug.** A test that states both a result and what a `mut` argument leaves, `f(mut a) == v leaves w`, was read with `v leaves w` as the expected value. That was then compared with the printed result alone, so every such case failed, however right the program was.
- **The voided tasks.** `merge-unique` and `next-permutation` are the first tasks with such tests. In the first run, every run of both failed every case, Sonnet's included, and the agents spent all 10 repairs chasing a failure that was not there.
- **The cost.** That took US$4.60 of the run's US$4.99, and the budget then stopped the run early.
- **The fix.** The harness now reads such a test as both parts and prints the outcome in the same form (commit "Read a test with both a result and what a mut argument leaves").
- **What counts.** The first run's results for those two tasks are void. Their programs are kept, since each still checks or fails as `mn check` says; the counts below leave them out.

## Result

| Task | `claude-sonnet-5-5` solved | first program | `claude-haiku-5-5` solved | first program |
| :--- | :--- | :--- | :--- | :--- |
| `spiral` (both runs) | 10 of 10 | 10 | 7 of 7 | 7 |
| `multiply-digits` | 5 of 5 | 5 | 5 of 5 | 5 |
| `next-permutation` (second run) | 5 of 5 | 5 | 5 of 5 | 5 |
| `bucket` | 4 of 5 | 4 | 2 of 5 | 2 |
| `merge-unique` (second run) | 5 of 5 | 5 | 5 of 5 | 5 |
| `day-of-year` | 5 of 5 | 5 | 5 of 5 | 3 |
| **All** | **34 of 35** | **34** | **29 of 32** | **27** |

- **`bucket`'s four failures are the agents' own**, each on hidden tests only:
  - Sonnet's third run declared `val width = hi - lo + 1` without a type. It took `i32`, so on the full `i32` range the exact 4294967296 trapped where it landed.
  - Three of Haiku's runs misread the boundary rule: twice a value above `hi` was clamped to `hi` instead of going to the last bucket, and once `x == hi` was put in the last bucket.
- **Two diagnostics in all the valid runs.** Both were Tier 1's `E0502`, on Haiku's `day-of-year`, a result Tier 1 could not bound. Each was fixed in one repair by the `guard` the diagnostic offered.

## Findings

1. **The harder tasks mostly saturate too.** 63 of the 67 valid runs were solved, 61 of them by the first program. Only `bucket` tells the models apart (4 of 5 against 2 of 5), and the first-program share barely does (34 of 35 against 27 of 32). Within M1's small language, these tasks stay within reach of both tiers. Whether to run the first measured benchmark on this corpus, or to wait for M2's tasks to separate the models, is the open question [`benchmark-difficulty`](../../../docs/design-review.md#benchmark-difficulty) is left with.
2. **Agents prove before Tier 1 asks, and never with an invariant.** No program writes a loop `invariant`. Every `merge-unique` program, and most `bucket` and `day-of-year` ones, instead state what Tier 1 cannot prove up front, as the card advises:
   - `guard` with `unreachable`;
   - refined `var` declarations such as `var n: usize[0..=16]`, which let Tier 1 prove the indexes and the count with no run-time check.
   The tasks meant to need the checker's help got it from the card before any diagnostic.
3. **An unannotated `val` takes its operands' type.** In `val width = hi - lo + 1`, the exact value is narrowed into `i32` where it lands, and the run-time check caught a real overflow. But only a hidden test reached it: Tier 1 leaves the narrowing as a run-time check, and nothing at compile time pointed it out. A diagnostic or a lint for a narrowing Tier 1 cannot prove, at a binding whose type was inferred, could catch it before the program runs.
4. **The harness bug cost more than the tasks did.** The two voided tasks used US$4.60 of the US$5.36 the runs spent. A test of the harness on every form a task file allows would have caught it; the fix tests all three forms.

## Limits

- Six tasks, run five times each from the same prompt; many runs drew the same program.
- These are tuning tasks, public as the card is: results on them overstate what agents would do on tasks the card was not tuned on. Only the held-out set, at the first benchmark, can show that.
- Counts of guards, invariants and refined bindings in finding 2 are read from the last programs by a script, not by the harness.
