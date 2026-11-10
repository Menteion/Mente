```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i < 8 and j < 8 {
      if a[i] <= b[j] {
        v = a[i]
        i += 1
      } else {
        v = b[j]
        j += 1
      }
    } else if i < 8 {
      v = a[i]
      i += 1
    } else {
      guard j < 8 else {
        unreachable("loop condition keeps j below 8")
      }
      v = b[j]
      j += 1
    }
    if k == 0 {
      out[0] = v
      k += 1
    } else if v != out[k - 1] {
      guard k < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[k] = v
      k += 1
    }
  }
  k
}
```