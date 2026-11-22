---
id: smt-confirm-canary
status: draft
date: 2026-11-22
type: feature
affectedFiles: [compiler/mente-cli/src/smt_independent.rs, compiler/mente-cli/tests/smt-known/**, compiler/mente-cli/tests/smt.rs, .github/workflows/smt.yml, .github/gate-commands.txt, briefs/smt-confirm-canary.md, briefs/archive/smt-confirm-canary.md]
protectedFiles: [CLAUDE.md, spec/**, compiler/mente-sema/**, compiler/mente-interp/**, compiler/mente-smt/**]
---

# Send a real Z3 `sat` through `mn smt --confirm` on every run

## Intent

Since Tier 1's fix of root cause 1 (pull request 182), the independent cross-check of pull request 191 has no known soundness bug. So Z3 answers `unsat` to every query of the job, and no real `sat` passes through `mn smt --confirm` in CI. A break anywhere on that path would fail closed, but it would only be found the day a real bug appears: Z3's model format, the model reader, the evaluator, the run on the interpreter, or the match of its trap (Fable's review of round 7).

When this round is done, every run of the job sends one known `sat` down the whole path, as a permanent canary. One way:
- a test-only program in `tests/smt-known`, and a check of it written as if Tier 1 had erased it, which it did not;
- Z3 finds the counterexample;
- `--confirm` confirms it on the interpreter as a known bug.

The job fails when the canary is not confirmed, as it does for any expected bug that is missing.

## Negative Scope

- Tier 1 and the encoding do not change: the canary goes into the export from outside, and is marked as a canary wherever it is printed.
- It is never counted as a soundness bug of Tier 1, and the comparison with the base neither drops it nor counts it.

## Not Empowered

- `compiler/mente-sema/**`, `compiler/mente-interp/**`, `compiler/mente-smt/**`, `spec/**`, `CLAUDE.md`.
- `.github/**` changes are Norman's to approve as code owner.

## Acceptance Criteria

- [ ] Each run of the job writes the canary's query, Z3 answers it `sat`, and `--confirm` reports it confirmed on the interpreter, by its file, function and obligation.
- [ ] A break planted on each piece of the path fails the job: the canary not written, its model not read, the run that would not trap, and the trap matched at another place. Each plant is run on a dispatched run of the job.

## Invariants

- [ ] The canary cannot pass for a real bug of Tier 1, nor a real bug for the canary.
- [ ] The workflow's pinned lines name the canary's path, and the policy holds them.
