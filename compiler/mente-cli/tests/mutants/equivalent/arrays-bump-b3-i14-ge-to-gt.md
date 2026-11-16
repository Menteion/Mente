---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i14/ge>gt
reason: xs[i] + 10 of a u8 is at least 10, never 0, so `> 0` and `>= 0` agree
---

A sum or product of unsigned values, narrowed back: never negative, and at least its constant addend, so a lower bound moved below that changes nothing.
