I can't make further progress on the function body, because the failure report contains no information about the array the function leaves behind. Four versions with different write strategies have all returned the same result, and I've traced both examples by hand: the expected `leaves` arrays are what this algorithm produces. So the failing check is probably not in the merge logic.

Instead of a fifth rewrite, I'd like to isolate the `mut` behavior with a function that does nothing else. Run this one through the same harness, with a `leaves` check on the first slot:

```mente
pub fn mut_probe(mut out: [4]i32) -> i32 {
  out[0] = 7
  0
}
```

Call it as `mut_probe(mut [0, 0, 0, 0]) == 0 leaves [7, 0, 0, 0]`.

- If that passes, element writes to a `mut` array are copied back correctly, and the problem is specific to something in `merge_unique`. In that case, please also send the actual `leaves` value the harness printed for the first example, and I'll compare it slot by slot with the expected array.
- If that fails, the interpreter is not copying `mut` array elements back on return, and the bug is in the interpreter, not in this function. Please report that to whoever owns the interpreter.

Once I have one of those results, I'll revise `merge_unique` or point at the interpreter bug.