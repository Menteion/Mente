---
date: 2026-11-17
fixed-by: https://github.com/Menteion/Mente/pull/175
area: local checks
missed-by: No test read the words of a refused name, so the message described the very names it refused, those with a word that starts with a digit.
guard: fn test_a_refused_name_says_which_part_of_the_rule_it_breaks in mente-cli
---
