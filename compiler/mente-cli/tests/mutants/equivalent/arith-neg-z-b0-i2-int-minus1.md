---
mutant: compiler/mente-cli/tests/mutants/arith.mn::neg/b0.i2/int-1
reason: '-a of an i8 lies in -127..=128, so the narrowing''s lower bound holds as `>= -129` wherever it holds as `>= -128`'
---
