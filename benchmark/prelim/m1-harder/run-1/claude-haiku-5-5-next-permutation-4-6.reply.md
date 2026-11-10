I'm not going to submit another revision. The function hasn't changed in substance for four rounds, and the feedback is identical each time, so the next version would be a guess.

Every version I've written mutates `a`, either element by element or by copying into `b` and assigning `a = b`. All of them return the right `bool` and fail the leaves check the same way. A change to the function's internals can't alter that, so the cause is outside the function body. The likely causes are the harness passing a literal as the `mut` argument, or the interpreter not copying `mut` array parameters back to the caller.

I need one piece of information to decide which. Run this and send me the output:

```text
var v: [6]i32 = [1, 2, 3, 4, 5, 6]
next_permutation(mut v)
v
```

- If `v` prints `[1, 2, 3, 4, 6, 5]`, the function is correct, and the fault is in the harness's handling of literal `mut` arguments. The examples need to use variables, or the harness needs fixing.
- If `v` still prints `[1, 2, 3, 4, 5, 6]`, the interpreter isn't copying `mut` array parameters back. That's a bug in the reference interpreter, and no version of this function will pass until it's fixed.

Once you have that output, I'll write the examples or the function to match.