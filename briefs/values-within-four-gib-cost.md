---
id: values-within-four-gib-cost
status: deferred
trigger: when pull request 215 merges
type: chore
affectedFiles: [compiler/mente-codegen/src/**, compiler/mente-mir/src/**, compiler/mente-cli/src/mir_run.rs, compiler/mente-cli/src/mir_run/differential.rs, briefs/values-within-four-gib-cost.md, briefs/archive/values-within-four-gib-cost.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**, compiler/mente-sema/**, compiler/mente-interp/**]
---

# Make the programs of values within 4 GiB cheap on MIR and native code

## Intent

`array_within_4_gib.mn`, `struct_within_4_gib.mn` and `enum_within_4_gib.mn` of the conformance suite each take as long on MIR and native code as many other programs together: about 45 s in the unit tests' build on a laptop, and about 100 s on the AArch64 runner, 55% of the stop the longer tests get (`.config/nextest.toml`). They were the critical path of the conformance suite's single test, at 63% of its stop on AArch64 on 2026-11-22, and are why its parts give them one of their own, the first group of `HEAVY` in `compiler/mente-cli/src/mir_run/differential.rs`. Each call passes a 64 KiB argument, `[65536]u8` or a struct or variant of that size, and MIR and native code copy it whole, as values are, through every step of the call: building the argument, the call itself, and native code's process per call. When this round is done, that cost is found at its root and removed there: by measuring where the time goes (MIR's interpreter, the lowering, Cranelift, or the native program's setup) and changing the path that spends it, never by fewer calls or a smaller argument in the programs. The programs then leave `HEAVY`, and their part holds its share of the rest. Its owner is stack-budget, once its fix of Tier 1's nested loops has merged.

## Negative Scope

- No change to what the programs test: a value within 4 GiB, its type and the calls' arguments stay.
- No change to item 25's stack budget or to how a frame counts a value.
- No longer stop for a test instead of a cheaper path.

## Invariants

- [ ] The three execution paths agree on the programs, as `test_three_paths_agree_on_the_conformance_suite_part_1_of_5` shows.
- [ ] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [ ] The pull request names where the time went, with the measurement before and after.
- [ ] Each of the three programs takes no more than five of the suite's other programs together in the unit tests' build, and the first group of `HEAVY` is gone, with the report of the tests' times showing every part of the conformance suite under 45% of its stop on every runner.
