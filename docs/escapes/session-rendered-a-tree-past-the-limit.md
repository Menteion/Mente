---
date: 2026-11-23
fixed-by: https://github.com/Menteion/Mente/pull/224
area: checker
missed-by: The nesting tests and the fuzz tests' deep shapes held only that a chain past the limit is E0127 and crashes no phase, not that the parser hands out no tree past the limit, nor what the incremental session does with one; mn fuzz's time limit caught it.
guard: fn test_a_loop_nests_no_tree_past_the_limit in mente-syntax
---

`mn fuzz` (issue 220) found the incremental session taking 185 s on a 4 MB module, an `ensures` of a million links of `+ 1`, that the whole module's check reported in 1 s. Both reported one diagnostic, `E0127`, a tree past the nesting limit.

The body parser stopped at the limit only where it descended. A chain it reads in a loop, `a + b + c` or `x[0][0]`, pushes all it has read one level deeper at each link, which no descent counts, so the parser built the whole tree, measured it afterwards and handed it out with the error. The whole check stops at a body's parse error. The session went on to build its callers' context from the tree, rendering each clause into text by nesting each link's text in the next: work growing with the square of the links, and on AArch64 a stack overflow.

The parser now measures what a loop nests one level deeper before it does, from the height of the operand, worked out once per node (`Parser::nests`, `Heights`). A condition and an operator's right side are read one level below the node that holds them, and a parenthesis counts as a level of the parser's descent and none of the tree's, so the count is the tree's own. At the limit the parser reports `E0127` and cuts the tree, as a descent does, so no tree it hands out is higher. A tree higher all the same, by a construct its count missed, is withheld (`within_the_limit`).

`test_a_loop_nests_no_tree_past_the_limit` holds each loop-built shape to exactly the limit and one link more to `E0127` with a tree no higher; `test_a_session_reports_what_the_whole_check_does_past_the_nesting_limit` holds the session to the whole check's diagnostics on those shapes far past the limit; the fuzz tests' deep shapes and the conformance program `chain_beyond_256_levels.mn` hold the chains too.
