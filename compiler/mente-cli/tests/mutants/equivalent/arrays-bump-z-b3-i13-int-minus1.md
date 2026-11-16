---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i13/int-1
group: unsigned-sum-or-product-never-negative
reason: xs[i] + 10 of a u8 is at least 10, so `>= -1` holds wherever `>= 0` does
---
