---
title: Mente Milestone Reviews
version: 0.1.0
status: active
date: 2026-11-11
---

# Mente — Milestone Reviews (`docs/milestone-reviews.md`)

The plan is a hypothesis. So after M0, after M1 and after every later milestone, it is reviewed against the evidence:
- the spikes' findings;
- the agent-benchmark results;
- the latency and performance baselines;
- how long the milestone took.

This comes from [`foundation-milestones`](design-review.md#foundation-milestones). Each review checks two lists in [docs/design-brief.md](design-brief.md) section 7.7: the triggers written down in advance, under "Re-planning points", and the milestone's exit criteria, under "Every milestone ends when". A change that follows from a review enters [docs/design-review.md](design-review.md) as an entry, and the plan changes only with Norman.

## After M0 (2026-11-09)

The two parts of M0 ran back to back, built by agent sessions with Norman deciding:
- **M0a**, tag `m0a`, ran from the merge of pull request 3 (2026-11-08 12:41), which recorded the decisions it starts from, to the merge of pull request 13 (19:24).
- **M0b**, tag `m0b`, ran from there to the merge of pull request 21 (2026-11-09 03:08).

**Outcome: the plan stands.** No spike failed, no trigger fired, and no register entry is reopened. The review changed four things:
- The milestones are tagged, as section 7.7 asks and as M0 had not done.
- Checkpoint (a) of M1 now says how its incremental architecture must work.
- An approval of a dependency now says what it covers.
- Seven register entries that had been applied but were still marked `decided` are now `done`.

### The triggers

- **A spike fails.** Neither did.
  - **Tier 1's latency** ([`tier1-latency-spike`](design-review.md#tier1-latency-spike)):
    - Re-parsing and re-checking only the edited function kept the 99th percentile between 0.29 and 0.48 ms on the three CI platforms, a tenth of the 5 ms target.
    - Re-parsing the whole module on every edit gave 4.2 to 13.4 ms, which misses the target.
    - So checkpoint (a) of M1 parses and checks per item, with spans relative to each item, as `ROADMAP.md` now says.
    - The two questions the spike raised were decided before M1: [`loop-condition-candidates`](design-review.md#loop-condition-candidates) and [`refinement-inference`](design-review.md#refinement-inference).
  - **Debug and unwind information through Cranelift** ([`cranelift-debug-spike`](design-review.md#cranelift-debug-spike)):
    - gdb, lldb and cdb stopped on source lines, walked the stack through every Cranelift frame and stepped. This held on x86-64 Linux, AArch64 Linux and x86-64 Windows.
    - Checkpoint (c) of M1 writes the sections Cranelift does not, as the spike did, and adds cdb to the debugger session in CI.
- **Agents fail mostly on proof errors, or on invented APIs and other languages' habits.** There is no evidence yet:
  - the tasks are fixed and locked ([`benchmark-corpus`](design-review.md#benchmark-corpus));
  - the first run is at the end of M1, so the review after M1 is the first to check these two triggers.
- **The Tier-1 latency target is missed.** The prototype did not miss it when it re-parsed only the edited function. Tier 1 itself is built in checkpoint (b), and is measured then under the protocol in `ROADMAP.md`.
- **A milestone takes far longer than the ones before it.** M0 is the first, so there is nothing to compare it with. M0a took about 6¾ hours and M0b about 7¾. But M0 wrote documents, tools and prototypes, while M1 builds the compiler, so M0's time is no baseline for M1. Git history records when each of M1's checkpoints ends, and the review after M1 compares the checkpoints with one another.

### Baselines

- **The `mn view --spec` projection.** The median meets its 0.1 ms target on the CI runners. The CI job "Projection benchmark" fails any change that is more than 30% slower than its base. The protocol and the last result are in `ROADMAP.md`.
- **The Tier-1 re-check.** There is only the spike prototype's result, given above. A spike is never evidence for a claim.
- **CI time.** It decides what can run on every pull request (section 7.7, criterion 14). During M0b:
  - each job of the workflow "CI" finished in under a minute on every platform;
  - `cargo mutants --in-diff` took 0.3 to 7 minutes;
  - the spike's workflow, which builds Cranelift and drives the debuggers, took about 1.5 minutes a run.

  So CI time does not yet limit what runs on every pull request. It is measured again once Cranelift joins the compiler in checkpoint (c).

### Exit criteria

The criteria of section 7.7 whose instruments are active from M0 hold, with two exceptions:

- **Criterion 17, the version tag.** Neither M0a nor M0b was tagged, and no naming had been set. Norman decided the naming on 2026-11-09 ([`milestone-tags`](design-review.md#milestone-tags)):
  - `m0a` marks the merge of pull request 13, and `m0b` the merge of pull request 21;
  - the results the two milestones recorded now name these tags.
- **Criterion 2, conformance programs for every new claim.** M0b's new claims are all `Specified`, and the compiler cannot check them yet. They are the claims in `spec/core-calculus.md` and `spec/grammar.md`, and those of the two decisions. A program that must fail with a code the compiler does not report yet would only fail the runner. So each claim's programs, both ones that must pass and ones that must fail, come with the checkpoint of M1 that implements it, and criterion 2 is checked there.

Criteria 4 to 6, 11, 15, 16 and 18 begin in M1, and criterion 10 in M2.

### Also found

- **Seven register entries were applied but still marked `decided`.**
  - The commit "Apply the decisions made before M0 to the roadmap, the spec and the brief" rewrote the documents for five entries but left their status as it was: [`ai-first-purpose`](design-review.md#ai-first-purpose), [`foundation-gate`](design-review.md#foundation-gate), [`foundation-milestones`](design-review.md#foundation-milestones), [`long-term-order`](design-review.md#long-term-order) and [`overflow-check-cost`](design-review.md#overflow-check-cost).
  - Two more entries were settled when `foundation-milestones` was decided: [`roadmap-vertical-slice`](design-review.md#roadmap-vertical-slice) and [`roadmap-ai-priorities`](design-review.md#roadmap-ai-priorities).
  - All seven are now `done`, each linking what it changed.
- **Approved crates bring other crates with them.**
  - For the Cranelift spike, Norman approved seven direct crates and the main crates they bring in.
  - The build also uses twelve crates outside Cranelift that the approval did not name: `allocator-api2`, `arbitrary`, `cfg-if`, `crc32fast`, `equivalent`, `fnv`, `foldhash`, `heck`, `indexmap`, `libc`, `mach2` and `memchr`.
  - Its `Cargo.lock` lists eight more crates that this configuration never builds: `serde`, `serde_core`, `serde_derive`, `proc-macro2`, `quote`, `syn`, `unicode-ident` and `stable_deref_trait`.
  - Norman decided on 2026-11-09 that an approval names the direct dependencies and covers what they bring in. The pull request that adds or upgrades a dependency lists every crate `Cargo.lock` gains ([`dependency-approval`](design-review.md#dependency-approval)).
  - `test_crates_depend_only_on_allowed_crates` checks direct dependencies only. At checkpoint (c), the crate list goes to Norman again, as agreed when he approved it, and the policy then admits `gimli` and `object` beside Cranelift in `mente-codegen`.

## After M1 (2026-11-11)

M1, the thin slice, ran from the merge of the review of M0 (pull request 22, 2026-11-09 08:53) to tag `m1`. Its checkpoints ended with these pull requests:
- **Checkpoint (a)**, parse and check: pull request 42 (2026-11-09 18:48), about ten hours.
- **Checkpoint (b)**, refinements and Tier 1: pull request 46 (23:25), about four and a half hours.
- **Checkpoint (c)**, MIR and native code: pull request 56 (2026-11-10 05:24), about six hours.
- **The last item**: the language card, the Odin document, the multi-language harness, the first benchmark run, the performance baselines and the documentation site, through pull request 70 (12:33), about seven hours.
- **The quality instruments the checkpoints had skipped**, and the bugs they found, through pull request 107 (2026-11-11 13:27), about twenty-five hours more.

**Outcome: the plan stands, with two changes of method.** No trigger fired. But M1 closed its checkpoints before the instruments the quality-system table schedules for them existed. Once those instruments ran, they found about thirty-five compiler bugs in two days, and one of them, the mutation gate, had itself been passing what it had not checked. So every change now carries its checks ([`quality-per-change`](design-review.md#quality-per-change)), and an instrument's result counts only where it proves something: a mutant counts as caught only when a test fails ([`mutation-timeouts`](design-review.md#mutation-timeouts)).

### The triggers

- **A spike fails.** M1 had no spike; M0b's two held (the review of M0).
- **Agents fail mostly on proof errors.** It did not fire, by the measure the benchmark scores, which is whether a run is solved:
  - Every unsolved run of the first measured run failed on a wrong result, never on a proof ([`benchmark/results/m1`](../benchmark/results/m1/README.md)).
  - First programs are another matter: 12 of Mente's 16 failing first programs on public tasks were proofs Tier 1 could not make (`E0502`), and so was one of the 2 on held-out tasks. Each was repaired in one step with the fix its diagnostic offered.
  - So [`gradual-verification`](design-review.md#gradual-verification) stands. The share of first programs failing on proofs is the figure to watch as M2 widens refinements.
- **Agents fail mostly on invented APIs or other languages' habits.** It did not fire: one failing first program on public tasks, and one on held-out tasks, used another language's idiom.
- **The Tier-1 latency target is missed.** It was not: the protocol's re-check of an edit took a 99th percentile of 1.88 ms against 5 ms. The fuzzer found two shapes Tier 1 handled too slowly, against Guardrail #1:
  - nested loops took time exponential in their depth; each loop is now analysed once per entry state, and 40 nested loops check in milliseconds (pull request 87);
  - a function's size cost time growing with its cube, 2 s for 200 nested calls, since Tier 1's octagon is one dense matrix over all of a function's variables. Past 128 variables a function is now proven over each variable's bounds alone, which Norman decided ([`tier1-variable-cap`](design-review.md#tier1-variable-cap)); the largest function of the corpus has 53.
- **A milestone takes far longer than the ones before it.** It did not fire.
  - M1 took about 53 hours, against about 7 for M0a and 8 for M0b. But M1 built the compiler, which M0 did not, so M0 is no baseline (the review of M0).
  - Inside M1, the checkpoints took from four and a half to ten hours each.
  - The last twenty-five hours were rework: the instruments the checkpoints skipped, and the bugs they found. Building each instrument with the code it checks is the change this review makes.

### Baselines

- **Tier 1's re-check after an edit:** a 99th percentile of 1.88 ms under its protocol, which the CI job "Tier-1 latency" runs on every change.
- **The projection:** under its 0.1 ms target, unchanged.
- **Native code against C, C++ and Rust:** the first gaps of the tight-loop kernels, from 2.2 to 18.5 times on x86-64 and from 4.7 to 29.5 times on AArch64 ([`perf/README.md`](../perf/README.md)). The gate's tolerances are 5% of time and 1% of instructions. One exception, `dot` on AArch64, was accepted on 2026-11-10.
- **The agent benchmark:**
  - On public tasks, Mente solved 100% with Sonnet and 96% with Haiku, as many as Odin, with more of its errors caught while building.
  - On the adversarial tasks, Mente caught 13 of 20 runs, Odin 10 and Rust 0.
  - Every language solved every held-out task.
- **CI time**, per pull request, including the wait for a runner:
  - the workflow "CI" about 20 to 35 minutes;
  - `cargo-mutants` on the diff about 25, a baseline through nextest included;
  - the generated-program comparison about 27;
  - the SMT cross-check about 6;
  - Performance about 20;
  - the fuzzer's short run about 5.

  They run in parallel, so a change waits about half an hour, and queues when several are pushed at once. Every check still fits a pull request. The benchmark, the fuzzer's 45-minute run and the full mutation run, sixteen shards of half an hour to two hours each, are kept for a schedule, a dispatch or the milestone's end.

### Exit criteria

Each criterion of section 7.7 active in M1:

1. **The spec changed first:** held. `spec-guard claims` requires every `Implemented` claim to name a unit test.
2. **Conformance programs:** held for every error code, which a test requires, each with a program that must fail and a near miss that must pass. Claims without an error code, such as native code, are checked by tests rather than by conformance programs.
3. **Error codes:** held. The catalogue's tests require each code's `mn explain` text, golden output, programs and fixes. Its last new codes came from the fuzzer's findings, each decided by Norman: `E0127` for nesting deeper than 256 levels, `E0223` for an array of more than 4,294,967,295 bytes, and `E0224` for a call of a function without a body.
4. **The three paths agree:** held, on the conformance suite and on generated programs, on all three platforms of the CI matrix:
   - seeds 0 to 399 on x86-64 and AArch64 Linux, and 0 to 149 on Windows, with native code: no difference;
   - the conformance suite, exhaustive up to 65,536 calls a function through MIR: no call differs.

   Before that, the generator found eleven bugs and the MIR mutation gate three. Each was fixed in the spec's favour, three of them in the reference interpreter.
5. **Soundness by execution and the SMT cross-check:** held.
   - Generated programs reach no trap Tier 1 proved unreachable.
   - Z3 answers `unsat` to every obligation Tier 1 discharges across the corpus, on every change to `mente-sema` (`.github/workflows/smt.yml`).
   - Both instruments found one wrong proof, the same one: `u128` had lost its lower bound (pull request 75).
6. **The grammar and the generator cover the syntax, and fuzzing finds no crash:** held, by the measure Norman set on 2026-11-11: one 45-minute run with no failure.
   - The generator covers M1's syntax; 91% of its programs check.
   - The fuzzer found twenty-one problems in seven short runs on pull requests and three 45-minute runs; the fourth 45-minute run, 77,403 inputs, found none. By kind:
     - **A crash.** Deep nesting overflowed the compiler's stack, settled by a limit Norman decided (`E0127`, [`nesting-limit`](design-review.md#nesting-limit)); a long chain past it later slipped through when the body had another error. Layouts of huge sizes or alignments overflowed while their fix was computed.
     - **A hang.** Tier 1's two slow shapes, under the triggers above.
     - **A program that checks but cannot be lowered.** A `todo` as a value and as an index; an array too large to be a value (`E0223`, [`array-size-limit`](design-review.md#array-size-limit)); a call of a function without a body, a declaration nothing in M1 can run (`E0224`, [`call-without-body`](design-review.md#call-without-body)); a refinement's bound that named an array.
     - **A `--watch` session and the whole module's check reading the code differently,** nine times, and malformed headers the whole check passed: a stray `]` in a return type, a contract's `(}`, a body the file ends inside. Each was fixed where it was found until the family was closed at its root: the whole check now reads each item within that item's own tokens, as a session reads its text, so no scan of one item runs into the next (pull request 105).

     Each finding replays as a test in `compiler/mente-cli/tests/fuzz`.
7. **The MIR mutation gate covers the new MIR:** held. 1,136 mutants of lowered code are each killed or recorded as equivalent with a written reason. `cargo-mutants` on the changed compiler code leaves no survivor without a written reason, but only since 2026-11-11:
   - Until then the gate counted a timed-out mutant as caught, and on a pull request cargo-mutants' baseline tested only the mutated crate, so its timeout was often its 20 s minimum. Of 141 timeouts in eleven runs, 94 had proved nothing.
   - The tests now run through nextest, which stops a test after 40 s, and a timeout fails the job ([`mutation-timeouts`](design-review.md#mutation-timeouts)). Run again so, the functions those timeouts touched left 39 mutants missed, and the first full run of the compiler, about 4,690 mutants in sixteen shards, 15 more.
   - Each is killed by a test, or, where no program can tell it from the original, recorded with its reason in `.cargo/mutants.toml`: redundant checks were removed rather than excused.
   - Run again after those fixes (run 42920746208), the full run leaves no mutant missed. Two shards lost their runners to mutants whose tests took all 16 GB of memory, until a guard stopped such a test, which fails it and so counts its mutant as caught; run again so, they missed none (pull request 107). The one survivor elsewhere, Tier 1's forgetting of what a call of a function without a body may change, which `E0224` leaves no program to reach, is excluded until M4's `extern` functions.
8. **Compiling twice gives identical output:** held, for MIR and for the object of every target from any machine. On Windows an executable is not compared, since MSVC's `link.exe` gives each program database a new identifier.
9. **Latency targets and baselines:** held, with `dot`'s accepted exception.
10. **The cost report:** begins in M2.
11. **The language card and the benchmark:** held. The card's snippets pass `mn check`, and the run is recorded.
12. **`docs/STATUS.md` in the same commit:** held, though its table of crates had drifted since checkpoint (c) until pull request 71 corrected it.
13. **Hygiene and the runtime:** held. Every function of the generated native runtime, and every import, is listed in [the platform layer](platform-layer.md) with its reason, and a test fails on one that is not.
14. **Every check runs automatically:** held. The generator, the SMT cross-check, the MIR gate, `cargo-mutants` on the diff and a short fuzz run each run on every pull request. The fuzzer's 45-minute run is scheduled nightly and the full mutation run weekly, both also dispatched by hand; the benchmark is dispatched at the milestone's end.
15. **Every proven fact reaches MIR:** held for what M1 proves. Each bounds, overflow and contract check Tier 1 proves is left out of MIR, and tests read that back. The MIR gate found one erasure missed: narrowings of an array literal's constant elements, which Tier 1 proves, stay in MIR. They cost time, not soundness, and are a task for M2.
16. **Debug information:** held. gdb and cdb step through the new constructs in CI.
17. **The tag:** `m1`, and the results recorded at M1's end name it.
18. **The documentation site:** rebuilt and published under `m1`, marked pre-release.

### Also found

- **About thirty-five compiler bugs in two days, at the edges.** The instruments found a wrong proof, an octagon overflow and two slow shapes of Tier 1; programs that check but could not be lowered; disagreements between the paths on 128-bit, mixed-sign and misshapen values; a `--watch` session that read the code otherwise than the whole check; malformed code the parser passed; and a native `main` whose frame grew with every call it made, until a few dozen calls of 64 KiB arrays overflowed Windows' 1 MiB stack. Hand-written tests had probed none of these edges. Their record, and the rules that follow from it, are in [`quality-per-change`](design-review.md#quality-per-change).
- **An instrument that passes what it has not checked.** The mutation gate's timeouts were the clearest case, and a hand-run mutant check of mine once read a stale build. An instrument's result now counts only where it proves something, and a mutant is checked against a rebuilt test.
- **Values that meet in one machine type.** [`wide-intermediates`](design-review.md#wide-intermediates) kept every operation's value within 128 bits, but values also meet in an `if`'s branches, beside an operation's operands and among an array literal's elements. Norman decided `E0222` for the `if` on 2026-11-10, and the other two follow the same rule.
- **An array is a value kept in one piece, so its size has a limit:** 4,294,967,295 bytes, an integer of no one type counting sixteen, the widest machine type it may be kept in ([`array-size-limit`](design-review.md#array-size-limit)).
- **A language-integrated database** is recorded as a candidate for stage 4 ([`language-integrated-database`](design-review.md#language-integrated-database)).

### Gaps M1 leaves

- **A large array still overflows a native stack.** An array is kept on the stack, so one well under the limit overflows a thread's stack (1 MiB on Windows, 8 MiB on Linux) when copied into a few frames. `@max_stack` (M5) is where a function's stack is bounded, and `List<T>` (M2) keeps its elements apart from the value.
- **A declaration's header is not read by M1's grammar** ([`example-bodies`](design-review.md#example-bodies)), so a malformed one, a contract ending in an operator say, passes. Since `E0224` nothing can call it, and nothing compiles it.
- **Past 128 variables a function loses Tier 1's relations** ([`tier1-variable-cap`](design-review.md#tier1-variable-cap)): a proof that needs one fails there, and the function is to be split.
- **A proven narrowing of an array literal's constant element stays in MIR** (criterion 15). It costs time, not soundness; M2 leaves it out.
