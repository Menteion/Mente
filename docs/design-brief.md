# Mente Design Brief for Claude Code

Written 2026-11-06 for a Claude Code session that continues Mente development from commit `bdffc1c` (2026-11-05 08:42, `origin/main`). It says what the design is, what is still open, how to work here, and the long-term plan. Read it together with `CLAUDE.md`, `docs/STATUS.md` and `ROADMAP.md`. All three are accurate at `bdffc1c`; the problems found since then are entries in `docs/design-review.md`.

**Before writing any code**, do three things and then wait for Norman's approval:

1. Read this brief, `docs/STATUS.md`, `ROADMAP.md`, `docs/design-review.md`, and the `spec/*.md` files that govern the first step.
2. List anything you disagree with or find contradictory, in this brief or in the spec.
3. Propose a plan for the next milestones of section 7.7 ([`foundation-milestones`](design-review.md#foundation-milestones)), in the order of `ROADMAP.md`. Each step must say how it will be verified.

## 1. Starting point

- **The repository is at `bdffc1c`.** It ends a review that checked the documents' claims and maturity labels against the code. `docs/STATUS.md` says what the compiler does, and names the next step: milestone M0a (the `@spec-family` tools, CI and Rust hygiene), then M0b, then M1.
- **What exists:**
  - six crates: `mente-syntax`, `mente-ast`, `mente-sema`, `mente-mir`, `mente-codegen`, `mente-cli`;
  - a declaration parser; the three digests and `mn view --spec`;
  - the `@layout` and `bitstruct` provers;
  - the `from`-clause check (`E0311`);
  - token-level `@total` and `@branchless` checks;
  - a hand-written straight-line MIR region with an interpreter and a mutation gate;
  - `mn spec`.
- **What does not exist:** function bodies are not parsed, nothing is type-checked, no `.mn` code is lowered, and no machine code is emitted. `docs/STATUS.md` lists every gap.
- **Stand-ins, since removed:** `mente-codegen` held `CarryFlagAbiLowering` and `emit_spirv_binary`, placeholders for two `Hypothesis` features, with a test asserting a value the code itself set. They broke the "No stand-ins" rule in section 5 and were removed in M0a ([`codegen-placeholders`](design-review.md#codegen-placeholders)); the crate stays, empty, until M1.
- **Labels older than the evidence rule:** most `Implemented` claims in `spec/` are backed by a directive that names a function or a type, not a test. Under the rule in section 5, each needs a test or a lower label; [`implemented-evidence`](design-review.md#implemented-evidence) decided how M0a finds and resolves them, with `spec-guard claims`.

## 2. What Mente is

Mente (`.mn`) is a statically verified, hardware-aware language meant to cover everything from application code down to cache-line structures and SIMD loops with one type system (`spec/philosophy.md`). It targets three problems:

- High-level languages give up layout control and determinism, while low-level ones demand lifetimes or unchecked pointers.
- Checks that could be proven once at compile time are repeated on every call.
- Compiler IRs cannot carry intervals, alignment or branch-freedom.

Its five axioms:

1. One language from services down to cache lines.
2. Checks are proven and erased, never repeated at run time.
3. Ownership, mutation, errors and effects are visible in signatures without reference sigils or lifetimes.
4. Layout and micro-architectural constraints are compile-time contracts, never hints.
5. Each concept has one canonical spelling.

None of Mente's features is new on its own. Refinement types, units of measure, mutable value semantics, capability effects and explicit layouts each exist elsewhere (SPARK and Liquid Haskell, F#, Swift and Hylo, Koka, Zig and C). Mente's value is that they are designed to work together under one rule: **the interface is the signature**. Protect that rule above all; most other decisions follow from it.

A review on 2026-11-06 judged the design sound but incomplete, at 4 out of 5 (a reviewer's judgement, not a measurement), and the implementation an early front-end prototype. Section 4 lists what is missing.

Norman's goal for Mente is to be the general-purpose language best suited to AI agents writing code: shaped around how agents write code and around the problems they meet, while people review the result and decide whether to trust it. It was decided on 2026-11-08 as [`ai-first-purpose`](design-review.md#ai-first-purpose), with a core-subset principle: an agent can write correct, reasonably fast code using only the core of the language (types, `val` and `var`, parameter modes, `?T`, `T ! E` and effects), and refinements, units, explicit layouts and hardware contracts appear only where the code asks for them. `spec/philosophy.md` states both. "Best suited" is a claim like any other, so it is measured, by the agent benchmark of [`success-workload`](design-review.md#success-workload), against Rust and a control language, with core-only tasks scored apart.

Mechanical sympathy stays a principle of the whole language under this goal, not a layer for specialists. Agents are weakest at exactly the hardware knowledge other languages leave implicit: cache lines, padding, pointer chasing, branches in hot loops, false sharing. So Mente's defaults fit the hardware ([`sympathetic-defaults`](design-review.md#sympathetic-defaults)), its costs are reported in a form agents can act on ([`cost-report`](design-review.md#cost-report)), and its hardware contracts check what matters most.

**Problems of AI-written code, and Mente's answer to each.** The catalogue of [`ai-coding-problems`](design-review.md#ai-coding-problems), decided on 2026-11-09, lives in [`spec/philosophy.md`](../spec/philosophy.md) section 3: each problem agents meet, the mechanism that answers it and that mechanism's status, and how the agent benchmark or a check shows whether it works. Every problem without an answer becomes an `open` entry, and some problems are outside what any language can solve, which the catalogue says rather than promising otherwise.

## 3. The design worth keeping

Each item gives the rule and why it beat the alternative. The spec file named in each heading has the full text.

These are design rules, not descriptions of the compiler: apart from the items listed in section 1, every one of them is `Specified`.

### 3.1 The interface is the signature (`holographic-module.md`)

- One `.mn` file per module. Everything a dependent compiles against is written in, or derived from, declarations: types and layouts, signatures, parameter modes, return provenance (`from`), effects (`with`), contracts, and contract attributes such as `@max_stack`.
- The compiler never infers an interface fact from a body. The only bodies that reach dependents are generic `pub fn`s and `@inline` ones, tracked by their own digest.
- There are three digests:
  - `spec_hash` covers the interface.
  - `instance_hash` covers bodies that dependents instantiate or inline.
  - `body_hash` covers everything else.

  A dependent recompiles only when something it uses changes.
- Why: an inferred fact lets a body edit in one module break a caller in another while every signature stays the same.
- This rule recurs below:
  - `@total`, `@no_alloc`, `@max_stack` and `@branchless` compose from signatures.
  - Tier 1 is modular: it assumes a callee's `ensures` and never reads its body.
  - A task's stack size never depends on a callee's body.

### 3.2 Refinements and two-tier contracts (`refinement-and-fixed-point.md`, `ghost-contracts-and-totality.md`; Guardrail #1)

- A scalar can be refined as `T[a..b]` or `T[a..=b]`, and the refined type is a subtype of `T`.
  - A value enters a refinement only when its interval is known to lie inside: an in-range literal, arithmetic on refined operands, or narrowing by `guard` or `if`.
  - Anything else is a compile error that carries a counterexample and a `guard` prescription.
- A bound may name only a binding that cannot change: a read-only parameter, a `val`, or a compile-time parameter. It may never name a `var` or a `mut` parameter.
- Integer and fixed-point overflow, narrowing and division by zero trap in every build unless Tier 1 proves them away. Wrapping and saturation are only ever explicit: `wrap<T>`, `sat<T>`, `+%`, `+|`. When and where a trap occurs follows [`overflow-check-cost`](design-review.md#overflow-check-cost): intermediate results are exact and checked where a value is narrowed, and checks may be merged within regions that have no externally visible effect.
- Tier 1 uses intervals plus octagons (`+-x +-y <= c`) and nothing else, so it is polynomial and never times out.
- Contract clauses are classified by syntax alone:
  - The octagon fragment is proven by `mn check` at every definition and every call.
  - Anything else (`or`, `!=`, products, calls, quantifiers) must be routed with `@prove` (Tier 2, `mn prove`) or `@checked` (a runtime check); otherwise it is `E0503`.
- A fact outside octagons comes from a callee whose contract states it inside them, its body proven once by Tier 2. The canonical case is `midpoint(lo, hi)`, with `requires lo < hi` and `ensures lo <= result and result < hi`.
- Every failed obligation reports a concrete counterexample valuation and a copy-pasteable one-line `guard` that fixes it.
- `@total` needs a `decreases` measure on every loop and recursion, and may call only `@total` functions.
- Why: letting a solver choose the tier, or widening Tier 1 to polyhedra or a time-budgeted solver, would make whether code compiles depend on solver internals and machine speed.

### 3.3 Dimensional units (`dimensional-units.md`)

- A unit has two parts:
  - a dimension: integer exponents over the base units declared with `unit`, domain units such as `token` and `byte` included;
  - a scale: an exact rational number.
- `+`, `-` and every comparison need the same unit, scale included (`E0601`). `*` and `/` combine dimensions and scales.
- `.to<U>()` is the only conversion. Converting across dimensions is `E0602`; on an integer quantity, converting towards a coarser unit is `E0603`.
- Units are erased after checking. The only instructions they add are the conversions written in the source.
- Why: implicit conversion on `+` hides a multiply and a rounding inside an operator, and comparing dimensions alone lets `ms + sec` add raw numbers.
- **Write these probes when you implement units.** They are the cases a units checker most easily gets wrong:
  - An unknown unit name is an error, not a new base unit.
  - Only numeric types take a unit, so `Handle<Node>` is not a quantity.
  - `1.0 / t` has the inverse unit.
  - A quantity compared with a plain number is `E0601`.
  - `5_u32` is a `u32`.
  - A `unit` declaration that does not parse is reported where it is written.
  - `.to<U>()` multiplies at run time. Check it by running a program as soon as `.mn` code runs, in the MIR interpreter and then in native code. Code first runs in M1: on the reference interpreter at its first checkpoint, and in the MIR interpreter and native code at its third ([`foundation-milestones`](design-review.md#foundation-milestones)).

### 3.4 Memory without lifetimes (`mutable-value-semantics.md`, `regions-and-pools.md`; Guardrail #3)

**Parameter modes**

- Parameters are read-only by default, or marked `mut`, `own`, or `in` (for arenas). The mode is written at the declaration and at every call site: `f(mut a)`, `g(own b)`, `h(in arena)`.
- `mut` means copy-in/copy-out, written back on every exit, including `raise` and cancellation.
  - The Law of Exclusivity makes copy-in/copy-out indistinguishable from passing a pointer, so the compiler passes a pointer wherever the storage is contiguous.
  - It also covers elements of `SoA` and `Tiled`, which have no address.
- The Law of Exclusivity: no two arguments of one call may overlap when either is `mut`, and the same holds between the branches of `parallel (...)`. An `own` argument is dead after the call.

**Provenance**

- A returned view's provenance comes from the signature, never from the body.
  - By default, the view may borrow from any read-only, `mut` or `in` parameter that is or holds a view or an arena.
  - `from p, q` narrows that set.

**Arenas, pools and reference counting**

- `Arena.scoped(cap) { arena => ... }` is the only way to open an arena; `cap` is optional.
  - It is reclaimed in O(1).
  - Each task bumps its own chunk, so allocation needs no atomics.
  - A type with a destructor cannot go in an arena (`E0312`).
- `Pool<T>` with `Handle<T>` (32-bit slot plus 32-bit generation) handles graphs and cycles. `pool[h]` and `pool[mut h]` yield `?T`; there is no `get_mut` and no panicking index.
- Reference counting has two types:
  - `Local<T>` is non-atomic and task-bound (`E0320`).
  - `Shared<T>` is atomic.

  Clones are always explicit.
- `E0310` never shows region IDs. It always prescribes `in arena: Arena` or `Pool<T>` / `Handle<T>`.

**Why:** lifetime parameters are viral and fail on cyclic graphs, and a pointer-based `mut` cannot express an element of a structure-of-arrays.

### 3.5 Data layout (`morphic-layouts.md`)

- `List<T>`, `SoA<T>` and `Tiled<T, lanes: N>` keep the same `p[i].x` syntax across AoS, SoA and AoSoA layouts.
- `@layout(size, align)` with `@bytes(start..end)` makes struct bytes exact:
  - no hidden padding;
  - natural alignment unless the struct is `@packed`;
  - little-endian unless `@endian(big)`;
  - errors `E0410` to `E0419`.
- `bitstruct` with `@bits` partitions a backing integer exactly (`E0420` to `E0425`). `uN` and `iN` exist for any width from 1 to 128.
- These two provers are the most solid code in the repository. Keep their tests green as the parser changes underneath them.

### 3.6 Types and errors (`algebraic-types-and-null.md`, `traits-and-generics.md`, `error-handling-abi.md`)

- `?T` is narrowed with `guard val v = opt else ...`, `?.` and `??`. There is no `.unwrap()` anywhere.
- `enum` needs exhaustive `match`. Ad-hoc unions are written `A | B`, and `is` narrows them.
- `T ! E` is for recoverable failure: a plain `return`, `raise`, postfix `!`, and `catch`. Absence (`?T`) and failure (`T ! E`) stay separate.
- Generics are written `<T>`, never `::<>`. Compile-time values are written `comptime N: K`, and a C#-style rule tells type arguments from comparisons.
- A trait-typed parameter means static dispatch. `dyn Trait` is the only way to get a vtable.

### 3.7 Effects, concurrency and simulation (`capability-effects.md`, `structured-concurrency.md`, `disjoint-parallelism.md`)

**Effects**

- A function is pure unless it declares `with IO`, `Net`, `Clock`, `Rand`, `FFI` or a user-declared effect. A caller's set must cover its callees', less what enclosing `handle` blocks install.
- `with` only declares and `handle` only installs.
- Handlers are tail-resumptive only. With no captured continuations, a static handler compiles to a direct call and `own` stays sound.
- Effect variables (`comptime E: Effects`) let one `map` serve both pure and effectful functions, so libraries never split into sync and async versions.

**Concurrency**

- There is no `async`/`await`. Tasks run on stackful fibers whose stacks are reserved virtual memory.
- `parallel (...)` is lexical: it cancels siblings on error, and its children read parent bindings without `Arc`.
- `parallel_for` over a structural partition (`chunks_mut`, `split_at_mut`, `Tiled` tiles) needs no proof; a partition by index arithmetic needs `@prove`.
- `Channel.send(own msg)` moves a message. `Mutex<T>` is reachable only through `with_lock`.

**Simulation**

- `SimClock`, `SimNet` and `SimSched(seed)` make tests deterministic. Scheduling is itself an effect, so a failing interleaving replays from its seed.

### 3.8 Hardware contracts (`hardware-contracts.md`)

- `@branchless` means no data-dependent branch:
  - `if` and `match` lower to selects, and every arm must be total and effect-free.
  - Only loops with a constant trip count are allowed, and they are unrolled.
  - Every trap must be proven away.
  - Every callee must be `@branchless` or inlined.
  - After code generation, the machine code is disassembled and any conditional jump is rejected (`E0403`).
- `@simd(lanes: N)` sits on a loop, never on a function, and counts lanes rather than register width.
- `@no_alloc`, `@max_stack(bytes)` and `@total` compose from signatures.
  - They reach through traits and `dyn` when the trait method declares them.
  - On `extern` declarations they are trusted and quarantined.
- Contracts are hard errors, never hints.

### 3.9 MIR and the backend (`mir-and-codegen.md`; Guardrail #2)

- `MirRegionSoA` stores parallel arrays indexed by `NodeId(u32)`, with a `ValueLattice` interval per node. It has no dependencies.
- Every Mente-level optimisation completes in MIR: bounds erasure, contract assumptions, `Tiled` offsets, branchless selects, inlining decisions and `@simd` vectorisation. Only then does `mente-codegen`, the only crate allowed to depend on Cranelift, lower MIR to Cranelift IR.
- Three Cranelift facts shape the lowering:
  - Vectors are at most 128 bits, so wider lane groups are split.
  - A float or vector `select` becomes a branch on x64, so `@branchless` code lowers it to `bitselect` or an integer select.
  - There are no flags in the calling convention, so `T ! E` returns as a tag and a payload in two registers.
- Until the performance stage replaces it with a backend written in Mente (section 7.5), native code comes from the real `cranelift-codegen` crate, never from hand-assembled bytes that imitate it.
- The mutation gate flips MIR opcodes in memory. It passes when every mutant is killed or marked equivalent with a written reason.

### 3.10 comptime and FFI (`comptime-and-ffi.md`)

- There are no macros. `comptime for` and `comptime if` work over `Type.of<T>()`; they are pure and run under a step budget (`E0701`).
- An `extern "C"` declaration needs `with FFI`.
  - A refined foreign result must declare an error type, and it is checked at the boundary.
  - `RawPtr<T>` may appear only in `with FFI` code.

### 3.11 Surface syntax (`surface-syntax.md`)

- Bindings are `val` and `var`, never `let`.
- Blocks use braces, with no semicolons.
- Logic uses `and`, `or` and `not`.
- `guard`, `|>`, and `if`/`match` as expressions.
- Literal suffixes: `200.0_meter`, `0.5_q16_16`, `255_u8`. An unknown suffix is an error.

## 4. Open design problems

These are why the design scored 4 rather than 5. Each problem with a key is an entry in `docs/design-review.md`, marked `open` or `proposed`. Settle it there, with Norman, before writing code that depends on it.

1. **Rules for combining features are missing.**
   - Closures: what they capture and in which mode, whether one capturing a `mut` binding may escape, and how they meet effect variables ([`closure-captures`](design-review.md#closure-captures)).
   - Trait coherence: the orphan rule and overlapping implementations ([`trait-coherence`](design-review.md#trait-coherence)).
   - Memory ordering for `Shared<T>` and atomics ([`atomics-memory-ordering`](design-review.md#atomics-memory-ordering)).
   - Error types across `!`: whether `f()!` inside a `T ! E2` function needs a conversion from `E1` to `E2`, and how it is written ([`error-conversion`](design-review.md#error-conversion)).
   - Refinements, units and fixed point in one type, such as `f64[0.0..=1e12]<kg>` ([`combined-scalar-types`](design-review.md#combined-scalar-types)).
   - `sat<T>` and plain `T` in one expression, comparison or assignment, as example 02's `integrate_cell` writes them ([`sat-mixing`](design-review.md#sat-mixing)).
   - `@no_alloc` around `parallel_for`, whose tasks each reserve a stack ([`no-alloc-task-stacks`](design-review.md#no-alloc-task-stacks)).
2. **The flagship example breaks the spec.** Example 01's `kinetic_energy` cannot be accepted: its `ensures result >= 0.0_joule` is false when `vel` is `NaN` or infinite, interval arithmetic cannot prove the product non-negative, and a `@branchless` function has no `@checked` fallback ([`float-square-ensures`](design-review.md#float-square-ensures)). It shows how problem 1 bites: a clause can be in the Tier-1 fragment by syntax and still be unprovable from the body.
3. **Tier 1 is not fully specified.**
   - Loops: a relation between two `var`s (`lo <= hi`) cannot be written in a refinement, and the spec does not say whether Tier 1 infers it at a loop head by widening or a loop declares it ([`tier1-loops`](design-review.md#tier1-loops)).
   - Counterexamples: octagons over-approximate, so a valuation Tier 1 reports may be unreachable. The diagnostic must say that Tier 1 cannot rule the valuation out, not that the input fails ([`tier1-counterexample`](design-review.md#tier1-counterexample)). This changes the wording of Guardrail #1 in `CLAUDE.md`.
4. **Tier 2 does not exist.** The body of `midpoint` and every `@prove` clause are therefore trusted axioms, not proofs. Say so wherever they are used, until Tier 2 runs ([`tier2-trusted`](design-review.md#tier2-trusted)). Tier 2 is planned inside the foundation, with an external solver and a proof cache ([`tier2-phase`](design-review.md#tier2-phase), section 7.2).
5. **"Zero-cost" is conditional.** Overflow traps in every build and Tier 1 is deliberately weak, so real code will need `guard`s, `@checked` clauses or lemma functions. Measure how many the examples need before promising performance ([`zero-cost-measured`](design-review.md#zero-cost-measured)). The run-time cost of overflow checks is lowered by two semantic rules, exact intermediate results checked only where a value is narrowed, and imprecise trap regions that merge checks where no effect is visible, and by four measures: checks removed by Tier 1's intervals in all code, loop versioning, prescriptions in the cost report that tell an agent how to remove a remaining check, and a figure of its own to measure the cost ([`overflow-check-cost`](design-review.md#overflow-check-cost)).
6. **The surface area is large.** Each feature is small, but refinements, units, value semantics, regions, effects, layouts and contracts form a large matrix, and the language is only as good as its least-specified intersection. Prefer finishing intersections over adding features.
7. **Purpose and target workload.** No phase says what `v0.4` must achieve or for whom. Section 10 of `docs/design-review.md` sets the goal of the general-purpose language best suited to AI agents writing code ([`ai-first-purpose`](design-review.md#ai-first-purpose), decided 2026-11-08) and proposes what follows from it:
   - a catalogue of the problems agents meet, each with its answer ([`ai-coding-problems`](design-review.md#ai-coding-problems), the table in section 2);
   - a documentation site built from the repository's own sources, published under every milestone tag as pre-release and stable from the gate ([`docs-site`](design-review.md#docs-site));
   - mechanical sympathy kept as a principle of the whole language: defaults that fit the hardware ([`sympathetic-defaults`](design-review.md#sympathetic-defaults)) and a cost report agents can act on ([`cost-report`](design-review.md#cost-report));
   - signatures a reviewer can trust: no global mutable state, `with FFI` the only way past the effect system ([`capability-trust`](design-review.md#capability-trust));
   - dedicated errors for forms carried over from other languages, such as `let` and `&mut` ([`foreign-idiom-diagnostics`](design-review.md#foreign-idiom-diagnostics));
   - machine-readable diagnostics with applicable fixes ([`diagnostics-as-api`](design-review.md#diagnostics-as-api));
   - a compact, tested language reference and an API lookup ([`language-card`](design-review.md#language-card));
   - proofs asked for only where a refinement or contract is written ([`gradual-verification`](design-review.md#gradual-verification));
   - the cost of copying owning collections ([`value-copy-cost`](design-review.md#value-copy-cost)) and the standard library's scope ([`stdlib-scope`](design-review.md#stdlib-scope)), both open;
   - an honest form for unfinished code ([`todo-marker`](design-review.md#todo-marker)), mutation testing of the user's own functions ([`user-mutation-testing`](design-review.md#user-mutation-testing)), and a list of everything a change affects ([`impact-analysis`](design-review.md#impact-analysis));
   - a roadmap reordered for that goal ([`roadmap-ai-priorities`](design-review.md#roadmap-ai-priorities)).

   The goal is measured by an agent benchmark, with a Menteion kernel beside it as the systems workload ([`success-workload`](design-review.md#success-workload)).
8. **Hypotheses and later work.** These stay unbuilt until their stage of the long-term plan (section 7):
   - `@kernel`, until an execution model exists (stage 4);
   - the native backend written in Mente that replaces Cranelift (stage 3);
   - the Carry-Flag ABI, until that backend exists and a benchmark against the two-register return runs (stage 3);
   - self-hosting (stage 2), and `mn bind` (stage 4).

## 5. How to work here

**Labels**

- `Implemented` means three things together: the compiler does it today, a unit test fails when the behaviour breaks, and a directive in the spec names that test. Anything else is `Specified`.
- A claim is a list item that starts with its one label written as code. An `Implemented` claim's evidence is an `@assert-count` indented under it that names a `fn test_` test, and a claim another document cites carries `@spec-item id`. `spec-guard claims` checks all of this; a label inside a sentence or in parentheses is not a claim (`CLAUDE.md`).
- Raising a label is part of the same commit as the code and the test that justify it.

**Probes before claims**

- For every check, write small `.mn` programs that must fail with a specific error code, and others that must pass. Keep them as tests.
- For anything that reaches native code, build it, run it, and assert its output or exit code.
- A test that only restates the implementation proves nothing. Ask what program would break the feature, and write that program.

**No stand-ins**

- A feature that does not work yet stays `Specified`. Never write code that only looks like it: a constant where a measurement belongs, a proof that returns `true`, an emitter that prints a fixed string.

**Honest documents**

- Update `docs/STATUS.md` in the same commit as the behaviour it describes, and never let it claim more than the spec labels.
- A number in a document needs its measurement protocol in `ROADMAP.md`, the date, the machine, and the command that produced it.
- Never write "complete" or "100%" without that evidence.

**Scope**

- Work one milestone at a time, in the order of `ROADMAP.md`: M0a, M0b, then M1 to M9. Take nothing from a later stage, and add no milestones, without Norman.
- M1 is a thin slice that runs end to end, and every milestone after it keeps the whole pipeline running ([`foundation-milestones`](design-review.md#foundation-milestones), which also settled [`roadmap-vertical-slice`](design-review.md#roadmap-vertical-slice) and [`roadmap-ai-priorities`](design-review.md#roadmap-ai-priorities)).
- Follow the long-term order of section 7 ([`long-term-order`](design-review.md#long-term-order), decided): no self-hosting work before the foundation gate passes, and no backend replacement or tuning of generated code before self-hosting is done. The performance work that belongs to the foundation is listed in section 7.5. A new domain waits only for what it depends on.

**Measure the goal early**

- Run the agent benchmark of [`success-workload`](design-review.md#success-workload) at the end of M1, the first milestone whose programs run end to end, and again at the end of every milestone, within the budget Norman sets. A result against the goal is worth most while the design can still change cheaply.

**Before every commit**

- Run the four verification commands in `CLAUDE.md`.
- Keep repository text in English (`test_repository_text_is_english`).
- Do not write counts of files, directives or tests into documents.

**Ask Norman first** before:

- changing a design decision;
- changing the roadmap's scope or order;
- deciding an `open` or `proposed` entry of `docs/design-review.md`.

**Reporting**

- Each report says what you verified and how, and what you did not do.

**Windows**

- With Smart App Control on, newly built unsigned executables (test binaries, `mn build` output) are blocked with os error 4551. Norman turned it off on 2026-11-06.

## 6. Where to start (M0a, M0b and M1)

[`foundation-milestones`](design-review.md#foundation-milestones), decided on 2026-11-08, replaces the phases with milestones. `ROADMAP.md` lists each with its steps, and section 7.7 below holds their exit criteria.

**M0a: documentation tools, CI and Rust hygiene**

- Wire `@spec-family` with exactly pinned versions: `@spec-family/spec-guard` 0.32.0, `@spec-family/spec-graph` 0.23.1 and `@spec-family/spec-brief` 0.13.0. Run them only through the npm scripts, or with `npx --no-install` and the full `@spec-family/` name. Never patch `node_modules` or the tools' repository; write down a false positive or a missing feature and report it to Norman.
- `.spec-guard.json` declares the `.mn` comment syntax (`"comments": { ".mn": "c-like" }`), so no change to the tool is needed, and configures the claim check: an `Implemented` claim needs test evidence, named with the prefix `fn test_`. `.spec-graph.json` adds claims, checklists (`ROADMAP.md`), the ledger (`docs/STATUS.md`), the register (`docs/design-review.md`), the measurement protocols and orientation; `.spec-brief.json` maps the labels and the settled register statuses. `ROADMAP.md`, under M0a, has the exact files.
- `npm run verify:docs` runs `spec-guard`, `spec-guard prove`, `spec-guard claims`, `spec-graph check` and `spec-brief lint --strict`, and runs in CI beside the four commands of `CLAUDE.md`.
- Rewrite every claim in `spec/*.md` in the tools' convention, and resolve every finding with a test seen to fail under `cargo-mutants`, or with a lower label ([`implemented-evidence`](design-review.md#implemented-evidence)). Remove the `mente-codegen` placeholders ([`codegen-placeholders`](design-review.md#codegen-placeholders)). Link every `done` entry of the register to the documents it changed, with its commit.
- A trial run on `bdffc1c` with this brief and the new register found 9 claim findings and 48 graph errors, all in text written before the convention; M0a fixes them.
- Keep the Node tooling to these three tools. `mn spec` stays as the Rust-side check, and rejects a directive that cannot fail.

**M0b: design closure for M1**

- The core calculus and the normative grammar for M1.
- The agent-benchmark tasks, each marked with the earliest milestone that can run it and as core-only or not, and the control language ([`success-workload`](design-review.md#success-workload)).
- The two spikes M1 depends on: Tier 1 against its latency protocol, and debug and unwind information through Cranelift.

**M1: the thin slice**, in three checkpoints:

1. Parse and check, run on the reference interpreter, with `mn check --watch`.
2. Refinements and Tier 1, with bounds erasure.
3. MIR, the MIR interpreter, and native code through Cranelift, with the three execution paths compared.

Each checkpoint lands with its probes as tests and its spec labels raised in the same commit.

- Decide [`foreign-idiom-diagnostics`](design-review.md#foreign-idiom-diagnostics) before checkpoint 1 ([`diagnostics-as-api`](design-review.md#diagnostics-as-api) is decided), [`tier1-loops`](design-review.md#tier1-loops), [`tier1-counterexample`](design-review.md#tier1-counterexample), [`tier2-trusted`](design-review.md#tier2-trusted) and [`gradual-verification`](design-review.md#gradual-verification) before checkpoint 2, and [`debug-info`](design-review.md#debug-info) before checkpoint 3. [`float-square-ensures`](design-review.md#float-square-ensures) belongs to M5.

## 7. The long-term plan

Norman decided the order of the work on 2026-11-06 ([`long-term-order`](design-review.md#long-term-order)): build a foundation solid enough to write the compiler in, then self-host, then replace Cranelift with a backend written in Mente and make the code it generates fast. New domains are planned later, and each starts once what it depends on exists, not after the performance stage. `ROADMAP.md` follows this order since 2026-11-08. Besides the order itself, the milestones of section 7.7 ([`foundation-milestones`](design-review.md#foundation-milestones)) and the gate of section 7.3 ([`foundation-gate`](design-review.md#foundation-gate)) are decided; the rest of this section is proposed.

| Stage | What it holds | Ends when |
| :--- | :--- | :--- |
| 1. Foundation | Milestones M0a, M0b and M1 to M9 (section 7.7, [`foundation-milestones`](design-review.md#foundation-milestones)): Phases 0 to 4, reordered; Tier 2 ([`tier2-phase`](design-review.md#tier2-phase)); the standard library and packages; the agent-facing tools of `docs/design-review.md` section 10; the foundation's share of performance work (section 7.5) | The foundation gate passes ([`foundation-gate`](design-review.md#foundation-gate)) |
| 2. Self-hosting | The compiler's front end, checker, MIR and driver translated into Mente, phase by phase ([`self-hosting-plan`](design-review.md#self-hosting-plan)) | The bootstrap fixed point and diverse double-compiling pass, the Mente compiler has been the default for a release, and the Rust front end is retired |
| 3. Performance | A native backend written in Mente that replaces Cranelift, and the optimisations measurements call for ([`performance-stage`](design-review.md#performance-stage)) | The new backend is the default, and each workload meets the target its protocol sets |
| 4. New domains | GPU through `@kernel`, the browser and WebAssembly, scripting, interop with machine-learning libraries, `mn bind` | Planned later, each from an entry of its own; each starts once what it depends on exists |

### 7.1 Why this order

- Self-hosting writes the largest Mente program there will be. On a foundation that still moves, every design change would be paid for twice, in the language and in the compiler written in it; on one that still holds unproven claims, a compiler bug and a language bug look the same. So the foundation is finished and checked first.
- Performance work needs a real workload and a backend Mente controls. After self-hosting, the compiler itself is that workload, and a backend written in Mente can do what Cranelift cannot: vectors wider than 128 bits, and the Carry-Flag ABI. Replacing Cranelift also completes self-hosting: from then on, building the compiler needs no Rust toolchain. Only the verification tools still do, since they stay in Rust on purpose (section 7.4).
- The self-hosted compiler and the standard library become the largest body of idiomatic Mente, which the language card and agents can draw examples from ([`language-card`](design-review.md#language-card)).
- The compiler's speed is not deferred with the rest of performance. It is the agent's feedback loop, so its latency targets hold at every stage, self-hosting included.
- New domains wait only for what they depend on. A script is the program agents write most often, so scripting and `mn bind` do not wait for a faster backend; only WebAssembly needs the performance stage's emitter.

### 7.2 Tier 2 in the foundation

- `mn prove` writes each obligation as SMT-LIB 2 and runs an external solver process, so no solver is linked and no C++ enters the build. The solver and its version are configured and recorded.
- Results are cached under the obligation's digests and the solver version. `mn check` reads the cache and never runs a solver, so it stays predictable; a missing or stale entry is an error that names `mn prove`.
- The cache is part of the source tree and is committed with the code, so `mn check` gives the same answer on every machine and in CI with no solver installed.
- A solver answer of unknown, or a timeout, is an error, never a pass.
- Tier 2 lets a signature promise more than octagons can state (`is_sorted(items)`, products, quantifiers), which widens what a reviewer can trust from signatures alone. It also proves `midpoint` and index-arithmetic `parallel_for` partitions, which then stop being trusted axioms.

### 7.3 The foundation gate

Self-hosting starts only when all of these hold, each with its evidence (decided as [`foundation-gate`](design-review.md#foundation-gate) on 2026-11-08):

1. Milestones M0a to M9 (section 7.7) are done. Every `open` or `proposed` entry of `docs/design-review.md` names the later stage it belongs to; an entry that names none counts as foundation, and none of those may remain.
2. Every exit criterion of section 7.7 holds over the whole foundation, not only over the last milestone.
3. Every spec claim inside the foundation is `Implemented` under the evidence rule, and `npm run verify:docs` passes, `spec-guard claims` included. Every `Hypothesis` is explicitly outside the foundation.
4. The fuzzing, generator and soundness campaigns have run for the length their protocols set, with every finding fixed and none left open. A proof that execution contradicts is a compiler bug, never a flaky test.
5. One real program that was not written for the test suite, `mn fmt`, is written in Mente and in use.
6. [`success-workload`](design-review.md#success-workload) meets a target Norman sets before it is measured for the gate, [`zero-cost-measured`](design-review.md#zero-cost-measured) has dated records, and performance baselines against C and Rust are recorded. The gate sets no speed target for generated code.
7. An independent review of the spec and the core calculus, by someone who did not write them, has no finding left open.
8. A stability policy says which language changes are allowed after the gate, and how existing code migrates. The first stable version of the documentation site is published with it ([`docs-site`](design-review.md#docs-site)).
9. Every check runs automatically: on every pull request where it fits the CI time, and otherwise on a schedule or at each milestone's end, as the agent benchmark and long fuzzing campaigns do. A check that runs only by hand does not count.

### 7.4 Self-hosting

Planned as [`self-hosting-plan`](design-review.md#self-hosting-plan). Stage 2 is far off, so its details are re-planned at the foundation gate with the evidence stage 1 produces. What is fixed now is its direction, and the work stage 1 must do so that stage 2 is a translation rather than a rescue.

#### What stage 1 owes stage 2

- **Canonical forms for every phase's output.** Tokens, the AST with its digests, the typed AST with its diagnostics, and MIR each have a deterministic serialised form that can be compared byte for byte. Stage 1's determinism checks need them anyway; in stage 2 they let each phase be replaced alone and compared byte for byte with the Rust phase it replaces.
- **A translatable subset of Rust, enforced.** The principle of writing the Rust compiler the way Mente code would be written (section 7.7) becomes lints in CI: no lifetimes in structs, no language feature without a Mente counterpart, data held in index-based and `SoA` tables.
- **Verification tools stay in Rust.** The reference interpreter, the program generator and the conformance runner are never self-hosted. A self-hosted compiler checked only by itself would rest on a circular argument; the Rust reference interpreter stays an independent judge.
- **The runtime and the standard library are written in Mente** wherever the language allows, from the milestone that builds them. What cannot be, such as fiber context switches and system calls, forms a minimal platform layer, listed with the reason for each item.

#### How the compiler is rewritten

- **By translation, phase by phase**, mostly by agents, with differential testing as the judge. Go 1.5 translated its C compiler into Go with an automatic translator; whether Mente builds one is decided at the gate.
- **The order:** `mente-syntax`; `mente-ast`, whose digests must match bit for bit, an easy first comparison; `mente-mir`, which has no dependencies and a clear spec; `mente-sema`, the largest and hardest, done when the most Mente experience exists; then `mente-cli`.
- **The boundary while the compiler is mixed.** In one process, phases exchange flat `SoA` tables through `extern "C"`, defined as `@layout` structs on the Mente side and mirrored with `#[repr(C)]` on the Rust side, so that Mente's own layout guarantee carries the boundary. Across processes, they exchange the canonical forms.
- **A phase replaces its Rust version only when** its output is byte-identical on the conformance suite, on generated programs and on the fuzzing corpus; it meets the latency targets and the memory baselines under the protocols in `ROADMAP.md`, because a compiler running on Cranelift's code may be slower than the Rust one built through LLVM, and a slower compiler slows every agent's feedback loop; and every exit criterion of the foundation still holds.
- **`mente-codegen` stays in Rust**, calling Cranelift behind an `extern "C"` boundary, until the performance stage replaces both with a backend written in Mente (section 7.5).

#### The compiler as Mente's showcase

- Every phase is pure; only the driver declares `with IO`.
- Refinements prove `NodeId` indices in bounds, so the compiler's own table accesses carry no bounds checks.
- `SoA` tables and arenas give it mechanical sympathy, and it is the largest real sample for [`cost-report`](design-review.md#cost-report) and [`zero-cost-measured`](design-review.md#zero-cost-measured).
- **A friction log.** Every workaround written around a limit of the language is logged, and none may hide a language bug behind `@checked`, `todo` or any other means. Each entry becomes an entry of `docs/design-review.md`, handled under the stability policy.
- The rewrite is the largest agent experiment Mente will have: its repair iterations and kinds of failure are recorded as evidence for the goal of [`ai-first-purpose`](design-review.md#ai-first-purpose).

#### Checking the bootstrap

- **Fixed point.** The Rust compiler (stage 0) builds the Mente compiler (stage 1), stage 1 builds stage 2, stage 2 builds stage 3, and stages 2 and 3 are bit-identical.
- **Diverse double-compiling** (David A. Wheeler's method). The same compiler source is compiled by two independent compilers, the Rust one and the Mente one; each result then compiles the source again, and the two final results must be bit-identical. This rules out an error or a back door that copies itself from one compiler generation to the next.
- **The compiler's source uses only what the previous release has `Implemented`**, so that release N always compiles with release N−1 and the bootstrap chain never breaks.

#### Retiring the Rust front end

- The Rust compiler stays the reference and the fallback until the Mente compiler has been the default for a release. Then the Rust front end retires; the Rust reference interpreter and the verification tools stay.
- From then on, each release keeps a pinned binary of the release before it as its stage 0.
- How a second, independent path for diverse double-compiling survives the Rust front end's retirement is decided at the end of the stage.

#### Re-planning points

- After the first phase, the parser, the cost of translation and the friction log are reviewed.
- **A Mente phase misses a latency target.** Its algorithms and data layout are examined first. If that is not enough, specific stage-3 work may be brought forward, which changes the decided order and therefore needs Norman.
- **Translation needs a change to the language.** It goes through the stability policy, never around it in the compiler's code.

#### Stage 2 ends when

1. Every phase is replaced, and the fixed point and diverse double-compiling pass.
2. The Mente compiler has been the default for a release, and the Rust front end is retired.
3. The only code of the compiler not written in Mente is the listed platform layer and `mente-codegen` with Cranelift, which stage 3 replaces; the verification tools stay in Rust on purpose.
4. Every entry of the friction log is handled.
5. Every exit criterion of the foundation still holds.

### 7.5 Performance

Performance work is split by what it depends on. Five kinds belong to the foundation, because the language promises them, or because changing them after self-hosting would mean rewriting the compiler:

- **Mechanical sympathy by default.** Defaults that fit the hardware ([`sympathetic-defaults`](design-review.md#sympathetic-defaults)), and a cost report that shows agents and reviewers what their code costs ([`cost-report`](design-review.md#cost-report)), from M2 on.
- **The compiler's own speed.** It is the agent's feedback loop, so the Tier-1 re-check and projection targets hold at every stage.
- **The performance semantics the spec promises** (Guardrail #2): bounds checks erased where Tier 1 proves them, contracts as MIR assumptions, `@branchless` selects, `@layout` and `Tiled` offsets, inlining of `@inline` and generic callees, and `@simd` through Cranelift's 128-bit vectors. Without them, "proven and erased" is an empty promise.
- **Tight loops** ([`tight-loops`](design-review.md#tight-loops)). Loop optimisations in MIR (moving invariant work out of loops, strength reduction, induction-variable simplification, unrolling), each rewrite rule proven by an SMT solver before it is admitted; and vectorisation by contract instead of a heuristic auto-vectoriser, with [`cost-report`](design-review.md#cost-report) telling an agent why a hot loop is not vectorised and what would vectorise it. Both arrive in M5, and a tight-loop benchmark against C, C++ and Rust records the gap kernel by kernel from M1. They work on MIR, so Cranelift gains at once and stage 3's backend keeps them; only the 128-bit vector ceiling waits for stage 3.
- **Decisions that set performance and are expensive to change:**
  - data representation and the ABI: the two-register `T ! E`, `mut` passed by pointer, the layout of `String`, closures and `dyn`, monomorphised generics;
  - [`value-copy-cost`](design-review.md#value-copy-cost), fibers, arenas and reference counting;
  - when and where a trap occurs: whether intermediate results are computed exactly and checked only where a value is narrowed, and whether overflow checks may be merged and deferred within regions that have no externally visible effect ([`overflow-check-cost`](design-review.md#overflow-check-cost)), semantic decisions made with the core calculus in M0;
  - a MIR that keeps every fact the language proves (the absence of aliasing that the Law of Exclusivity guarantees, purity from effects, intervals, provenance, layouts, lane counts and contracts) and does not bake in Cranelift's limits (`@simd` stays a lane count until lowering). A fact lost before MIR is one no backend can use, and these facts are what stage 3's backend has that LLVM does not;
  - performance baselines from M1 on, which may not regress, recording hardware counters beside time where the platform exposes them: cache misses, branch mispredictions, instructions per cycle.

The performance stage (stage 3) holds the rest:

- Until self-hosting is done, a figure about the speed of generated code is a baseline, never a claim.
- Workloads: the self-hosted compiler, the Menteion kernel of [`success-workload`](design-review.md#success-workload), and benchmarks against C and Rust, each under a protocol in `ROADMAP.md`.
- The stage replaces Cranelift with a native backend written in Mente, for x86-64 and AArch64 (decided in [`long-term-order`](design-review.md#long-term-order)). The new backend becomes the default only when it gives the same results as Cranelift on the whole corpus, neither its generated code nor its compile latency is worse under the protocols, and its debug, unwind and profiler information is at least as complete ([`debug-info`](design-review.md#debug-info)); until then Cranelift stays the default.
- The invariants in `CLAUDE.md` and `mir-and-codegen.md` that name Cranelift change in the same commit that makes the new backend the default. Until [`long-term-order`](design-review.md#long-term-order) is applied, `mir-and-codegen.md` still calls this backend a `Hypothesis`, pursued only if Cranelift is measured short.
- Beyond the replacement, each change is made only where a measurement shows the need: inlining and vectorisation in MIR beyond what [`tight-loops`](design-review.md#tight-loops) does in stage 1, 256- and 512-bit vectors, and the Carry-Flag ABI experiment on the new backend.

**What the backend aims at.** The aim is the best backend for Mente, not a better LLVM. Each dimension has its own target, measured against equivalent C, C++ and Rust programs compiled through LLVM, program against program rather than backend against backend:

| Dimension | Target |
| :--- | :--- |
| Predictability; guarantees (`@simd`, `@branchless`, `@no_alloc`, `@max_stack`); verifiability; compile speed | Surpass LLVM |
| Ordinary code whose speed is set by memory, aliasing and allocation; hot code that carries contracts | Match or beat the LLVM-compiled equivalent |
| Tight loops whose speed is set by instruction-level optimisation | Keep the gap to LLVM measured, and narrow it: in stage 1 through [`tight-loops`](design-review.md#tight-loops), in stage 3 through vector width and scheduling |
| Breadth of targets; depth of general optimisation (decades of pass tuning, per-microarchitecture scheduling models) | Not a goal |

How:

- **The facts MIR keeps** (above). Freedom from aliasing is a language rule that Mente proves, not an assumption handed to the backend, and bounds checks are erased by proof, not by a heuristic range analysis.
- **No undefined behaviour.** Every failure is a defined trap, so the backend needs no poison or undef semantics, and is smaller and easier to verify for it.
- **Every rewrite rule proven before it is admitted**, by an SMT solver through Tier 2's machinery, in the manner of Alive2, which has found many errors in LLVM's own rewrites.
- **A fast tier and an optimising tier**, built from the same MIR, so that the check-and-test loop stays fast while release builds optimise.
- **Missed optimisations reported** through [`cost-report`](design-review.md#cost-report), in a form an agent can act on: a loop that was not vectorised, a check that was left in, and why.
- **LLVM as a measuring stick only.** Benchmarks compare against `clang` and `rustc` output; LLVM is never linked.
- **The cost of trapping on overflow is measured as a figure of its own**, since C and Rust release builds do not pay it. Exact intermediate results, Tier 1's intervals, imprecise trap regions, loop versioning and the cost report's prescriptions are the means of reducing it ([`overflow-check-cost`](design-review.md#overflow-check-cost)).

### 7.6 New domains

Planned later, each from an entry of its own in `docs/design-review.md`, and each may start as soon as what it waits for exists:

- GPU: an execution model for `@kernel`, then SPIR-V.
- The browser and WebAssembly: an emitter that produces WebAssembly, which Cranelift does not, so it builds on the backend of the performance stage.
- Scripting: the interpreter, with fast start-up and a command that runs a single file.
- Interop with machine-learning libraries: `mn bind`.
- `mn bind` itself: C headers, and Rust libraries through `cbindgen` headers, once the accepted C dialect is written down.

### 7.7 The foundation, milestone by milestone

Decided as [`foundation-milestones`](design-review.md#foundation-milestones) on 2026-11-08, with M0 split into M0a and M0b and the spikes moved to just before the milestones that need them; it settled [`roadmap-vertical-slice`](design-review.md#roadmap-vertical-slice) and [`roadmap-ai-priorities`](design-review.md#roadmap-ai-priorities). Below, "M0" means M0a and M0b together unless one is named. The aim is the strongest foundation that can be built: designed before it is coded, checked by independent means at every step, with every check automated, so that a weak part is found while it is cheap to fix rather than at the gate.

#### Principles

- **Design before code.** A milestone starts only when the register entries it names are `decided`, and when the core calculus (below) covers what it builds.
- **Riskiest first.** A mechanism whose feasibility is uncertain is tried early, in a spike, so that a dead end is found while it is cheap.
- **Verification before features.** M0 builds the quality system below and runs every instrument in CI, so each feature lands with its checks and no check depends on someone remembering to run it.
- **Independent evidence.** A behaviour counts as checked only when something other than its own implementation confirms it: a second interpreter, a solver, a generated program, an exhaustive enumeration, a test written from the spec.
- **Always runnable.** From the end of M1 on, every widening keeps the whole pipeline running, from parsing to native code; inside M1, each checkpoint keeps what exists so far running.
- **Intersections before features.** A feature is finished only when its rules with every feature already built are written and tested.
- **Mechanical sympathy throughout.** Every feature is designed with its hardware cost in view, fits [`sympathetic-defaults`](design-review.md#sympathetic-defaults), and reports its cost through [`cost-report`](design-review.md#cost-report).
- **One milestone in flight.** Work merges in small pull requests that keep every check green; the next milestone starts when the last one's exit criteria hold.
- **Write the Rust compiler the way Mente code would be written:** index-based structures such as `NodeId`, no designs that lean on lifetimes, module boundaries that match the future Mente modules, and an incremental, query-based architecture from M1, because the Tier-1 latency target and a language server need it and it cannot be added later. Lints in CI keep the code inside that translatable subset from M0 on (section 7.4). Self-hosting is then a translation, not a redesign.

#### Design closure

- **A core calculus.** Before M1, write the static and dynamic semantics of the core language precisely: the typing and refinement rules, parameter modes and exclusivity, provenance, and evaluation. Before each later milestone, extend it to what that milestone builds. Design entries are decided against it, so a decision that breaks another shows up on paper first. Where it lives, a spec file or a document beside `spec/`, is decided in M0.
- **A normative grammar.** The surface grammar is written in EBNF. The parser accepts exactly that grammar, and the fuzzer and the program generator are driven from it.
- **A mechanised model** of the core calculus, in Lean for example, is the strongest evidence of soundness and also the most expensive. Whether to build one is decided in M0. If it is built, it is a real proof checked in CI, never an export that only looks like one.

#### The quality system

Each instrument is switched on in the milestone where it first has something to check, and extended by every milestone after it. M0 builds only what M0 can already use, so that it does not become a long stretch of infrastructure with no language in it. The instruments are written in Rust and stay there for good, so that they remain independent of the self-hosted compiler (section 7.4).

| Instrument | What it catches | Active from |
| :--- | :--- | :--- |
| A conformance suite, organised by spec file, of programs that must pass and programs that must fail with a given code. It does not depend on the implementation, so the self-hosted compiler later runs it unchanged | Behaviour that drifts from the spec | M0 (the runner, on today's declaration checks) |
| An error-code catalogue with `mn explain`, golden diagnostics in text and JSON, and every machine-applicable fix applied in a test and the result checked again | Diagnostics that mislead, and fixes that do not fix | M0 (the catalogue of today's codes), M1 (`mn explain`) |
| Three execution paths: a reference interpreter over the typed AST, the MIR interpreter, and native code, compared on every program; a trap anywhere in the same trap region counts as agreement ([`overflow-check-cost`](design-review.md#overflow-check-cost)). Whether the reference interpreter is a crate of its own, which changes the crate list in `CLAUDE.md`, is decided in M0 | A lowering bug (reference against MIR) and a code-generation bug (MIR against native), where two paths that both start from MIR would agree on the same wrong MIR | M1, checkpoint (c) |
| A generator of well-typed programs, in the manner of Csmith, and grammar-driven fuzzing of the parser and the checker. The generator starts narrow, with programs that carry no refinements or guard every obligation with `guard`, and widens with each milestone; generating programs that pass Tier 1 and the Law of Exclusivity is a project of its own and is not attempted at once | Miscompilations and crashes that hand-written tests miss | M1 (fuzzing at checkpoint (a), the generator at (c)) |
| Soundness by execution, exhaustive over small integer types and random otherwise; and every obligation Tier 1 discharges cross-checked in CI by an SMT solver installed on the runner, never in the build | A proof that is wrong | M1, checkpoint (b) |
| Mutation testing at two levels: `cargo-mutants` on the Rust compiler itself, and the MIR gate on lowered code | Tests that check nothing, in the compiler as well as in user code | M0 (`cargo-mutants`), M1 (the MIR gate on lowered code) |
| A CI matrix of x86-64 Linux, x86-64 Windows and AArch64, with every output compared across runs and platforms | Non-determinism, and results that differ by platform; fixed point must be bit-identical everywhere | M0 |
| Latency, performance and hardware-counter benchmarks, each with a protocol and a regression tolerance; and a tight-loop benchmark of typical kernels, each with C, C++ and Rust equivalents, recording the gap kernel by kernel | A slower compiler, slower generated code, and where tight loops trail | M0 (projection, which exists), M1 (Tier 1 and generated code) |
| The agent benchmark: tasks fixed before measuring, a held-out set the language card is never tuned on, several runs per task with the variance reported, and adversarial tasks, such as smuggling a network call into a pure function, that must fail | A goal measured on tasks chosen to flatter it | M1 (tasks fixed in M0) |
| Rust hygiene: `rustfmt`, `clippy` with warnings as errors, `unsafe` forbidden outside the runtime and FFI code, a dependency policy, coverage reported | Weak code that the self-hosting translation would inherit | M0 |

#### Spikes before the milestone that needs them

Each risky mechanism gets a small experiment before the milestone that depends on it. A spike lives outside the compiler crates, is tested, and is never cited as evidence for a claim; what it finds is written into the register.

- `@branchless` code through Cranelift on x86-64 and AArch64, with the machine-code check ([`cranelift-float-select`](design-review.md#cranelift-float-select)): before M5.
- Copy-in/copy-out of `SoA` and `Tiled` elements, lowered through MIR and Cranelift: before M3.
- Tier 1 on the generated module of the Tier-1 protocol in `ROADMAP.md`, against its latency target: in M0b.
- Fiber stacks reserved and switched on Windows and Linux, with guard pages: before M7.
- A tail-resumptive effect handler compiled to a direct call: before M4.
- `T ! E` returned in two registers through Cranelift's multi-value returns: before M2.
- Debug and unwind information through Cranelift: DWARF on Linux, PDB and CodeView on Windows, readable by gdb, lldb, WinDbg and perf ([`debug-info`](design-review.md#debug-info)): in M0b.

#### Milestones

| Milestone | What it holds | Entries decided first | Runs in full at the end |
| :--- | :--- | :--- | :--- |
| M0a Preparation | `@spec-family` pinned, every claim rewritten in the tools' convention and every finding resolved; the codegen placeholders removed; CI (GitHub Actions; the repository has none yet) on every pull request across the platform matrix; the instruments of the quality system that are active from M0; the lints that keep the Rust compiler inside a translatable subset | [`ai-first-purpose`](design-review.md#ai-first-purpose), [`foundation-milestones`](design-review.md#foundation-milestones), [`foundation-gate`](design-review.md#foundation-gate), [`codegen-placeholders`](design-review.md#codegen-placeholders), [`implemented-evidence`](design-review.md#implemented-evidence), [`diagnostics-as-api`](design-review.md#diagnostics-as-api), [`success-workload`](design-review.md#success-workload), [`overflow-check-cost`](design-review.md#overflow-check-cost) (all decided 2026-11-08) | — |
| M0b Design closure | The core calculus and the grammar for M1; the agent-benchmark tasks, fixed before anything is measured, each marked with its earliest milestone and as core-only or not, and the control language; the two spikes M1 needs | — | — |
| M1 Thin slice | Integers, `bool`, fixed-size arrays `[N]T` (values, so no provenance yet), functions with scalar parameter modes, `if`, `while` with `break` and `continue`, and `guard`, as `spec/core-calculus.md` defines them; refinements and Tier 1 with counterexamples and `guard` prescriptions; intermediate results computed exactly and checked where they are narrowed; bounds and overflow checks erased where Tier 1 proves them, in refined and unrefined code alike; `todo`; the body parser (Pratt, the C#-style type-argument rule, error recovery, dedicated errors for other languages' idioms), with the syntax probe of [`syntax-probe`](design-review.md#syntax-probe) as soon as it parses; the incremental architecture, with `mn check --watch`, in which the Tier-1 latency protocol is measured; canonical serialised forms of every phase's output; all three execution paths; `mn explain`; the language card, first version; the first agent-benchmark run; the first baselines, with hardware counters, and the first gaps of the tight-loop benchmark; the documentation site, first published under the milestone's tag. Built in three checkpoints, each keeping every active check green: (a) parse and check, run on the reference interpreter, with `mn check --watch`; (b) refinements and Tier 1, with bounds erasure; (c) MIR, the MIR interpreter and native code carrying line tables, symbols and unwind information, with the three paths compared | [`foreign-idiom-diagnostics`](design-review.md#foreign-idiom-diagnostics), [`tier1-loops`](design-review.md#tier1-loops), [`tier1-counterexample`](design-review.md#tier1-counterexample), [`tier2-trusted`](design-review.md#tier2-trusted), [`gradual-verification`](design-review.md#gradual-verification), [`language-card`](design-review.md#language-card), [`todo-marker`](design-review.md#todo-marker), [`debug-info`](design-review.md#debug-info), [`docs-site`](design-review.md#docs-site) | The slice's conformance programs |
| M2 Data, views and exclusivity | `struct`, `enum` with exhaustive `match`, `?T`, `T ! E` with `raise`, `!` and `catch`; slices and `str` as views with provenance, `from` checked in bodies, and `E0310`, in the same milestone so that no view ever goes unchecked; every parameter mode, with the Law of Exclusivity, `own` invalidation and copy-in/copy-out; `List<T>` and `String`; the cost report begins: type layouts, and the runtime checks Tier 1 left in place, each with the contract or refinement that would remove it | [`error-conversion`](design-review.md#error-conversion), [`value-copy-cost`](design-review.md#value-copy-cost), [`sympathetic-defaults`](design-review.md#sympathetic-defaults), [`cost-report`](design-review.md#cost-report) | Example 01's binary search |
| M3 Regions and layouts | Arenas with a chunk per task, `Pool<T>` / `Handle<T>`, `Local<T>` / `Shared<T>`; `@layout` structs and `bitstruct`s in native code; `List`, `SoA` and `Tiled` behind one syntax, with copy-in/copy-out projections; the cost report adds copies and allocations | — | Example 04's pool |
| M4 Abstraction, effects and FFI | Monomorphised generics, traits with coherence, `dyn`, closures with their capture modes; capability effects, effect variables, `handle` blocks, no global mutable state, the per-module effect summary; `extern "C"` with `with FFI`; the `comptime` interpreter under its step budget; the cost report adds `dyn` calls | [`trait-coherence`](design-review.md#trait-coherence), [`closure-captures`](design-review.md#closure-captures), [`capability-trust`](design-review.md#capability-trust) | Example 04 |
| M5 Numerics and hardware contracts | IEEE floats in refinements (outward rounding, `NaN`), units with scales and `.to<U>()`, fixed point and `dec`, `sat` and `wrap` with their operators, combined scalar types, bit-identical across the CI matrix; `@branchless` with `E0403`, `@simd` through Cranelift's 128-bit vectors, `@no_alloc`, `@max_stack`, and `@total` measures checked; overflow checks merged within trap regions, vector loops included, and hoisted out of loops by loop versioning; MIR loop optimisations (invariant code motion, strength reduction, induction-variable simplification, unrolling), each rewrite rule proven by SMT; vectorisation by contract, with the cost report's prescriptions for hot loops that are not vectorised | [`combined-scalar-types`](design-review.md#combined-scalar-types), [`float-square-ensures`](design-review.md#float-square-ensures), [`sat-mixing`](design-review.md#sat-mixing), [`no-alloc-task-stacks`](design-review.md#no-alloc-task-stacks), [`zero-cost-measured`](design-review.md#zero-cost-measured), [`tight-loops`](design-review.md#tight-loops) | Example 01; example 02's `integrate_cell` |
| M6 Standard library and agent tools | Text, collections chosen for their cache behaviour, files, time, processes, and JSON through `comptime`; packages; mutation testing of user code; impact analysis; the language card complete, with `mn guide` and `mn api`; `mn fmt`, written in Mente; a getting-started guide and the standard library's API documentation, generated from signatures, on the documentation site | [`stdlib-scope`](design-review.md#stdlib-scope), [`user-mutation-testing`](design-review.md#user-mutation-testing), [`impact-analysis`](design-review.md#impact-analysis) | `mn fmt`, in use; the internal preview |
| M7 Concurrency and simulation | Fibers on reserved stacks that debuggers and profilers can walk, `parallel (...)`, `parallel_for` over structural partitions, `Sched` with deterministic simulation (`SimClock`, `SimNet`, `SimSched`), the memory model, networking and HTTP; the cost report adds atomic operations and cache lines shared between tasks | [`atomics-memory-ordering`](design-review.md#atomics-memory-ordering) | Examples 02 and 03 |
| M8 Tier 2 | `mn prove` with an external solver, the proof cache in the source tree, `midpoint` proven, `@prove`, index-arithmetic partitions | [`tier2-phase`](design-review.md#tier2-phase) | Example 01 with no trusted axiom left |
| M9 Hardening and gate | Long fuzzing, generator and soundness campaigns; the stability policy; a tutorial for people, the first stable documentation, and an entry point for language models such as `llms.txt`; the agent-benchmark target; an independent review of the spec and the core calculus by someone who did not write them (a person, or a fresh agent session given only the spec); every condition of section 7.3 | — | Every example and every `@example` |

#### Versions inside the foundation

- **Every milestone ends with a version tag**, so that every benchmark result and baseline names the version it measured. The tag is the milestone's name in lower case, such as `m1`, on the merge commit that ends it ([`milestone-tags`](design-review.md#milestone-tags)). A tag is not a release, and nothing is promised about it.
- **The end of M6 is the first usable Mente:** an internal preview, marked unstable, with no promise of compatibility. It is used for Norman's own tools and for agent tasks beyond the benchmark, because real use finds what fixed benchmark tasks do not. M6 is the earliest point at which real programs can be written, since files, time and processes arrive with the standard library.
- **There is no public release, and no separate minimum-viable track, before the gate.** Users before the design settles would either freeze its mistakes or have to follow its changes; the stability policy begins at the gate (section 7.3), and until then the language may still change.

#### Re-planning points

The plan is a hypothesis, and the spikes and the first agent-benchmark runs can overturn parts of it. So after M0, after M1, and after every later milestone, it is reviewed against the evidence: the spikes' findings, the agent-benchmark results, the latency and performance baselines, and how long the milestone took. These triggers are written down in advance, so the evidence decides rather than hindsight:

- **A spike fails.** The design entry it tested is reopened, and the milestone that depends on it does not start until the entry is decided again. If `@branchless` code cannot be kept free of conditional jumps on Cranelift, for example, [`cranelift-float-select`](design-review.md#cranelift-float-select) and the definition of `@branchless` are revisited.
- **Agents fail mostly on proof errors.** [`gradual-verification`](design-review.md#gradual-verification) and Tier 1's diagnostics are revisited before refinements are widened further.
- **Agents fail mostly on invented APIs or other languages' habits.** [`language-card`](design-review.md#language-card) and [`foreign-idiom-diagnostics`](design-review.md#foreign-idiom-diagnostics) are strengthened before the next milestone.
- **The Tier-1 latency target is missed.** The incremental architecture is fixed before any new feature lands.
- **A milestone takes far longer than the ones before it.** It is split, and the order of the remaining milestones is reconsidered.
- **Real use of the M6 preview finds a gap the benchmark missed.** It enters the register, and is weighed before M7 starts.

Each review is recorded in [`docs/milestone-reviews.md`](milestone-reviews.md). A change that follows from a review enters `docs/design-review.md` as an entry, and the plan changes only with Norman.

#### Every milestone ends when

Each criterion applies from the milestone in which the instrument it uses becomes active (the quality-system table above).

1. The spec changed first: new behaviour entered `spec/` as `Specified` with its directives, and labels rose to `Implemented` only with their tests, in the commit that justifies them.
2. Every new spec claim has conformance programs, both ones that must pass and ones that must fail.
3. Every new error code has a catalogue entry with its `mn explain` text, golden output in text and JSON, a program that must fail with it and a near-miss that must pass, and a test that applies each machine-applicable fix and checks the result again.
4. The reference interpreter, the MIR interpreter and native code agree on the conformance suite and on generated programs, on every platform of the CI matrix.
5. Soundness by execution finds no reachable trap that Tier 1 or Tier 2 proved unreachable, and the SMT cross-check agrees with every obligation Tier 1 discharged.
6. The grammar and the program generator cover the new syntax, and fuzzing finds no crash.
7. The MIR mutation gate covers the new MIR, and `cargo-mutants` on the changed compiler code leaves no survivor without a written reason.
8. Compiling twice gives identical output, on every platform.
9. The latency targets hold, and no baseline, of time or of hardware counters, regresses beyond the tolerance its protocol sets.
10. From M2 on, the cost report covers what the milestone added.
11. The language card covers what the milestone added, its snippets compile, and the agent benchmark has run again with its result recorded.
12. `docs/STATUS.md` changed in the same commit as the behaviour it describes.
13. The Rust hygiene checks pass, the translatable-subset lints included, and whatever the milestone added to the runtime or the standard library is written in Mente, or is listed in the platform layer with its reason.
14. Every check above runs automatically: on every pull request where it fits the CI time, and otherwise on a schedule or at the milestone's end. A check that runs only by hand does not count.
15. Every fact the milestone's features prove reaches MIR, and a test reads it back there.
16. From M1's checkpoint (c) on, native code carries debug, unwind and profiler information for what the milestone added, and a scripted debugger session in CI steps through the new constructs and checks their source lines.
17. The milestone's version is tagged, and every result it recorded names that tag.
18. From M1, the documentation site is rebuilt from the milestone's sources and published under its tag, marked pre-release, with the maturity of everything it describes shown.
