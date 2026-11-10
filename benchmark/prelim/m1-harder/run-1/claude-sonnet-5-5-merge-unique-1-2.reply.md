```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var work: [16]i32 = out
  var i: usize = 0
  var j: usize = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    var from_a: bool = true
    if i < 8 {
      v = a[i]
      if j < 8 {
        if b[j] < v {
          v = b[j]
          from_a = false
        }
      }
    } else if j < 8 {
      v = b[j]
      from_a = false
    }
    if from_a {
      i += 1
    } else {
      j += 1
    }
    var fresh: bool = true
    if k > 0 {
      if work[k - 1] == v {
        fresh = false
      }
    }
    if fresh and k < 16 {
      work[k] = v
      k += 1
    }
  }
  out = work
  return k
}
```