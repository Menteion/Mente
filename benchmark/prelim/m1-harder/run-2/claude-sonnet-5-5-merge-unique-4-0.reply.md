```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var k: usize[0..=16] = 0
  var last: i32 = 0
  var seen: bool = false
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i < 8 {
      if j < 8 {
        if a[i] < b[j] {
          v = a[i]
        } else {
          v = b[j]
        }
      } else {
        v = a[i]
      }
    } else {
      guard j < 8 else {
        unreachable("loop condition needs i < 8 or j < 8")
      }
      v = b[j]
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
    if i < 8 {
      if a[i] == v {
        i += 1
      }
    }
    if j < 8 {
      if b[j] == v {
        j += 1
      }
    }
  }
  k
}
```