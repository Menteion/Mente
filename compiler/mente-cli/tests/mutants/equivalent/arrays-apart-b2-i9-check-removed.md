---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::apart/b2.i9/check-removed
reason: every element of the literal is a constant in 0..=255, so this narrowing check never fails
---

Narrowing an array literal into its declared type: each element is a constant that fits, 0 and 255 among them, so the check never fails. A literal without a 0 or a 255 would leave more of these; the corpus's literals hold both.
