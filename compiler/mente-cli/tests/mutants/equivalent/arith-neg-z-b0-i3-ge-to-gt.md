---
mutant: compiler/mente-cli/tests/mutants/arith.mn::neg/b0.i3/ge>gt
reason: '-a of an i8 lies in -127..=128, never at -128, so `> -128` and `>= -128` agree'
---
