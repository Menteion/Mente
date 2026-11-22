# CLAUDE.md — Project Switchboard

This file is the switchboard, not the rulebook.

1. **Live implementation status**: Read [`docs/STATUS.md`](docs/STATUS.md) first. It is the single source of truth for what is `Implemented` vs. `Specified` across the compiler's crates and names the immediate next step. Open design problems, and what is proposed for each, are in [`docs/design-review.md`](docs/design-review.md); a design change starts from its entry there.
2. **Language and compiler specification**: Read [`spec/README.md`](spec/README.md). Each file in `spec/*.md` defines exactly **one orthogonal mechanism and design decision** (Specification + Decision Rationale & Rejected Alternatives + Executable `<!-- @assert-* -->` assertions). Load only the specific `spec/<topic>.md` file governing the crate you are editing.
3. **Engineering roadmap**: Read [`ROADMAP.md`](ROADMAP.md) for the four stages of the work and stage 1's milestones, M0a, M0b and M1 to M9. [`docs/design-brief.md`](docs/design-brief.md) holds the design, the working rules and the long-term plan in full.
4. **Agent benchmark**: [`benchmark/README.md`](benchmark/README.md) defines the corpus. Its held-out set lives in the private repository `Menteion/mente-bench-heldout`, cloned beside this one as `..\mente-bench-heldout`. Never read it, or held-out results task by task, while working on the language card or on anything a benchmark agent is shown; only `npm run check:benchmark -- --held-out` opens it. Run no shell command on `..mente-bench-heldout` either (`cat`, `rg`, `git -C`, `node`): a deny rule on the file tools does not cover the shell.

---

## Non-Negotiable Architectural Invariants

