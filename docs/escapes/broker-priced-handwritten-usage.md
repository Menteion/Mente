---
date: 2026-11-16
fixed-by: https://github.com/Menteion/Mente/pull/160
area: harness
missed-by: The broker's pricing tests, which priced only usage written by hand, so the first real usage, with a field they never showed, was counted at the worst in a run.
guard: node test "the broker prices every usage the API has answered it with, at what its tokens cost"
---
