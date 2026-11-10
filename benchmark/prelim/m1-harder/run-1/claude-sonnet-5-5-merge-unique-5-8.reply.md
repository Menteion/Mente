I have no further change that I can justify. The count has matched on every attempt, and I hand-traced both examples to the expected `out` contents. The failure output only prints the returned value, so I cannot tell what the interpreter sees in `out` afterwards, and the last three structurally different versions failed identically. I am resubmitting the direct version, which writes straight into `out`:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var n: usize[0..=16] = 0
  var have = false
  var last: i32 = 0
  var v: i32 = 0
  while i < 8 or j < 8 {
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
      if j < 8 {
        v = b[j]
        j += 1
      }
    }
    if not have or v != last {
      if n < 16 {
        out[n] = v
        n += 1
      }
      last = v
      have = true
    }
  }
  n
}
```