---
date: 2026-11-17
fixed-by: https://github.com/Menteion/Mente/pull/178
area: local checks
missed-by: The key's tests took names from the corpus alone, none with a capital or a word that starts with a digit, so the key was never tried on every valid function name.
guard: fn test_mutant_keys_are_total_and_distinct in mente-cli
---
