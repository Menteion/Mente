```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var n: usize[0..=16] = 0
  var started = false
  var last: i32 = 0
  var v: i32 = 0
  while i < 8 and j < 8 {
    if a[i] <= b[j] {
      v = a[i]
      i += 1
    } else {
      v = b[j]
      j += 1
    }
    if not started or v != last {
      guard n < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[n] = v
      n += 1
      last = v
      started = true
    }
  }
  while i < 8 {
    v = a[i]
    i += 1
    if not started or v != last {
      guard n < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[n] = v
      n += 1
      last = v
      started = true
    }
  }
  while j < 8 {
    v = b[j]
    j += 1
    if not started or v != last {
      guard n < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[n] = v
      n += 1
      last = v
      started = true
    }
  }
  n
}
```