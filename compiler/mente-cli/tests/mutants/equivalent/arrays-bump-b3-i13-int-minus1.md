---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i13/int-1
reason: xs[i] + 10 of a u8 is at least 10, so `>= -1` holds wherever `>= 0` does
---

A sum or product of unsigned values, narrowed back: never negative, and at least its constant addend, so a lower bound moved below that changes nothing.
