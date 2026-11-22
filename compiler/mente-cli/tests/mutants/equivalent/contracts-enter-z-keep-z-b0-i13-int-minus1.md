---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::enter keep/b0.i13/int-1
group: mut-refinement-checked-again
reason: k after keep returns is 2 * k0 below 20, so at most 18, and `< 19` holds wherever `< 20` does
---
