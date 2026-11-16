---
mutant: compiler/mente-cli/tests/mutants/structs.mn::field/b0.i4/enter-exact
reason: field is entered from outside alone, so its call of bump stands on two frames of a few hundred bytes, far below the budget
---

A count made exact (`enter-exact`) differs from the original only where the stack runs out at its call (`core-calculus.md`, item 25). None of these calls is reached through a recursion but `down`'s, which `decreases n`, a u8, holds to 255 calls: whatever the input, the stack at each holds a few frames of a few hundred bytes at most, far below the budget of 1,048,576 bytes. stack.mn's recursion reaches the budget, and kills the operator there.
