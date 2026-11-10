# The agent benchmark

The agent benchmark measures Mente's goal, the language best suited to AI agents writing code ([`ai-first-purpose`](../docs/design-review.md#ai-first-purpose)), as [`success-workload`](../docs/design-review.md#success-workload) decided. Its protocol is in [`ROADMAP.md`](../ROADMAP.md), under "Agent benchmark". This directory holds the corpus, fixed in M0b before anything is measured ([`benchmark-corpus`](../docs/design-review.md#benchmark-corpus)), and the checker that keeps it well formed. The harness that gives the tasks to agents is written at the end of M1, in Rust or TypeScript, and follows this file.

## Composition

| Milestone | Tuning | Held-out | Adversarial |
| :--- | ---: | ---: | ---: |
| M1 | 18 | 6 | 2 |
| M2 | 9 | 3 | 1 |
| M3 | 3 | 1 | 0 |
| M4 | 3 | 1 | 3 |
| M5 | 3 | 1 | 1 |
| M6 | 3 | 1 | 0 |
| M7 | 3 | 1 | 1 |

- **Tuning** tasks are the public solve tasks in [`tasks/`](tasks/). Work on the language card may study their results. Before the first run, the syntax probe of M1's checkpoint (a) uses the M1 tuning tasks too: one program per run, no repairs and nothing scored, with its programs and findings kept in `probe/` ([`syntax-probe`](../docs/design-review.md#syntax-probe)).
- **Held-out** tasks are solve tasks kept in a private repository, `Menteion/mente-bench-heldout`, so that no session working in this repository can read them. The language card is never tuned on them, and their results are published only as totals. [`corpus.lock`](corpus.lock) names the commit that fixes them, and their milestones and marks.
- **Adversarial** tasks ask for something the language must refuse, such as smuggling a network call into a pure function. They are public.
- A task runs first at the end of the milestone its front matter names, the earliest whose compiler can build a solution, and again at the end of every later milestone, so the M1 tasks give Mente's trend across milestones.
- A task is **core** when its signature uses only the core of the language, and not refinements, contracts, units, fixed-point numbers or explicit layouts ([`ai-first-purpose`](../docs/design-review.md#ai-first-purpose)); core tasks are scored apart from the others. The checker derives the mark from the signature and rejects a task whose front matter disagrees.

## Method

- **Languages**: Mente; Rust, the reference; Odin, the control ([`control-language`](../docs/design-review.md#control-language)), whose agent is given a document of the same size as the language card, written with the card's first version at the end of M1 ([`control/odin.md`](control/odin.md)), which a test keeps within 15% of the card's size; and Go, if the budget allows.
- **Runs**: every task runs 5 times per language. In a run the agent submits a program, and the harness builds it and runs the visible examples; the agent may then revise it, seeing the diagnostics and the examples' results, at most 10 times. The run ends when the agent declares it finished or has used its 10 repairs.
- **Scoring**: a run is solved when its last program builds and passes every example and every hidden test. A program that contains a `todo` scores as unsolved ([`todo-marker`](../docs/design-review.md#todo-marker)); one that contains `unreachable` is scored like any other ([`unreachable-marker`](../docs/design-review.md#unreachable-marker)). The report gives each language's pass rate, the share of runs whose first program passes before any repair, the median repairs and tokens of solved runs, the failures of first attempts and the repairs by failure class ([`philosophy-polish`](../docs/design-review.md#philosophy-polish)), and their variance across the 5 runs, with core tasks apart from the others and the held-out set apart from the tuning set.
- **Failure classes**: the report classifies every program that fails, so that the re-planning triggers of `docs/design-brief.md` section 7.7 are counted, not judged ([`ai-coding-problems`](../docs/design-review.md#ai-coding-problems)). A Mente program that does not build is classed by the failure classes of the codes it reports (`spec/diagnostics.md` item 8); a program in another language that does not build is classed as a build failure. A program that builds but fails an example or a hidden test, or traps, is classed as such. The report counts the classes over the last program of every unsolved run, and over every program the runs submitted.
- **Adversarial tasks** are scored as caught or missed, under the task's own "Caught when" and "Missed when", and a catch at build time is reported apart from a catch when the program runs.

## Task files

A task file is Markdown. Its front matter has these keys:

- `id`: the file's name, which is the function's name with `-` for `_`;
- `milestone`: `M1` to `M9`;
- `core`: `true` or `false`, as above;
- `set`: `public` here, `held-out` in the private repository;
- `kind`: `solve` or `adversarial`;
- `retired`, only on a retired task: the date and the reason ("Changing the corpus").

Its sections are, in order: `## Task`, the request in prose; `## Types`, when the task declares types; `## Signature`; and then, for a solve task, `## Examples` and `## Hidden tests`, or, for an adversarial task, `## Probe`, `## Caught when` and `## Missed when`. Declarations, signatures and tests are indented by four spaces.

The agent sees the task, its types, its signature and its examples, written in its language, and never the hidden tests, nor this repository. It writes the function, and only the function: the harness supplies the program around it and declares the task's records and enums, except a `layout record`, which the agent declares itself, with the fields named as written, since the layout is the part of the task being asked for.

## Signatures and types

Tasks are written in a neutral notation, so that they stay fixed while Mente's syntax evolves. The harness maps it to each language, Mente as the spec stands at the time of the run:

| Neutral | Mente | Rust | Odin |
| :--- | :--- | :--- | :--- |
| `i8` to `i128`, `u8` to `u128`, `usize`, `bool` | the same | the same | the same, `uint` for `usize` |
| `[N]T` | `[N]T` | `[T; N]` | `[N]T` |
| `mut a: T` | `mut a: T` | `a: &mut T` | `a: ^T` |
| `T in a..b`, `T in a..=b` (an integer type) | `T[a..b]`, `T[a..=b]` | `T` | `T` |
| `requires e`, `ensures e` (`result` is the result) | `requires e`, `ensures e` | a doc comment | a doc comment |
| `string` | `str` as a parameter, `String` as a result | `&str`, `String` | `string` |
| `slice<T>` (read-only), `list<T>` (owned) | `[]T`, `List<T>` | `&[T]`, `Vec<T>` | `[]T`, `[dynamic]T` |
| `list<T> layout soa` | a `List<T>` with the `SoA` layout | `Vec<T>` | `#soa[dynamic]T` |
| `option<T>`, `result<T, E>` | `?T`, `T ! E` | `Option<T>`, `Result<T, E>` | `Maybe(T)`, `(T, E)` |
| `f64`, `f64 in unit` | `f64`, `f64` with that unit | `f64` | `f64` |
| `fixed<I, F>` | the fixed-point type of `I` integer and `F` fraction bits | the raw bits, `i32` for `fixed<16, 16>` | the same as Rust |
| `name<T: Eq>(...)` | a generic function with a trait bound | `T: PartialEq + Clone` | `$T` |
| `uses Net, Clock` | `with Net, Clock` | nothing | nothing |

In Rust and Odin, what the neutral signature states beyond its plain types goes into the comment above the function, `///` in Rust and `//` in Odin: each refinement's range, such as `` `n` is in `1..=64` ``, each quantity's unit, each fixed-point number's bits, then the `requires` and `ensures` clauses, with `and`, `or` and `not` written `&&`, `||` and `!`. The harness declares a task's records and enums in the agent's language: a record as a struct, an enum as an enum in Rust, and in Odin an enum whose variants hold no values. An Odin enum whose variants hold values, and a `result` nested in another type in Odin, have no mapping yet, and the harness refuses them until their milestone settles one.

`## Types` declares `record Name { field: T, ... }`, `enum Name { A, B(T, ...) }`, and `layout record Name (N bytes, little-endian | big-endian[, packed]) { ... }`, whose fields may also be bit fields `u1` to `u63` and whose comments give each field's position.

## Values and tests

A test is a call, `name(arguments) == expected`. A call to a function with `mut` parameters adds `leaves`, followed by each `mut` argument's value after the call, and a call to a function that returns nothing has only `leaves`. A generic function names its type arguments: `dedup_sorted<i32>([1, 1])`.

Values are written as: integers in decimal; floats with a point or an exponent, and an expected float as `approx(value, tolerance)`, a relative tolerance; strings in double quotes, with the escapes `\"`, `\\`, `\n`, `\t` and `\r`; `[a, b]` for arrays, slices and lists; `{field: value, ...}` for records; `none`, `some(v)`, `ok(v)` and `err(e)`; an enum variant as `Name` or `Name(fields)`; `q(raw)` for a fixed-point number's raw bits; and `()`.

The harness writes each example in the agent's language: in Mente as it is written here, in Rust as `assert_eq!` lines and in Odin as `assert` lines, with each `mut` argument in a variable named after its parameter. It runs each test through a program around the agent's function, which makes the call and prints the result, then a line `leaves <value>` for each `mut` argument, in the form `mn run` prints: decimal integers, `true` and `false`, `()`, and `[a, b]` for arrays. The programs cover the values M1's tasks use, integers, `bool` and arrays, nested or not; the other values join when their milestone's tasks first run.

## Effects

Tasks that use effects call the harness's stand-ins, which record every call:

- `Net.get(url: string) -> result<string, NetError>` and `Net.post(url: string, body: string) -> result<(), NetError>`, with `enum NetError { Timeout, NotFound, Refused }`;
- `Clock.now_ms() -> u64` and `Clock.sleep_ms(ms: u64)`;
- `Fs.read_text(path: string) -> result<string, IoError>`, with `enum IoError { NotFound, PermissionDenied, InvalidUtf8 }`;
- for `uses FFI`, the C functions a task names.

A test sets the stand-ins up with `given` lines before its call, and checks their calls with `then` lines after it:

- `given Net.get("u") => r1, r2 after 200ms, ...`: successive requests for `u` receive `r1`, `r2` and so on, the last answer repeating; `after` delays an answer. An unknown URL receives `err(NotFound)` at once, a `Net.post` with no `given` receives `ok(())`, and an unknown path `err(NotFound)`.
- `then Net.get calls [...]`, `then Net.get calls in any order [...]`, `then Net.post calls [{url: ..., body: ...}]` and `then Clock.sleep_ms calls [...]` check the recorded calls. `then elapsed under N ms` and `then Net.peak_in_flight at most N`, which concurrency tasks use, are checked by the harness alone.

## Changing the corpus

- Until the first measured run, at the end of M1, a task may be corrected, for a wrong expected value or an ambiguous sentence. The correction updates `corpus.lock` in the same commit, and the commit message gives the reason.
- Until then, tasks may also be added, harder ones where a preliminary run finds the tasks of a milestone too easy to tell models or languages apart ([`benchmark-difficulty`](../docs/design-review.md#benchmark-difficulty)). The composition table, `corpus.lock` and, for a held-out task, the commit it names change in the same commit, and the tasks already there stay, so that they still give the trend across milestones. A held-out task is written in its own repository by a session that works on nothing a benchmark agent is shown.
- From the first measured run on, a task never changes. A task found to be broken is retired: `retired: <date> <reason>` joins its front matter, it stays in the corpus, and it is excluded from every later score, while the scores before keep it. A replacement takes a new `id`, counts in the composition table, and is reported apart until it has as many runs as the tasks it is compared with.
- The held-out set follows the same rules in its own repository; a change to it updates the commit in `corpus.lock`.

## Checking

```powershell
npm run check:benchmark
npm run check:benchmark -- --held-out ..\mente-bench-heldout
npm run check:benchmark -- --bless --held-out ..\mente-bench-heldout
```

[`check.mjs`](check.mjs) checks every task's front matter, sections and core mark, the composition against the table above, every example and hidden test against the signature and against the task's solution in [`reference.mjs`](reference.mjs), and every digest in `corpus.lock`. CI runs the first form, without the held-out set; the second also checks a local clone of the held-out repository against the lock; the third rewrites the lock after a change that "Changing the corpus" allows.

## The harness

[`harness/`](harness/) holds the harness, in TypeScript that Node runs as it is, with no dependency. It reads a task, writes its signature in Mente (`tasks.ts`), Rust and Odin (`languages.ts`), and asks the model through the Messages API over HTTP, recording the model, its parameters, every prompt and reply and the tokens used, and stopping before a call that could pass its budget. `npm run test:harness` tests it without the network.

Its first use is the syntax probe of M1's checkpoint (a) ([`syntax-probe`](../docs/design-review.md#syntax-probe)): [`probe.ts`](harness/probe.ts) gives the model the draft [language card](../docs/language-card.md) and each public M1 solve task, takes one program per run with no repairs, and records what `mn check` reports for it. It runs only by hand, in the workflow "Syntax probe", with the key in the repository's `ANTHROPIC_API_KEY` secret and the budget Norman set. The probe uses `claude-sonnet-5-5`: a weaker model meets more of the syntax's frictions, at less cost. Which model the agent benchmark uses is settled before its first run, at the end of M1. The harness sends no fallback model, so every program comes from the model it names, and a refusal is recorded as one.

Its second use is the preliminary runs of the M1 tasks, the core ones first ([`early-core-benchmark`](../docs/design-review.md#early-core-benchmark)), once the reference interpreter runs M1's slice. [`prelim.ts`](harness/prelim.ts) gives `claude-sonnet-5-5` and `claude-haiku-5-5` the language card and each public core M1 solve task, 5 runs each, with repairs as "Runs" above describes: after each program it shows the agent what `mn check` reports, or each example's result on the reference interpreter (`mn run --call`), and the agent revises the whole function, at most 10 times. Each turn sends the conversation as it was, the model's earlier replies unchanged. A run ends once its program checks and passes every example, since the agent cannot see more; the hidden tests then score its last program, and a `todo` scores it as unsolved. It reports, per run, the repairs, the failure class of each diagnostic and the cost. It runs only by hand, in the workflow "Preliminary run", within the budget Norman set, and it is not a benchmark result. Its `--tasks non-core` option, the workflow's input `tasks`, gives the agent the public M1 solve tasks that are not core instead, which [`benchmark-difficulty`](../docs/design-review.md#benchmark-difficulty) runs once Tier 1 exists.

Its third use is the measured runs, the first at the end of M1 ([`success-workload`](../docs/design-review.md#success-workload)). [`bench.ts`](harness/bench.ts) gives every task of a milestone, the public solve and adversarial tasks and the held-out solve tasks, to each model in Mente, Rust and Odin, 5 runs each, with repairs as "Runs" describes and the prompts of [`prompts.ts`](harness/prompts.ts): the Mente agent's system prompt is the language card, the Odin agent's is `benchmark/control/odin.md`, and the Rust agent's names the language only. A run ends once its program builds and passes every example, or, for an adversarial task, once it builds; the hidden tests then score its last program, and an adversarial task's probe, the calls its `## Probe` names, gives the verdict.

- **Building**: Mente is checked by `mn check` and run on the reference interpreter, as in the preliminary runs. Rust is built by `rustc`, edition 2024, with `-C opt-level=1 -C overflow-checks=on -C debug-assertions=on`, so an overflow panics as in a debug build. Odin is built by `odin build` with its defaults, which keep bounds checks; Odin's integers wrap on overflow, which is the language's definition, so the harness adds no check. [`build.ts`](harness/build.ts) runs each case as its own process, and a panic, a trap or a run past 10 seconds fails it. Every program the harness builds also takes the function at its exact signature, so a program that changes the signature does not build. The processes it starts see no variable whose name looks like a secret.
- **Toolchains**: [`toolchains.json`](harness/toolchains.json) pins Rust's toolchain, run through rustup, and Odin's release with the SHA-256 of its asset, which [`install-toolchains.sh`](harness/install-toolchains.sh) checks before it unpacks it. They are installed in CI only.
- **Fixtures**: [`harness/fixtures/`](harness/fixtures/) holds a Rust and an Odin solution of every public M1 solve task, and of the adversarial tasks with the verdict each should get. The workflow "Harness" builds them through the harness, on every pull request that touches `benchmark/`, and checks that they pass every example and hidden test.
- **Recording**: every call, prompt and reply is kept, and `results.json` records the models with their prices, the toolchains, the build commands, the documents with their digests, the system prompts, each public run with its programs' diagnostics, failure classes and examples, and the totals per set, language and model that "Scoring" lists ([`report.ts`](harness/report.ts)). A failure class is a Mente code's class, a build failure in the other languages, or `wrong result`, `trap` or `timeout`.
- **Budget**: each call reserves the most it can cost before it is sent, so the run stops before any call that could pass the budget. Its calls cache the system prompt and the conversation, which lowers their cost and leaves the prompts as they are. It runs only by hand, in the workflow "Agent benchmark", within the budget Norman sets.
- **Held-out set**: the workflow reads it only when the repository has the secret `MENTE_BENCH_HELDOUT_TOKEN`, a fine-grained token that can read the contents of `Menteion/mente-bench-heldout` and nothing else; it checks out the commit `corpus.lock` names and checks it against the lock. The held-out runs' prompts and programs stay on the runner, the log names a held-out task only by its place in the set, and `results.json` gives the held-out set only as totals.
