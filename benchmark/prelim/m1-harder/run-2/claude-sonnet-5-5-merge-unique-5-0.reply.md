```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var n: usize[0..=16] = 0
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
      guard j < 8 else {
        unreachable("both inputs are exhausted")
      }
      v = b[j]
      j += 1
    }
    var fresh = true
    if n > 0 {
      if out[n - 1] == v {
        fresh = false
      }
    }
    if fresh {
      guard n < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[n] = v
      n += 1
    }
  }
  n
}
```