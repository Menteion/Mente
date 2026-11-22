---
id: smt-pure-conditions
status: active
date: 2026-11-22
type: refactor
affectedFiles: [compiler/mente-smt/src/**, compiler/mente-cli/src/smt_independent.rs, briefs/smt-pure-conditions.md, briefs/archive/smt-pure-conditions.md]
protectedFiles: [CLAUDE.md, spec/**, compiler/mente-sema/**, compiler/mente-interp/**, compiler/mente-smt/Cargo.toml, .github/**]
---

# Assert pure conditions in the independent encoding, and drop the code that encoded effects in them

## Intent

On 2026-11-22 Norman decided that these places must be pure, with no assignment and no `mut` argument:
- the conditions of `while`, `guard` and `if`;
- the scrutinee of a `match` over a `bool`;
- every contract clause;
- the operands of `and` and `or`.

Pull request 196 put the rule on main: the checker refuses any other, as E0236. This round lands in the follow-up of pull request 191, together with the briefs.

The independent encoding ([`smt-independent-encoding`](../docs/design-review.md#smt-independent-encoding), pull request 191) evaluates those places with the same `eval` it uses for any expression. So it carries their effects into the bindings it joins: a `mut` argument's value copied back, and a block's bindings, around `guard`, `while`, `if`, `match`, the contracts and the short-circuit of `and` and `or`. No program the checker accepts reaches that code any more.

When this round is done:
- The encoding evaluates conditions as pure.
- It asserts the invariant instead of encoding around it: a condition that changes a binding is a defect of the encoding, never encoded. It is reported the way an obligation with no site is, `unmatched` failing the export.
- The code that joined a condition's effects is gone.

## Negative Scope

- The checker's rule, E0236, and its tests are pull request 196's, not this round's.
- The queries of programs the checker accepts do not change. On the construct programs and the job's corpus, the export's keys are the same before and after this round.
- The interpreter does not change. It runs whatever the checker accepts.

## Not Empowered

- `compiler/mente-sema/**`, `compiler/mente-interp/**`, `spec/**`, `CLAUDE.md`, `.github/**`, and mente-smt's dependencies.

## Invariants

- [x] The rule that refuses an impure condition, scrutinee, clause or operand is on main (pull request 196, E0236).
- [ ] Each place the encoding evaluated such an expression with effects is listed in the pull request, with the code removed or turned into the assertion.
- [x] A test hands the encoding a condition that writes a binding: a tree the checker would refuse, built for the test, the way `test_a_check_that_loses_its_site_is_verified_less_not_moved` builds an encoder that loses sites. The export reports it as a defect and exits 1. The test fails with the assertion planted away.
- [ ] The agreement tests find no disagreement, and the export's keys on the job's corpus, built on CI, match the base's. The comparison with the base shows no check verified less.
