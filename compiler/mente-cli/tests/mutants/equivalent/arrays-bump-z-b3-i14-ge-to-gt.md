---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i14/ge>gt
group: unsigned-sum-or-product-never-negative
reason: xs[i] + 10 of a u8 is at least 10, never 0, so `> 0` and `>= 0` agree
---
