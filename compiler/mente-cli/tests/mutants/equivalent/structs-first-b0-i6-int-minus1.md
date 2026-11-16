---
mutant: compiler/mente-cli/tests/mutants/structs.mn::first/b0.i6/int-1
reason: p.a + k of two u8 is never negative, so its narrowing's lower bound holds as `>= -1` wherever it holds as `>= 0`
---

A sum of two u8 fields or parameters, narrowed into u8: never negative, as arith.mn's add.
