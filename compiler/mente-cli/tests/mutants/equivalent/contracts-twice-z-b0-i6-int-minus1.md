---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::twice/b0.i6/int-1
group: doubled-value-even
reason: x * 2 is even, never 255, so `<= 254` and `<= 255` agree
---
