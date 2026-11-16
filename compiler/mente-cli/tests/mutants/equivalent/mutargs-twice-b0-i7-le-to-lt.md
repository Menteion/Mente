---
mutant: compiler/mente-cli/tests/mutants/mutargs.mn::twice/b0.i7/le>lt
reason: x * 2 is even, never 255, so `< 255` and `<= 255` agree
---

x * 2 is even, so it is never 255: below 256 it is at most 254.
