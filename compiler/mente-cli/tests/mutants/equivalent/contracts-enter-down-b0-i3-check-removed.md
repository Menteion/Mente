---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::enter down/b0.i3/check-removed
reason: the entry's measure n is a u8, never negative, so the check that it is not negative never fails
---

A `decreases` measure that is a u8 is never negative.
