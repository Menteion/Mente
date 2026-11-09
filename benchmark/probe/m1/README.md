# Syntax probe of M1

The syntax probe of M1's checkpoint (a) ([`syntax-probe`](../../../docs/design-review.md#syntax-probe)), run once, on 2026-11-09. Nothing here is scored, and no result here is a benchmark result.

## How it ran

- **Workflow run**: "Syntax probe", run 42568777641, on commit `9bd58b2`. `results.json` records the run in full.
- **Model**: `claude-sonnet-5-5` with its default thinking and effort, Messages API version `2026-10-21`, and no fallback model.
- **What the model saw**: the draft [language card](../../../docs/language-card.md) of that commit as its system prompt, then each of the 12 public M1 solve tasks. For each task it saw the description, the signature written in Mente, and the examples.
- **Runs**: 3 runs per task, one program per run, with no repairs and no compiler feedback.
- **Checking**: `mn check --json` on each program, with the body parser and the diagnostics for other languages' idioms.
- **Cost**: US$0.37 of the US$5 budget, 36 programs, all ending normally.

Each `<task>-<run>.mn` holds a program as checked, and the matching `.reply.md` holds the model's reply.

## Result

**All 36 programs passed `mn check` with no diagnostic.** No program used another language's idiom (`let`, `&&`, `;`, `as` or any other), and none used a construct of a later milestone. With the card in its context, the model wrote Mente's own spellings throughout: `val` and `var`, `while`, `guard`, `mut` parameters and arguments, refined types, `requires` and `ensures`.

So the probe found no friction in M1's syntax. It asks for no change to the grammar and no new idiom diagnostic. Before checkpoint (a) builds on the syntax, two kinds of finding remain: one in the card, and some in what the parser does not check.

## Findings

1. **The card held a task's answer.** The card's example of contracts was `clamp`, with the same signature as the benchmark task `clamp`, and all three runs of that task copied it word for word, so those runs show nothing. The card now uses examples that no task asks for. Before the card's first version, its examples are checked against the task list.
2. **The card drew agents into measures nobody asked for.** 19 of the 36 programs wrote `decreases` on an ordinary loop, as the card's `count_down` example did. The spec gave a measure a meaning only in a `@total` function. Norman decided [`loop-decreases`](../../../docs/design-review.md#loop-decreases):
   - a written measure is proven like an invariant;
   - Tier 1 gains the facts of `%` and of division by a constant, so measures such as gcd's `decreases y` or a binary search's `decreases hi - lo` are provable;
   - the card no longer shows a measure on an ordinary loop.
3. **M1 had no way to say "this cannot happen".** One `percent` program guarded its refined result, which Tier 1 cannot prove (`part * 100 / whole <= 100`), with `todo("unreachable: part <= whole")`. That program would score as unsolved and fail a release build. Norman decided [`unreachable-marker`](../../../docs/design-review.md#unreachable-marker): a built-in `unreachable("reason")`, which the body parser now reads, and which the card teaches.
4. **What the type checker and Tier 1 will meet**, in programs the parser accepts:
   - **Exact arithmetic did its job.** Programs compared `mid * mid <= n` for `u64` values, and summed eight `i64` values into an `i128` binding. Under exact arithmetic neither overflows, where Rust's `u64` multiplication would.
   - **Indexes of other integer types.** One program indexed an array with a `u32`. `core-calculus.md` now says so explicitly: an index may have any integer type, and is checked against the array's range.
   - **Non-linear refinements need a `guard`.** `percent`'s refined result depends on `part * 100 / whole`, which intervals and octagons cannot bound by `part <= whole`. Tier 1 will report a counterexample and suggest a `guard`, as [`gradual-verification`](../../../docs/design-review.md#gradual-verification) intends.
   - **Refinements on locals.** Several programs refined their loop counters (`var i: usize[1..=8]`) and wrote invariants that the loop-condition candidates of [`loop-condition-candidates`](../../../docs/design-review.md#loop-condition-candidates) would infer. These are proofs the code did not need, which the card's revised guidance discourages.

## Limits

- One model, twelve small tasks, three runs each, and a card in the prompt. Several tasks gave the same program in all three runs.
- The probe sees syntax only. Whether these programs compute the right results waits for the reference interpreter, and whether they pass Tier 1 waits for checkpoint (b). The first agent benchmark, at the end of M1, measures both, with repairs.
