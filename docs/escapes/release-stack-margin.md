---
date: 2026-11-16
fixed-by: https://github.com/Menteion/Mente/pull/158
area: interpreter
missed-by: The debug tests, whose stack margin was larger than a release build's, so a deep recursion crashed only in release.
guard: fn test_a_segment_starts_from_the_least_a_switch_begins_with in mente-interp
root: held
---
