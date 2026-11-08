# The agent benchmark

The agent benchmark measures Mente's goal, the language best suited to AI agents writing code ([`ai-first-purpose`](../docs/design-review.md#ai-first-purpose)), as [`success-workload`](../docs/design-review.md#success-workload) decided. Its protocol is in [`ROADMAP.md`](../ROADMAP.md), under "Agent benchmark". This directory holds the corpus, fixed in M0b before anything is measured ([`benchmark-corpus`](../docs/design-review.md#benchmark-corpus)), and the checker that keeps it well formed. The harness that gives the tasks to agents is written at the end of M1, in Rust or TypeScript, and follows this file.

## Composition

| Milestone | Tuning | Held-out | Adversarial |
| :--- | ---: | ---: | ---: |
| M1 | 12 | 4 | 2 |
| M2 | 9 | 3 | 1 |
| M3 | 3 | 1 | 0 |
| M4 | 3 | 1 | 3 |
| M5 | 3 | 1 | 1 |
| M6 | 3 | 1 | 0 |
| M7 | 3 | 1 | 1 |

- **Tuning** tasks are the public solve tasks in [`tasks/`](tasks/). Work on the language card may study their results.
- **Held-out** tasks are solve tasks kept in a private repository, `Menteion/mente-bench-heldout`, so that no session working in this repository can read them. The language card is never tuned on them, and their results are published only as totals. [`corpus.lock`](corpus.lock) names the commit that fixes them, and their milestones and marks.
- **Adversarial** tasks ask for something the language must refuse, such as smuggling a network call into a pure function. They are public.
- A task runs first at the end of the milestone its front matter names, the earliest whose compiler can build a solution, and again at the end of every later milestone, so the M1 tasks give Mente's trend across milestones.
- A task is **core** when its signature uses only the core of the language, and not refinements, contracts, units, fixed-point numbers or explicit layouts ([`ai-first-purpose`](../docs/design-review.md#ai-first-purpose)); core tasks are scored apart from the others. The checker derives the mark from the signature and rejects a task whose front matter disagrees.

## Method

- **Languages**: Mente; Rust, the reference; Odin, the control ([`control-language`](../docs/design-review.md#control-language)), whose agent is given a document of the same size as the language card, written with the card's first version at the end of M1; and Go, if the budget allows.
- **Runs**: every task runs 5 times per language. In a run the agent submits a program, and the harness builds it and runs the visible examples; the agent may then revise it, seeing the diagnostics and the examples' results, at most 10 times. The run ends when the agent declares it finished or has used its 10 repairs.
- **Scoring**: a run is solved when its last program builds and passes every example and every hidden test. A program that contains a `todo` scores as unsolved ([`todo-marker`](../docs/design-review.md#todo-marker)). The report gives each language's pass rate, the median repairs and tokens of solved runs, and their variance across the 5 runs, with core tasks apart from the others and the held-out set apart from the tuning set.
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

`## Types` declares `record Name { field: T, ... }`, `enum Name { A, B(T, ...) }`, and `layout record Name (N bytes, little-endian | big-endian[, packed]) { ... }`, whose fields may also be bit fields `u1` to `u63` and whose comments give each field's position.

## Values and tests

A test is a call, `name(arguments) == expected`. A call to a function with `mut` parameters adds `leaves`, followed by each `mut` argument's value after the call, and a call to a function that returns nothing has only `leaves`. A generic function names its type arguments: `dedup_sorted<i32>([1, 1])`.

Values are written as: integers in decimal; floats with a point or an exponent, and an expected float as `approx(value, tolerance)`, a relative tolerance; strings in double quotes, with the escapes `\"`, `\\`, `\n`, `\t` and `\r`; `[a, b]` for arrays, slices and lists; `{field: value, ...}` for records; `none`, `some(v)`, `ok(v)` and `err(e)`; an enum variant as `Name` or `Name(fields)`; `q(raw)` for a fixed-point number's raw bits; and `()`.

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
- From the first measured run on, a task never changes. A task found to be broken is retired: `retired: <date> <reason>` joins its front matter, it stays in the corpus, and it is excluded from every later score, while the scores before keep it. A replacement takes a new `id`, counts in the composition table, and is reported apart until it has as many runs as the tasks it is compared with.
- The held-out set follows the same rules in its own repository; a change to it updates the commit in `corpus.lock`.

## Checking

```powershell
npm run check:benchmark
npm run check:benchmark -- --held-out ..\mente-bench-heldout
npm run check:benchmark -- --bless --held-out ..\mente-bench-heldout
```

[`check.mjs`](check.mjs) checks every task's front matter, sections and core mark, the composition against the table above, every example and hidden test against the signature and against the task's solution in [`reference.mjs`](reference.mjs), and every digest in `corpus.lock`. CI runs the first form, without the held-out set; the second also checks a local clone of the held-out repository against the lock; the third rewrites the lock after a change that "Changing the corpus" allows.
