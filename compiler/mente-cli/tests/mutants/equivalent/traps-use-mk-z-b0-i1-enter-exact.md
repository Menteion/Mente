---
mutant: compiler/mente-cli/tests/mutants/traps.mn::use_mk/b0.i1/enter-exact
group: count-made-exact
reason: use_mk is entered from outside alone, so its call of mk stands on two frames of a few dozen bytes, far below the budget
---
