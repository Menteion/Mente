---
date: 2026-11-17
fixed-by: https://github.com/Menteion/Mente/pull/177
area: local checks
missed-by: The placement test checked only which entries it found, never its message, so the message cited an entry that does not state the rule and named no fix, and the rule stood in no rule of the register.
guard: fn test_a_decided_entry_without_a_place_is_found in mente-cli
---
