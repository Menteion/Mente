```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i >= 8 {
      guard j < 8 else {
        unreachable("the loop condition leaves j below 8")
      }
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
    var fresh = true
    if k > 0 {
      if out[k - 1] == v {
        fresh = false
      }
    }
    if fresh {
      guard k < 16 else {
        unreachable("each output value consumes an input element, so at most 16 are written")
      }
      out[k] = v
      k += 1
    }
  }
  k
}
```