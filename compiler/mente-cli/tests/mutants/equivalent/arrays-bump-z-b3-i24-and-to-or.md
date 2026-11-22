---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i24/and>or
group: index-checked-for-read-and-write
reason: the write's index check repeats the read's on the same unchanged i, which passed it, so the check holds however its halves combine
---
