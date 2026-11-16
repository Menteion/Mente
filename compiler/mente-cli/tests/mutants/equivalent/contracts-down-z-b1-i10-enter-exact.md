---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::down/b1.i10/enter-exact
group: count-made-exact
reason: down recurses at most 255 calls deep, since its measure n is a u8 that each call makes smaller, and 255 of its frames are far below the budget
---
