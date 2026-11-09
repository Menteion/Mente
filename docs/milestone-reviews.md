---
title: Mente Milestone Reviews
version: 0.1.0
status: active
date: 2026-11-09
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
