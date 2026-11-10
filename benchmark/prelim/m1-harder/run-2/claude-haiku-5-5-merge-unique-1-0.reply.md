```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize = 0
  var j: usize = 0
  var k: usize[0..=16] = 0
  while i < 8 or j < 8 {
    val takeA: bool = if i >= 8 { false } else if j >= 8 { true } else { a[i] <= b[j] }
    val v: i32 = if takeA { a[i] } else { b[j] }
    if takeA {
      i += 1
    } else {
      j += 1
    }
    val fresh: bool = if k == 0 { true } else { out[k - 1] != v }
    if fresh {
      guard k < 16 else {
        unreachable("at most 16 distinct values can be written")
      }
      out[k] = v
      k += 1
    }
  }
  k
}
```