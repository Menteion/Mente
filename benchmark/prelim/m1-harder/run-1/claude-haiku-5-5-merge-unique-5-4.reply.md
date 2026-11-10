I can't find the cause of the leaves mismatch from these results, and I've made five attempts that all fail the same way, so I'm not confident another body change will fix it.

I traced the algorithm by hand on the first example. It produces 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13, 15 and returns 12, which matches the expected leaves. The second example also traces correctly. The return values match in every run, so the merge and dedupe logic appears right. The failure is in how the `mut out` contents come back to the caller. My guesses about element writes, whole-array assignment, and explicit `return` didn't change that. The report doesn't show the actual leaves, so I can't tell whether the interpreter drops the write-back, or the array contents are wrong in some way I can't see.

Here is the current version, which uses whole-array assignment and an explicit `return`:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var res: [16]i32 = out
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  var last: i32 = 0
  var seen = false
  while i + j < 16 {
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
    if not seen or v != last {
      res[k] = v
      k += 1
      last = v
      seen = true
    }
  }
  guard k <= 16 else {
    unreachable("at most 16 distinct values")
  }
  out = res
  return k
}
```

To settle this, I need the full `leaves` array the interpreter returns for the first example, not just the count. If it shows the expected 12 values, the bug is in the mutable argument handling, and the function is fine. If it shows something else, I can locate the error from that.