1. **Single-decision atomic specs (`spec/*.md`)**:
   - Never re-introduce numeric filename prefixes (`00_`..`08_`), nested subdirectories inside `spec/`, `DEC-XXX` numbers or a record of any kind named by a number, or decorative emoji warning headers. That rule is held by structure (Norman decided, 2026-11-17): a record's name is its file's name, in the directory of its declared kind (an escaped bug in `docs/escapes/`, a task brief in `briefs/`, a task of the agent benchmark in `benchmark/tasks/`), or its register entry's key, which the one key rule of `compiler/mente-cli/src/records.rs` holds; a spec, a document of `spec/` that is no record, is named by a key too, which the spec layout's test holds; and every Markdown file belongs to one declaration, so that no directory of records stands undeclared; and a citation of a record is a link whose anchor or file is such a key, which `spec-graph` checks. Held by review only: a number written in prose.
     <!-- @assert-count target="compiler/mente-cli/tests/records.rs" symbol="fn test_the_specs_are_files_named_by_a_key_with_no_pictograph" min="1" reason="spec/ holds only README.md and files named by a key, with no subdirectory, numeric prefix or pictograph" -->
     <!-- @assert-count target="compiler/mente-cli/tests/escapes.rs" symbol="fn test_every_escape_has_a_key_an_area_and_a_guard" min="1" reason="Every file of docs/escapes is an escaped bug named by its key" -->
     <!-- @assert-count target="compiler/mente-cli/tests/escapes.rs" symbol="fn test_the_record_holds_only_files_named_by_a_key" min="1" reason="The key rule refuses a file of the record named otherwise, planted" -->
     <!-- @assert-count target="compiler/mente-cli/tests/register.rs" symbol="fn test_every_entry_is_named_by_a_key" min="1" reason="Every register entry, and so the anchor of every citation of one, is named by a key" -->
     <!-- @assert-count target="compiler/mente-cli/tests/records.rs" symbol="fn test_a_key_is_lowercase_words_joined_by_hyphens" min="1" reason="A record's key, and so a file's name, is never a number" -->
     <!-- @assert-count target="compiler/mente-cli/tests/records.rs" symbol="fn test_every_kind_is_a_directory_of_files_named_by_a_key" min="1" reason="Every record of a declared kind, a task brief included, is a file named by its key" -->
     <!-- @assert-count target="compiler/mente-cli/tests/records.rs" symbol="fn test_every_markdown_file_is_declared_once" min="1" reason="Every Markdown file is a record of one declared kind, one declared shared file, or a document of one declared set, so no new directory of records stands undeclared" -->
   - A claim is a list item that starts with its one maturity label written as code: `Implemented`, `Specified`, `Target` or `Hypothesis`. A label inside a sentence or in parentheses is not a claim, and `spec-guard claims` does not read it; write it as a claim. One claim carries one label. A label in prose is reported as information, not refused: held by review only.
     <!-- @assert-count target="package.json" symbol="spec-guard claims" min="1" reason="spec-guard claims reads every claim's label" -->
     <!-- @assert-count target=".github/workflows/ci.yml" symbol="npm run verify:docs" min="1" reason="The Documentation job runs the claims check" -->
   - `Implemented` means the compiler does it today. Its evidence is an `<!-- @assert-count -->` indented under the item, whose `symbol` starts with `fn test_` and names a unit test that fails when the behaviour breaks. A directive that names a function or a type shows only that the name exists, and a `.mn` example using a syntax never makes it `Implemented`.
     <!-- @assert-count target=".spec-guard.json" symbol='"Implemented": { "evidence": "test" }' min="1" reason="An Implemented claim needs a test as its evidence" -->
     <!-- @assert-count target=".spec-guard.json" symbol='"testPrefixes": ["fn test_"]' min="1" reason="The evidence names a unit test" -->
   - `Specified` and `Hypothesis` need no evidence. A `Target` needs its measurement protocol under "Measurement protocols" in `ROADMAP.md`.
     <!-- @assert-count target=".spec-graph.json" symbol='"claims": ["Target"]' min="1" reason="spec-graph requires a protocol for each Target claim" -->
   - A claim another document cites carries `<!-- @spec-item id="..." -->` on the line above it.
     <!-- @assert-count target="package.json" symbol='"spec-graph": "spec-graph check"' min="1" reason="spec-graph check, which the Documentation job runs, reports a citation of a claim that names no item" -->
   - `<!-- @assert-present -->`, `<!-- @assert-count -->` and `<!-- @assert-absence -->` directives still pin the structural facts a spec states, such as files that exist and spellings the examples must or must not use.
     <!-- @assert-count target="compiler/mente-cli/src/main.rs" symbol="fn test_all_spec_assertions_and_graph_pass" min="1" reason="Every directive holds" -->
     <!-- @assert-count target="compiler/mente-cli/src/main.rs" symbol="fn test_spec_directives_that_cannot_fail_are_rejected" min="1" reason="A directive that cannot fail is refused" -->
   - Do not write counts of files, directives or tests into documents; `mn spec` and `cargo test` print them. Held by review only: a check for numerals would flag the numbers prose rightly holds, such as dates, versions and the limits a rule states.
