---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::enter keep/b0.i19/check-removed
group: mut-refinement-checked-again
reason: keep's body already checked k against [0..20) after its only change, so this check never fails
---
