---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::twice/b0.i4/int-1
group: unsigned-sum-or-product-never-negative
reason: x * 2 of a u8 is never negative, so `>= -1` holds wherever `>= 0` does
---
