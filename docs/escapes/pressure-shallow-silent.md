---
date: 2026-11-17
fixed-by: https://github.com/Menteion/Mente/pull/176
area: local checks
missed-by: The pressure tests fed hand-written logs alone, so no test ran the log command on a real history, whole or shallow, and a shallow one was counted without a word.
guard: fn test_the_log_command_reads_a_real_history in mente-cli
---
