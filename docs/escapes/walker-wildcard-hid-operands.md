---
date: 2026-11-14
fixed-by: https://github.com/Menteion/Mente/pull/129
area: checker
missed-by: 'No lint: a walker''s `_` arm hid the operand kinds, and the checker rejected an operand that leaves its statement.'
guard: fn test_exhaustive_crates_deny_wildcard_arms in mente-cli
---
