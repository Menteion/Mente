---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i23/lt>le
reason: the write's index check repeats the read's on the same unchanged i, which already trapped at 5, so `<= 5` and `< 5` agree
---

`xs[i] += 10` checks the index once for the read and again for the write, at the same place and with the same message: i has not changed, so the read's check fails first and the write's never.
