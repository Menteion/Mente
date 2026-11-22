---
mutant: compiler/mente-cli/tests/mutants/arrays.mn::bump/b3.i23/lt>le
group: index-checked-for-read-and-write
reason: the write's index check repeats the read's on the same unchanged i, which already trapped at 5, so `<= 5` and `< 5` agree
---
