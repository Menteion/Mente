---
mutant: compiler/mente-cli/tests/mutants/contracts.mn::down/b1.i5/int+1
group: arithmetic-always-fits
reason: n - k of two u8 is at most 255, so `<= 256` holds wherever `<= 255` does
---
