---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b6.term/swap
reason: this computes whether the measure equals the last one, which is and-ed with false, so the result is false whatever it is
---

A measure of one clause compares lexicographically with nothing after it: `smaller or (equal and false)`. The equality is and-ed with false, so nothing that computes it changes the result.
