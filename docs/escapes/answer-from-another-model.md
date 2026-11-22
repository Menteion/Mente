---
date: 2026-11-16
fixed-by: https://github.com/Menteion/Mente/pull/161
area: harness
missed-by: The broker's tests, whose fake answers named no model, so nothing compared the model an answer names with the one asked for.
guard: 'node test "the broker hands back no answer from another model than the one asked: it refuses it at the worst it reserved, says so, and admits nothing more"'
root: held
---
