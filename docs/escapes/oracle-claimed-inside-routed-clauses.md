---
date: 2026-11-22
fixed-by: https://github.com/Menteion/Mente/pull/215
area: harness
missed-by: The conformance suite and the containment oracle's own unit test, neither of which held a routed `ensures` with a block in it, so the oracle never met a point Tier 1 walks nowhere by design.
guard: fn test_tier1_states_contain_every_value_the_interpreter_reaches in mente-cli
root: held
---

The first version of the containment oracle read a statement point with no state recorded as one Tier 1 held unreachable, and reported it when the reference interpreter reached it. That reading holds only where Tier 1 walks. Tier 1 walks a routed `ensures`, one outside its fragment that `@checked` checks at run time and `@prove` trusts, nowhere, while the interpreter evaluates it at every return. So an `ensures (if result > 3 { result > 3 } else { result <= 3 })` on a `@checked` function, or a `match` with a block arm on a `@prove` one, raised a false alarm by construction. CI was green only because no program yet held such a clause. The first review of pull request 215 found it.

Tier 1 now names the clauses it never enters (`States::unwalked`), and the oracle makes no claim at any node under them, which `Tree::subtree` finds. The conformance program `routed_clauses_holding_blocks.mn` and the oracle's unit test hold such clauses. Since pull request 217 the program generator writes them too, so that the oracle meets them in every comparison: `@checked` and `@prove` functions whose `requires`, `ensures`, loop invariants and measures hold an `if` or a `match` with a block arm. A second guard holds that: `test_generated_programs_reach_every_construct_of_the_grammar` in mente-cli requires each kind of clause holding each form under each attribute, each of its ways taken by a run, and a `requires` of a `@checked` function that traps a call.
