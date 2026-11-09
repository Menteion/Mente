# Preliminary run of the non-core M1 tasks

The second preliminary run of [`benchmark-difficulty`](../../../docs/design-review.md#benchmark-difficulty), run once, on 2026-11-09, as soon as Tier 1 existed: the core M1 tasks had saturated ([`../m1`](../m1/README.md)), and refinements and contracts are what the checker and Tier 1 make agents work for. It is not a benchmark result, and it never opened the held-out set.

## How it ran

- **Workflow run**: "Preliminary run", run 42648013994, with `tasks: non-core`, on commit `7d8882b`. `results.json` records the run in full.
- **Models**: `claude-sonnet-5-5` and `claude-haiku-5-5`, as in the first run, each with its default thinking and effort, Messages API version `2026-10-21`, and no fallback model.
- **What the model saw**: the [language card](../../../docs/language-card.md) of that commit as its system prompt, then each public M1 solve task that is not core: its description, its signature in Mente and its examples. The 4 tasks are `clamp`, `index-of-max`, `lower-bound` and `percent`. Their signatures carry a refined result or a `requires`, so a program must satisfy Tier 1 to check.
- **Runs**: 5 runs per task and model, 40 in all, following [`benchmark/harness/prelim.ts`](../../harness/prelim.ts), as in the first run: after each program the harness showed the agent what `mn check` reported, or else each example's result on the reference interpreter; the agent could revise the whole function up to 10 times; a run ended once its program checked and passed every example, and the hidden tests then scored the last program.
- **Cost**: US$0.23 of the US$5 budget. The 20 Sonnet calls used 70,730 input and 6,861 output tokens; the 22 Haiku calls used 79,167 and 17,477.

Each `<model>-<task>-<run>-<attempt>.mn` holds a program as checked, and the matching `.reply.md` holds the reply it came from.

## Result

**All 40 runs were solved: every last program checked, passed every example and every hidden test, and held no `todo`.**

| Model | Solved | First attempt failed | Repairs |
| :--- | :--- | :--- | :--- |
| `claude-sonnet-5-5` | 20 of 20 | 0 | 0 |
| `claude-haiku-5-5` | 20 of 20 | 2 | 2 |

- Both failures were Tier 1's `E0502`, a refinement it could not prove, in Haiku's second `index-of-max` run and fifth `percent` run. Each was fixed in one repair, by the `guard` the diagnostic offered, with `unreachable` in place of the `todo` the suggestion holds:
  ```text
  12:3 E0502 Tier 1 cannot rule out `best = 8, i = 8`, outside `usize[0..8]`, the type of the result of `index_of_max` (fix: insert `guard best < 8 else { todo("best < 8 is false here") }`)
  ```
- No other diagnostic was drawn, and no example or hidden test failed.

## Findings

1. **The non-core tasks are saturated too.** With 2 failures in 40 runs, all repaired at once, the run cannot separate the models or measure repairs. Both preliminary runs together leave the next step of [`benchmark-difficulty`](../../../docs/design-review.md#benchmark-difficulty): harder tasks.
2. **Tier 1's diagnostics led to a fix in one repair, both times.** The valuation and the `guard` prescription of Guardrail #1 are the first repair loop the preliminary runs observed on a proof failure.
3. **Agents state what Tier 1 cannot prove with a `guard`, mostly before any diagnostic.** 14 of the 40 last programs hold a `guard` whose only role is to establish a fact Tier 1 cannot prove: all 10 `percent` programs, three of Haiku's `lower-bound` programs, which search by halving, and one of Haiku's `index-of-max` programs. Only 2 of the 14 answer a diagnostic; the others follow the card's advice. 11 end in `unreachable`, which traps if the fact ever fails; 3 return an invented value instead (`return 16`, `return 100`), which would hide the error a trap reports.
4. **Where a guard was taken, a stricter fix existed for 4 of the 14.** The `index-of-max` and halving `lower-bound` programs that needed a guard read a `var` declared without a refinement (`best: usize`, `hi: usize`). Declared as `usize[0..8]` and `usize[0..=16]`, the same programs check with no guard: Tier 1 proves the result, so no run-time check is left. 9 of the 10 `index-of-max` programs declared `best` that way unprompted. The diagnostic offers only the guard, so an agent that follows it keeps a run-time check that a proof could remove. Proposed in [`refine-binding-fix`](../../../docs/design-review.md#refine-binding-fix).
5. **`percent` stays beyond Tier 1.** Its result rests on `part * 100 / whole`, a product and a quotient that octagons cannot bound, as the syntax probe foresaw. Every program guards it; only `mn prove` (Tier 2) could remove the guard.

## Limits

- Four small tasks, run five times each from the same prompt; many runs drew the same program.
- These are tuning tasks: the syntax probe ran on them and the card was revised after it, so results on them overstate what agents would do on tasks the card was not tuned on. Only the held-out set, at the first benchmark, can show that.
- The count of guards in finding 3 is read from the last programs by hand, not by the harness.
