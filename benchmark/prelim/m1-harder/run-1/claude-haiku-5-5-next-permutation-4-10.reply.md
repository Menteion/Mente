I'm not going to send the function again. It hasn't changed since the second revision, and the leaves check has failed the same way for every version. The three examples in the task call the function with array literals as `mut` arguments, and I can't change those.

Two things would let me make progress:

1. The output of the variable check: `var v: [6]i32 = [1, 2, 3, 4, 5, 6]`, then `next_permutation(mut v)`, then `v`. If `v` is `[1, 2, 3, 4, 6, 5]`, the function is correct and the harness is failing on literal `mut` arguments.
2. The card's rule for a `mut` argument that is a literal, if the spec has one. If literals aren't places, the examples can't pass as written, and the task needs changing.