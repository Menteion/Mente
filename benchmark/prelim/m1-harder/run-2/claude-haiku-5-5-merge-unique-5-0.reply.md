```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var n: usize = 0
  while i < 8 or j < 8 {
    var cur: i32 = 0
    if i < 8 {
      if j < 8 {
        if a[i] <= b[j] {
          cur = a[i]
          i += 1
        } else {
          cur = b[j]
          j += 1
        }
      } else {
        cur = a[i]
        i += 1
      }
    } else {
      cur = b[j]
      j += 1
    }
    var fresh: bool = true
    if n > 0 {
      if out[n - 1] == cur {
        fresh = false
      }
    }
    if fresh {
      guard n < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[n] = cur
      n += 1
    }
  }
  guard n <= 16 else {
    unreachable("at most 16 distinct values")
  }
  n
}
```