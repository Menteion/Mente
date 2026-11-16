---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::walk/b2.term/else
reason: the measure already passed the check that it is not negative, so this branch always takes its second target
---

The loop measure's comparison runs after the check that the measure is not negative.
