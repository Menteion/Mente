I've now tried three ways of writing the result, and all three give the same failure: the count is correct, but the `mut` argument's contents don't match. I traced the merge by hand on both examples, and the logic produces the expected leaves, so I don't think the bug is in the merge. The likely cause is how the reference interpreter writes a `mut` array argument back, or how the harness reads it. I can't see that from here, so I'm not going to keep rewriting the body. Here is the simplest correct version of the merge:

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

To go further I need one of these from the interpreter: the full `leaves` array it actually produced for the first example, or a minimal repro showing a `mut` array element write not reaching the caller. With that I can tell whether the function or the write-back is at fault.