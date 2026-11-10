```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var k: usize[0..=16] = 0
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
        unreachable("the loop condition leaves j below 8 when i is 8")
      }
      v = b[j]
      j += 1
    }
    var emit = true
    if k > 0 {
      emit = out[k - 1] != v
    }
    if emit {
      guard k < 16 else {
        unreachable("at most one value is written per step, and there are at most 16 steps")
      }
      out[k] = v
      k += 1
    }
  }
  k
}
```