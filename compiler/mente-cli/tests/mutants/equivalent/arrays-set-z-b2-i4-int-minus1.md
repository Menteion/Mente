---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::set/b2.i4/int-1
group: array-literal-constants-fit
reason: every element of the literal is a constant in 0..=255, so `>= -1` holds wherever `>= 0` does
---
