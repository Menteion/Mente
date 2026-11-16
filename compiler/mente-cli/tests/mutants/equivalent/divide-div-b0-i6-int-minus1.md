---
mutant: compiler/mente-cli/tests/mutants/divide.mn::div/b0.i6/int-1
reason: a quotient of two i32 is at least -2147483648 (-2147483648 / 1), so `>= -2147483649` holds wherever `>= -2147483648` does
---

Arithmetic whose result always fits.
