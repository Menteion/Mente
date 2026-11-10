I'm stuck, and sending the same function again won't help. The return values are right on all three examples, and the leaves check fails in every version, including the one that writes to `a` directly and the one that copies to `b` and assigns back. I've traced the current function by hand on each example, and it produces the expected arrays, so the logic isn't the problem.

I can't see the leaves value the harness reads, and I can't run `mn`, so I can't check the interpreter's behaviour myself. Without that, I'm guessing at the cause, and another rewrite would just be another guess.

Please send me either of these:

1. The output of this, run through `mn`:
   ```text
   var v: [6]i32 = [1, 2, 3, 4, 5, 6]
   next_permutation(mut v)
   v
   ```
   If `v` shows `[1, 2, 3, 4, 6, 5]`, the function is fine and the harness is losing the copy-back for literal `mut` arguments. The examples then need variables, or the harness needs fixing.

2. The spec text for `mut` arguments that are array literals, or for copy-back of array parameters. If it says literals aren't allowed as `mut` arguments, that explains the failure, and I'll rewrite the examples to use variables.

Once I have one of those, I'll give you the function and examples that match the interpreter's behaviour.