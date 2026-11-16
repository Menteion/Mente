---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b4.i0/flip
reason: block 4 runs only for a negative measure, which the check before it has already trapped on
---

The loop measure's comparison runs after the check that the measure is not negative.
