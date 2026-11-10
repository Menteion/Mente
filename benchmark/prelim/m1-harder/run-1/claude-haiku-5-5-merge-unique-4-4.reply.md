I still can't make the leaves match from inside the function, and I don't want to keep resubmitting variants that I can't verify. The function's logic is the same as in my last reply, and I traced it by hand on both examples. It produces the expected leaves.

One hypothesis fits all four reports. Both examples pass a literal, such as `mut [0, 0, ...]`, as the `mut` argument. The card says a `mut` parameter receives a place, which means a variable, not a literal. The interpreter may be unable to copy the result back into a literal, so the leaves never match. If so, the failure is in how the examples are written, not in the function.

Here is the same function, unchanged:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var n: usize = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i < 8 {
      if j < 8 {
        if a[i] <= b[j] {
          v = a[i]
          i += 1
        } else {
          v = b[j]
          j += 1
        }
      } else {
        v = a[i]
        i += 1
      }
    } else {
      v = b[j]
      j += 1
    }
    if n == 0 {
      out[n] = v
      n += 1
    } else {
      if out[n - 1] != v {
        out[n] = v
        n += 1
      }
    }
  }
  guard n <= 16 else {
    unreachable("at most 16 distinct values come from two arrays of 8")
  }
  n
}
```

To test the hypothesis, the harness should call each example with a variable, for example `var out: [16]i32 = [0, 0, ...]` and then `merge_unique(a, b, mut out)`, and check `out` afterward. If that passes, the problem was the literal argument, and the examples need changing, not the function. If it still fails, please send me the full `leaves` array the interpreter returned for the first example.