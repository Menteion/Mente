---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::apart/b2.i8/and>or
group: array-literal-constants-fit
reason: every element of the literal is in 0..=255, where `>= 0 and <= 255` and `>= 0 or <= 255` are both true
---
