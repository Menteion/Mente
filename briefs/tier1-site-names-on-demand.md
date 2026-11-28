---
id: tier1-site-names-on-demand
status: deferred
trigger: when pull request 216 merges
type: chore
affectedFiles: [compiler/mente-sema/src/tier1.rs, compiler/mente-sema/src/octagon.rs, briefs/tier1-site-names-on-demand.md, briefs/archive/tier1-site-names-on-demand.md]
protectedFiles: [.github/**, CLAUDE.md, spec/**, compiler/mente-syntax/**]
---

# Name Tier 1's variables only when an internal error names one

## Intent

Tier 1's setup, `Fx::new` in `compiler/mente-sema/src/tier1.rs`, writes a name for every variable before it proves anything (`site_names`): "in `f`, the value of `x % 3` at 4:5" for a site variable, copying the source text of every expression of the function, those with no variable too. The names serve one reader: the internal error the octagon stops the compile with when a read meets an evaluation the state does not hold (`Octagon::name_of` in `octagon.rs`), which should never fire. Every check of every function still pays for them: the text of each expression, so work and memory of the function's size times its height, up to 256 times its size at the nesting limit. Issue 220's probe measured 415 ms for this step alone on one function of 125,000 expressions, a tree the parser no longer hands out.

When this round is done, a name is made only when an internal error asks for one, from what the octagon is given: the function's name, each variable's kind and span, and the function's source, shared rather than copied. The error's text stays the same. A deterministic count holds the setup linear, on the meter pull request 216 gives Tier 1. Its owner is stack-budget, once pull request 216, which rewrites `tier1.rs`, has merged.

## Negative Scope

- No change to the internal error's text, or to when it fires.
- No change to what Tier 1 proves or reports.
- No wall-clock bound: the guard is a count, the same on every runner.

## Invariants

- [ ] Tier 1's diagnostics on the conformance suite and the examples are unchanged, as the golden diagnostics show.
- [ ] `node tools/quick.mjs` passes before the push.

## Acceptance Criteria

- [ ] A test plants a stale read and holds the internal error's text equal to today's.
- [ ] A test holds the setup's count of work linear in a function's size and height, and fails on today's eager names.
