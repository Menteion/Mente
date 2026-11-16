---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::twice/b0.i6/int-1
reason: x * 2 is even, never 255, so `<= 254` and `<= 255` agree
---

x * 2 is even, so it is never 255: below 256 it is at most 254.
