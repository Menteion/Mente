---
mutant: compiler/mente-cli/tests/mutants/traps.mn::noisy/b0.i4/ge>gt
reason: x + 1 of a u8 is at least 1, never 0, so `> 0` and `>= 0` agree
---

A sum or product of unsigned values, narrowed back: never negative, and at least its constant addend, so a lower bound moved below that changes nothing.
