```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var n: usize[0..=16] = 0
  while i < 8 or j < 8 {
    val v: i32 = if i < 8 {
      if j < 8 {
        if a[i] < b[j] { a[i] } else { b[j] }
      } else {
        a[i]
      }
    } else {
      if j < 8 { b[j] } else { unreachable("loop condition needs i or j below 8") }
    }
    guard n < 16 else {
      unreachable("at most 16 distinct values")
    }
    out[n] = v
    n += 1
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
  n
}
```