---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::enter keep/b0.i18/and>or
group: mut-refinement-checked-again
reason: keep's body already checked k against [0..20) after its only change, so both halves hold and `or` gives what `and` gives
---
