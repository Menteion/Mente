---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b6.term/else
group: one-clause-measure
reason: this computes whether the measure equals the last one, which is and-ed with false; the measure already passed the check that it is not negative, so the cast this arm makes is exact too
---
