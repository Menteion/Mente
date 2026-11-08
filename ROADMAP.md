# Mente (`mn`) — Engineering Roadmap

**Status ledger:** [docs/STATUS.md](docs/STATUS.md)  
**Canonical specification:** [spec/README.md](spec/README.md) (one single-decision file per mechanism)  
**Design review:** [docs/design-review.md](docs/design-review.md), the register of design problems and how each was settled  
**Design brief:** [docs/design-brief.md](docs/design-brief.md), the design, the working rules and the long-term plan in full

The work runs in four stages, in the order [`long-term-order`](docs/design-review.md#long-term-order) set on 2026-11-06: (1) a foundation solid enough to write the compiler in, (2) self-hosting, (3) performance, with a backend written in Mente replacing Cranelift, and (4) new domains, each starting once what it depends on exists. Stage 1 is built in the milestones [`foundation-milestones`](docs/design-review.md#foundation-milestones) set on 2026-11-08, and ends when [`foundation-gate`](docs/design-review.md#foundation-gate) passes. A milestone starts when the register entries it names are `decided` and the one before it has met the exit criteria of [docs/design-brief.md](docs/design-brief.md) section 7.7; each ends with a version tag. Stages 2 to 4 are planned in outline only, and re-planned in detail when the stage before them ends.

---

## Done before M0

- [x] The specification is split into emoji-free, single-decision files in [`spec/`](spec/README.md), with no `DEC-XXX` or `Station` numbering; a directive in [spec/README.md](spec/README.md) keeps every file present (commit "Baseline: Mente as found on 2026-11-04").
- [x] Executable `<!-- @assert-* -->` directives across `spec/*.md`, run by `cargo run --bin mn -- spec`, which rejects a directive that cannot fail and ignores comments in `.mn` files ([`test_spec_directives_that_cannot_fail_are_rejected`](compiler/mente-cli/src/main.rs), [`test_spec_directives_on_mn_files_ignore_comments`](compiler/mente-cli/src/main.rs); commit "Keep comments from satisfying directives on .mn files").
- [x] The documents say what the compiler does: maturity labels, names and measured numbers corrected across `spec/`, [docs/STATUS.md](docs/STATUS.md) and the crate comments (commit "Say what the compiler does, and list the open design problems").
- [x] A zero-dependency `.mn` lexer and declaration parser in `compiler/mente-syntax` ([`test_lexer_and_parser_on_basic_mn`](compiler/mente-syntax/src/lib.rs), [`test_from_clause_names_return_provenance`](compiler/mente-syntax/src/lib.rs); commit "Read every interface fact from the signature").
- [x] Three 128-bit structural digests (`spec_hash`, `instance_hash`, `body_hash`) and the Holographic Spec Blueprint projection in `compiler/mente-ast` ([`test_private_field_change_changes_spec_hash`](compiler/mente-ast/src/lib.rs), [`test_instantiable_bodies_go_into_instance_hash`](compiler/mente-ast/src/lib.rs); commit "Read every interface fact from the signature"); the projection meets its median target under the protocol below.
- [x] The exact `@layout(size, align)` + `@bytes` prover, with field alignment and `@packed`, and the `bitstruct` + `@bits` prover, with field widths, in `compiler/mente-sema` ([`test_layout_fields_sit_at_aligned_offsets`](compiler/mente-sema/src/lib.rs), [`test_bitstruct_field_type_must_match_its_range`](compiler/mente-sema/src/lib.rs); commit "Check field alignment in @layout, and fix its byte order").
- [x] Return-view provenance read from the signature: the `from` clause is parsed, hashed and checked against the parameter list, `E0311` ([`test_from_clause_must_name_a_kept_parameter`](compiler/mente-sema/src/lib.rs); commit "Read every interface fact from the signature").
- [x] A straight-line `MirRegionSoA` (`NodeId(u32)`), its `ValueLattice` record, an in-memory interpreter and an opcode mutation gate in `compiler/mente-mir`, exercised on one hand-written region ([`test_branchless_integrate_cell_mir_and_mutation_prover`](compiler/mente-mir/src/lib.rs); commit "Make the checkers report what they check").
- [x] Repository text in English ([`test_repository_text_is_english`](compiler/mente-cli/src/main.rs); commit "Write every example in English, and keep the repository English").

---

## Stage 1: the foundation

### M0a: documentation tools, CI and Rust hygiene

**Decided first:** [`ai-first-purpose`](docs/design-review.md#ai-first-purpose), [`foundation-milestones`](docs/design-review.md#foundation-milestones), [`foundation-gate`](docs/design-review.md#foundation-gate), [`codegen-placeholders`](docs/design-review.md#codegen-placeholders), [`implemented-evidence`](docs/design-review.md#implemented-evidence), [`diagnostics-as-api`](docs/design-review.md#diagnostics-as-api), [`success-workload`](docs/design-review.md#success-workload), [`overflow-check-cost`](docs/design-review.md#overflow-check-cost), all decided on 2026-11-08.

- [x] **Step 0.1 (`package.json`)** (commit "Wire the @spec-family documentation tools"): a private package, `engines` node `>=22`, with devDependencies pinned exactly (no `^`): `@spec-family/spec-guard` `0.32.0`, `@spec-family/spec-graph` `0.23.1`, `@spec-family/spec-brief` `0.13.0`, and `package-lock.json` committed. Scripts: `spec-guard` over `"spec/*.md" "docs/*.md" README.md ROADMAP.md CLAUDE.md`, `spec-guard prove` over the same patterns, `spec-guard claims`, `spec-graph check`, `spec-brief lint --strict`, and `verify:docs` running all five in order. The tools run only through these scripts or `npx --no-install` with the full `@spec-family/` name. Add the `npm` ecosystem to `.github/dependabot.yml` in the same change.
- [x] **Step 0.2 (`.spec-graph.json`)** (commit "Wire the @spec-family documentation tools"):
  ```json
  { "patterns": ["spec/*.md", "docs/*.md", "README.md", "ROADMAP.md", "CLAUDE.md", "examples/README.md"],
    "strict": true,
    "ignoreReferences": ["LICENSE-*", "../LICENSE-*"],
    "claims": { "files": ["spec/*.md"], "labels": { "Implemented": "done", "Specified": "open", "Target": "open", "Hypothesis": "open" } },
    "checklists": ["ROADMAP.md"],
    "ledgers": ["docs/STATUS.md"],
    "registers": { "files": ["docs/design-review.md"], "resolution": "Proposed resolution",
      "statuses": { "open": {}, "proposed": { "needs": ["resolution"] }, "decided": { "needs": ["resolution"] }, "done": { "closes": true, "needs": ["citation", "commit"] } } },
    "protocols": { "files": ["ROADMAP.md"], "heading": "Measurement protocols", "result": "Last result",
      "fields": { "*": ["Timed", "Corpus", "Method", "Machine"] }, "claims": ["Target"] },
    "orientation": { "files": ["CLAUDE.md", "docs/design-brief.md"], "extensions": [".md", ".rs", ".mn", ".toml"], "roots": ["spec"] } }
  ```
- [x] **Step 0.25 (`.spec-guard.json`)** (commit "Wire the @spec-family documentation tools"): `.mn` comments are declared in the configuration, with no change to the tool, and the claim check is configured beside them:
  ```json
  { "comments": { ".mn": "c-like", "LICENSE-APACHE": "none" },
    "claims": { "files": ["spec/*.md"],
      "labels": { "Implemented": { "evidence": "test" }, "Specified": {}, "Target": {}, "Hypothesis": {} },
      "testPrefixes": ["fn test_"] } }
  ```
- [x] **Step 0.3 (`.spec-brief.json` and `briefs/`)** (commit "Wire the @spec-family documentation tools"): `spec-brief init`, then add `"claims": { "labels": { "Implemented": "done", "Specified": "open", "Target": "open", "Hypothesis": "open" } }` and `"registers": { "settled": ["decided", "done"] }`. A brief's front matter may name `raises: [spec/<file>.md#<claim id>]` and `awaits: [docs/design-review.md#<key>]`.
- [x] **CI** (commit "Run the verification commands and mutation testing on GitHub Actions"): GitHub Actions on every pull request, with every action pinned to a full commit SHA: a Rust job running the four `cargo` commands of `CLAUDE.md` on x86-64 Linux, x86-64 Windows and AArch64 Linux, and a documentation job running `npm ci` and `npm run verify:docs`. The repository's ruleset then requires both. Mutation testing runs there too, never on a laptop: `cargo mutants --in-diff` on each pull request, and a full run split across runners with `--shard` on a schedule and on demand, its `mutants.out` kept as an artifact.
- [x] Every claim in `spec/*.md` rewritten in the tools' convention ([`implemented-evidence`](docs/design-review.md#implemented-evidence)), and every finding of `spec-guard claims` resolved: a test that fails when the behaviour breaks, seen to fail under `cargo-mutants`, or a lower label (commit "Rewrite the spec's claims in the tools' convention, with test evidence").
- [x] The `mente-codegen` placeholders removed, with the sentences and directives that name them ([`codegen-placeholders`](docs/design-review.md#codegen-placeholders); commit "Remove the mente-codegen placeholders").
- [x] Every `done` entry of [docs/design-review.md](docs/design-review.md) links the documents it changed and names its commit (commit "Link every done register row to what it changed").
- [x] Diagnostics in the structured form of [`diagnostics-as-api`](docs/design-review.md#diagnostics-as-api), and the error-code catalogue of today's codes ([spec/diagnostics.md](spec/diagnostics.md); commit "Report every error as a structured diagnostic, with a catalogue").
- [x] The quality-system instruments active from M0: the conformance runner over today's declaration checks, `cargo-mutants` on the compiler, the projection benchmark with a regression tolerance, and a check that compiling twice gives identical output ([conformance/README.md](conformance/README.md); commit "Add the conformance suite, reproducible-output checks and the projection benchmark").
- [x] Rust hygiene: `rustfmt`, `clippy` with warnings as errors, no `unsafe` outside the runtime and FFI code, a dependency policy, coverage reported, and lints that keep the compiler inside a subset of Rust that translates to Mente (commit "Check format, lints, dependencies and coverage in CI").

### M0b: design closure for M1

- [x] The core calculus of M1's language, in a new normative spec file, [`spec/core-calculus.md`](spec/core-calculus.md) ([`core-calculus-home`](docs/design-review.md#core-calculus-home); commit "Write the core calculus of M1").
- [x] The normative EBNF grammar of M1's surface syntax, in a new spec file, [`spec/grammar.md`](spec/grammar.md) ([`grammar-home`](docs/design-review.md#grammar-home); commit "Write the grammar of M1's surface syntax").
- [x] The agent-benchmark tasks, fixed before anything is measured, each marked with the earliest milestone that can run it and as core-only or not, with a held-out set and adversarial tasks, in [`benchmark/`](benchmark/README.md); and the control language ([`success-workload`](docs/design-review.md#success-workload), [`benchmark-corpus`](docs/design-review.md#benchmark-corpus); protocol below; commit "Fix the agent-benchmark tasks before anything is measured").
- [x] Spike: Tier 1 on the generated module of the Tier-1 protocol below, against its latency target, in [`spikes/tier1-latency`](spikes/tier1-latency/README.md): the target held for a per-function re-check, and two questions for M1 arose ([`tier1-latency-spike`](docs/design-review.md#tier1-latency-spike); commit "Record the Tier-1 latency spike's result").
- [ ] Spike: debug and unwind information through Cranelift, DWARF on Linux and PDB with CodeView on Windows ([`debug-info`](docs/design-review.md#debug-info)). Its Cranelift, DWARF and object crates go to Norman for approval first, under the dependency policy.
- [x] Decided: the reference interpreter is a crate of its own, `mente-interp` ([`reference-interpreter-crate`](docs/design-review.md#reference-interpreter-crate)), and stage 1 builds no mechanised model of the core calculus ([`mechanised-model`](docs/design-review.md#mechanised-model)) (commit "Apply the M0b decisions to the spec, the roadmap and CLAUDE.md").

### M1: the thin slice

**Decided first:** [`foreign-idiom-diagnostics`](docs/design-review.md#foreign-idiom-diagnostics), [`tier1-loops`](docs/design-review.md#tier1-loops), [`tier1-counterexample`](docs/design-review.md#tier1-counterexample), [`tier2-trusted`](docs/design-review.md#tier2-trusted), [`gradual-verification`](docs/design-review.md#gradual-verification), [`language-card`](docs/design-review.md#language-card), [`todo-marker`](docs/design-review.md#todo-marker), [`debug-info`](docs/design-review.md#debug-info), [`docs-site`](docs/design-review.md#docs-site).

Integers, `bool`, fixed-size arrays `[N]T`, functions with scalar parameter modes, `if`, `while` with `break` and `continue` ([`loop-exits`](docs/design-review.md#loop-exits)), and `guard`, run end to end, as [`spec/core-calculus.md`](spec/core-calculus.md) defines them. Runs in full at the end: the slice's conformance programs.

- [ ] **Checkpoint (a), parse and check**: the body parser (Pratt, the C#-style type-argument rule, error recovery, dedicated errors for other languages' idioms, errors for unknown characters); name resolution and the type checker for the slice; `todo`; the reference interpreter over the typed AST, in its own crate `mente-interp`, which joins `CLAUDE.md`'s crate list; the incremental architecture with `mn check --watch`; canonical serialised forms of every phase's output; `mn explain`.
- [ ] **Checkpoint (b), refinements and Tier 1**: intervals plus octagons with counterexamples and `guard` prescriptions, intermediate results computed exactly and checked where they are narrowed, and bounds and overflow checks erased where Tier 1 proves them, in refined and unrefined code alike. **Engineering Guardrail #1 (Solver Predictability)** holds from here on.
- [ ] **Checkpoint (c), MIR and native code**: lowering to MIR, the MIR interpreter, and native code through Cranelift (`mente-codegen`) carrying line tables, symbols and unwind information, stepped through in CI by a scripted `gdb` session on Linux, with the three execution paths compared.
- [ ] The language card, first version: `docs/language-card.md`, the core subset first, within a word budget a test checks ([`language-card`](docs/design-review.md#language-card)); the control language's document, for Odin, of the same size; the benchmark harness, in Rust or TypeScript, following [`benchmark/README.md`](benchmark/README.md); the first agent-benchmark run; the first baselines, with hardware counters, and the first gaps of the tight-loop benchmark; the documentation site, built with mdBook from the repository's own sources and first published under the milestone's tag once Norman enables GitHub Pages ([`docs-site`](docs/design-review.md#docs-site)).

### M2: data, views and exclusivity

**Decided first:** [`error-conversion`](docs/design-review.md#error-conversion), [`value-copy-cost`](docs/design-review.md#value-copy-cost), [`sympathetic-defaults`](docs/design-review.md#sympathetic-defaults), [`cost-report`](docs/design-review.md#cost-report). **Spike before it:** `T ! E` returned in two registers through Cranelift's multi-value returns.

- [ ] `struct`, `enum` with exhaustive `match`, `?T`, and `T ! E` with `raise`, `!` and `catch`.
- [ ] Slices and `str` as views with provenance, `from` checked in bodies, and **Engineering Guardrail #3**'s `E0310`, in the same milestone, so that no view is ever unchecked.
- [ ] Every parameter mode, with the Law of Exclusivity, `own` invalidation and copy-in/copy-out; `List<T>` and `String`.
- [ ] The cost report begins: type layouts, and the runtime checks Tier 1 left in place.

Runs in full at the end: example 01's binary search.

### M3: regions and layouts

**Spike before it:** copy-in/copy-out of `SoA` and `Tiled` elements, lowered through MIR and Cranelift.

- [ ] Arenas with a chunk per task, `Pool<T>` / `Handle<T>`, and `Local<T>` / `Shared<T>`.
- [ ] `@layout` structs and `bitstruct`s in native code: alignment, little-endian bytes, `@packed` and `@endian(big)`.
- [ ] `List`, `SoA` and `Tiled` behind one syntax, with copy-in/copy-out projections; the cost report adds copies and allocations.

Runs in full at the end: example 04's pool.

### M4: abstraction, effects and FFI

**Decided first:** [`trait-coherence`](docs/design-review.md#trait-coherence), [`closure-captures`](docs/design-review.md#closure-captures), [`capability-trust`](docs/design-review.md#capability-trust). **Spike before it:** a tail-resumptive effect handler compiled to a direct call.

- [ ] Monomorphised generics, traits with coherence, `dyn`, and closures with their capture modes.
- [ ] Capability effects, effect variables, `handle` blocks, no global mutable state, and the per-module effect summary.
- [ ] `extern "C"` with `with FFI`: checked inbound values, `RawPtr<T>` and the tagged-return wrappers.
- [ ] The hermetic `comptime` interpreter under its step budget; the cost report adds `dyn` calls.

Runs in full at the end: example 04.

### M5: numerics and hardware contracts

**Decided first:** [`combined-scalar-types`](docs/design-review.md#combined-scalar-types), [`float-square-ensures`](docs/design-review.md#float-square-ensures), [`sat-mixing`](docs/design-review.md#sat-mixing), [`no-alloc-task-stacks`](docs/design-review.md#no-alloc-task-stacks), [`zero-cost-measured`](docs/design-review.md#zero-cost-measured), [`tight-loops`](docs/design-review.md#tight-loops). **Spike before it:** `@branchless` code through Cranelift on x86-64 and AArch64, with the machine-code check.

- [ ] IEEE floats in refinements (outward rounding, `NaN`), units with scales and `.to<U>()`, fixed point and `dec`, `sat` and `wrap` with their operators, and combined scalar types, bit-identical across the CI matrix.
- [ ] **Engineering Guardrail #2**: every Mente-level optimisation completes in `MirRegionSoA` before lowering. `@branchless` with `E0403`, `@simd(lanes: N)` through Cranelift's 128-bit vectors, `@no_alloc`, `@max_stack`, and `@total` measures checked.
- [ ] Overflow checks merged within trap regions and hoisted out of loops by loop versioning; MIR loop optimisations, each rewrite rule proven by an SMT solver; vectorisation by contract, with the cost report's prescriptions.

Runs in full at the end: example 01, and example 02's `integrate_cell`.

### M6: standard library and agent tools

**Decided first:** [`stdlib-scope`](docs/design-review.md#stdlib-scope), [`user-mutation-testing`](docs/design-review.md#user-mutation-testing), [`impact-analysis`](docs/design-review.md#impact-analysis).

- [ ] Text, collections chosen for their cache behaviour, files, time, processes, and JSON through `comptime`; packages.
- [ ] Mutation testing of user code, impact analysis, and the complete language card with `mn guide` and `mn api`.
- [ ] `mn fmt`, written in Mente; a getting-started guide and the standard library's API documentation.

Runs in full at the end: `mn fmt`, in use; the internal preview, marked unstable.

### M7: concurrency and simulation

**Decided first:** [`atomics-memory-ordering`](docs/design-review.md#atomics-memory-ordering). **Spike before it:** fiber stacks reserved and switched on Windows and Linux, with guard pages.

- [ ] Fibers on reserved stacks that debuggers and profilers can walk, `parallel (...)`, and `parallel_for` over structural partitions.
- [ ] `Sched` with deterministic simulation (`SimClock`, `SimNet`, `SimSched`), the memory model, networking and HTTP.
- [ ] The cost report adds atomic operations and cache lines shared between tasks.

Runs in full at the end: examples 02 and 03.

### M8: Tier 2

**Decided first:** [`tier2-phase`](docs/design-review.md#tier2-phase).

- [ ] `mn prove` with an external solver, the proof cache in the source tree, `midpoint` proven, `@prove`, and index-arithmetic `parallel_for` partitions.

Runs in full at the end: example 01, with no trusted axiom left.

### M9: hardening and the gate

- [ ] Long fuzzing, generator and soundness campaigns; the stability policy; a tutorial for people, the first stable documentation, and an entry point for language models such as `llms.txt`.
- [ ] The agent-benchmark target; an independent review of the spec and the core calculus by someone who did not write them; every condition of [`foundation-gate`](docs/design-review.md#foundation-gate).

Runs in full at the end: every example and every `@example`.

---

## Stage 2: self-hosting

The compiler's front end, checker, MIR and driver are translated into Mente phase by phase, with differential testing as the judge, in the order `mente-syntax`, `mente-ast`, `mente-mir`, `mente-sema`, `mente-cli`; `mente-codegen` stays in Rust with Cranelift until stage 3. The stage ends when the bootstrap fixed point and diverse double-compiling pass, the Mente compiler has been the default for a release, and the Rust front end is retired. Proposed in [`self-hosting-plan`](docs/design-review.md#self-hosting-plan), and re-planned in detail at the foundation gate.

## Stage 3: performance

A native backend written in Mente replaces Cranelift for x86-64 and AArch64, becoming the default only when it gives the same results on the whole corpus with no worse generated code, compile latency, or debug and profiler information. Beyond the replacement, each change follows a measurement: 256- and 512-bit vectors, deeper inlining and vectorisation, and the Carry-Flag ABI experiment of `error-handling-abi.md`, run beside the two-register return and kept only if it wins. Proposed in [`performance-stage`](docs/design-review.md#performance-stage).

## Stage 4: new domains

Each is planned from an entry of its own and starts once what it depends on exists: GPU through an execution model for `@kernel`, then SPIR-V; the browser and WebAssembly, which needs stage 3's emitter; scripting, through the interpreter; interop with machine-learning libraries; and **Engineering Guardrail #4**, `mn bind`, for C headers and for Rust libraries through their `cbindgen` headers, once the accepted C dialect is written down.

---

## Measurement protocols

A `Target` in `spec/*.md` is a number with a protocol here. A figure without a protocol is not a target, and is removed or relabelled `Hypothesis`.

### `mn view --spec` projection: median under 0.1 ms per module

- **Timed**: parsing a module whose source is already in memory, computing its three digests, and projecting the blueprint. File reading and process start are excluded.
- **Corpus**: every module in `examples/`, and the largest standard-library module once one exists.
- **Method**: release build; per module, 100 warm-up runs, then 1,000 timed runs; report the median and the 99th percentile. The harness is the ignored test `view_projection_protocol` in `compiler/mente-cli`:
  ```powershell
  cargo test --release -p mente-cli -- --ignored --nocapture view_projection_protocol
  ```
- **Regression tolerance**: the CI job "Projection benchmark" builds the base branch and the change on one GitHub x86-64 `ubuntu-24.04` runner, measures both, and fails when a median of the change exceeds the base's by more than 30%. A fixed baseline does not work on shared runners: two runs of the same code on 2026-11-08 differed by a factor of 1.8.
- **Machine**: record the CPU, OS and power mode with each result.
- **Last result** (2026-11-08, GitHub x86-64 `ubuntu-24.04` runners, the CI job): median 22.5–36.5 µs and 99th percentile 35.7–48.1 µs in one run; median 40.2–67.8 µs in the next, on another runner, for the same code. The median meets the target either way; nothing is promised for the 99th percentile. On 2026-11-05 an AMD Ryzen 7 7735HS under Windows 11 measured a median of 45–92 µs.

### Tier-1 re-check after an edit: 99th percentile under 5 ms

- **Timed**: in a running `mn check --watch` or language-server process, from an edit being applied to the source in memory until Tier 1 has re-proven every obligation the edit can affect and produced its diagnostics.
- **Corpus**: the `examples/` modules, plus a generated module of 200 functions with 20 obligations each (loop bounds, slice indexing, chains of `requires`). Each run applies one random single-token edit inside one function body.
- **Method**: release build; 100 warm-up edits, then 1,000 timed edits; report the median and the 99th percentile.
- **Machine**: as above.
- **Last result**: none yet for Tier 1 itself, which M1 builds. On 2026-11-09 the M0b spike's prototype, on the generated module alone and on the CI runners, re-checked an edit in a median of 249 to 305 µs and a 99th percentile of 288 to 479 µs, re-parsing only the edited function ([`spikes/tier1-latency/README.md`](spikes/tier1-latency/README.md); commit "Record the Tier-1 latency spike's result"). A spike is never evidence for a claim.

### Agent benchmark: Mente for agents, against Rust and a control language

- **Timed**: for each task and language, whether the agent's program passes the task's hidden tests, the repair iterations it took, and the tokens it used.
- **Corpus**: the tasks fixed in M0b, in [`benchmark/`](benchmark/README.md), whose composition table gives their number per milestone ([`benchmark-corpus`](docs/design-review.md#benchmark-corpus)), each marked with the earliest milestone that can run it and as needing only the core of the language or not; a held-out set never used to tune the language card, kept in a private repository whose commit `benchmark/corpus.lock` names; adversarial tasks, such as smuggling a network call into a pure function, that must fail; the same tasks in Rust, in Go if the budget allows, and in Odin, the control language ([`control-language`](docs/design-review.md#control-language)), given documentation the size of the language card.
- **Method**: every task runs 5 times per language, and a run allows the agent at most 10 repairs after its first program, as `benchmark/README.md` sets out; the report gives the pass rate, the median repair iterations and tokens, and their variance, with core-only tasks scored apart from the others. The primary results are Mente's trend across milestones and its standing against the control language; the gap to Rust is reported beside them, together with the training-data confound it carries. Runs happen at the end of each milestone, within a budget Norman sets, through a harness kept in the repository and written in Rust or TypeScript. A solution that contains a `todo` scores as unsolved ([`todo-marker`](docs/design-review.md#todo-marker)).
- **Machine**: the model, its version, its parameters and the prompts, recorded with every run.
- **Last result**: none yet; the tasks were fixed in M0b (commit "Fix the agent-benchmark tasks before anything is measured"), and the first run is at the end of M1.
