I'm resubmitting the same function, because I still have no evidence of a bug in it. Both examples return the expected values (12 and 8), and tracing them by hand gives exactly the expected `out` arrays. The report never shows the `out` it actually saw, and seven structurally different versions produced the identical report. That points to the `leaves` check rather than the algorithm.

I won't change the code again without something to go on. A print of the interpreter's actual `out` for the second example (the all-`-1` one) would settle it.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  while i < 8 or j < 8 invariant k <= 16 {
    var v: i32 = 0
    if i < 8 {
      v = a[i]
      if j < 8 {
        if b[j] < v {
          v = b[j]
        }
      }
    } else {
      if j < 8 {
        v = b[j]
      }
    }
    if k < 16 {
      out[k] = v
      k += 1
    }
    while i < 8 {
      if a[i] != v {
        break
      }
      i += 1
    }
    while j < 8 {
      if b[j] != v {
        break
      }
      j += 1
    }
  }
  k
}
```