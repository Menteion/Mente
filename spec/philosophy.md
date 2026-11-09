---
title: First-Principles Design Philosophy and Founding Axioms
version: 0.2.0
status: active
date: 2026-11-09
---

# First-Principles Design Philosophy and Founding Axioms

**Governs:** Workspace-wide architecture (`compiler/*`, `examples/*`, `spec/*`)

## 1. Mission statement

Mente (`.mn`) is a universal, statically verified, hardware-sympathetic programming language designed from first principles to resolve three chronic engineering failure modes of existing systems and application languages:

1. **The ergonomic vs. mechanical-sympathy divide**: High-level languages (Python, TypeScript, Go, C#) sacrifice cache locality, deterministic deallocation, and SIMD guarantees for developer velocity, whereas low-level languages (C++23, Rust, Zig) burden application logic with lifetime sigils (`<'a>`), borrow-checker archaeology, or unchecked pointer arithmetic.
2. **Runtime boundary and contract redundancy**: Traditional languages re-evaluate array bounds, null checks, and defensive preconditions at runtime on every call instead of proving them once at compile time and erasing them from the instruction stream.
3. **Opaque compiler intermediate representations**: Upstream C++ compiler infrastructures (`LLVM` / `MLIR`) represent programs as pointer-heavy heap object graphs that cannot natively reason about value refinement intervals, hardware cache-line alignment, or branchless execution contracts.

## 2. The six founding axioms

1. **Universality by layers**: A single language syntax and type system MUST scale from high-level domain pipelines and network services down to 64-byte L1 cache-line structures and `@branchless` SIMD loops. It does so through a small core that is safe, strict and simple, and opt-in layers (layouts, `@branchless`, `@simd`, regions and pools, units, fixed-point numbers) that add guarantees or control without changing the meaning of core code. Reaching GPU/NPU compute through `@kernel` stays a hypothesis until it has an execution model (`hardware-contracts.md`).
2. **Proof-erased safety with visible cost**: Every memory, bounds, dimensional unit, and precondition check (`requires` / `ensures`) MUST be discharged statically at compile time whenever Tier 1 proves it, and then emits no run-time branch and feeds its proven intervals into the optimizer (`Contract-Guided Optimization`). A check that stays is visible to the reader: `mn view` marks every run-time check Tier 1 leaves, from M1's checkpoint (b) ([`mn view --checks`](comptime-and-ffi.md#mn-view-checks)). No cost is hidden: no hidden control flow, allocation, conversion or copy.
3. **Local reasoning from signatures and call sites**: Ownership (read-only by default, `mut`, `own`), mutation, error propagation (`!`), and side-effect capabilities (`with IO, Net`) MUST be explicit at function signatures and call sites without reference sigils (`&`, `&mut`, `*`) or viral lifetime parameters (`<'a>`).
4. **Hardware mechanical sympathy as a static contract**: Data layout (`AoS`, `SoA`, `Tiled`, `@layout`, `bitstruct`) and micro-architectural execution constraints (`@branchless`, `@simd`, `@no_alloc`, `@max_stack`) are compile-time verified invariants, never silent compiler hints.
5. **Canonical single-path representation**: Every semantic concept has exactly one canonical syntactic form, making `.mn` source code unambiguous for both human engineers and automated synthesis/verification agents.
6. **Every rejection teaches**: Every error names the rule it enforces and carries a fix: machine-applicable where it is certain, a suggestion otherwise; a failed proof carries a counterexample and a `guard` (Engineering Guardrail #1, `diagnostics.md`). A rule whose violations cannot be explained this way is not added.

### The order of the axioms' aims

Safety and strictness come first, and performance is reached on top of them, never traded against them. Where the aims conflict, the earlier one wins (Norman, 2026-11-09; [`strict-defaults`](../docs/design-review.md#strict-defaults), [`philosophy-polish`](../docs/design-review.md#philosophy-polish)):

1. **Safety**: no undefined behaviour, and no silently wrong value.
2. **Strictness**: one meaning for each construct, and a mistake rejected as early as it can be found.
3. **Predictable cost**: a reader sees what each construct costs (axiom 2).
4. **Learnability of the core**: a small core with familiar spellings.
5. **Peak performance**: reached through the opt-in layers of axiom 1, so it never makes the core harder.
6. **Brevity**.

Convenience for agents comes from diagnostics, fixes and the language card (axiom 6), never from relaxing an earlier aim. Performance comes from what strictness makes possible: a check Tier 1 proves is erased, a `mut` argument of its parameter's exact type is passed by address, and a value that never changes is a `val`.

### Non-goals

Each is decided in the spec file it names; a new one starts from an entry of the register ([`philosophy-polish`](../docs/design-review.md#philosophy-polish)).

- No garbage collection: memory is values, regions and pools (`regions-and-pools.md`).
- No exceptions: an error is a value of `T ! E`, propagated with `!` (`error-handling-abi.md`).
- No null: an optional value is `?T` (`algebraic-types-and-null.md`).
- No macros: `comptime` instead (`comptime-and-ffi.md`).
- No global mutable state (`capability-effects.md`).
- No implicit conversions and no casts (`core-calculus.md`, item 2).
- No references or lifetimes: parameter modes instead (`mutable-value-semantics.md`).

### How the language changes

Agents learn Mente from its language card, not from training data, so a change costs less than in a language models were trained on, as long as the card changes with it (Norman, 2026-11-09; [`language-evolution`](../docs/design-review.md#language-evolution)).

- `Specified`: before 1.0, which is the foundation gate, any change may break existing code. The pull request that makes it updates the language card, the tests of the card's snippets, the conformance suite and the examples, and `mn` offers a machine-applicable migration where the change is mechanical.
- `Specified`: after 1.0, a breaking change is admitted only with a machine-applicable migration, which `mn migrate` applies to existing code, as axiom 6 asks of every rejection. A change that cannot be migrated mechanically is considered for an edition, through the register.
- `Specified`: the language card names the language version it describes and ships with the compiler. `mn card` prints the card of the installed version, so an agent always reads the card of the compiler it uses; the stable address serves the latest.

## 3. Goal: the general-purpose language best suited to AI agents

- Mente aims to be the general-purpose programming language best suited to AI agents writing code, while people review the result and decide whether to trust it ([`ai-first-purpose`](../docs/design-review.md#ai-first-purpose), decided 2026-11-08).
- "Best suited" is a claim, so it is measured: by the agent benchmark of [`success-workload`](../docs/design-review.md#success-workload), against Rust and a control language, under its protocol in `ROADMAP.md`.
- `Hypothesis`: strict rules pay off for agents. They raise the failures of a first attempt, and in return lower the repairs a solve needs and raise the final pass rate. The agent benchmark reports both by failure class from the end of M1, so the cost of each strict rule is visible ([`philosophy-polish`](../docs/design-review.md#philosophy-polish)).
- The goal does not narrow axiom 1. Mechanical sympathy stays a principle of the whole language: defaults fit the hardware, hardware costs are visible to agents and reviewers, and the hardware contracts are its strongest form. Agents are weakest at exactly this knowledge, which other languages leave implicit, so making it explicit and checked is part of what suits Mente to them.
- **The core subset.** No model has been trained on Mente, so an agent learns every mechanism from its context, and the language's surface area is the main cost of the goal. An agent can write correct, reasonably fast code using only the core of the language: types, `val` and `var`, parameter modes, `?T`, `T ! E` and effects. Refinements, units, explicit layouts and hardware contracts appear only where the code asks for them, and a refinement or a contract asks for a compile-time proof only where it is written. Code without refinements or contracts is a supported way to write Mente, not a draft: it is checked for types, effects, parameter modes and exclusivity; overflow, a zero divisor and an out-of-bounds index trap at run time; and a check Tier 1 cannot prove in such code stays a run-time check, never a compile-time error ([`gradual-verification`](../docs/design-review.md#gradual-verification)). The agent benchmark scores tasks that need only the core apart from those that use the other mechanisms.

### Problems agents meet, and Mente's answer to each

The catalogue of [`ai-coding-problems`](../docs/design-review.md#ai-coding-problems), decided on 2026-11-09. Every design decision made for agents names the row it serves. A problem with no answer is an `open` entry in the register, and a problem outside what any language can solve is marked so rather than promised. The last column says how the answer is shown to work: by the agent benchmark, whose report classifies a program that fails to build by the failure classes of its codes ([`diagnostics.md`](diagnostics.md) item 8), or by a check. "Not measured" marks an answer that nothing checks yet. What no answer reaches is listed under "What Mente cannot solve" below.

| Problem agents meet | Mente's answer | Status | Measured by |
| :--- | :--- | :--- | :--- |
| Every mechanism must be learned from context, since no model has seen Mente | The core subset: correct, reasonably fast code using only types, `val` and `var`, parameter modes, `?T`, `T ! E` and effects, with the other mechanisms only where the code asks for them | In the spec (`philosophy.md`, [`ai-first-purpose`](../docs/design-review.md#ai-first-purpose)) | Core tasks, scored apart from the others |
| Names and APIs that do not exist | Unknown names are errors; a compact language card; `mn api` looks a declaration up | Decided ([`language-card`](../docs/design-review.md#language-card)): the card in M1, `mn api` in M6 | Benchmark failures classed as unknown names |
| Habits from other languages (`let`, `&mut`, `async`, `.unwrap()`) | A dedicated error naming the Mente spelling, with a fix | In the spec ([`foreign-idiom-diagnostics`](../docs/design-review.md#foreign-idiom-diagnostics)), built in M1's checkpoint (a) | Benchmark failures classed as foreign idioms; the syntax probe of M1's checkpoint (a) ([`syntax-probe`](../docs/design-review.md#syntax-probe)) |
| Vague errors that lead to guess-and-retry loops | Coded, machine-readable diagnostics with applicable fixes; Tier 1's counterexample and `guard` | Implemented for today's codes ([`diagnostics-as-api`](../docs/design-review.md#diagnostics-as-api)); the counterexample in the spec ([`tier1-counterexample`](../docs/design-review.md#tier1-counterexample)), built in M1's checkpoint (b) | Repairs per solved run; a test that applies each machine-applicable fix (`docs/design-brief.md` section 7.7, criterion 3) |
| Too much code to read to know what a function does | Effects, modes, errors, provenance and contracts in the signature; `mn view --spec` | In the spec (`holographic-module.md`); the projection implemented | Tokens per solved run; the projection's size against the source |
| A change that breaks distant callers | No interface fact inferred from a body; three digests; a list of what a change affects | In the spec, digests implemented; proposed: [`impact-analysis`](../docs/design-review.md#impact-analysis) | Not measured: every benchmark task is a single function |
| Hidden side effects, in the code or in its dependencies | Capability effects; no global mutable state; `with FFI` the only way out; a per-module effect summary | In the spec (`capability-effects.md`), built in M4; proposed: [`capability-trust`](../docs/design-review.md#capability-trust) | Adversarial tasks, caught at build time |
| Fighting lifetimes and the borrow checker | Mutable value semantics, arenas and pools, with no lifetimes | In the spec, built in M2 and M3 | Repairs per solved run on the M2 and M3 tasks, against Rust |
| Range, overflow and unit bugs | Refinements, traps on overflow in every build, units; intermediate results computed exactly, so `(lo + hi) / 2` cannot overflow | In the spec (`refinement-and-fixed-point.md`, `dimensional-units.md`; exact intermediates and trap regions from [`overflow-check-cost`](../docs/design-review.md#overflow-check-cost)), built in M1's checkpoint (b) and M5 | Hidden tests at the edges of each range; soundness by execution (`docs/design-brief.md` section 7.7, criterion 5) |
| Proof errors that trap the agent in a loop | Proofs asked for only where a refinement or contract is written; a predictable Tier 1 | In the spec ([`gradual-verification`](../docs/design-review.md#gradual-verification), Guardrail #1) | Benchmark failures classed as proof errors, a re-planning trigger |
| Unfinished work that looks finished | `todo("reason")`, listed by `mn check` and rejected by release builds; contracts and `@example`s check the behaviour they state | In the spec ([`todo-marker`](../docs/design-review.md#todo-marker)), built in M1's checkpoint (a) | A solution containing `todo` scores as unsolved |
| Tests that check nothing | Mutation testing of the user's own functions | Proposed: [`user-mutation-testing`](../docs/design-review.md#user-mutation-testing), for M6 | Not measured: benchmark agents write no tests |
| Flaky, unreproducible concurrency failures | Structured concurrency, exclusivity across `parallel` branches, seeded simulation | In the spec (`structured-concurrency.md`, `capability-effects.md`), built in M7 | The M7 tasks' checks on recorded calls and timing |
| Inconsistent style across agents and runs | One spelling per concept; `mn fmt` | In the spec (axiom 5); `mn fmt` built in M6 | Not measured |
| Slow feedback loops | A Tier-1 re-check target; the interpreter for tests; digests for incremental builds | Target with its protocol in `ROADMAP.md`, met by the M0b spike's prototype ([`tier1-latency-spike`](../docs/design-review.md#tier1-latency-spike)) | The Tier-1 latency protocol, from M1 |
| No training data for a new language | The language card, published at a stable address an agent can fetch; the examples; later, the self-hosted compiler and standard library as a corpus | Decided ([`language-card`](../docs/design-review.md#language-card), [`docs-site`](../docs/design-review.md#docs-site)), from M1; proposed: [`self-hosting-plan`](../docs/design-review.md#self-hosting-plan) | Mente against the control language, Odin, which models have also seen little of |
| Real tasks lean on libraries: text, files, time, JSON, HTTP, databases | A standard library with one way to do each thing; packages; C libraries through `extern "C"`, and later `mn bind` | Open: [`stdlib-scope`](../docs/design-review.md#stdlib-scope); `extern "C"` in M4, the standard library in M6, `mn bind` in stage 4 | The M6 and M7 tasks; until then, not measured |
| Code that works but fights the hardware: pointer chasing, padding, branches in hot loops, false sharing | Defaults that fit the hardware; a cost report per function and per type; hardware contracts where it matters most | Proposed: [`sympathetic-defaults`](../docs/design-review.md#sympathetic-defaults), [`cost-report`](../docs/design-review.md#cost-report); the hardware contracts in the spec | Baselines with hardware counters; the tight-loop benchmark's gaps |
| Code that is correct but slow, unknown to the agent | The cost report; baselines that record hardware counters; the performance stage | Proposed: [`cost-report`](../docs/design-review.md#cost-report), [`performance-stage`](../docs/design-review.md#performance-stage) | Baselines with hardware counters, from M1 |
| Misreading what a requirement means, or choosing the wrong design | Outside what a language can solve. Contracts and `@example`s make the behaviour the agent intended explicit and checkable, so a reviewer sees what it understood | Outside a language | Not separable from other failures |

### What Mente cannot solve

Some problems agents meet are outside what any language can solve, and some limits are Mente's own. They are written down here so that no decision or claim promises otherwise. Each says what reduces it, where anything does. The review after each milestone, in `docs/milestone-reviews.md`, watches the ones that evidence can show.

**Outside Mente's control**

- **No model has been trained on Mente.** The language card, the documentation site, the examples and later the self-hosted compiler reduce the cost ([`language-card`](../docs/design-review.md#language-card), [`docs-site`](../docs/design-review.md#docs-site), [`self-hosting-plan`](../docs/design-review.md#self-hosting-plan)). But Mente's share of any future training data will stay small for years: without users there is little Mente code, and without that code agents write Mente less well.
- **The ecosystem is small.** `extern "C"`, and later `mn bind`, reach C libraries. They do not reach the long tail that agents rely on in other languages: cloud SDKs, database drivers, authentication, payments ([`stdlib-scope`](../docs/design-review.md#stdlib-scope)). And what comes through FFI carries none of Mente's guarantees, as below.
- **Models change.** The agent benchmark measures the models of its day, and stronger models may close the gap that Mente's design opens for today's. So a result holds only for the model it names ([`success-workload`](../docs/design-review.md#success-workload)).
- **Reviewers must learn Mente too.** Under the goal, people decide whether to trust what agents write, from its signatures, contracts and effects. A reviewer who does not know Mente cannot do that. This is the same network effect as training data, on the people's side.

**Limits no language escapes**

- **Most of an agent's failures are not the language's.** Misreading a requirement, choosing the wrong design, or misunderstanding an existing codebase or the environment it runs in: a language reduces only the failures it causes, however well it is designed. The benchmark, whose tasks state their requirements exactly, does not measure the rest.
- **A proof is only as good as its contract.** Tier 1 and Tier 2 prove what a contract says. A wrong or weak contract is proven just the same, and gives false confidence. Only a reviewer can judge a contract; `@example`s and mutation testing of the user's own code ([`user-mutation-testing`](../docs/design-review.md#user-mutation-testing)) make a weak one easier to see.
- **The guarantees stop at the FFI boundary.** Code behind `with FFI` can do anything, so its effects, its memory safety and its contracts are trusted, not checked ([`capability-trust`](../docs/design-review.md#capability-trust)). The more a program leans on C libraries, the less of it Mente vouches for.
- **Some properties cannot be proven.** Tier 1 stays inside a fragment it decides predictably, and Tier 2's solver is incomplete and unpredictable outside that fragment. Some obligations will always remain run-time checks or trusted axioms ([`tier2-trusted`](../docs/design-review.md#tier2-trusted)).

**Trade-offs that can be tuned, not removed**

- **Every check is something to learn.** Refinements, contracts and units catch bugs, but each mechanism costs an agent context and tokens. No design is both the most checked and the smallest. The core subset is one point between the two, and the benchmark judges it by scoring core tasks apart from the others.
- **Some tight loops will trail C and Rust.** Overflow checks are a cost paid on purpose, and Cranelift optimises less than mature backends until stage 3 brings a backend of Mente's own ([`tight-loops`](../docs/design-review.md#tight-loops), [`performance-stage`](../docs/design-review.md#performance-stage)).

**Risks of the project**

- **Scale and time.** One person with agents builds a foundation of ten milestones, then self-hosting, then a backend ([`long-term-order`](../docs/design-review.md#long-term-order)). The field may change before Mente matures.
- **The benchmark cannot show everything.** Its tasks are single functions. Changes across a large codebase, effects across modules and long maintenance are not measured. The benchmark shows how Mente compares on such tasks against Rust and a control language ([`benchmark-corpus`](../docs/design-review.md#benchmark-corpus)); beyond them, the goal's wider claim stays a hypothesis.

<!-- @assert-present file="Cargo.toml,compiler/mente-syntax/src/lib.rs,compiler/mente-ast/src/lib.rs,compiler/mente-sema/src/lib.rs,compiler/mente-mir/src/lib.rs,compiler/mente-codegen/src/lib.rs,compiler/mente-cli/src/main.rs" reason="All six core Mente compiler crates exist in the workspace" -->
