---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::enter keep/b0.i13/int+1
reason: keep's body already checked k against [0..20) after its only change, so `< 21` holds wherever `< 20` does
---

The entry checks the `mut` place's refinement again after the call returns. keep's own body checks it after its one change, the call of twice, so the entry's check never fails, and k is then 2 * k0 below 20, so at most 18.
