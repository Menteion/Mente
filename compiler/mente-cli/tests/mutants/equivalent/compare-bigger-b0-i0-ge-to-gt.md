---
mutant: compiler/mente-cli/tests/mutants/compare.mn::bigger/b0.i0/ge>gt
reason: '`>=` and `>` differ only at a == b, where both arms give the same value'
---

An `if` whose arms agree where the comparison changes.
