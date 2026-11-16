---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i25/check-removed
reason: the write's index check repeats the read's on the same unchanged i, which already trapped wherever this one would
---

`xs[i] += 10` checks the index once for the read and again for the write, at the same place and with the same message: i has not changed, so the read's check fails first and the write's never.
