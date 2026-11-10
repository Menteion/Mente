```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
    val v: i32 = if i < 8 and j < 8 {
      if a[i] < b[j] { a[i] } else { b[j] }
    } else if i < 8 {
      a[i]
    } else {
      b[j]
    }
    guard k < 16 else {
      unreachable("each output value consumes at least one input element, so k stays below 16")
    }
    out[k] = v
    k += 1
    while i < 8 and a[i] == v {
      i += 1
    }
    while j < 8 and b[j] == v {
      j += 1
    }
  }
  k
}
```