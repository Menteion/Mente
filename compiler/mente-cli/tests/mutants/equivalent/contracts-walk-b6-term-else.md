---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b6.term/else
reason: this computes whether the measure equals the last one, which is and-ed with false; the measure already passed the check that it is not negative, so the cast this arm makes is exact too
---

A measure of one clause compares lexicographically with nothing after it: `smaller or (equal and false)`. The equality is and-ed with false, so nothing that computes it changes the result.
