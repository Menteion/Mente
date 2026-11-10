I'm not going to resubmit the function. It's the same logic as the last six replies, and the `leaves` failure hasn't changed.

I've checked the algorithm by hand against all three examples, and it gives the expected arrays and booleans. Since the same three failures show up across every implementation I've written, the cause is most likely outside the function body.

The one test that can separate the two causes is the variable check from my earlier reply. Please run `var v: [6]i32 = [1, 2, 3, 4, 5, 6]`, then `next_permutation(mut v)`, and check whether `v` ends as `[1, 2, 3, 4, 6, 5]`.

- If it passes, the harness can't write back to a literal passed as `mut`. The examples would need to pass a variable, and that change belongs to the harness or the example format, not the function.
- If it fails, send me the array the harness reads back after the call, and I'll fix the function to match.