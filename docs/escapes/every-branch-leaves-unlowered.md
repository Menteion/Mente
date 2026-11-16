---
date: 2026-11-14
fixed-by: https://github.com/Menteion/Mente/pull/129
area: MIR
missed-by: The program generator, which wrote no value whose every branch leaves, so its lowering failed unseen.
guard: fn test_a_value_whose_every_branch_leaves_lowers_to_no_value
---
