---
date: 2026-11-22
fixed-by: https://github.com/Menteion/Mente/pull/208
area: Tier 1
missed-by: The comparison a pull request runs, which checked 400 generated programs, too few to hold one compound remainder by a site value in about 220; and lin_add, which cancelled two terms of one variable without comparing the evaluations they read, so that the empty state needed no read of either.
guard: fn test_a_value_of_two_evaluations_of_one_variable_stops_the_compile in mente-sema
root: held
---

A compound remainder `y %= e`, where `e` has a site variable (an index, a field read through an index, a remainder, an `if` or a `match`), gave its result `e`'s site variable. It then related the remainder to its divisor through that one variable, `v <= v - 1`, which emptied the state, and Tier 1 proved whatever followed. That was a wrong proof on `main` from pull request 43 (4d9d4fe, 2026-11-09), the first Tier 1, until pull request 197's stamps turned it into an internal error. The 4000 generated programs that pull request 204 checked without running met it in 18.

The fix gives each compound remainder a variable of its own. Two guards hold the class. The comparison now checks ten times as many seeds as it runs, at most 4000. And `lin_add` refuses one value that combines two evaluations of a variable, which catches the empty state at the sum that makes it, whether or not a later relation reads a stamp.
