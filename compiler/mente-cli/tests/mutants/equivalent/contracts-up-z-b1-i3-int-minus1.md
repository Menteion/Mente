---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::up/b1.i3/int-1
group: unsigned-sum-or-product-never-negative
reason: x + d of two u8 is never negative, so `>= -1` holds wherever `>= 0` does
---
