---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i25/check-removed
group: index-checked-for-read-and-write
reason: the write's index check repeats the read's on the same unchanged i, which already trapped wherever this one would
---
