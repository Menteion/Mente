---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::grid/b5.i8/and>or
reason: every element of the literal is in 0..=255, where `>= 0 and <= 255` and `>= 0 or <= 255` are both true
---

Narrowing an array literal into its declared type: each element is a constant that fits, 0 and 255 among them, so the check never fails. A literal without a 0 or a 255 would leave more of these; the corpus's literals hold both.
