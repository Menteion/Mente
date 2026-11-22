---
date: 2026-11-15
fixed-by: https://github.com/Menteion/Mente/pull/155
area: checker
missed-by: The conformance frames, which held no `.len` of a place and no branch whose value follows an expression statement, so two mutants of the frame module survived.
guard: fn test_conformance_programs_give_the_frames_they_state in mente-sema
---
