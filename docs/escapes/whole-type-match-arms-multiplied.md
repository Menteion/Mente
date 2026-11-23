---
date: 2026-11-23
fixed-by: https://github.com/Menteion/Mente/pull/222
area: generator
missed-by: The comparison a pull request runs and the coverage test, whose fixed seeds held no function whose matches of 256 arms nest, and no bound on the size of a generated function, so that a function of 201880 expressions and statements, and one whose E0508 claims sat 1219 to a function, passed every check but the nightly's.
guard: fn test_a_function_past_the_bound_of_its_size_is_a_problem in mente-cli
---

From 7529877 (2026-11-12), the program generator writes a match that covers an 8-bit type by its literals: 256 arms, each a full expression or a block, of which a call takes one, so the rest are text no call runs, and an arm could hold another such match. A function's text multiplied 256 times a level, to 201,880 expressions and statements over seeds 0 to 2999. An arm that narrowed a binding the scrutinee reads, which the arm's pattern bounds, held a narrowing Tier 1 proves always traps (`E0508`), and the few hundred calls that check those claims reached few of them. The nightly of the range of 2026-11-22 found 1635 of its 2021 claims never reached, above the 1114 its bar allows, 1219 of them in one function of 13,715 lines (run 45061416116 on 9e6858d).

A first fix kept the arms full and only kept them from nesting another such match and from the scrutinee's bindings. Its own measurement over seeds 0 to 15999 found a function of 57,532 with no match nested at all: four matches of 256 arms, each arm calling a function of refined parameters, and each call a guard of each argument before the statement. Each arm of a match past 16 arms is now a leaf, a literal or a discard of one (`MAX_FULL_ARMS`), which reads no binding and nests nothing. Two rates give back the constructs the arms' mass reached most: a divisor known to be zero, and the return of an array of no type.

The guard holds the class: every attempt the generator writes is held to a bound on a function's expressions and statements (`MAX_FUNCTION_NODES`, 24,000, four times the largest function of 42,000 programs measured on runners), in `generate::checked`, so the coverage test, every comparison and every nightly fail on a function whose text multiplies, and each run reports its largest function.
