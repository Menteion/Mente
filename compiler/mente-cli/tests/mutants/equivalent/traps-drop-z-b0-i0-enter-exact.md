---
mutant: compiler/mente-cli/tests/mutants/traps.mn::drop/b0.i0/enter-exact
group: count-made-exact
reason: drop is entered from outside alone, so its call of noisy stands on two frames of a few dozen bytes, far below the budget
---
