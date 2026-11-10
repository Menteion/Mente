I'm not going to send another copy of the same function. I've now sent the same algorithm five times, and that isn't helping you.

The failure pattern points at the interpreter rather than the function. Return values are correct, and the array is wrong even in the one-swap case. The `leaves` mismatch doesn't depend on the loop structure either. I haven't been able to find a change to the function that would make `leaves` match.

Here is a diagnostic that isolates the question. It writes to a `mut` array using only constant indices, with no loops and no computed indices.

```mente
pub fn touch(mut a: [6]i32) -> bool {
  a[4] = 6
  a[5] = 5
  true
}
```

Run it as `var v: [6]i32 = [1, 2, 3, 4, 5, 6]`, then `touch(mut v)`, then read `v`.

- If `v` is `[1, 2, 3, 4, 6, 5]`, the `mut` write-back works, and the problem is in how the function body writes to `a` with computed indices or inside loops. I can then rewrite the function to avoid that.
- If `v` is unchanged, the interpreter is not writing `mut` parameters back at all. That is a bug in the interpreter, and no version of this function can pass until it's fixed.

Please send me the value of `v` after the call.