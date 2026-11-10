```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var n: usize = 0
  while i < 8 or j < 8 {
    var take_a = false
    if i < 8 {
      if j < 8 {
        take_a = a[i] <= b[j]
      } else {
        take_a = true
      }
    }
    val v: i32 = if take_a { a[i] } else { b[j] }
    if take_a {
      i += 1
    } else {
      j += 1
    }
    if n == 0 {
      out[n] = v
      n += 1
    } else {
      if out[n - 1] != v {
        out[n] = v
        n += 1
      }
    }
  }
  n
}
```