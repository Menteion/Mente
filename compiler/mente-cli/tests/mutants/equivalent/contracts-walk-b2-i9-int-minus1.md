---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b2.i9/int-1
reason: the measure already passed the check that it is not negative, so `< -1` is false like `< 0`
---

The loop measure's comparison runs after the check that the measure is not negative.
