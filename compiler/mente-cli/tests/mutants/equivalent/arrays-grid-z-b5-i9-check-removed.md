---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::grid/b5.i9/check-removed
group: array-literal-constants-fit
reason: every element of the literal is a constant in 0..=255, so this narrowing check never fails
---
