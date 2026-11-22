---
date: 2026-11-17
fixed-by: https://github.com/Menteion/Mente/pull/178
area: local checks
missed-by: The gate's tests recorded only mutants of the programs they ran, and a record of another program was skipped as no stale one, so a record of a program the gate never runs passed unread.
guard: fn test_records_and_survivors_have_their_places in mente-cli
---
