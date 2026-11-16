---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::down/b1.i10/enter-exact
reason: down recurses at most 255 calls deep, since its measure n is a u8 that each call makes smaller, and 255 of its frames are far below the budget
---

A count made exact (`enter-exact`) differs from the original only where the stack runs out at its call (`core-calculus.md`, item 25). None of these calls is reached through a recursion but `down`'s, which `decreases n`, a u8, holds to 255 calls: whatever the input, the stack at each holds a few frames of a few hundred bytes at most, far below the budget of 1,048,576 bytes. stack.mn's recursion reaches the budget, and kills the operator there.
