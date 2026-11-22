---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b9.i15/int-1
group: unsigned-sum-or-product-never-negative
reason: i + step of two u8 is never negative, so `>= -1` holds wherever `>= 0` does
---
