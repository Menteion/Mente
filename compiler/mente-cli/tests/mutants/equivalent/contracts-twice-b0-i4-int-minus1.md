---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::twice/b0.i4/int-1
reason: x * 2 of a u8 is never negative, so `>= -1` holds wherever `>= 0` does
---

A sum or product of unsigned values, narrowed back: never negative, and at least its constant addend, so a lower bound moved below that changes nothing.
