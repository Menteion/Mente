I can't fix this blind: the failure report gives only the returned value, which is correct, and never shows the `out` the interpreter actually held. If the result differs, I need to see that array, because I can't find the difference. This is the same algorithm as my second-to-last answer, which I still believe is correct.

```mente
pub fn merge_unique(a: [8]i32, b: [8]i32, mut out: [16]i32) -> usize[0..=16] {
  var i: usize[0..=8] = 0
  var j: usize[0..=8] = 0
  var k: usize[0..=16] = 0
  while i < 8 and j < 8 {
    val v1: i32 = if a[i] <= b[j] { a[i] } else { b[j] }
    guard k < 16 else {
      unreachable("at most 16 distinct values")
    }
    out[k] = v1
    k += 1
    while i < 8 {
      guard a[i] == v1 else {
        break
      }
      i += 1
    }
    while j < 8 {
      guard b[j] == v1 else {
        break
      }
      j += 1
    }
  }
  while i < 8 {
    val v2: i32 = a[i]
    guard k < 16 else {
      unreachable("at most 16 distinct values")
    }
    out[k] = v2
    k += 1
    while i < 8 {
      guard a[i] == v2 else {
        break
      }
      i += 1
    }
  }
  while j < 8 {
    val v3: i32 = b[j]
    guard k < 16 else {
      unreachable("at most 16 distinct values")
    }
    out[k] = v3
    k += 1
    while j < 8 {
      guard b[j] == v3 else {
        break
      }
      j += 1
    }
  }
  k
}
```