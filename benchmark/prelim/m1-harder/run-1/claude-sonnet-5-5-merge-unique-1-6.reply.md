```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var n: usize = 0
  var have_last: bool = false
  var last: i32 = 0
  while i < 8 or j < 8 {
    var v: i32 = 0
    if i < 8 {
      v = a[i]
      if j < 8 {
        if b[j] < v {
          v = b[j]
        }
      }
    } else if j < 8 {
      v = b[j]
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
    var is_new: bool = true
    if have_last {
      if last == v {
        is_new = false
      }
    }
    if is_new {
      guard n < 16 else {
        unreachable("at most 16 distinct values")
      }
      out[n] = v
      n += 1
      last = v
      have_last = true
    }
  }
  guard n <= 16 else {
    unreachable("n never exceeds 16")
  }
  n
}
```