---
date: 2026-11-16
fixed-by: https://github.com/Menteion/Mente/pull/204
area: generator
missed-by: The program generator's own checks, which held its model of the type a literal takes against nothing but the checker's refusals, so that a program the checker accepted with another type than the model's passed unseen; and the comparison a pull request runs, whose 400 programs were too few to hold one the checker refused in about 250 to 500.
guard: fn test_a_literal_written_in_another_type_than_the_checker_gives_is_a_problem in mente-cli
---

From pull request 138 (ddb445c, 2026-11-15), the program generator wrote a bit operation of literals alone, `~0` or `8 & 0`, in the type of the operand it stood beside. The checker gives such literals the type of the smallest expression around them that has one (`core-calculus.md`, item 5), which the operands written after them decide as much as the one before. From 2026-11-16 the nightly comparison met a program the checker refused for it, `E0212` or `E0222`, in one of 250 to 500, and a program it accepted could run with another value than the generator modelled, which nothing compared.

The fix keeps, with each expression the generator writes, the type its bit operations of literals alone were written in, and refuses every place that would give them another. The guard holds the class wherever that model is held: for every program the generator writes and the checker accepts, `literals_as_written` holds the type each such operation was written in against the checker's own table of the types literals take, in the coverage test and in the seeds the comparison checks without running, so that dropping any one of the generator's checks fails at once.
