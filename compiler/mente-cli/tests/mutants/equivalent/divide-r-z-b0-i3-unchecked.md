---
mutant: compiler/mente-cli/tests/mutants/divide.mn::r/b0.i3/unchecked
group: arithmetic-always-fits
reason: a % b has a magnitude below |b| and the sign of a, so it always fits in i128 and its overflow check never fires
---
