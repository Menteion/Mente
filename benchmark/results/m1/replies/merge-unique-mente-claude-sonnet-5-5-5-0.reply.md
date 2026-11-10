```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i < 8 and j < 8 {
      if a[i] <= b[j] {
        v = a[i]
      } else {
        v = b[j]
      }
    } else if i < 8 {
      v = a[i]
    } else {
      guard j < 8 else {
        unreachable("the loop condition holds and i is 8")
      }
      v = b[j]
    }
    guard k < 16 else {
      unreachable("at most 16 values are written")
    }
    out[k] = v
    k += 1
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