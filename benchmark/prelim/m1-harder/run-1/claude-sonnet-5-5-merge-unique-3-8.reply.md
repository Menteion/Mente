I have no new fix to offer. The function below is unchanged from my last answer, and I traced both examples by hand again. It returns 12 and 8, and `out` holds `[1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13, 15, 0, 0, 0, 0]` and `[1, 2, 3, 4, 5, 6, 7, 8, -1, -1, -1, -1, -1, -1, -1, -1]`, as the examples require. I've tried six structurally different versions, and the report is identical for every one: it shows the correct returned value and never the `out` the interpreter ended with. If you can show me that final array, I can pin down the difference.

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