2. **Forbidden language anti-patterns in `.mn` code**:
   - Never write `let` or `let mut` (use `val` and `var`).
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_other_languages_idioms_name_the_mente_spelling" min="1" reason="let, let mut, references, await, unwrap and turbofish are refused with their Mente spelling" -->
   - Never write `&T`, `&mut T`, or `<'a>` lifetimes (use MVS parameter modes: default read-only, `mut`, `own`, `in`).
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_other_languages_idioms_name_the_mente_spelling" min="1" reason="A reference passed in a call is refused with its Mente spelling" -->
     <!-- @assert-count target="compiler/mente-sema/src/lib.rs" symbol="fn test_references_in_types_are_refused" min="1" reason="A reference type is refused wherever a type stands" -->
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_lifetimes_and_async_are_idioms_wherever_they_stand" min="1" reason="Lifetimes are refused wherever they stand" -->
   - Never write `async` or `await` (use `with IO, Net` effects and `parallel (...)`).
     <!-- @assert-count target="compiler/mente-syntax/src/lib.rs" symbol="fn test_lifetimes_and_async_are_idioms_wherever_they_stand" min="1" reason="async is refused wherever it stands" -->
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_other_languages_idioms_name_the_mente_spelling" min="1" reason="await is refused with its Mente spelling" -->
   - Never write `.unwrap()` or `::<>` turbofish.
     <!-- @assert-count target="compiler/mente-syntax/src/body.rs" symbol="fn test_other_languages_idioms_name_the_mente_spelling" min="1" reason="unwrap and turbofish are refused with their Mente spelling" -->
   - Never link upstream C++ `LLVM/MLIR` into `compiler/mente-mir` (keep `MirRegionSoA` self-contained pure Rust, indexed by `NodeId(u32)`). `mente-mir` has no dependencies; native code goes through Cranelift, a pure-Rust crate that only `mente-codegen` may depend on, and so do `gimli` and `object`, which write its object files and debug information (Norman approved them on 2026-11-10). The reference interpreter `mente-interp` never depends on `mente-mir` or `mente-codegen`, so that it shares no code with the paths it checks. Every other crate depends only on the workspace's own crates: a new external dependency needs Norman's approval, and `test_crates_depend_only_on_allowed_crates` holds the line. An approval names the direct dependencies and covers the crates they bring in; the pull request that adds or upgrades one lists every crate `Cargo.lock` gains ([`dependency-approval`](docs/design-review.md#dependency-approval)).
     <!-- @assert-count target="compiler/mente-cli/tests/gates.rs" symbol="fn test_crates_depend_only_on_allowed_crates" min="1" reason="Each crate depends only on what its rule allows" -->
     <!-- @assert-count target="compiler/mente-cli/tests/gates.rs" symbol="fn test_the_lock_holds_only_approved_crates" min="1" reason="The lock holds only approved crates" -->
3. **English-only repository text**:
   - Code, comments, documentation, examples, commit messages and pull request text are written in English, even when the conversation with Norman is in Chinese.
     <!-- @assert-count target=".github/scripts/policy/commits.test.mjs" symbol="the Commits check refuses unfinished subjects, CJK text and a changed base" min="1" reason="The Commits check refuses CJK text in commit messages and pull request text" -->
   - `test_repository_text_is_english` in `compiler/mente-cli` fails on any CJK character in a file of the working tree (`.git`, `target` and `node_modules` aside). Commit messages and pull request text are outside its reach, and the Commits check refuses CJK text in them. A future fixture that must hold non-English text (a lexer test of Unicode string literals, say) goes in a directory the test is changed to skip, named in the test.
     <!-- @assert-count target="compiler/mente-cli/tests/gates.rs" symbol="fn test_repository_text_is_english" min="1" reason="No CJK character in a file of the working tree" -->
4. **The design register and the documentation tools**:
   - Cite an entry of [`docs/design-review.md`](docs/design-review.md) as a link whose anchor is its key, such as [`interface-from-signature`](docs/design-review.md#interface-from-signature); `spec-graph` reports a key that is misspelled or renamed. A `done` entry links the documents it changed and ends with `(commit "subject")`.
     <!-- @assert-count target=".spec-graph.json" symbol='"done": { "closes": true, "needs": ["citation", "commit"] }' min="1" reason="A done entry needs its citation and its commit" -->
     <!-- @assert-count target="compiler/mente-cli/tests/register.rs" symbol="fn test_every_entry_is_named_by_a_key" min="1" reason="Every entry is named by a key" -->
   - A task brief's front matter may name `raises: [spec/<file>.md#<claim id>]` and `awaits: [docs/design-review.md#<key>]`. `spec-brief lint` rejects a claim that does not exist or an awaited entry that is not yet `decided` or `done`, and `spec-brief archive` refuses a raised claim that is not yet `Implemented`.
     <!-- @assert-count target="package.json" symbol="spec-brief lint --strict" min="1" reason="spec-brief lint reads every brief" -->
   - Run the `@spec-family` tools only through the npm scripts, or with `npx --no-install` and the full `@spec-family/` name. Never patch `node_modules` or the tools' own repository (`C:\Repos\specs`); write down a false positive or a missing feature and report it to Norman. Held by review only.
5. **Four Mandatory Engineering Guardrails**:
   - **Guardrail #1 (Solver Predictability — [`ghost-contracts-and-totality.md`](spec/ghost-contracts-and-totality.md))**: Tier-1 stays strictly within polynomial-time Interval + Octagon/DBM ($\pm x \pm y \le c$) domains and, whenever an obligation fails, MUST emit a concrete valuation Tier 1 cannot rule out (never called an input that fails) plus a copy-pasteable 1-line `guard` prescription as a suggested fix. The restriction to those domains is held by review only; the test caps the variables Tier 1 takes.
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_a_failed_refinement_names_a_valuation_and_a_guard" min="1" reason="A failed obligation names a valuation and a guard" -->
     <!-- @assert-count target="compiler/mente-sema/src/tier1.rs" symbol="fn test_a_function_past_the_variable_limit_is_proven_over_bounds" min="1" reason="Tier 1 stays within its polynomial domains" -->
   - **Guardrail #2 (Dual-Track Codegen — [`mir-and-codegen.md`](spec/mir-and-codegen.md))**: Complete all high-level semantic optimizations (bounds erasure, Ghost Contract `Assume`, `AoSoA` offsets, `@branchless` select) in `MirRegionSoA` before lowering to Cranelift or any later backend. Bounds erasure and Ghost Contract `Assume` have their tests; `AoSoA` offsets and the `@branchless` select are held by review only until they are built.
     <!-- @assert-count target="compiler/mente-codegen/src/lower.rs" symbol="fn test_tier1_proofs_leave_bounds_and_overflow_checks_out_of_mir" min="1" reason="Tier 1's proofs erase bounds in MIR, before lowering" -->
     <!-- @assert-count target="compiler/mente-codegen/src/lower.rs" symbol="fn test_tier1_proofs_leave_contract_checks_out_of_mir" min="1" reason="A ghost contract Tier 1 proves becomes an Assume in MIR, before lowering" -->
   - **Guardrail #3 (Provenance Escape Diagnostics — [`regions-and-pools.md`](spec/regions-and-pools.md))**: Never expose internal region IDs in errors; `E0310` MUST prescribe either `in arena: Arena` or `Pool<T>` / `Handle<T>`. Held by review only until `E0310` is built.
   - **Guardrail #4 (Ecosystem Absorption — [`comptime-and-ffi.md`](spec/comptime-and-ffi.md))**: `mn bind` auto-synthesizes quarantined `extern "C"` + `with FFI` + `@layout` bindings from C headers, and from Rust libraries through their `cbindgen` headers (stage 4). Held by review only until `mn bind` is built.
6. **Every change carries its checks** ([`quality-per-change`](docs/design-review.md#quality-per-change); `docs/design-brief.md` section 5):
   - Write a rule's edge cases as conformance programs before its code: type limits and 0, negative values entering unsigned types, mixed signedness, `i128` beside `u128`, nested `if`, and out-of-type or misshapen arguments. Settle a case the spec leaves open in the spec first: when two execution paths, or a path and the spec, can give different results for a case, write the case into the governing spec item before changing any path, with Norman when it is a design choice, and cite that item in the fix. Held by review only.
   - Define each fact about the language once, and read it everywhere. Test every special case the code makes. Held by review only.
   - A fix comes with a test seen to fail without it. Mutants shows each changed line tested; that the test fails without the fix is held by review only.
     <!-- @assert-count target=".github/required-checks.txt" symbol="Mutants in the diff" min="1" reason="Mutation testing of the lines a change touches is a required check" -->
     <!-- @assert-count target=".github/scripts/policy/mutation.test.mjs" symbol="the Mutants floor counts what the diff changes, as cargo-mutants marks it" min="1" reason="The Mutants floor counts what the diff changes" -->
   - Every problem found, a bug, a slow path or a slip in the process, is weighed for a guard against its whole class, strongest first: a required check of `main`, then a test or a lint, then a written rule. Memory alone is never the guard. The weighing is held by review only.
   - A bug that escaped the author's own checks (found by a review, by CI on a later change or by a user) comes with the change to the instrument that should have caught it, and the pull request names that instrument and what it lacked ([`guard-per-problem`](docs/design-review.md#guard-per-problem)). That every escape is recorded, and that its pull request names the instrument, is held by review only.
     <!-- @assert-count target="compiler/mente-cli/tests/escapes.rs" symbol="fn test_every_escape_has_a_key_an_area_and_a_guard" min="1" reason="Each recorded escape names the instrument that missed it and its guard" -->
     <!-- @assert-count target="compiler/mente-cli/tests/evidence.rs" symbol="fn test_each_guard_is_found_in_its_runners_list" min="1" reason="A guard stands only where its runner runs it" -->
   - Instrument work for a feature (generator rules, coverage tables, oracles) is briefed to a session other than the feature's author where it can be, and a reviewer's reproduction becomes a conformance program and a rule of the program generator in the fix's pull request. Held by review only.
   - New syntax or a new rule extends the program generator in the same pull request; `test_generated_programs_reach_every_construct_of_the_grammar` fails on syntax no generated program reaches. A checkpoint closes only when the instruments scheduled for it run in CI and pass.
     <!-- @assert-count target="compiler/mente-cli/src/mir_run/coverage.rs" symbol="fn test_generated_programs_reach_every_construct_of_the_grammar" min="1" reason="Generated programs reach every construct of the grammar" -->
     <!-- @assert-count target=".github/scripts/policy/required-checks.test.mjs" symbol="every instrument scheduled for a finished milestone is a required check" min="1" reason="A finished milestone's instruments are required checks" -->

---

## Verification (Quick Checks Before Every Push, the Rest on CI)

The required checks of `.github/required-checks.txt` are the gate: CI runs every command below on each push, on x86-64 Linux, AArch64 Linux and Windows, with pinned tools and a record, again in the merge queue and on `main`. Locally, before a push, run only the quick checks (Norman, 2026-11-15): a full local run repeats CI's work on the laptop and gates nothing.

```powershell
# Before every push, locally: the checks the changed files call for (tools/quick.mjs holds which),
# at half the laptop's cores, and the tests the change adds or changes, each seen to fail
# without the fix and to pass with it (`npm ci` once first)
node tools/quick.mjs -- -p <crate> <filter>
```

```powershell
# On CI (ci.yml and the other required workflows), not locally
cargo test                                  # in CI: cargo nextest run --profile ci
cargo run --bin mn -- spec
cargo run --bin mn -- test --mutate         # each example against its .expected, then the demo MIR region and its mutation gate
cargo run --bin mn -- view --spec examples/02_mechanical_sympathy_simd.mn
npm run verify:docs
npm run check:benchmark
npm run test:benchmark
npm run test:ci
cargo fmt --all --check
cargo clippy --all-targets --workspace -- -D warnings
```

- Push each round once, with all of it: every push runs every required workflow, Mutants included, and a later push cancels the earlier runs.
- In CI the tests run through `cargo nextest run --profile ci`, under the time limits of `.config/nextest.toml` and the memory limits of `.github/scripts/limits.sh`.
- **Heavy jobs run on GitHub Actions.** `cargo-mutants`, `mn fuzz` beyond a smoke run, generated-program comparisons at scale, benchmarks and the latency protocols run on GitHub Actions through `workflow_dispatch` on the working branch.
- **Local runs are capped.** On the laptop, build, test and run Rust through `node tools/quick.mjs`, `node tools/cargo.mjs <cargo arguments>` or `node tools/mn.mjs <mn arguments>`, not raw `cargo` or a binary under `target/` (`tools/local-limits.mjs`; the reset of 2026-11-22). A run through them holds one of the machine's two slots to its end, at half the cores, and runs its commands in a job object that refuses a process more than 8 GB of private committed memory and the tree more than 16 GB, and ends the tree at 30 minutes, or at `--minutes`, at most 60; `mn` itself runs under 4 GB, 8 GB and 5 minutes. The tools refuse to run until cargo's runner, in `~/.cargo/config.toml` on the laptop, is `tools/capped.mjs`, and name the lines to add: it holds every test binary cargo starts to the same limits, however cargo was started. The hook of `.claude/settings.json` only reads a command's text: it refuses the common raw forms and names the command to run instead. The tools refuse the Store's `pwsh` as a capped command and keep the WindowsApps directories off a capped `PATH`. Known limits: pagefile-backed shared memory is not counted against the job, and `pwsh` started any way other than a PATH search (ShellExecute, `start`, `Start-Process`, or a WindowsApps path) leaves the job, with every process it starts.
- **Pushes go through a pre-push hook.** `core.hooksPath` is `C:/Repos/Mente/tools/git-hooks`, set once on the laptop, and the tools refuse to run until it is. git's pre-push hook refuses a push unless each commit it sends has the tree `node tools/quick.mjs` last passed in that worktree on a still tree, or is already on the remote's main, which the hook asks of the remote at the unrewritten push URL. Merge main into a branch before its push, and run quick on exactly the tree you push. The hook and cargo's runner run from the main checkout, so the main checkout stays on `main` and is fast-forwarded after every merge, until `briefs/gate-tools-from-main.md` lands. The rule in `.claude/settings.json` refuses `--no-verify` and writes of `core.hooksPath`.
- **A pull request's body is checked before it is sent.** `node .github/scripts/commits-body.mjs <file>` runs the Commits check's body rules (no CJK text; the root-cause declaration) on a draft body; run it before creating or editing one.
- **Merging.** `main` takes changes only through its merge queue, one pull request at a time as a merge commit, behind the required checks of `.github/required-checks.txt`. A pull request is enqueued at its head SHA only once every check of that head has finished green, Mutants included; a run cancelled by a later push counts as not run. The queue then runs the required checks on the merged result, so a branch needs no merge of `main` to be tested against it. Never enable auto-merge.
- **Worktrees.** A session's worktree and branch are removed (`git worktree remove`, then the branch) when its pull request merges or the session is archived; nothing stays under `.claude/worktrees` or `.wt`.

---

## Crate-to-Spec Routing Table

| Crate | Governing `spec/*.md` files |
| :--- | :--- |
| `compiler/mente-syntax` | [`holographic-module.md`](spec/holographic-module.md), [`surface-syntax.md`](spec/surface-syntax.md), [`grammar.md`](spec/grammar.md), [`core-calculus.md`](spec/core-calculus.md), [`diagnostics.md`](spec/diagnostics.md) |
| `compiler/mente-ast` | [`holographic-module.md`](spec/holographic-module.md) |
| `compiler/mente-sema` | [`core-calculus.md`](spec/core-calculus.md), [`refinement-and-fixed-point.md`](spec/refinement-and-fixed-point.md), [`dimensional-units.md`](spec/dimensional-units.md), [`algebraic-types-and-null.md`](spec/algebraic-types-and-null.md), [`traits-and-generics.md`](spec/traits-and-generics.md), [`mutable-value-semantics.md`](spec/mutable-value-semantics.md), [`regions-and-pools.md`](spec/regions-and-pools.md), [`morphic-layouts.md`](spec/morphic-layouts.md), [`ghost-contracts-and-totality.md`](spec/ghost-contracts-and-totality.md), [`capability-effects.md`](spec/capability-effects.md), [`diagnostics.md`](spec/diagnostics.md) |
| `compiler/mente-mir` | [`core-calculus.md`](spec/core-calculus.md), [`mir-and-codegen.md`](spec/mir-and-codegen.md), [`hardware-contracts.md`](spec/hardware-contracts.md), [`disjoint-parallelism.md`](spec/disjoint-parallelism.md) |
| `compiler/mente-codegen` | [`error-handling-abi.md`](spec/error-handling-abi.md), [`hardware-contracts.md`](spec/hardware-contracts.md), [`mir-and-codegen.md`](spec/mir-and-codegen.md) |
| `compiler/mente-interp` | [`core-calculus.md`](spec/core-calculus.md) |
| `compiler/mente-cli` | [`comptime-and-ffi.md`](spec/comptime-and-ffi.md), [`holographic-module.md`](spec/holographic-module.md), [`diagnostics.md`](spec/diagnostics.md) |
| `compiler/mente-threads` | none: the one place threads start, held by `clippy.toml`, the workspace's lints in `Cargo.toml` and `.config/nextest.toml` |
