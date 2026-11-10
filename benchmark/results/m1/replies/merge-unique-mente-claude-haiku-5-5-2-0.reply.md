```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize[0..=16] = 0
  var seen = false
  var last: i32 = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i >= 8 {
      v = b[j]
      j += 1
    } else if j >= 8 {
      v = a[i]
      i += 1
    } else if a[i] <= b[j] {
      v = a[i]
      i += 1
    } else {
      v = b[j]
      j += 1
    }
    if not seen or v != last {
      guard k < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[k] = v
      k += 1
      last = v
      seen = true
    }
  }
  k
}
```