I could not find a fault in the logic. Every version returns the expected count, and I traced both examples by hand to the expected contents of `out`. The failure output only shows the returned value, so I cannot see what the interpreter thinks `out` holds afterwards. This is the simplest correct form, with the same two-pointer merge and a check against the last value written:

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var n: usize[0..=16] = 0
  var have = false
  var last: i32 = 0
  while i < 8 or j < 8 {
    var take_a = false
    if i < 8 {
      if j < 8 {
        take_a = a[i] <= b[j]
      } else {
        take_a = true
      }
    }
    var v: i32 = 0
    if take_a {
      guard i < 8 else {
        unreachable("take_a implies i < 8")
      }
      v = a[i]
      i += 1
    } else {
      guard j < 8 else {
        unreachable("when a is not taken, the loop condition leaves j < 8")
      }
      v = b[j]
      j += 1
    }
    if not have or v != last {
      guard n < 16 else {
        unreachable("distinct values written never exceed 16")
      }
      out[n] = v
      n += 1
      last = v
      have = true
    }
  }
  n
}
```