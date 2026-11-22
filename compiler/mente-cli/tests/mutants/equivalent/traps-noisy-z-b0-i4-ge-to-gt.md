---
mutant: compiler/mente-cli/tests/mutants/traps.mn::noisy/b0.i4/ge>gt
group: unsigned-sum-or-product-never-negative
reason: x + 1 of a u8 is at least 1, never 0, so `> 0` and `>= 0` agree
---
