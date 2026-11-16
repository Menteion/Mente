---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::down/b1.i13/check-removed
reason: the measure m of the recursive call is a u8, never negative, so the check that it is not negative never fails
---

A `decreases` measure that is a u8 is never negative.
