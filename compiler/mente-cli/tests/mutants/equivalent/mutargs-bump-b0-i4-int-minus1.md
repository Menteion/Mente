---
mutant: compiler/mente-cli/tests/mutants/mutargs.mn::bump/b0.i4/int-1
reason: x + 1 of a u8 is at least 1, so `>= -1` holds wherever `>= 0` does
---

A sum or product of unsigned values, narrowed back: never negative, and at least its constant addend, so a lower bound moved below that changes nothing.
