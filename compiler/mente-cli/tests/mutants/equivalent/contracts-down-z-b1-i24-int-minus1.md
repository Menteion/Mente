---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::down/b1.i24/int-1
group: unsigned-sum-or-product-never-negative
reason: m + down(m, k) of two u8 is never negative, so `>= -1` holds wherever `>= 0` does
---
