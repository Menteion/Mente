---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::down/b1.i24/int-1
reason: m + down(m, k) of two u8 is never negative, so `>= -1` holds wherever `>= 0` does
---

A sum or product of unsigned values, narrowed back: never negative, and at least its constant addend, so a lower bound moved below that changes nothing.
