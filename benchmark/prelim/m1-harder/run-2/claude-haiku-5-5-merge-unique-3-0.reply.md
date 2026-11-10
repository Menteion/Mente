```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize = 0
  while i < 8 or j < 8 {
    var from_a = false
    if i < 8 {
      if j < 8 {
        if a[i] <= b[j] {
          from_a = true
        }
      } else {
        from_a = true
      }
    }
    var v: i32 = 0
    if from_a {
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
      out[k] = v
      k += 1
    }
  }
  guard k <= 16 else {
    unreachable("at most 16 values")
  }
  k
}
```