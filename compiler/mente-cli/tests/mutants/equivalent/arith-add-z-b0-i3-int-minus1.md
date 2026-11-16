---
mutant: compiler/mente-cli/tests/mutants/arith.mn::add/b0.i3/int-1
reason: a + b of two u8 is never negative, so its narrowing's lower bound holds as `>= -1` wherever it holds as `>= 0`
---
