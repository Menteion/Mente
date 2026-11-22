---
mutant: compiler/mente-cli/tests/mutants/loops.mn::total/b2.i11/int-1
group: unsigned-sum-or-product-never-negative
reason: s + step of two u8 is never negative, so `>= -1` holds wherever `>= 0` does
---
