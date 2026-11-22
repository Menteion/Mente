---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i22/int+1
group: index-checked-for-read-and-write
reason: the write's index check repeats the read's on the same unchanged i, which already trapped at 5, so `< 6` and `< 5` agree
---
