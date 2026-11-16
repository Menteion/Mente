---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::down/b1.i18/or>xor
reason: the right side, `equal and false`, is always false, and `smaller xor false` is `smaller or false`
---

A measure of one clause compares lexicographically with nothing after it: `smaller or (equal and false)`. The equality is and-ed with false, so nothing that computes it changes the result.
