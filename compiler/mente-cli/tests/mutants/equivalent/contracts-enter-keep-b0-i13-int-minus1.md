---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::enter keep/b0.i13/int-1
reason: k after keep returns is 2 * k0 below 20, so at most 18, and `< 19` holds wherever `< 20` does
---

The entry checks the `mut` place's refinement again after the call returns. keep's own body checks it after its one change, the call of twice, so the entry's check never fails, and k is then 2 * k0 below 20, so at most 18.
