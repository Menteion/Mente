I couldn't find a defect. Tracing both examples by hand gives the expected return value and the expected contents of `out`, and the interpreter's report shows the return value matching (12 and 8). The mismatch it flags must be in the `leaves` check, and I can't see what it compares. I've gone back to the simplest version, which writes straight into `out`:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
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