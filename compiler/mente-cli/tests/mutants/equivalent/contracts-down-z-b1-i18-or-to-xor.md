---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::down/b1.i18/or>xor
group: one-clause-measure
reason: the right side, `equal and false`, is always false, and `smaller xor false` is `smaller or false`
---
