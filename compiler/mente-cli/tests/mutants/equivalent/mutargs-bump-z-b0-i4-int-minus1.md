---
mutant: compiler/mente-cli/tests/mutants/mutargs.mn::bump/b0.i4/int-1
group: unsigned-sum-or-product-never-negative
reason: x + 1 of a u8 is at least 1, so `>= -1` holds wherever `>= 0` does
---
