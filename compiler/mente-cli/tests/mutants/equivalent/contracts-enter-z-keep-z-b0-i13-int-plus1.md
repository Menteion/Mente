---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::enter keep/b0.i13/int+1
group: mut-refinement-checked-again
reason: keep's body already checked k against [0..20) after its only change, so `< 21` holds wherever `< 20` does
---
