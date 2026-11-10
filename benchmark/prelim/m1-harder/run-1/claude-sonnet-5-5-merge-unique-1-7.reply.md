I still can't find a bug in this function, and I can't explain why the interpreter reports a failure. I have rewritten it several ways, and tracing each version by hand gives the expected count and `out` contents for both examples. Every rewrite fails with the identical message. That message shows only the returned count, which is correct, and not the final contents of `out`. So I suspect how the interpreter copies the `mut` argument back, though I have not confirmed it. I am giving the same plain version below rather than changing the algorithm again.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var n: usize = 0
  var have_last: bool = false
  var last: i32 = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i < 8 {
      v = a[i]
      if j < 8 {
        if b[j] < v {
          v = b[j]
        }
      }
    } else if j < 8 {
      v = b[j]
    }
    if i < 8 {
      if a[i] == v {
        i += 1
      }
    }
    if j < 8 {
      if b[j] == v {
        j += 1
      }
    }
    var is_new: bool = true
    if have_last {
      if last == v {
        is_new = false
      }
    }
    if is_new {
      guard n < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[n] = v
      n += 1
      last = v
      have_last = true
    }
  }
  guard n <= 16 else {
    unreachable("n never exceeds 16")
  }
  n
}
```