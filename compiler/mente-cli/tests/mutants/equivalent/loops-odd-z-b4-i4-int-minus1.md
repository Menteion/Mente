---
mutant: compiler/mente-cli/tests/mutants/loops.mn::odd/b4.i4/int-1
group: unsigned-sum-or-product-never-negative
reason: s + w of two u8 is never negative, so `>= -1` holds wherever `>= 0` does
---
