# The agent benchmark at the end of M1

The first measured run of the agent benchmark ([`benchmark/README.md`](../../README.md); the protocol under "Agent benchmark" in [`ROADMAP.md`](../../../ROADMAP.md)): each M1 task given to an agent in Mente, in Rust, the reference, and in Odin, the control language. The tasks were fixed before it, and from it on they never change.

## How it ran

- **Workflow**: "Agent benchmark", run 42754141549, on commit `6305c99`, 2026-11-10; the result recorded for tag `m1`. `results.json` records the run in full, the held-out set as totals only.
- **Tasks**: the 20 public M1 tasks (18 solve tasks, 7 of them not core, and 2 adversarial ones), and the 6 held-out M1 solve tasks, read from the private repository at the commit `corpus.lock` names.
- **Models**: `claude-sonnet-5-5` and `claude-haiku-5-5`, each with its default thinking and effort, Messages API version `2026-10-21`, no fallback model, prompt caching on the system prompt.
- **Languages and what each agent saw**:
  - The Mente agent got the [language card](../../../docs/language-card.md)'s first version as its system prompt.
  - The Odin agent got [`benchmark/control/odin.md`](../../control/odin.md), a document of the same size.
  - The Rust agent got no document.
  - Each then saw the task, its signature and its examples, written in its language.
- **Runs**: 5 runs per task, language and model, 780 in all.
  - After each program, the agent saw what building reported, or else each example's result, and could revise its program up to 10 times.
  - The hidden tests scored the last program.
  - Mente programs ran on the reference interpreter.
  - Rust built with overflow checks on, and Odin with its defaults: bounds checks on, integers wrapping.
- **Sandbox**: every agent build and program ran without network, without the API key, and unable to see the workspace or another run's folder ([`benchmark/harness/sandbox.sh`](../../harness/sandbox.sh)).
- **Cost**: US$2.05 of a US$50 budget. A run of the public tasks alone, made before the held-out set could be read (run 42750810167, US$1.53), gave figures within a run or two of these.

[`replies/`](replies/) holds every reply of the public runs, named `<task>-<language>-<model>-<run>-<attempt>.reply.md`; each holds the program the agent submitted.

## Results

### Solve tasks

| Set | Language | Model | Solved | First program passes | Not core: solved / first program |
| :--- | :--- | :--- | :--- | :--- | :--- |
| public | Mente | Sonnet | 90 of 90 (100%) | 94% | 100% / 86% |
| public | Mente | Haiku | 86 of 90 (96%) | 88% | 89% / 69% |
| public | Odin | Sonnet | 90 of 90 (100%) | 100% | 100% / 100% |
| public | Odin | Haiku | 86 of 90 (96%) | 96% | 89% / 89% |
| public | Rust | Sonnet | 89 of 90 (99%) | 99% | 97% / 97% |
| public | Rust | Haiku | 85 of 90 (94%) | 94% | 86% / 86% |
| held-out | Mente | Sonnet | 30 of 30 (100%) | 97% | 100% / 100% |
| held-out | Mente | Haiku | 30 of 30 (100%) | 97% | 100% / 90% |
| held-out | Odin | either | 30 of 30 (100%) | 100% | 100% / 100% |
| held-out | Rust | either | 30 of 30 (100%) | 100% | 100% / 100% |

- Every core task was solved in every run, in every language, by both models.
- Every unsolved run is of one task, `bucket`:
  - Rust: 5 of Haiku's runs and 1 of Sonnet's;
  - Odin: 4 of Haiku's;
  - Mente: 4 of Haiku's.
  - Each last program built and gave a wrong result on a hidden test: a boundary misread, or an intermediate that overflowed.
- **Mente's first programs failed more often than the others'**, because a refinement or contract Tier 1 could not prove is an error when the program is built (failure class "proof", `E0502`). Each was repaired by the `guard` or refined binding the diagnostic offered: all 5 of Haiku's `index-of-max` runs and all 5 of Sonnet's `percent` runs took one repair. One first program used another language's idiom (failure class "foreign idiom"). Odin and Rust have no such errors to give, so their first programs fail only by giving wrong results.

### Adversarial tasks

Each asks for something the language must refuse. A run is caught when the program is refused at build time or traps when the probe calls it, and missed otherwise.

| Task | Mente | Odin | Rust |
| :--- | :--- | :--- | :--- |
| `step-up`, an increment that overflows `u8` | 9 of 10 caught, at run time | 8 of 10, at run time | 0 of 10 |
| `to-byte`, a narrowing that loses a value | 4 of 10, at run time | 2 of 10, at run time | 0 of 10 |

None was caught at build time. In Mente a value that leaves its type traps where it lands. The runs that missed wrote code that never let the value leave its type: a saturating or clamping reading of the request.

## Findings

1. **Against the control language, Mente solves as much, with more of its errors caught while building.** On public tasks the two solve the same number with each model: 90 with Sonnet, 86 with Haiku. Mente's first programs fail more often only because Tier 1 rejects at build time what it cannot prove, and every such rejection was repaired by the fix the diagnostic offered. This is the primary comparison the protocol names.
2. **Mente catches more adversarial programs than either language**, all at run time: 13 of 20, against Odin's 10 and Rust's 0. Rust's overflow checks would have trapped too, but no Rust run wrote the arithmetic that overflows.
3. **The solve tasks saturate, the held-out set included.** Every language and model solved at least 94%. M1's language is small, and its tasks stay within reach of both tiers, as the preliminary runs foresaw ([`benchmark-difficulty`](../../../docs/design-review.md#benchmark-difficulty)). These tasks give Mente's trend across milestones from here; M2's tasks are expected to separate more.
4. **Tokens cannot be compared across languages here.** The median solved run used about 4,300 to 5,100 tokens in Mente, 3,700 to 4,000 in Odin, and 550 to 920 in Rust. That is mostly the document each prompt carries, the card or the Odin document, which Rust's does not.
5. **The gap to Rust carries the training-data confound**, which the protocol names: models have seen far more Rust than Mente or Odin. Here the gap is within a few runs either way.

## Limits

- Five runs per task, language and model. The difference between this run and the public-only one before it, up to two runs per row, shows how far a single run moves.
- Twenty-six M1 tasks, most of them solved by every agent. The counts that differ rest on one task, `bucket`, and on the two adversarial tasks.
- The public tasks are tuning tasks, which the card was revised after; the held-out set, which it never saw, saturated too, so it cannot show the difference.
