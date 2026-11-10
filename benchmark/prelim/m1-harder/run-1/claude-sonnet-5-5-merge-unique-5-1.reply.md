```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var n: usize[0..=16] = 0
  var v: i32 = 0
  var take_a = false
  var fresh = false
  while i < 8 or j < 8 {
    take_a = false
    if i < 8 {
      if j < 8 {
        take_a = a[i] <= b[j]
      } else {
        take_a = true
      }
    }
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
    fresh = true
    if n > 0 {
      guard n >= 1 else {
        unreachable("n > 0")
      }
      if out[n - 1] == v {
        fresh = false
      }
    }
    if fresh {
      guard n < 16 else {
        unreachable("distinct values written never exceed 16")
      }
      out[n] = v
      n += 1
    }
  }
  n
}
```