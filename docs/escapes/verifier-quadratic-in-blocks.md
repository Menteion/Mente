---
date: 2026-11-16
fixed-by: https://github.com/Menteion/Mente/pull/158
area: MIR
missed-by: Nothing measured the verifier on a long function, whose table grew with the square of its blocks; CI's time limit caught it.
guard: fn test_the_verifier_reads_a_long_function_in_linear_time in mente-mir
root: held
---
