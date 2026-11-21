---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::keep/b0.i0/enter-exact
group: count-made-exact
reason: keep is entered from outside alone, so its call of twice stands on two frames of a few dozen bytes, far below the budget
---
