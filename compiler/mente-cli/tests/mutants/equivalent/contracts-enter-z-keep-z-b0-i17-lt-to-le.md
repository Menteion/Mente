---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::enter keep/b0.i17/lt>le
group: mut-refinement-checked-again
reason: keep's body already checked k against [0..20) after its only change, so k is never 20 and `<= 20` agrees with `< 20`
---
