# Preliminary run of the core M1 tasks

The preliminary run of [`early-core-benchmark`](../../../docs/design-review.md#early-core-benchmark), run once, on 2026-11-09, as soon as the reference interpreter could run M1's slice. It is not a benchmark result, and it never opened the held-out set.

## How it ran

- **Workflow run**: "Preliminary run", run 42595587286, on commit `5192dfa`. `results.json` records the run in full.
- **Models**: `claude-sonnet-5-5`, as in the syntax probe, and the weaker `claude-haiku-5-5`, each with its default thinking and effort, Messages API version `2026-10-21`, and no fallback model.
- **What the model saw**: the [language card](../../../docs/language-card.md) of that commit as its system prompt, then each public core M1 solve task: its description, its signature in Mente and its examples. The 8 tasks are `count-primes-below`, `gcd`, `isqrt`, `max-subarray-sum`, `mean8`, `reverse-bits`, `reverse-in-place` and `sort8`.
- **Runs**: 5 runs per task and model, 80 in all, following [`benchmark/harness/prelim.ts`](../../harness/prelim.ts).
  - After each program, the harness showed the agent what `mn check` reported, or else each example's result on the reference interpreter.
  - The agent could revise the whole function up to 10 times.
  - A run ended once its program checked and passed every example. The hidden tests then scored the last program.
- **Cost**: US$0.49 of the US$5 budget. The 40 Sonnet calls used 139,455 input and 17,966 output tokens; the 41 Haiku calls used 144,247 and 32,329.

Each `<model>-<task>-<run>-<attempt>.mn` holds a program as checked, and the matching `.reply.md` holds the reply it came from.

## Result

**All 80 runs were solved: every last program checked, passed every example and every hidden test, and held no `todo`.**

| Model | Solved | First attempt failed | Repairs |
| :--- | :--- | :--- | :--- |
| `claude-sonnet-5-5` | 40 of 40 | 0 | 0 |
| `claude-haiku-5-5` | 40 of 40 | 1 | 1 |

- No program, in any attempt, drew a single diagnostic from `mn check`, the strict rules of [`strict-defaults`](../../../docs/design-review.md#strict-defaults) included.
- The one failure was Haiku's third `isqrt` run. Its first program checked, but returned 1 for `isqrt(16)`. Shown the failing examples, Haiku fixed it in one repair.
- The interpreter checked every refinement and contract the programs wrote at run time ([`interpreter-checks-contracts`](../../../docs/design-review.md#interpreter-checks-contracts)); none failed.

## Findings

1. **An agent can write correct Mente from the card alone, on these tasks, at both tiers tried.** The weaker model matched the stronger one. This is the bet behind [`language-card`](../../../docs/design-review.md#language-card), and the run is the first evidence for it beyond syntax.
2. **The tasks are saturated, so the run cannot tell more.** With 1 failure in 80 runs, it cannot separate the models, and it says nothing about repairs. The research comparison warned of this trap in VeraBench, where most models score near 100%. The first agent benchmark needs tasks hard enough to fail sometimes: longer functions, several functions per task, and tasks that need the checker's help.
3. **The strictness bet went unmeasured.** No program drew a diagnostic, so the `Hypothesis` that strict rules pay off for agents (`philosophy.md`, section 3) got no data either way. It needs tasks on which agents make the mistakes the strict rules catch.
4. **These tasks are tuning tasks.** The syntax probe ran on them and the card was revised after it, so results on them overstate what agents would do on tasks the card was not tuned on. Only the held-out set, at the first benchmark, can show that.

## Limits

- Eight small, classic algorithms. A model may know each by heart in other languages, and only has to spell it in Mente.
- Five runs per task, each from the same prompt; several tasks drew the same program in every run.
- Core tasks only: no refinements or contracts were asked for, and Tier 1 does not exist yet.
