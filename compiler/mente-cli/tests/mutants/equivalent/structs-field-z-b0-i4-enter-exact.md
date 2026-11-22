---
mutant: compiler/mente-cli/tests/mutants/structs.mn::field/b0.i4/enter-exact
group: count-made-exact
reason: field is entered from outside alone, so its call of bump stands on two frames of a few hundred bytes, far below the budget
---
