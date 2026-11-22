---
date: 2026-11-17
fixed-by: https://github.com/Menteion/Mente/pull/175
area: local checks
missed-by: The record's tests planted one bad entry at a time, so none saw a run stop at the first kind of problem and leave a broken file unreported.
guard: fn test_every_problem_of_the_record_is_reported_at_once in mente-cli
---
